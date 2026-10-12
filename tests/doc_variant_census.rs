//! **No live `PARTICLE_FOG` mention under `docs/render/`** (RD4 Part C, render-wave plan AD-8).
//!
//! The volumetrics design's pass-2 D24 removed the `-D PARTICLE_FOG` draw axis: the particle fog
//! fetch reads an always-bound `fog_integrated` binding under a boot-resolved gate word, so
//! `particle_draw.vs` is 6 variants and `particle_draw.fs` at most 36. Three sibling designs
//! (VFX, VOLUMETRICS, TRANSPARENCY-UPDATE) were written against pass 1 and priced the axis. RD4
//! rewrote every site; the token may stay where history needs it, but only on a line that says
//! the axis is **withdrawn** and names the decision that withdrew it (`AD-8`) — the marker RD4
//! writes as `(withdrawn, AD-8)`. A design that re-introduces the axis as live text reds here, so
//! the next rung never reads a stale variant count.
//!
//! Both words are required because one is not enough: at the base, a TRANSPARENCY-UPDATE D-U12
//! row named `PARTICLE_FOG` as live while carrying `withdrawn` for an unrelated `-D FOG` row, so a
//! `withdrawn`-only census passed it (the RD4 red-first found 12 of the 13 enumerated lines).
//!
//! # What it asserts
//!
//! 1. Every line of every `.md` under `docs/render/` that contains `PARTICLE_FOG` also contains
//!    `withdrawn` and `AD-8`. The failure lists every offending `path:line`, not only the first.
//! 2. The walk is not vacuous: `docs/render` exists and at least one `.md` file was read.
//! 3. The detector itself can fail (`census_flags_a_live_mention_fixture`): an inline fixture
//!    written under `CARGO_TARGET_TMPDIR` holds an unmarked line, a half-marked line (`withdrawn`
//!    without `AD-8`) and a marked line; the census must flag the first two by name and pass the
//!    third — on every run, not only at the red-first.

use std::fs;
use std::path::{Path, PathBuf};

/// The token whose every mention must be marked.
const TOKEN: &str = "PARTICLE_FOG";

/// The words a historical mention carries on its own line (RD4's `(withdrawn, AD-8)` marker).
const MARKERS: [&str; 2] = ["withdrawn", "AD-8"];

/// Collects every `.md` file under `dir`, recursively, in a stable order.
fn collect_md(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("invariant: {} is readable: {e}", dir.display()))
        .map(|e| e.expect("invariant: directory entry is readable").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_md(&path, out);
        } else if path.extension().is_some_and(|x| x == "md") {
            out.push(path);
        }
    }
}

/// Returns `(files scanned, offending "path:line: text" rows)` for the tree under `dir`.
fn census(dir: &Path) -> (usize, Vec<String>) {
    let mut files = Vec::new();
    collect_md(dir, &mut files);
    let mut live = Vec::new();
    for path in &files {
        let text = fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("invariant: {} is UTF-8 text: {e}", path.display()));
        for (i, line) in text.lines().enumerate() {
            if line.contains(TOKEN) && !MARKERS.iter().all(|m| line.contains(m)) {
                live.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
            }
        }
    }
    (files.len(), live)
}

#[test]
fn particle_fog_mentions_are_withdrawn() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs").join("render");
    assert!(dir.is_dir(), "anti-vacuity: {} does not exist", dir.display());
    let (files, live) = census(&dir);
    assert!(files >= 1, "anti-vacuity: no .md file under {}", dir.display());
    assert!(
        live.is_empty(),
        "{} line(s) under docs/render mention `{TOKEN}` without the marker words {MARKERS:?} \
         (AD-8 removed the axis; rewrite the line or mark it `(withdrawn, AD-8)`):\n{}",
        live.len(),
        live.join("\n")
    );
}

#[test]
fn census_flags_a_live_mention_fixture() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("doc_variant_census_fixture");
    // A previous run's fixture is replaced, never appended to.
    if root.exists() {
        fs::remove_dir_all(&root).expect("invariant: CARGO_TARGET_TMPDIR is writable");
    }
    let sub = root.join("nested");
    fs::create_dir_all(&sub).expect("invariant: CARGO_TARGET_TMPDIR is writable");
    fs::write(
        sub.join("live.md"),
        "# fixture\nThe VS bound is 12 with `-D PARTICLE_FOG` armed.\n",
    )
    .expect("invariant: CARGO_TARGET_TMPDIR is writable");
    fs::write(
        root.join("marked.md"),
        "The `PARTICLE_FOG` axis is withdrawn (AD-8).\nNo token on this line.\n\
         The `-D FOG` rows are withdrawn; particles fog through `PARTICLE_FOG`.\n",
    )
    .expect("invariant: CARGO_TARGET_TMPDIR is writable");
    fs::write(root.join("ignored.txt"), "PARTICLE_FOG in a non-markdown file\n")
        .expect("invariant: CARGO_TARGET_TMPDIR is writable");

    let (files, live) = census(&root);
    assert_eq!(files, 2, "the walk must read exactly the two .md fixtures, recursively");
    assert_eq!(live.len(), 2, "exactly the unmarked and the half-marked line are live: {live:?}");
    assert!(
        live[0].contains("marked.md:3:") && live[0].contains("PARTICLE_FOG"),
        "the half-marked row (`withdrawn` without `AD-8`) names its file and line: {}",
        live[0]
    );
    assert!(
        live[1].contains("live.md:2:") && live[1].contains("PARTICLE_FOG"),
        "the unmarked row names its file and line: {}",
        live[1]
    );
}
