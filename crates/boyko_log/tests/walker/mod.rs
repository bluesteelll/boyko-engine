//! The ONE walker, shared by `code_registry.rs` and `print_census.rs`.
//!
//! # Why it is a module and not a copy
//!
//! The corpus specifies it in as many words: *"it is the **same walker** that backs
//! `print_census.rs`, so its `#[cfg(test)]`-region rule and its `src/bin/` exclusion are written
//! once and exercised by two tests."* Two copies of a walker are two answers to one question --
//! "is this file production code?" -- and the day they disagree, one gate reds on a file the other
//! excuses, with no way to tell which is right from either failure message.
//!
//! It lives in `tests/walker/` rather than `tests/walker.rs` because cargo builds every `tests/*.rs`
//! as its own test target; a subdirectory module is compiled INTO its consumers instead.
//!
//! # `allow(dead_code)`, with its reason
//!
//! The two consumers use different subsets -- `print_census.rs` needs neither `doc_files` nor the
//! LIT stream, `code_registry.rs` needs both -- and each binary compiles the whole module. Without
//! this, `-D warnings` would red on functions that ARE used, by the other test.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Repository root, from this crate's manifest directory.
pub(crate) fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<pkg> is two levels below the repo root")
        .to_path_buf()
}

/// Every `.rs` file under `crates/` and `src/`, sorted.
pub(crate) fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for base in ["crates", "src"] {
        collect(&root.join(base), "rs", &mut out);
    }
    out.sort();
    out
}

pub(crate) fn collect(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            // `target/` is build output, not source, and it is enormous.
            if p.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect(&p, ext, out);
        } else if p.extension().is_some_and(|x| x == ext) {
            out.push(p);
        }
    }
}

/// The documentation corpus check 4 scans.
///
/// An **explicit list**, not a glob over `docs/`. Two exclusions carry reasons:
///
/// - **`docs/archive/**` is out.** It holds completed-phase planning documents that must never be
///   edited again, and it contributes three codes (`B9000`, `B9003`, `W9003`) that exist in no
///   source file and no current document. Seeding those as `Pending` would promise emitters that
///   will never arrive.
/// - **`docs/*.md` at top level is out at THIS rung**, and that is a real weakening stated rather
///   than hidden. The two superseded monoliths (`docs/LOGGING-SYSTEM-PLAN.md`,
///   `docs/PROFILING-SYSTEM-PLAN.md`) carry 65 prefixed literals for codes this registry does not
///   yet contain — they are the documents the corpus was carved *from*, and they are slated for
///   retirement. Arming check 4 over them today reds on documents nobody will fix. The corpus
///   directory obeys the bare-number rule and is scanned in full. When the monoliths are retired,
///   `docs/*.md` joins this list, and that is a one-line change.
pub(crate) fn doc_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect(&root.join("docs").join("diagnostics"), "md", &mut out);
    out.sort();
    out
}

/// One file, split into the streams the checks consume.
pub(crate) struct Streams {
    /// Source with comments, literals and test-only files removed.
    pub(crate) code: String,
    /// The contents of string and char literals only.
    pub(crate) lit: String,
}

/// Split one Rust source file into CODE and LIT.
///
/// Handles `//`/`///`/`//!` line comments, `/* */` block comments (nested, as Rust allows),
/// `"…"` strings with backslash escapes, `r#"…"#` raw strings, and `'…'` char literals. Lifetime
/// ticks (`'a`) are distinguished from char literals by looking ahead for the closing quote.
pub(crate) fn split_streams(src: &str) -> Streams {
    let b = src.as_bytes();
    let mut code = String::with_capacity(src.len());
    let mut lit = String::new();
    let mut i = 0;

    while i < b.len() {
        // Line comment.
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment, nested.
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            let mut depth = 1usize;
            i += 2;
            while i < b.len() && depth > 0 {
                if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if b[i] == b'*' && i + 1 < b.len() && b[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            code.push(' ');
            continue;
        }
        // Raw string: r"…", r#"…"#, r##"…"##
        if b[i] == b'r' {
            let mut j = i + 1;
            let mut hashes = 0usize;
            while j < b.len() && b[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < b.len() && b[j] == b'"' {
                j += 1;
                let start = j;
                loop {
                    if j >= b.len() {
                        break;
                    }
                    if b[j] == b'"' {
                        let mut k = j + 1;
                        let mut seen = 0usize;
                        while k < b.len() && b[k] == b'#' && seen < hashes {
                            seen += 1;
                            k += 1;
                        }
                        if seen == hashes {
                            lit.push_str(&src[start..j]);
                            lit.push('\n');
                            i = k;
                            break;
                        }
                    }
                    j += 1;
                }
                if j >= b.len() {
                    i = b.len();
                }
                code.push(' ');
                continue;
            }
        }
        // Normal string.
        if b[i] == b'"' {
            i += 1;
            let start = i;
            while i < b.len() {
                if b[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if b[i] == b'"' {
                    break;
                }
                i += 1;
            }
            lit.push_str(&src[start..i.min(b.len())]);
            lit.push('\n');
            i = (i + 1).min(b.len());
            code.push(' ');
            continue;
        }
        // Char literal, distinguished from a lifetime by the closing quote.
        if b[i] == b'\'' {
            let close = if i + 2 < b.len() && b[i + 1] == b'\\' {
                (i + 2..b.len().min(i + 8)).find(|&k| b[k] == b'\'')
            } else if i + 2 < b.len() && b[i + 2] == b'\'' {
                Some(i + 2)
            } else {
                None
            };
            if let Some(k) = close {
                lit.push_str(&src[i + 1..k]);
                lit.push('\n');
                i = k + 1;
                code.push(' ');
                continue;
            }
        }
        code.push(b[i] as char);
        i += 1;
    }

    Streams { code, lit }
}

/// [`split_streams`] plus the `#[cfg(test)]` region rule — *"what is PRODUCTION code?"*.
///
/// # Why this is a SEPARATE step, learnt by breaking it *(L8c)*
///
/// The two questions look like one and are not:
///
/// * *"is this text production code?"* — what `print_census.rs` and the orphan checks ask. Comments,
///   literals **and** in-`src` test regions are all out.
/// * *"does a TEST name this code?"* — what `code_registry.rs`'s check 5 asks. Comments are out
///   (a sentence about a code is not an observation of it), literals are IN (a frozen
///   `"boyko-E…"` in a `#[should_panic]` is a real one), and test regions are the **subject**.
///
/// Folding the region rule into the split answered the second question with the first. For a file
/// that is not wholly test-only, check 5 takes the tail from its first `#[cfg(test)]` — a tail that
/// *begins* with the attribute — so the strip deleted all of it. One run reported **23** Live codes
/// as unobserved, including every observer written by the rung that added the rule.
pub(crate) fn production_code(src: &str) -> String {
    strip_cfg_test_regions(&split_streams(src).code)
}

/// Blank out every `#[cfg(test)]`-gated item from an already-split CODE stream.
///
/// # The specification said this and nothing did it *(measured at L8c)*
///
/// The corpus defines CODE as *"source text with `//`/`///`/`//!` line comments, `/* */` block
/// comments, string/char literals **and `#[cfg(test)]` regions** removed"*. The first three were
/// implemented; the fourth was not, and the gap was invisible for five rungs because the only
/// consumers were checks that look for a code IDENTIFIER — and an identifier inside an in-`src`
/// test module is a use like any other, so nothing reddened.
///
/// `print_census.rs` is the first consumer for which the gap is a **false positive**: it found
/// `boyko_rhi_vulkan/src/device.rs` and `boyko_ui/src/layout.rs` printing, and both prints are
/// inside `#[cfg(test)] mod tests`.
///
/// # Why brace matching here is SOUNDER than the precedent it follows
///
/// `scripts/check_hotpath_exceptions.py` does the same job and documents its own weakness: *"Brace
/// counting is naive about braces inside strings and comments."* This runs on the CODE stream,
/// where strings and comments have **already been removed** — so that failure mode is structurally
/// absent rather than accepted. The residual weakness is the honest one: an unbalanced brace
/// (impossible in a file that compiles) would over-reach to end of file.
///
/// # Why the whole gated ITEM and not just `mod`
///
/// `#[cfg(test)]` also gates free functions and `use` lines that exist only to serve tests —
/// `boyko_ui/src/layout.rs` has four before its test module. A rule that only understood `mod`
/// would leave a `#[cfg(test)] fn` printing in the census, which is the same false positive one
/// level down. A declaration (`#[cfg(test)] mod NAME;`, the CROSS-FILE case) ends at its `;` and
/// consumes nothing — that case belongs to [`test_only_files`], and consuming the rest of the file
/// here would silently delete the production code after it.
fn strip_cfg_test_regions(code: &str) -> String {
    const GATES: [&str; 3] = ["#[cfg(test)]", "#[cfg(all(test", "#[cfg(any(test"];
    let mut out = code.to_string();
    // Repeatedly find the FIRST surviving gate and blank its item. Iterating from the front each
    // time keeps nested gates correct: an inner one inside an outer one is already blanked.
    while let Some(at) = GATES.iter().filter_map(|g| out.find(g)).min() {
        let b = out.as_bytes();
        // Scan for whichever comes first: the item's opening brace, or the `;` of a declaration.
        let mut i = at;
        let mut open = None;
        while i < b.len() {
            match b[i] {
                b'{' => {
                    open = Some(i);
                    break;
                }
                b';' => break,
                _ => i += 1,
            }
        }
        let end = match open {
            None => i.min(b.len()), // a declaration: blank the attribute only.
            Some(o) => {
                let mut depth = 0i32;
                let mut k = o;
                while k < b.len() {
                    if b[k] == b'{' {
                        depth += 1;
                    } else if b[k] == b'}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    k += 1;
                }
                (k + 1).min(b.len())
            }
        };
        // Blank rather than delete, so byte offsets a caller may hold stay meaningful and so the
        // loop cannot re-find the gate it just consumed.
        let blanked: String = out[at..end].chars().map(|c| if c == '\n' { '\n' } else { ' ' }).collect();
        out.replace_range(at..end, &blanked);
    }
    out
}

/// Files that contribute nothing to CODE.
///
/// Three classes, and the second is the one a within-file rule misses:
///
/// 1. **`tests/` and `benches/`** under a crate. Wholly test code by construction; no attribute
///    marks them and none is needed. This crate's own gate fixtures declare `LogTarget` impls that
///    check 7 would otherwise report as hand-written — a legitimate probe failing a gate that is
///    supposed to catch illegitimate ones.
/// 2. **Files reached by a `#[cfg(test)]`-gated `mod` declaration in ANOTHER file.** Measured in
///    this tree: 7 such files exist (`boyko_sdf_math/src/brick/tests.rs`,
///    `boyko_physics/src/solver/colored_tests.rs`, …) and a within-file rule classifies all of
///    them as production. This is the cross-file pre-pass.
/// 3. **`src/bin/`** — CLI entry points, which print by design.
pub(crate) fn test_only_files(root: &Path, files: &[PathBuf]) -> BTreeSet<PathBuf> {
    let mut marked = BTreeSet::new();

    for f in files {
        let s = f.to_string_lossy().replace('\\', "/");
        // `src/main.rs` is NOT excluded here, and the attempt to add it is worth recording
        // *(L8c)*. The argument looked sound — `src/bin/` is excluded because "CLI entry points
        // print by design", and a crate's `[[bin]]` root is a CLI entry point wherever cargo found
        // it. It was tried, and `code_registry.rs`'s check 3a immediately reddened on `E3001`:
        // `boyko_demo/src/main.rs` is a `[[bin]]` root AND the only source file of its crate, so
        // excluding it deleted a real production emitter from CODE and moved it into the TEST
        // corpus, failing checks 3a and 5 together.
        //
        // There is no structural property separating "a `main` that prints because printing is its
        // product" from "a `main` that is an application". So the two that genuinely print by
        // design take rows in `print_allowlist.txt` — where the distinction is a stated reason
        // rather than a guess made by a path pattern.
        if s.contains("/tests/") || s.contains("/benches/") || s.contains("/src/bin/") {
            marked.insert(f.clone());
        }
    }

    // Cross-file pre-pass: `#[cfg(test)]` [`#[path = "…"]`] `mod NAME;`
    for f in files {
        let Ok(src) = std::fs::read_to_string(f) else { continue };
        let lines: Vec<&str> = src.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let t = line.trim();
            if !(t.starts_with("#[cfg(test)]")
                || t.starts_with("#[cfg(all(test")
                || t.starts_with("#[cfg(any(test"))
            {
                continue;
            }
            let mut path_override: Option<String> = None;
            for probe in lines.iter().take((i + 4).min(lines.len())).skip(i + 1) {
                let p = probe.trim();
                if p.is_empty() {
                    continue;
                }
                if let Some(rest) = p.strip_prefix("#[path = \"") {
                    if let Some(end) = rest.find('"') {
                        path_override = Some(rest[..end].to_string());
                    }
                    continue;
                }
                if p.starts_with("#[") {
                    continue;
                }
                // `mod NAME;` -- a declaration, not `mod NAME {`.
                let decl = p.strip_prefix("pub ").unwrap_or(p);
                if let Some(rest) = decl.strip_prefix("mod ")
                    && let Some(name) = rest.strip_suffix(';')
                {
                    let dir = f.parent().unwrap_or(root);
                    let name = name.trim();
                    // Three resolutions, and the third is the one a naive rule misses. A module
                    // declared in `src/brick.rs` lives in `src/brick/`, not in `src/` -- the
                    // 2018-edition non-`mod.rs` form. Omitting it left
                    // `boyko_sdf_math/src/brick/tests.rs` classified as production, which is one
                    // of the very files the cross-file rule exists to catch.
                    let stem_dir = f.file_stem().map(|s| dir.join(s));
                    let candidates = match &path_override {
                        Some(rel) => vec![dir.join(rel)],
                        None => {
                            let mut v = vec![
                                dir.join(format!("{name}.rs")),
                                dir.join(name).join("mod.rs"),
                            ];
                            if let Some(sd) = stem_dir {
                                v.push(sd.join(format!("{name}.rs")));
                                v.push(sd.join(name).join("mod.rs"));
                            }
                            v
                        }
                    };
                    for c in candidates {
                        if c.is_file() {
                            marked.insert(c);
                        }
                    }
                }
                break;
            }
        }
    }

    marked
}
