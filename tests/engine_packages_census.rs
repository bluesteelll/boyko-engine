//! **The test `boyko_diag::sample::ENGINE_PACKAGES` promised for four rungs and did not have.**
//!
//! `profiling_partition!(Engine)` const-asserts `env!("CARGO_PKG_NAME")` against that array. A
//! name spelled wrong there is not a warning and not a lint — it is a crate that **cannot declare
//! an engine zone at all**, discovered only by the rung that first tries.
//!
//! Five rows were wrong. This workspace does not spell its members uniformly: most are hyphenated
//! (`boyko-ecs`), but `boyko_rhi`, `boyko_rhi_vulkan`, `boyko_image`, `boyko_sdf_math` and
//! `boyko_shaderdsl` carry underscores, and rung 1 wrote all twenty rows hyphenated. The first four
//! crates to actually write the partition line were all hyphenated and all correct, so nothing
//! contradicted the other five until profiling rung 5 needed `boyko_rhi_vulkan`.
//!
//! # What it asserts, in both directions
//!
//! 1. **Every name in `ENGINE_PACKAGES` is a real member's `[package] name`.** This is the clause
//!    that was failing: a row naming nothing is a crate that silently cannot participate.
//! 2. **Every member is either in `ENGINE_PACKAGES` or in the `USER_PACKAGES` exemption below.**
//!    A new engine crate added without a row would otherwise be discovered the same way — by
//!    whoever first writes `profiling_partition!(Engine)` in it and cannot compile.
//!
//! The exemption list is short and explicit rather than a heuristic. "Ends in `_demo`" or
//! "contains `bench`" would be a rule that quietly absorbs the next mistake.
//!
//! # Why it lives in the root package
//!
//! `CARGO_MANIFEST_DIR` **is** the repository root here, so no `../..` walking can point the scan
//! at the wrong tree — `internal_docs_anchors.rs`'s rationale, verbatim.

use std::collections::BTreeSet;
use std::path::PathBuf;

use boyko_diag::sample::ENGINE_PACKAGES;

/// Workspace members that are deliberately NOT engine packages.
///
/// A game and a benchmark harness declare `profiling_partition!(User)`: their zones belong in the
/// user region so a runaway scope costs the game's samples and not the engine's. That is the whole
/// point of the two-region split, so these are exemptions with a reason, not omissions.
const USER_PACKAGES: &[&str] = &[
    // The demo game.
    "boyko_demo",
    // The Bevy comparison harness.
    "bench-bevy-vs-boyko",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The `[package] name` of one manifest, read the way Cargo reads it: the FIRST `name =` row
/// inside the `[package]` table, ignoring the `name =` rows that `[[bin]]` / `[[bench]]` tables
/// carry further down.
fn package_name(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package && line.starts_with("name") {
            let value = line.split_once('=')?.1.trim();
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

/// Every workspace member's package name, read from `crates/*/Cargo.toml`.
fn member_names() -> BTreeSet<String> {
    let crates = repo_root().join("crates");
    let mut names = BTreeSet::new();
    let entries = std::fs::read_dir(&crates)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", crates.display()));
    for entry in entries.flatten() {
        let manifest = entry.path().join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest.display()));
        if let Some(name) = package_name(&text) {
            names.insert(name);
        }
    }
    names
}

#[test]
fn every_engine_package_row_names_a_real_workspace_member() {
    let members = member_names();

    // A parser that stopped parsing would leave `members` empty, every row would look bogus, and
    // the failure would read as "the list is wrong" rather than "the scan is dead".
    assert!(
        members.len() >= 20,
        "the manifest scan found only {} members — the parser is broken, not the workspace",
        members.len()
    );

    let bogus: Vec<&&str> =
        ENGINE_PACKAGES.iter().filter(|name| !members.contains(**name)).collect();
    assert!(
        bogus.is_empty(),
        "ENGINE_PACKAGES names {bogus:?}, which no workspace member is called. The const assert in \
         `profiling_partition!(Engine)` compares against `CARGO_PKG_NAME`, so a row that names \
         nothing is a crate that CANNOT declare an engine zone — and it fails at the rung that \
         first tries, not here, unless this gate runs. Members are: {members:?}"
    );
}

#[test]
fn every_workspace_member_is_classified_as_engine_or_user() {
    let members = member_names();
    let engine: BTreeSet<&str> = ENGINE_PACKAGES.iter().copied().collect();
    let user: BTreeSet<&str> = USER_PACKAGES.iter().copied().collect();

    let unclassified: Vec<&String> = members
        .iter()
        .filter(|m| !engine.contains(m.as_str()) && !user.contains(m.as_str()))
        .collect();
    assert!(
        unclassified.is_empty(),
        "workspace members {unclassified:?} are in neither ENGINE_PACKAGES nor this gate's \
         USER_PACKAGES. Decide which region their zones belong to and add the row — the decision \
         is one line, and leaving it undecided means whoever writes the partition line first \
         discovers it as a compile error in a crate they were not working on."
    );

    let both: Vec<&&str> = engine.intersection(&user).collect();
    assert!(both.is_empty(), "{both:?} is listed as BOTH an engine and a user package");
}

/// The instrument's own positive control: the parser recovers a name it is known to face, and one
/// of each spelling.
///
/// Without this, a `package_name` that returned `None` for everything would leave `members` empty,
/// and the length floor above would be the only thing standing between that and a green run — a
/// floor catches a dead scan, but it does not catch a scan that reads the WRONG `name =` row (the
/// `[[bin]]` one two tables down, which is a real shape in this workspace: `boyko-app`'s manifest
/// carries `name = "boyko-app"` under `[package]` and `name = "boyko_app"` under `[lib]`).
#[test]
fn the_manifest_parser_reads_the_package_table_and_not_a_later_one() {
    let members = member_names();
    assert!(
        members.contains("boyko-app"),
        "the parser missed `boyko-app`'s [package] name — it likely read a later table's `name =`"
    );
    assert!(
        !members.contains("boyko_app"),
        "the parser picked up `boyko-app`'s [lib] name instead of its [package] name"
    );
    assert!(
        members.contains("boyko_rhi_vulkan"),
        "the parser missed the underscore-named member this whole gate was written for"
    );
}
