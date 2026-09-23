//! UG-15 leg (1), the source census (03 §6), over the six census crates of this tree.
//!
//! Pass: the counted findings equal `ug15/allowlist-leg1.toml` exactly (empty at B3; edited only
//! in an owner-signed commit). The run prints what it read; a walk under the floors is RED, because
//! a census that read nothing has no zero to report.
//!
//! The second test pins every census crate's direct dependency set (critique W6): a dependency's
//! macro emits its tokens into the invoking crate at expansion, where neither the `syn` walk nor
//! M-a can see them, so a new macro source is RED until someone has read what it emits and
//! updated `ug15/pins/leg1-deps.pins` (`ug15 leg1 deps --write`).

use std::path::{Path, PathBuf};

use boyko_symcensus::source::{self, AllowEntry};

/// The census walked 326 files, 11 608 items, 5 745 macro bodies (72 `macro_rules!`) at B3; these floors sit below that with room for
/// deletions and far above an accidental empty or single-crate walk.
const MIN_FILES: usize = 250;
const MIN_ITEMS: usize = 5_000;
const MIN_MACRO_BODIES: usize = 1_000;
const MIN_MACRO_RULES: usize = 60;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("invariant: crates/boyko_symcensus").to_path_buf()
}

#[test]
fn leg1_counted_findings_equal_the_owner_signed_allowlist() {
    let root = root();
    let (findings, stats, per) = source::census(&root).unwrap_or_else(|r| panic!("{r}"));
    println!("UG-15 leg (1) read: {stats}");
    for (c, s) in &per {
        println!("  {c}: {s}");
    }
    assert!(stats.files >= MIN_FILES, "leg (1) walked {} files, under the floor {MIN_FILES}", stats.files);
    assert!(stats.items >= MIN_ITEMS, "leg (1) visited {} items, under the floor {MIN_ITEMS}", stats.items);
    assert!(stats.macro_bodies >= MIN_MACRO_BODIES, "leg (1) scanned {} macro bodies, under the floor {MIN_MACRO_BODIES}", stats.macro_bodies);
    assert!(stats.macro_rules >= MIN_MACRO_RULES, "leg (1) met {} macro_rules! bodies, under the floor {MIN_MACRO_RULES}", stats.macro_rules);
    assert!(stats.build_rs_strings > 0, "leg (1) scanned no build.rs string although boyko_diag has a build script");

    let text = std::fs::read_to_string(root.join("crates/boyko_symcensus/ug15/allowlist-leg1.toml")).expect("the leg-(1) allowlist exists");
    let allow = source::parse_allowlist(&text).unwrap_or_else(|r| panic!("{r}"));
    let mut got: Vec<AllowEntry> = findings
        .iter()
        .map(|f| AllowEntry { file: f.file.clone(), item: f.item.clone(), kind: f.kind.to_string(), reason: String::new() })
        .collect();
    got.sort();
    let mut want: Vec<AllowEntry> = allow.iter().map(|a| AllowEntry { reason: String::new(), ..a.clone() }).collect();
    want.sort();
    println!("UG-15 leg (1): {} counted, {} allowlisted", findings.len(), allow.len());
    for f in &findings {
        println!("  counted: {f}");
    }
    assert_eq!(got, want, "leg (1) RED: the counted set is not the owner-signed allowlist (counted above)");
}

#[test]
fn census_crate_dependency_sets_are_pinned() {
    let root = root();
    let rows = source::census_crate_deps(&root, std::ffi::OsStr::new(env!("CARGO"))).unwrap_or_else(|r| panic!("{r}"));
    let got: Vec<String> = rows.iter().map(ToString::to_string).collect();
    let pin_path = root.join("crates/boyko_symcensus/ug15/pins/leg1-deps.pins");
    let pinned = std::fs::read_to_string(&pin_path).expect("the dependency pin exists");
    let want: Vec<String> = pinned.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).map(str::to_owned).collect();
    println!("UG-15 leg (1) dependency pin: {} row(s) read from cargo metadata, {} pinned", got.len(), want.len());
    let proc_macros = rows.iter().filter(|r| r.target == "proc-macro").count();
    println!("  of which proc-macro: {proc_macros}");
    for r in got.iter().filter(|r| !want.contains(r)) {
        println!("  NEW (not pinned): {r}");
    }
    for r in want.iter().filter(|r| !got.contains(r)) {
        println!("  GONE (pinned, not in the graph): {r}");
    }
    assert!(!got.is_empty(), "cargo metadata gave no dependency row");
    assert_eq!(got, want, "leg (1) RED: a census crate's dependency set changed; read what the new dependency's macros emit, then re-pin");
}
