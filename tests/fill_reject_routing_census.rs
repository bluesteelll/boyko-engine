//! **A value `Assets::fill` rejects still owns live device memory; discarding it is a leak.**
//!
//! `Assets::fill(handle, value)` returns `Err((AssetError, value))` when `handle` no longer
//! resolves to a `Loading`/`Failed` row — for example a handle removed after staging, or a handle
//! staged twice. The store never took the value, so for a GPU-resident asset (`MeshGpu`,
//! `TextureGpu`, neither of which implements `Drop`) the device buffers, BLAS, image and bindless
//! slot inside it are freed ONLY if the caller routes the value into its orphan teardown queue
//! (`OrphanedMeshGpu`, `OrphanedTextureGpu`). `fill` carries a `#[must_use]` saying exactly that,
//! and `let _ =` silences it. That was defect B of the 2026-09-11 latent-defects checkpoint: from
//! b5312171 (rung A3b) through d5782d43, `crates/boyko_render/src/gpu_upload.rs` read
//! `let _ = assets.fill(staged.handle, gpu);`. The defect-B fix replaced it with
//! `if let Err((_, rejected)) = assets.fill(staged.handle, gpu)`, which hands the value to
//! `GpuUpload::orphan` and so to its orphan queue.
//!
//! This is the DEVICE-FREE tripwire against that defect coming back. The behaviour itself needs a
//! real device and is gated by `crates/boyko_app/tests/asset_upload_reject_mesh_leak.rs` and
//! `asset_upload_reject_texture_leak.rs`; this gate checks the source shape that caused it, so a
//! machine without a GPU still goes red on a regression.
//!
//! # What it asserts
//!
//! 1. **No production `Assets::fill` call discards its result.** A call is any two-argument
//!    `.fill(` method call, or a three-argument `Assets::fill(` path call, in a `src/` file outside
//!    a `#[cfg(test)] mod … { … }` block. Its result is DISCARDED when the statement is `let _ =` /
//!    `let _: T =`, a bare expression statement, a `drop(`/`forget(`/`matches!(` wrapper, a chain
//!    that throws the payload away (`.ok()`, `.is_ok()`, `.is_err()`, `.unwrap_or_default()`,
//!    `.unwrap_or(())`, `.unwrap_or_else(|_|`, `.map_err(|_|`), or an `Ok(..)` / `Err(_)` pattern.
//!    Everything else — `if let Err((_, v)) =`, `match`, `let Err(..) = … else`, a binding, an
//!    argument to a routing helper, `.expect(` — is CONSUMED.
//! 2. **The census found something to check.** At least one production call site must exist. If
//!    `fill` is renamed or its only caller deleted, this gate goes RED rather than reporting a clean
//!    tree over an empty set — it must then be re-pointed, not deleted.
//! 3. **The walk is not vacuous.** A floor on production `.rs` files visited.
//! 4. **The detector reads every shape it will meet** — a positive-control table run through the
//!    SAME masking / test-module / call-parsing pipeline, including the exact line the fix replaced.
//!
//! # Why a census and not `clippy::let_underscore_must_use`
//!
//! The lint would flag this site, but it is one workspace-wide switch over every `let _ =` of every
//! `#[must_use]` value in the tree, most of them deliberate (`let _ = ctx.wait_idle();`), so turning
//! it on is red for reasons that are not this defect. And a lint that finds nothing passes: it
//! cannot fail when the call it guards has moved out of its sight, which clause 2 requires.
//!
//! # What it cannot claim
//!
//! It reads masked bytes, not a syntax tree. Known blind spots, each a false GREEN on a shape that
//! does not occur in the tree today:
//! - `let _name = assets.fill(..);` (an underscore-prefixed binding that is never used) reads as
//!   consumed;
//! - a `match` whose `Err(_) =>` arm drops the payload reads as consumed (arms are not analysed);
//! - a generic argument containing a comma inside `<…>` (`fill(h, Foo::<A, B>::new())`) miscounts
//!   the arguments, so that call is not seen as a site;
//! - `#[cfg(all(test, …))] mod` blocks are treated as production code (a false RED at worst).
//!
//! # Why this lives in the root package
//!
//! `CARGO_MANIFEST_DIR` is the repository root here, so the walk cannot point at the wrong tree —
//! the rationale `internal_docs_anchors.rs` and `ignore_reasons_census.rs` already record.

use std::path::{Path, PathBuf};

/// Directories the walk skips outright. `.claude` holds parked worktrees of other branches, which
/// `gpu_blocking_reader_census.rs` measured turning a census red in a clean checkout.
const SKIP_DIRS: &[&str] = &["target", ".git", ".claude", "graphify-out", "book", "assets", "node_modules"];

/// A file under any of these path components is test / bench / example code, not production.
const NON_PRODUCTION_COMPONENTS: &[&str] = &["tests", "benches", "examples"];

/// Floor on production `src/` files visited. The tree holds several hundred; this exists only to
/// catch a walker that stopped walking.
const MIN_PRODUCTION_FILES: usize = 300;

/// What happened to the `Result` of one `fill` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fate {
    /// The `Err` payload is dropped on the floor — the defect. The string names the shape.
    Discarded(&'static str),
    /// The result reaches code that can route the payload.
    Consumed,
}

/// One production `fill` call.
struct FillSite {
    /// Repo-relative, `/`-separated.
    file: String,
    /// 1-indexed line of the `fill` token.
    line: usize,
    /// The statement text around the call, whitespace-collapsed, for the report.
    statement: String,
    fate: Fate,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Blank `out[from..to]` to spaces, keeping newlines so line numbers survive.
fn blank(out: &mut [u8], from: usize, to: usize) {
    let end = to.min(out.len());
    for byte in &mut out[from..end] {
        if *byte != b'\n' {
            *byte = b' ';
        }
    }
}

/// Every comment and every string / char literal replaced by spaces, byte for byte. Offsets and
/// line numbers are preserved, and no `.fill(` inside prose or a literal can be read as a call,
/// nor can a comma or bracket inside a literal derail argument counting.
fn mask_comments_and_literals(src: &str) -> Vec<u8> {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        let next = b.get(i + 1).copied();

        if c == b'/' && next == Some(b'/') {
            let end = b[i..].iter().position(|&x| x == b'\n').map_or(b.len(), |p| i + p);
            blank(&mut out, i, end);
            i = end;
            continue;
        }

        if c == b'/' && next == Some(b'*') {
            let mut depth = 0i32;
            let mut j = i;
            while j < b.len() {
                if b[j] == b'/' && b.get(j + 1) == Some(&b'*') {
                    depth += 1;
                    j += 2;
                } else if b[j] == b'*' && b.get(j + 1) == Some(&b'/') {
                    depth -= 1;
                    j += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    j += 1;
                }
            }
            blank(&mut out, i, j);
            i = j;
            continue;
        }

        // Raw strings: r"…", r#"…"#, br"…".
        let raw_start = (c == b'r' || (c == b'b' && next == Some(b'r'))) && (i == 0 || !is_ident(b[i - 1]));
        if raw_start {
            let mut j = if c == b'b' { i + 2 } else { i + 1 };
            let mut hashes = 0usize;
            while b.get(j) == Some(&b'#') {
                hashes += 1;
                j += 1;
            }
            if b.get(j) == Some(&b'"') {
                let mut k = j + 1;
                let end = loop {
                    if k >= b.len() {
                        break b.len();
                    }
                    if b[k] == b'"' && (0..hashes).all(|h| b.get(k + 1 + h) == Some(&b'#')) {
                        break k + 1 + hashes;
                    }
                    k += 1;
                };
                blank(&mut out, i, end);
                i = end;
                continue;
            }
        }

        if c == b'"' {
            let mut k = i + 1;
            while k < b.len() && b[k] != b'"' {
                if b[k] == b'\\' {
                    k += 1;
                }
                k += 1;
            }
            let end = (k + 1).min(b.len());
            blank(&mut out, i, end);
            i = end;
            continue;
        }

        // A char literal, as opposed to a lifetime: `'x'`, `'→'`, `'\n'`, `'\''`, `'\u{1F600}'`.
        if c == b'\'' {
            if next == Some(b'\\') {
                if b.get(i + 2) == Some(&b'\'') && b.get(i + 3) == Some(&b'\'') {
                    blank(&mut out, i, i + 4);
                    i += 4;
                    continue;
                }
                if let Some(p) = b.get(i + 2..).and_then(|rest| rest.iter().take(12).position(|&x| x == b'\'')) {
                    let end = i + 2 + p + 1;
                    blank(&mut out, i, end);
                    i = end;
                    continue;
                }
            } else if let Some(ch) = src.get(i + 1..).and_then(|rest| rest.chars().next()) {
                let close = i + 1 + ch.len_utf8();
                if b.get(close) == Some(&b'\'') {
                    blank(&mut out, i, close + 1);
                    i = close + 1;
                    continue;
                }
            }
        }

        i += 1;
    }
    out
}

/// Index of the byte that closes the bracket opened at `open`, counting `()`, `[]` and `{}`.
fn matching_close(code: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (k, &byte) in code.iter().enumerate().skip(open) {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(k);
                }
            }
            _ => {}
        }
    }
    None
}

/// Blank every `#[cfg(test)] … mod name { … }` block so its calls are not counted as production.
fn blank_test_modules(code: &mut [u8]) {
    const ATTR: &[u8] = b"#[cfg(test)]";
    let mut from = 0usize;
    while let Some(pos) = code[from..].windows(ATTR.len()).position(|w| w == ATTR) {
        let attr_start = from + pos;
        let mut k = attr_start + ATTR.len();
        // Skip whitespace and any further attributes between `#[cfg(test)]` and the item.
        loop {
            while k < code.len() && code[k].is_ascii_whitespace() {
                k += 1;
            }
            if code.get(k) == Some(&b'#') && code.get(k + 1) == Some(&b'[') {
                match matching_close(code, k + 1) {
                    Some(close) => k = close + 1,
                    None => break,
                }
            } else {
                break;
            }
        }
        let rest = &code[k..];
        let item_head_len = rest.iter().position(|&x| x == b'{' || x == b';').unwrap_or(rest.len());
        let head = String::from_utf8_lossy(&rest[..item_head_len]).to_string();
        let words: Vec<&str> = head.split_whitespace().collect();
        let is_inline_mod = rest.get(item_head_len) == Some(&b'{')
            && words.contains(&"mod")
            && words.first().is_some_and(|w| *w == "mod" || w.starts_with("pub"));
        if is_inline_mod && let Some(close) = matching_close(code, k + item_head_len) {
            blank(code, attr_start, close + 1);
            from = close + 1;
            continue;
        }
        from = attr_start + ATTR.len();
    }
}

/// `(index of the close paren, argument count)` for the call whose `(` is at `open`.
fn call_arguments(code: &[u8], open: usize) -> Option<(usize, usize)> {
    let close = matching_close(code, open)?;
    let mut depth = 0i32;
    let mut commas = 0usize;
    let mut last_significant: Option<u8> = None;
    for &byte in &code[open + 1..close] {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => commas += 1,
            _ => {}
        }
        if !byte.is_ascii_whitespace() {
            last_significant = Some(byte);
        }
    }
    let args = match last_significant {
        None => 0,
        Some(b',') => commas,
        Some(_) => commas + 1,
    };
    Some((close, args))
}

/// Start of the statement (or enclosing call argument list) containing byte `at`, and the byte
/// that bounds it. A `,` is deliberately NOT a boundary: it also separates generic arguments
/// (`let _: Result<(), _> =`), and an enclosing call is already found through its `(`.
fn statement_start(code: &[u8], at: usize) -> (usize, u8) {
    let mut depth = 0i32;
    let mut k = at;
    while k > 0 {
        k -= 1;
        match code[k] {
            b')' | b']' => depth += 1,
            b'(' | b'[' => {
                if depth == 0 {
                    return (k + 1, code[k]);
                }
                depth -= 1;
            }
            b'{' | b'}' | b';' if depth == 0 => return (k + 1, code[k]),
            b'>' if depth == 0 && k > 0 && code[k - 1] == b'=' => return (k + 1, b'>'),
            _ => {}
        }
    }
    (0, 0)
}

/// Text from just past the call's `)` to the end of the statement, including its terminator.
fn statement_rest(code: &[u8], close: usize) -> (usize, String) {
    let start = close + 1;
    let mut depth = 0i32;
    let mut k = start;
    while k < code.len() {
        match code[k] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            b'{' | b'}' | b';' | b',' if depth == 0 => {
                k += 1;
                break;
            }
            _ => {}
        }
        k += 1;
    }
    (k, String::from_utf8_lossy(&code[start..k.min(code.len())]).to_string())
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The rules. `prefix` is the statement text before the receiver's `.fill`, `bound` the byte that
/// opened the statement, `callee` the text before `bound` (for a wrapping call), `rest` the text
/// after the call's `)`.
fn classify(prefix: &str, bound: u8, callee: &str, rest: &str) -> Fate {
    let p = prefix.trim();
    let sp = squash(p);
    let sr = squash(rest);

    if let Some(after_let) = p.strip_prefix("let")
        && after_let.starts_with(char::is_whitespace)
        && let Some(after_underscore) = after_let.trim_start().strip_prefix('_')
    {
        let tail = after_underscore.trim_start();
        if tail.starts_with(':') || (tail.starts_with('=') && !tail.starts_with("==")) {
            return Fate::Discarded("`let _ =` drops the Err value");
        }
    }

    const DROPPING_CHAINS: &[&str] = &[
        ".ok()",
        ".is_ok()",
        ".is_err()",
        ".unwrap_or_default()",
        ".unwrap_or(())",
        ".err().is_some()",
        ".err().is_none()",
        ".unwrap_or_else(|_|",
        ".map_err(|_|",
    ];
    if DROPPING_CHAINS.iter().any(|chain| sr.starts_with(chain)) {
        return Fate::Discarded("a chained call throws the Err payload away");
    }

    if bound == b'(' {
        let callee = callee.trim_end();
        let wrapper_ends = |name: &str| {
            callee.ends_with(name)
                && callee[..callee.len() - name.len()].bytes().next_back().is_none_or(|b| !is_ident(b))
        };
        if wrapper_ends("drop") || wrapper_ends("forget") || callee.ends_with("matches!") {
            return Fate::Discarded("a drop/forget/matches! wrapper swallows the Err value");
        }
    }

    if sp.starts_with("ifletOk(")
        || sp.starts_with("whileletOk(")
        || sp.starts_with("letOk(")
        || sp.contains("Err(_)=")
        || sp.contains("Err(..)=")
        || sp.contains("Err((_,_))=")
    {
        return Fate::Discarded("the pattern binds no Err payload");
    }

    let starts_statement = matches!(bound, b';' | b'{' | b'}' | 0);
    let keyword_led = ["let ", "return", "match ", "if ", "while ", "break", "else"]
        .iter()
        .any(|kw| p.starts_with(kw));
    if starts_statement && sr == ";" && !p.contains('=') && !keyword_led {
        return Fate::Discarded("a bare expression statement drops the Result (only a warning)");
    }

    Fate::Consumed
}

/// Every production `fill` call in one file's source.
fn scan_source(rel: &str, src: &str) -> Vec<FillSite> {
    let mut code = mask_comments_and_literals(src);
    blank_test_modules(&mut code);

    let mut sites = Vec::new();
    let needle = b"fill";
    let mut from = 0usize;
    while let Some(pos) = code[from..].windows(needle.len()).position(|w| w == needle) {
        let at = from + pos;
        from = at + needle.len();

        let after = at + needle.len();
        if code.get(after).is_some_and(|&b| is_ident(b)) {
            continue;
        }
        let mut open = after;
        while open < code.len() && code[open].is_ascii_whitespace() {
            open += 1;
        }
        if code.get(open) != Some(&b'(') {
            continue;
        }

        // `.fill(` method form, or `Assets…::fill(` path form.
        let mut before = at;
        while before > 0 && code[before - 1].is_ascii_whitespace() {
            before -= 1;
        }
        let (anchor, wanted_args) = if before > 0 && code[before - 1] == b'.' {
            (before - 1, 2)
        } else if before >= 2 && &code[before - 2..before] == b"::" {
            let mut path_start = before - 2;
            while path_start > 0 {
                let b = code[path_start - 1];
                if is_ident(b) || matches!(b, b':' | b'<' | b'>' | b',') {
                    path_start -= 1;
                } else {
                    break;
                }
            }
            let path = String::from_utf8_lossy(&code[path_start..before]).to_string();
            if !path.contains("Assets") {
                continue;
            }
            (path_start, 3)
        } else {
            continue;
        };

        let Some((close, args)) = call_arguments(&code, open) else { continue };
        if args != wanted_args {
            continue;
        }

        let (stmt_start, bound) = statement_start(&code, anchor);
        let prefix = String::from_utf8_lossy(&code[stmt_start..anchor]).to_string();
        let callee_start = stmt_start.saturating_sub(1);
        let callee_from = code[..callee_start].iter().rposition(|&b| !(is_ident(b) || b == b':' || b == b'!')).map_or(0, |p| p + 1);
        let callee = String::from_utf8_lossy(&code[callee_from..callee_start]).to_string();
        let (rest_end, rest) = statement_rest(&code, close);

        let fate = classify(&prefix, bound, &callee, &rest);
        let line = code[..at].iter().filter(|&&b| b == b'\n').count() + 1;
        let statement = collapse(&String::from_utf8_lossy(&code[stmt_start..rest_end]));
        sites.push(FillSite { file: rel.to_string(), line, statement, fate });
    }
    sites
}

fn collect_production_rs(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if SKIP_DIRS.contains(&name.as_ref()) {
                continue;
            }
            collect_production_rs(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            let components: Vec<String> =
                rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
            let in_src = components.iter().any(|c| c == "src");
            let non_production = components.iter().any(|c| NON_PRODUCTION_COMPONENTS.contains(&c.as_str()));
            if in_src && !non_production {
                out.push(rel);
            }
        }
    }
}

/// Every production `fill` site in the tree, plus the number of production files walked.
fn census() -> (Vec<FillSite>, usize) {
    let root = repo_root();
    let mut files = Vec::new();
    collect_production_rs(&root, &root, &mut files);
    let mut sites = Vec::new();
    for rel in &files {
        let Ok(src) = std::fs::read_to_string(root.join(rel)) else { continue };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        sites.extend(scan_source(&rel_str, &src));
    }
    (sites, files.len())
}

#[test]
fn every_rejected_fill_value_is_routed_not_discarded() {
    let (sites, file_count) = census();

    assert!(
        file_count >= MIN_PRODUCTION_FILES,
        "the walk visited only {file_count} production src/ files — the walker is broken, not the tree"
    );
    assert!(
        !sites.is_empty(),
        "found NO production `Assets::fill` call in {file_count} src/ files. Either `fill` was renamed \
         or its only caller (the GPU upload drain) moved out of the census's sight — re-point this \
         gate at the new call; an empty set is not a clean tree"
    );

    println!("[fill census] {} production fill site(s) in {file_count} src/ files:", sites.len());
    for site in &sites {
        println!("  {}:{} {:?} `{}`", site.file, site.line, site.fate, site.statement);
    }

    let discarded: Vec<String> = sites
        .iter()
        .filter_map(|s| match s.fate {
            Fate::Discarded(why) => Some(format!("{}:{} ({why}): `{}`", s.file, s.line, s.statement)),
            Fate::Consumed => None,
        })
        .collect();

    assert!(
        discarded.is_empty(),
        "REGRESSION (defect B, a rejected GPU upload leaks device memory): these production \
         `Assets::fill` calls discard the Err value, which still owns live device buffers / a BLAS / an \
         image / a bindless slot with no Drop to free them:\n  {}\n\n\
         WHAT TO DO — route the rejected value into its orphan teardown queue, as the defect-B fix does:\n\
         \x20   if let Err((_, rejected)) = assets.fill(staged.handle, gpu) {{\n\
         \x20       A::orphan(rejected, orphans, retire_frame);\n\
         \x20   }}\n\
         (`GpuUpload::orphan` pushes onto `OrphanedMeshGpu` / `OrphanedTextureGpu`, which the runner \
         inserts, drains behind the fence gate every frame and force-drains at shutdown.)",
        discarded.join("\n  ")
    );
}

/// The detector's own positive control: each shape classified by hand, run through the same
/// masking, test-module blanking and call parsing the census uses. The floors above catch a
/// detector that found nothing; only this table catches one that reads `let _ =` as consumed.
#[test]
fn the_detector_classifies_every_shape_it_will_meet() {
    #[derive(Debug, PartialEq)]
    enum Want {
        Discarded,
        Consumed,
        NoSite,
    }
    use Want::{Consumed, Discarded, NoSite};

    let cases: &[(&str, Want)] = &[
        // The line d5782d43 shipped, verbatim — the defect the fix removed.
        ("let _ = assets.fill(staged.handle, gpu);", Discarded),
        ("let _: Result<(), _> = assets.fill(h, v);", Discarded),
        ("let _ =\n        self.assets\n            .fill(staged.handle, gpu);", Discarded),
        ("assets.fill(h, v);", Discarded),
        ("assets.fill(h, v).ok();", Discarded),
        ("if assets.fill(h, v).is_err() { misses += 1; }", Discarded),
        ("drop(assets.fill(h, v));", Discarded),
        ("std::mem::forget(assets.fill(h, v));", Discarded),
        ("if let Ok(()) = assets.fill(h, v) { hits += 1; }", Discarded),
        ("if let Err(_) = assets.fill(h, v) { misses += 1; }", Discarded),
        ("assets.fill(h, v).unwrap_or_else(|_| ());", Discarded),
        ("Assets::fill(&mut assets, h, v).ok();", Discarded),
        ("let c = '('; let _ = assets.fill(h, v);", Discarded),
        // The shapes a correct routing takes.
        (
            "if let Err((_, rejected)) = assets.fill(staged.handle, gpu) { orphans.push(rejected, epoch + RETIRE_DELAY); }",
            Consumed,
        ),
        ("match assets.fill(h, v) { Ok(()) => {} Err((_, v)) => orphans.push(v, rf), }", Consumed),
        ("let Err((_, v)) = assets.fill(h, g) else { continue; };", Consumed),
        ("assets.fill(h, v).unwrap_or_else(|(_, v)| orphans.push(v, rf));", Consumed),
        ("route_rejected(assets.fill(h, v), &mut orphans, rf);", Consumed),
        ("let result = assets.fill(h, v);", Consumed),
        ("assets.fill(h, v).expect(\"invariant: a reserved row\");", Consumed),
        // Not a site at all.
        ("buf[..n].fill(0);", NoSite),
        ("self.asleep.fill(false);", NoSite),
        ("// let _ = assets.fill(h, v);", NoSite),
        ("let s = \"let _ = assets.fill(h, v);\";", NoSite),
        ("let s = r#\"let _ = assets.fill(h, v);\"#;", NoSite),
        ("/* let _ = assets.fill(h, v); */", NoSite),
        ("let v = VecLike::fill(a, b);", NoSite),
    ];

    for (body, want) in cases {
        let src = format!("fn under_test() {{\n    {body}\n}}\n");
        let sites = scan_source("case.rs", &src);
        let got = match sites.as_slice() {
            [] => NoSite,
            [one] => match one.fate {
                Fate::Discarded(_) => Discarded,
                Fate::Consumed => Consumed,
            },
            many => panic!("case {body:?} produced {} sites, expected at most one", many.len()),
        };
        assert_eq!(&got, want, "the detector misread {body:?}");
    }
}

/// A `#[cfg(test)] mod` block is not production code: a discarded fill inside one is invisible to
/// the census, and one outside it in the same file is still seen.
#[test]
fn a_test_module_is_not_production_code() {
    let src = "\
pub fn shipped(assets: &mut A) {
    let _ = assets.fill(h, v);
}

#[cfg(test)]
#[allow(clippy::disallowed_types)]
mod tests {
    fn helper() { let _ = assets.fill(h, v); }
    fn nested() { if x { let _ = assets.fill(h, v); } }
}
";
    let sites = scan_source("case.rs", src);
    assert_eq!(sites.len(), 1, "exactly the shipped call is a production site, got {}", sites.len());
    assert_eq!(sites[0].line, 2, "the production site is the one on line 2");
    assert!(matches!(sites[0].fate, Fate::Discarded(_)), "and it is read as discarded");
}
