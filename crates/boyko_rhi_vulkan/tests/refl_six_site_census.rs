//! IBL1 R1 (`ibl1/cut.md` C10) — **the environment arm reaches all six lit producers, and only
//! under `-D REFL=1`**.
//!
//! The design twice shipped an ambient change to five of the six producers (the P39 class): the
//! sixth kept the old term and nothing went red, because each producer's own gate only pins its own
//! bytes. This census reads the six sources and requires, per source:
//!
//! - exactly ONE `eval_pbr_ambient_hemi(` call (the base SKY branch) and exactly ONE
//!   `eval_pbr_ambient_env(` call, the latter inside `#ifdef REFL` — six and six in all;
//! - exactly one `#include "env_ibl.hlsli"`, inside `#ifdef REFL`;
//! - the `gEnvMap` / `gEnvSamp` declarations inside `#ifdef REFL`.
//!
//! and over the manifest: exactly 20 REFL rows (`_refl.` modules), each with a committed `.spv`.
//! Comments are stripped before counting, so a doc line naming a call is not a call.

use std::path::PathBuf;

/// The six `lit`-writing producer sources.
const PRODUCERS: [&str; 6] = [
    "deferred_pbr.hlsl",
    "forward_opaque.fs.hlsl",
    "sdf_forward_march.comp.hlsl",
    "vb_resolve.comp.hlsl",
    "vb_shade.comp.hlsl",
    "vb_shade_split.comp.hlsl",
];

fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// `src` with `//` comments removed, line by line (the producers carry no `/* */` block that spans
/// code).
fn strip_line_comments(src: &str) -> Vec<String> {
    src.lines().map(|l| l.split("//").next().unwrap_or("").to_string()).collect()
}

/// For every line, whether it sits inside an `#ifdef REFL` block (nested conditionals tracked; the
/// REFL block's own `#else` arm is outside).
fn in_refl(lines: &[String]) -> Vec<bool> {
    // A stack of "is this level the REFL block's true arm".
    let mut stack: Vec<bool> = Vec::new();
    let mut out = Vec::with_capacity(lines.len());
    for l in lines {
        let t = l.trim();
        if t.starts_with("#ifdef") || t.starts_with("#ifndef") || t.starts_with("#if") {
            let here = t == "#ifdef REFL";
            out.push(stack.iter().any(|&b| b));
            stack.push(here);
            continue;
        }
        if t.starts_with("#else") || t.starts_with("#elif") {
            if let Some(top) = stack.last_mut() {
                *top = false;
            }
            out.push(stack.iter().any(|&b| b));
            continue;
        }
        if t.starts_with("#endif") {
            out.push(stack.iter().any(|&b| b));
            stack.pop();
            continue;
        }
        out.push(stack.iter().any(|&b| b));
    }
    out
}

/// `(line, in REFL)` for every line of `lines` containing `needle`.
fn hits(lines: &[String], refl: &[bool], needle: &str) -> Vec<(usize, bool)> {
    lines.iter().enumerate().filter(|(_, l)| l.contains(needle)).map(|(i, _)| (i + 1, refl[i])).collect()
}

#[test]
fn refl_six_site_census() {
    let mut problems = Vec::new();
    let (mut hemi_total, mut env_total) = (0, 0);
    for name in PRODUCERS {
        let text = std::fs::read_to_string(shaders_dir().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let lines = strip_line_comments(&text);
        let refl = in_refl(&lines);
        let hemi = hits(&lines, &refl, "eval_pbr_ambient_hemi(");
        let env = hits(&lines, &refl, "eval_pbr_ambient_env(");
        let include = hits(&lines, &refl, "#include \"env_ibl.hlsli\"");
        let decls: Vec<(usize, bool)> = hits(&lines, &refl, "gEnvMap;").into_iter().chain(hits(&lines, &refl, "gEnvSamp;")).collect();
        hemi_total += hemi.len();
        env_total += env.len();
        if hemi.len() != 1 || hemi.iter().any(|h| h.1) {
            problems.push(format!("{name}: eval_pbr_ambient_hemi( at {hemi:?} (want one, outside #ifdef REFL)"));
        }
        if env.len() != 1 || env.iter().any(|h| !h.1) {
            problems.push(format!("{name}: eval_pbr_ambient_env( at {env:?} (want one, inside #ifdef REFL)"));
        }
        if include.len() != 1 || include.iter().any(|h| !h.1) {
            problems.push(format!("{name}: #include \"env_ibl.hlsli\" at {include:?} (want one, inside #ifdef REFL)"));
        }
        if decls.len() != 2 || decls.iter().any(|h| !h.1) {
            problems.push(format!("{name}: gEnvMap/gEnvSamp declarations at {decls:?} (want two, inside #ifdef REFL)"));
        }
    }
    assert!(problems.is_empty(), "the environment arm is not on all six producers under -D REFL:\n{}", problems.join("\n"));
    assert_eq!((hemi_total, env_total), (6, 6), "six base sites and six environment sites");
}

/// The manifest's REFL axis has exactly 20 rows, each naming a committed module.
#[test]
fn manifest_lists_the_twenty_refl_rows() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/SHADER-VARIANT-MANIFEST.md");
    let text = std::fs::read_to_string(&manifest).unwrap_or_else(|e| panic!("{}: {e}", manifest.display()));
    let mut rows: Vec<String> = Vec::new();
    for line in text.lines().filter(|l| l.starts_with('|')) {
        for cell in line.split('|') {
            let cell = cell.trim().trim_matches('`');
            if cell.contains("_refl.") && cell.ends_with(".spv") && !cell.contains(' ') && !rows.iter().any(|r| r == cell) {
                rows.push(cell.to_string());
            }
        }
    }
    assert_eq!(rows.len(), 20, "the manifest's REFL rows: {rows:?}");
    for r in &rows {
        assert!(shaders_dir().join(r).exists(), "manifest row {r} has no committed .spv");
    }
}

/// The census can fail: a fixture with the arm outside `#ifdef REFL` and one with it missing.
#[test]
fn census_detects_a_missing_or_unguarded_arm() {
    let unguarded = "#include \"env_ibl.hlsli\"\nambient += eval_pbr_ambient_env(surf);\nambient += eval_pbr_ambient_hemi(surf);\n";
    let lines: Vec<String> = unguarded.lines().map(String::from).collect();
    let refl = in_refl(&lines);
    assert!(hits(&lines, &refl, "eval_pbr_ambient_env(").iter().all(|h| !h.1), "an unguarded arm reads as outside REFL");
    let guarded = "#ifdef REFL\nif (a) {\n#if HWRT\nx\n#endif\nambient += eval_pbr_ambient_env(surf);\n} else\n#endif\nambient += eval_pbr_ambient_hemi(surf);\n";
    let lines: Vec<String> = guarded.lines().map(String::from).collect();
    let refl = in_refl(&lines);
    assert_eq!(hits(&lines, &refl, "eval_pbr_ambient_env("), [(6, true)]);
    assert_eq!(hits(&lines, &refl, "eval_pbr_ambient_hemi("), [(9, false)]);
}
