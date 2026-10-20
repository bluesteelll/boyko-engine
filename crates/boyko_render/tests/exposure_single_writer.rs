//! Render wave PX1a test 7 (`plan/H4-PX1a-hdr-lit.md` Tests 7, rev 2 arch W2; `px1a/cut.md` C2, C8):
//! **the single-writer census of the pre-exposure.**
//!
//! After PX1a every consumer of the scene's exposure reads `ResolvedPost::pre_exposure`, which one
//! system writes; `LightingConfig::exposure` is demoted to an INPUT of that system. Over the
//! non-test source of every crate (`crates/*/src`, `#[cfg(test)]` items removed, comments
//! stripped) this census pins:
//!
//! 1. **`LightingConfig::exposure` is read only by the post policy** — every `.exposure` field read
//!    lies in `boyko_render/src/post/mod.rs`'s [`READ_EXPOSURE_IN`] functions (or is allow-listed
//!    by name in [`EXPOSURE_READ_ALLOWED`] with its reason). `LightingConfig` is the only type with
//!    an `exposure` field, so a field read IS a config read (pinned too).
//! 2. **`ResolvedPost` has one writer** — `resolve_post_policy` (`ResMut<ResolvedPost>`), plus the
//!    plugin's boot insert.
//! 3. **`ResolvedTaa` has one writer** — `resolve_taa_policy`, plus `AaPlugin`'s boot insert.

use std::fs;
use std::path::{Path, PathBuf};

/// The functions of `post/mod.rs` allowed to read `LightingConfig::exposure`.
const READ_EXPOSURE_IN: &[&str] = &["from_config", "next"];

/// Other `.exposure` reads, by `(path suffix, fn, reason)`. Empty: the boot placeholder light
/// header (`gpu_scene/mod.rs`) takes `ResolvedPost::default()`'s pre-exposure, not a config read.
const EXPOSURE_READ_ALLOWED: &[(&str, &str, &str)] = &[];

/// One sanctioned writer: `(path, fn, token)`.
type Writer = (&'static str, &'static str, &'static str);

/// The sanctioned writers: `(path suffix, fn, token)` — a `ResMut<T>` parameter of the single
/// writer system, or the plugin's boot `insert_resource(T::default())`.
const WRITERS: &[(&str, &[Writer])] = &[
    (
        "ResolvedPost",
        &[
            ("crates/boyko_render/src/post/mod.rs", "resolve_post_policy", "ResMut<ResolvedPost>"),
            ("crates/boyko_render/src/post/mod.rs", "build", "insert_resource(ResolvedPost::default())"),
        ],
    ),
    (
        "ResolvedTaa",
        &[
            ("crates/boyko_render/src/taa_config.rs", "resolve_taa_policy", "ResMut<ResolvedTaa>"),
            ("crates/boyko_render/src/aa_plugin.rs", "build", "insert_resource(ResolvedTaa::default())"),
        ],
    ),
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").canonicalize().expect("invariant: the root exists")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.map(|e| e.expect("invariant: directory entry is readable").path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Every non-test `src/` Rust file of every workspace crate, as `(relative path, text)`.
fn src_files() -> Vec<(String, String)> {
    let root = workspace_root();
    let mut files = Vec::new();
    let crates = fs::read_dir(root.join("crates")).expect("invariant: crates/ is readable");
    let mut dirs: Vec<PathBuf> = crates.map(|e| e.expect("invariant: entry").path().join("src")).collect();
    dirs.sort();
    for d in dirs {
        rust_files(&d, &mut files);
    }
    files
        .into_iter()
        .map(|p| {
            let rel = p.strip_prefix(&root).expect("invariant: under the root").to_string_lossy().replace('\\', "/");
            let text = fs::read_to_string(&p).unwrap_or_else(|e| panic!("invariant: {rel} is UTF-8: {e}"));
            (rel, text)
        })
        .collect()
}

/// `text` with `//` comments blanked and every `#[cfg(test)]` item removed (lines kept).
fn non_test_code(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip_depth: Option<i32> = None;
    let mut pending = false;
    for line in text.lines() {
        let code = line.split("//").next().unwrap_or("");
        if let Some(d) = skip_depth.as_mut() {
            *d += code.matches('{').count() as i32 - code.matches('}').count() as i32;
            if *d <= 0 {
                skip_depth = None;
            }
            out.push(String::new());
            continue;
        }
        if code.trim() == "#[cfg(test)]" {
            pending = true;
            out.push(String::new());
            continue;
        }
        if pending && code.contains('{') {
            pending = false;
            let d = code.matches('{').count() as i32 - code.matches('}').count() as i32;
            if d > 0 {
                skip_depth = Some(d);
            }
            out.push(String::new());
            continue;
        }
        if pending && code.trim_end().ends_with(';') {
            pending = false;
            out.push(String::new());
            continue;
        }
        out.push(code.to_string());
    }
    out
}

/// For each line, the `fn` it belongs to: the fn whose header is pending (its parameters span
/// the line) or the innermost fn whose body is open; `<module>` outside every fn.
fn fn_of_lines(lines: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.len());
    let mut stack: Vec<(String, i32)> = Vec::new();
    let mut pending: Option<String> = None;
    let mut depth = 0i32;
    for l in lines {
        let b = l.as_bytes();
        let mut i = 0;
        while i < b.len() {
            let at_word = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
            if at_word && b[i..].starts_with(b"fn ") {
                let name: String =
                    l[i + 3..].trim_start().chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                if !name.is_empty() {
                    pending = Some(name);
                }
                i += 3;
                continue;
            }
            match b[i] {
                b'{' => {
                    depth += 1;
                    if let Some(name) = pending.take() {
                        stack.push((name, depth));
                    }
                }
                b'}' => {
                    if stack.last().is_some_and(|(_, d)| *d == depth) {
                        stack.pop();
                    }
                    depth -= 1;
                }
                b';' => pending = None,
                _ => {}
            }
            i += 1;
        }
        let here = pending.clone().or_else(|| stack.last().map(|(n, _)| n.clone()));
        out.push(here.unwrap_or_else(|| String::from("<module>")));
    }
    out
}

/// `(line, fn)` of every `.exposure` FIELD read (not a call, not an assignment target).
fn exposure_reads(lines: &[String]) -> Vec<(usize, String)> {
    let fns = fn_of_lines(lines);
    let mut out = Vec::new();
    for (k, l) in lines.iter().enumerate() {
        let mut from = 0;
        while let Some(at) = l[from..].find(".exposure") {
            let pos = from + at;
            let rest = &l[pos + ".exposure".len()..];
            from = pos + ".exposure".len();
            if rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '(') {
                continue;
            }
            let t = rest.trim_start();
            if t.starts_with('=') && !t.starts_with("==") {
                continue;
            }
            out.push((k + 1, fns[k].clone()));
        }
    }
    out
}

#[test]
fn lighting_config_exposure_is_read_only_by_the_post_policy() {
    let mut offenders = Vec::new();
    let mut field_decls = 0;
    for (rel, text) in src_files() {
        let lines = non_test_code(&text);
        field_decls += lines.iter().filter(|l| l.trim_start().starts_with("pub exposure:")).count();
        for (line, f) in exposure_reads(&lines) {
            let in_policy = rel == "crates/boyko_render/src/post/mod.rs" && READ_EXPOSURE_IN.contains(&f.as_str());
            let allowed = EXPOSURE_READ_ALLOWED.iter().any(|&(p, n, _)| rel == p && f == n);
            if !in_policy && !allowed {
                offenders.push(format!("{rel}:{line} (fn {f})"));
            }
        }
    }
    assert_eq!(field_decls, 1, "LightingConfig is no longer the only type with an `exposure` field: classify the reads by type");
    assert!(
        offenders.is_empty(),
        "{} read(s) of LightingConfig::exposure outside the post policy: {offenders:#?}",
        offenders.len()
    );
}

#[test]
fn resolved_post_and_resolved_taa_each_have_one_writer() {
    let files = src_files();
    for &(ty, writers) in WRITERS {
        let mut found = Vec::new();
        for (rel, text) in &files {
            let lines = non_test_code(text);
            let fns = fn_of_lines(&lines);
            for (k, l) in lines.iter().enumerate() {
                let compact: String = l.chars().filter(|c| !c.is_whitespace()).collect();
                let tokens = [
                    format!("ResMut<{ty}>"),
                    format!("resource_mut::<{ty}>"),
                    format!("get_resource_mut::<{ty}>"),
                    format!("try_resource_mut::<{ty}>"),
                    format!("insert_resource({ty}"),
                ];
                for t in tokens.iter().filter(|t| compact.contains(t.as_str())) {
                    found.push((rel.clone(), fns[k].clone(), t.clone(), compact.clone()));
                }
            }
        }
        let rogue: Vec<String> = found
            .iter()
            .filter(|(rel, f, _, compact)| {
                !writers.iter().any(|&(p, n, tok)| rel == p && f == n && compact.contains(&tok.replace(' ', "")))
            })
            .map(|(rel, f, t, _)| format!("{rel} (fn {f}): {t}"))
            .collect();
        assert!(rogue.is_empty(), "{ty} has a writer besides its single writer: {rogue:#?}");
        for &(p, n, tok) in writers {
            assert!(
                found.iter().any(|(rel, f, _, compact)| rel == p && f == n && compact.contains(&tok.replace(' ', ""))),
                "{ty}'s single writer is missing: {p} fn {n} ({tok})"
            );
        }
    }
}

/// The scanners against fixtures they must flag and pass.
#[test]
fn the_scanners_reject_their_fixtures() {
    let src = "fn a(cfg: &LightingConfig) -> f32 {\n    cfg.exposure * 2.0\n}\nfn b(c: &mut LightingConfig) {\n    c.exposure = 3.0;\n    let _ = c.exposure_ratio();\n}\n#[cfg(test)]\nmod t {\n    fn c(c: &LightingConfig) -> f32 { c.exposure }\n}\n";
    let lines = non_test_code(src);
    assert_eq!(exposure_reads(&lines), [(2, "a".to_string())]);
    let multi = non_test_code("pub fn w(
    a: Res<A>,
    mut p: ResMut<ResolvedPost>,
) {
    p.x = 1;
}
");
    assert_eq!(fn_of_lines(&multi)[2], "w", "a parameter line belongs to its fn");
}
