//! OBS host test 7 (render-wave plan H0 Scope 7): **every GPU→host copy is followed by the
//! host-read barrier.**
//!
//! A fence wait makes device writes available, not visible, to the host; the dependency whose
//! destination is `HOST` / `HOST_READ` is what makes a readback well-defined. This census reads
//! the source of every crate (non-test code) and holds the tree to it:
//!
//! 1. Every `cmd_copy_image_to_buffer)(` call, and every `cmd_copy_buffer)(` call that is not a
//!    pinned UPLOAD (host staging → device table, [`UPLOADS`], each row with its reason), must be
//!    followed by a `record_host_read_barrier(` call before the next copy site or the next `fn`
//!    item — one helper per readback (a later readback's helper cannot cover an earlier one), and
//!    a loop over regions takes one barrier after the loop (critique O6).
//! 2. The RHI encoder is classified by its own rule, not the one above: `copy_buffer` and
//!    `copy_image_to_buffer` each record the barrier when the destination is MAPPED
//!    (`dst.mapped.is_some()`) — `copy_image_to_buffer` on both of its branches — and the raw
//!    copy inside `copy_image_to_buffer_many` is reached only from `copy_image_to_buffer`, which
//!    records the barrier after it returns.
//! 3. An unclassified new `cmd_copy_buffer` site is red by rule 1 (it is neither an upload nor
//!    followed by the helper).
//! 4. Not vacuous: at least [`MIN_READBACK_SITES`] readback sites and exactly `UPLOADS.len()`
//!    upload sites are found, and the encoder's three copy fns are found.

use std::fs;
use std::path::{Path, PathBuf};

/// The readback sites at the rung that added this census (11 raw recorder sites).
const MIN_READBACK_SITES: usize = 11;

/// The pinned UPLOAD copies: `(file suffix, a token of the call's source argument, reason)`.
/// Each copies host-written staging into a device buffer the GPU reads; no host reads the
/// destination, so no host-read barrier applies.
const UPLOADS: &[(&str, &str, &str)] = &[
    ("present/graph_bridge.rs", "upload.staging.buffer", "DM1 material upload: staging -> material table"),
    ("present/passes/vb.rs", "scene.light_staging.buffer", "light upload: staging -> light table (VB)"),
    ("present/passes/forward.rs", "scene.light_staging.buffer", "light upload: staging -> light table (Forward)"),
    ("present/passes/gbuffer.rs", "scene.light_staging.buffer", "light upload: staging -> light table (Deferred)"),
    ("present/passes/particles.rs", "act.emit_req_staging.buffer", "particle emit requests: staging -> device"),
    ("present/passes/particles.rs", "act.effects_staging.buffer", "particle effect table: staging -> device"),
];

const ENCODER: &str = "rhi_impl/encoder.rs";

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("invariant: {} is readable: {e}", dir.display()))
        .map(|e| e.expect("invariant: directory entry is readable").path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The file's lines with every `#[cfg(test)]` item removed (a test module runs no frame).
fn non_test_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip_depth: Option<i32> = None;
    let mut pending_cfg_test = false;
    for line in text.lines() {
        let t = line.trim();
        if let Some(d) = skip_depth.as_mut() {
            *d += line.matches('{').count() as i32 - line.matches('}').count() as i32;
            if *d <= 0 && line.contains('}') {
                skip_depth = None;
            }
            out.push(String::new());
            continue;
        }
        if t == "#[cfg(test)]" {
            pending_cfg_test = true;
            out.push(String::new());
            continue;
        }
        if pending_cfg_test && line.contains('{') {
            pending_cfg_test = false;
            let d = line.matches('{').count() as i32 - line.matches('}').count() as i32;
            if d > 0 {
                skip_depth = Some(d);
            }
            out.push(String::new());
            continue;
        }
        pending_cfg_test = false;
        out.push(line.to_owned());
    }
    out
}

fn is_fn_item(line: &str) -> bool {
    let t = line.trim_start();
    let t = t.strip_prefix("pub(crate) ").or_else(|| t.strip_prefix("pub ")).unwrap_or(t);
    let t = t.strip_prefix("const ").unwrap_or(t);
    let t = t.strip_prefix("unsafe ").unwrap_or(t);
    t.starts_with("fn ")
}

/// The text of the call starting at line `i`, up to its closing `);` (at most 10 lines).
fn call_text(lines: &[String], i: usize) -> String {
    let mut s = String::new();
    for l in lines.iter().skip(i).take(10) {
        s.push_str(l);
        s.push('\n');
        if l.trim_end().ends_with(");") {
            break;
        }
    }
    s
}

/// `true` iff a copy call starts on this line.
fn is_copy_site(line: &str) -> bool {
    line.contains("cmd_copy_image_to_buffer)(") || line.contains("cmd_copy_buffer)(")
}

/// `true` iff a helper call follows the copy at line `i` before the next copy site or the next
/// `fn` item — so a long recording fn with several readbacks needs one helper per readback, and a
/// later readback's helper cannot cover an earlier one.
fn helper_follows(lines: &[String], i: usize) -> bool {
    for l in &lines[i + 1..] {
        if is_fn_item(l) || is_copy_site(l) {
            return false;
        }
        if l.contains("record_host_read_barrier(") {
            return true;
        }
    }
    false
}

/// The body lines of `fn <name>(` in `lines` (from its line to the next `fn` item).
fn fn_body<'a>(lines: &'a [String], name: &str) -> &'a [String] {
    let start = lines
        .iter()
        .position(|l| is_fn_item(l) && l.contains(&format!("fn {name}(")))
        .unwrap_or_else(|| panic!("the encoder defines `fn {name}(`"));
    let end = lines[start + 1..]
        .iter()
        .position(|l| is_fn_item(l))
        .map_or(lines.len(), |k| start + 1 + k);
    &lines[start..end]
}

#[test]
fn every_gpu_to_host_copy_is_followed_by_the_host_read_barrier() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for entry in fs::read_dir(&crates).expect("invariant: crates/ is readable") {
        let src = entry.expect("invariant: entry").path().join("src");
        if src.is_dir() {
            rust_files(&src, &mut files);
        }
    }
    assert!(files.len() > 100, "anti-vacuity: {} source files", files.len());

    let mut readbacks = Vec::new();
    let mut uploads = Vec::new();
    let mut missing = Vec::new();
    let mut encoder_seen = false;
    let mut encoder_problems = Vec::new();
    for path in &files {
        let rel = path.to_string_lossy().replace('\\', "/");
        let text = fs::read_to_string(path).expect("invariant: UTF-8 source");
        if !text.contains("cmd_copy_image_to_buffer)(") && !text.contains("cmd_copy_buffer)(") {
            continue;
        }
        let lines = non_test_lines(&text);
        if rel.ends_with(ENCODER) {
            encoder_seen = true;
            encoder_problems = check_encoder(&lines);
            continue;
        }
        for (i, l) in lines.iter().enumerate() {
            if !l.contains("cmd_copy_image_to_buffer)(") && !l.contains("cmd_copy_buffer)(") {
                continue;
            }
            let site = format!("{rel}:{}", i + 1);
            let call = call_text(&lines, i);
            let upload = l.contains("cmd_copy_buffer)(")
                && UPLOADS.iter().any(|(f, token, _)| rel.ends_with(f) && call.contains(token));
            if upload {
                uploads.push(site);
            } else if helper_follows(&lines, i) {
                readbacks.push(site);
            } else {
                missing.push(site);
            }
        }
    }
    assert!(
        missing.is_empty() && encoder_problems.is_empty(),
        "{} GPU->host copy site(s) without a following record_host_read_barrier (or an \
         unclassified new cmd_copy_buffer site - pin it in UPLOADS with its reason if it is an \
         upload):\n{}\n{} encoder problem(s):\n{}",
        missing.len(),
        missing.join("\n"),
        encoder_problems.len(),
        encoder_problems.join("\n")
    );
    assert!(encoder_seen, "anti-vacuity: the census read {ENCODER}");
    assert_eq!(uploads.len(), UPLOADS.len(), "every pinned upload is found exactly once: {uploads:?}");
    assert!(
        readbacks.len() >= MIN_READBACK_SITES,
        "anti-vacuity: {} readback sites found, expected >= {MIN_READBACK_SITES}: {readbacks:?}",
        readbacks.len()
    );
}

/// Rule 2: the encoder's mapped-destination barrier. Returns one line per problem.
fn check_encoder(lines: &[String]) -> Vec<String> {
    let mut bad = Vec::new();
    for (name, want) in [("copy_buffer", 1), ("copy_image_to_buffer", 2)] {
        let body = fn_body(lines, name);
        let mapped = body.iter().filter(|l| l.contains("dst.mapped.is_some()")).count();
        let helper = body.iter().filter(|l| l.contains("record_host_read_barrier(")).count();
        if mapped < want || helper < want {
            bad.push(format!(
                "encoder `{name}`: {mapped} mapped check(s) and {helper} host-read barrier(s), \
                 expected {want} of each"
            ));
        }
    }
    let many = fn_body(lines, "copy_image_to_buffer_many");
    if !many.iter().any(|l| l.contains("cmd_copy_image_to_buffer)(")) {
        bad.push("encoder `copy_image_to_buffer_many` no longer records the raw copy".into());
    }
    let callers: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.contains("copy_image_to_buffer_many(") && !is_fn_item(l))
        .map(|(i, _)| i)
        .collect();
    let in_copy = fn_body(lines, "copy_image_to_buffer");
    if callers.len() != 1 || !in_copy.iter().any(|l| l.contains("copy_image_to_buffer_many(")) {
        bad.push(format!(
            "encoder `copy_image_to_buffer_many` must be reached only from `copy_image_to_buffer` \
             (which records the barrier after it returns); call lines: {callers:?}"
        ));
    }
    bad
}

/// The detector itself can fail: a readback with no helper and an unpinned buffer copy are found.
#[test]
fn the_census_flags_a_readback_without_the_barrier() {
    let src = "fn record(cmd: u32) {\n    unsafe {\n        (fns.cmd_copy_image_to_buffer)(\n            cmd,\n        );\n    }\n}\nfn other() {\n    record_host_read_barrier(fns, cmd, &[b]);\n}\n";
    let lines = non_test_lines(src);
    assert!(!helper_follows(&lines, 2), "a helper in the NEXT fn does not count");
    let ok = "fn record() {\n    (fns.cmd_copy_buffer)(cmd, a, b, 1, &r);\n    record_host_read_barrier(fns, cmd, &[b]);\n}\n";
    assert!(helper_follows(&non_test_lines(ok), 1));
    let two = "fn record() {\n    (fns.cmd_copy_image_to_buffer)(cmd, i, l, a, 1, &r);\n    (fns.cmd_copy_image_to_buffer)(cmd, i, l, b, 1, &r);\n    record_host_read_barrier(fns, cmd, &[b]);\n}\n";
    let two = non_test_lines(two);
    assert!(!helper_follows(&two, 1), "a later readback's helper does not cover an earlier one");
    assert!(helper_follows(&two, 2));
    let test_only = "#[cfg(test)]\nmod tests {\n    fn f() { (fns.cmd_copy_buffer)(cmd, a, b, 1, &r); }\n}\n";
    assert!(non_test_lines(test_only).iter().all(|l| !l.contains("cmd_copy_buffer")), "a #[cfg(test)] module is skipped");
}
