//! **The zero-dependency claim, as a gate rather than a comment.**
//!
//! # Why this file exists, and why profiling rung 10 is what wrote it
//!
//! `crates/boyko_diag/Cargo.toml` opens with the word **INVIOLABLE**: this crate is the bottom of
//! the workspace graph and depends on `std` only, so a single edge added here is reachable from
//! every crate in the engine. Two other manifests state the same property as settled fact —
//! `crates/boyko_app/Cargo.toml` says *"a tidy test pins `cargo tree -p boyko_diag` to exactly one
//! node"*, and `crates/boyko_threadpool/Cargo.toml` says *"`cargo tree -p boyko-diag -e
//! normal,build` is one node"*.
//!
//! **MEASURED at profiling rung 10: no such test existed.** Repo-wide grep for `cargo tree` across
//! `crates/**/*.rs` returned nothing. The property was true, and it was true by nobody having
//! broken it yet — which is precisely the shape this corpus has already paid for once, when
//! `ENGINE_PACKAGES`'s own doc promised *"a tidy test pins this list against the actual member
//! set"*, no such test existed, and **five of its twenty rows were wrong for four rungs**.
//!
//! Rung 10 is the right moment because rung 10 is what made it easier to break: this crate had no
//! `tests/` directory at all until then, and a `tests/` directory is exactly where somebody reaches
//! for `trybuild` or `proptest` and adds the first `[dev-dependencies]` line. A dev-dependency does
//! not reach a consumer's build — but it does reach `cargo tree`, it does reach the lockfile, and
//! it makes the manifests' flat claim false.
//!
//! # What this gate checks, and what it deliberately does not
//!
//! It reads the manifest as TEXT and asserts that every dependency table is empty. It does **not**
//! shell out to `cargo tree`: that is a process spawn and a network-capable resolver in a test for
//! a crate whose entire discipline is that it spawns nothing, and it would fail offline for a
//! reason having nothing to do with the property. The text is the source of truth anyway — an
//! empty `[dependencies]` in this file is what makes the tree one node, not the other way round.
//!
//! It cannot claim anything about `std` itself, nor about what the workspace `[lints]` table pulls
//! in, nor that a future edition change keeps the parse below honest. It claims that the four
//! dependency tables Cargo recognises are empty in this manifest.

/// Every table Cargo will take a dependency from. All four must be empty here.
///
/// `build-dependencies` is in the list even though this crate has no `build.rs` — MEASURED, there
/// is no build script anywhere in this workspace — because "there is no build script" is exactly
/// the kind of fact that stops being true quietly, and rung 14's `BOYKO_PROFILE` axis is scheduled
/// to add one *to this crate*.
const DEPENDENCY_TABLES: &[&str] = &[
    "[dependencies]",
    "[dev-dependencies]",
    "[build-dependencies]",
    "[target.",
];

/// The manifest, embedded at COMPILE time.
///
/// `include_str!` rather than `std::fs::read_to_string`: the path is resolved relative to this
/// source file by the compiler, so the test cannot fail for having been run from the wrong working
/// directory — a failure mode that would look exactly like the one it is written to catch.
const MANIFEST: &str = include_str!("../Cargo.toml");

/// **The gate.** No dependency table in this manifest has an entry.
///
/// The RED is one line: add `libc = "0.2"` under `[dependencies]`, or `trybuild = "1"` under a
/// `[dev-dependencies]` table, and this fires naming the table and the entry.
#[test]
fn the_bottom_of_the_graph_has_no_edges() {
    let mut current: Option<&str> = None;
    let mut offenders: Vec<String> = Vec::new();

    for raw in MANIFEST.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            // A new table. `[target.'cfg(...)'.dependencies]` is matched by prefix, which is what
            // the `"[target."` entry above is for: a platform-gated dependency is still a
            // dependency, and spelling it that way is the obvious way to smuggle one past a
            // check that only looked for the bare table name.
            current = DEPENDENCY_TABLES
                .iter()
                .find(|t| line.starts_with(**t))
                .copied();
            continue;
        }
        if let Some(table) = current {
            offenders.push(format!("{table} -> {line}"));
        }
    }

    assert!(
        offenders.is_empty(),
        "boyko-diag has grown {} dependency entr{}, and its manifest calls that INVIOLABLE:\n  {}\n\
         \n\
         This crate is the bottom of the workspace graph -- `boyko_log`, `boyko_threadpool`, \
         `boyko_ecs` and `boyko_rhi_vulkan` all depend on it -- so one edge here is reachable from \
         every crate in the engine. If the edge is genuinely needed, the decision belongs in \
         `docs/diagnostics/substrate/00-GOAL.md`'s growth rule and in the two other manifests that \
         state this property as settled fact, not only here.",
        offenders.len(),
        if offenders.len() == 1 { "y" } else { "ies" },
        offenders.join("\n  "),
    );
}

/// Non-vacuity: the parser above actually finds entries when there are any.
///
/// Without this, a walker that silently matched no table at all would make the gate green forever —
/// the failure mode of every text-scanning check, and one this campaign has met more than once (a
/// liveness check answering about a symbol that had been renamed, a source scan reddened by line
/// endings rather than by content).
#[test]
fn the_walker_would_notice_an_edge_if_there_were_one() {
    const FIXTURE: &str = "\
[package]
name = \"x\"

[dependencies]
libc = \"0.2\"

[dev-dependencies]
trybuild = \"1\"

[target.'cfg(windows)'.dependencies]
windows-sys = \"0.59\"
";
    let mut current: Option<&str> = None;
    let mut found: Vec<&str> = Vec::new();
    for raw in FIXTURE.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            current = DEPENDENCY_TABLES
                .iter()
                .find(|t| line.starts_with(**t))
                .copied();
            continue;
        }
        if current.is_some() {
            found.push(line);
        }
    }
    assert_eq!(
        found,
        vec!["libc = \"0.2\"", "trybuild = \"1\"", "windows-sys = \"0.59\""],
        "the walker missed an entry in a manifest that has three -- including the platform-gated \
         one, which is the spelling a bare table-name check would let through"
    );
}
