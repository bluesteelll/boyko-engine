//! **Reflection GATES G2 — the resolver gate on the named invocation (P-B).**
//!
//! For each ship target of D2 (`boyko_demo`, root `boyko-engine` — the list is owned by
//! `docs/REFLECTION-PLAN-GATES.md` D2 and by `tests/reflect_manifest_census.rs`, never
//! restated with different contents) this runs
//!
//! ```text
//! cargo tree -p <ship> -e features --edges normal,build --format "{p} {f}"
//! ```
//!
//! and asserts the resolved closure contains no `boyko-reflect` package and no enabled
//! feature named `reflect`. Three properties of that command line are decisions, not
//! defaults (GATES G2): **no `--workspace`** (a workspace invocation unifies — measured
//! in-tree, `cargo tree --workspace -e features --no-default-features` reported a feature
//! ENABLED that the flags asked to disable, `crates/boyko_ecs/Cargo.toml`); **`--edges
//! normal,build`** (dev-dependencies do not ship, and including them would red on a
//! fixture's own dev edge until someone relaxed the gate into decoration); **`-e
//! features`** (the failure mode watched is a *feature* that pulls a package in).
//!
//! # Matching is on PARSED package names, never on raw substrings — measured
//!
//! This worktree lives under a path containing the word `reflect`
//! (`D:\wt\reflect\crates\boyko_reflect`), and `{p}` prints every path-dep's path. A raw
//! `contains("reflect")` would red every line of a clean tree, and — worse — a raw
//! `contains("boyko_reflect")` (the LIB name) would stay GREEN on the underscore-needle
//! mutation while the path kept matching for the wrong reason. Every clause below
//! therefore parses each line into (package, enabled-features) first and compares whole
//! names.
//!
//! # The invocation guard, in the portable form — and why the env form is unbuildable
//!
//! GATES G2's fourth decision wants the harness to red when it was itself invoked under a
//! feature selection (`cargo test … --features reflect-dogfood/reflect` reaches
//! `boyko_demo` through unification — D15's recorded exposure). The plan offers two
//! forms. **The env form does not exist on this toolchain — MEASURED at landing
//! (2026-08-21):** `CARGO_ENCODED_ARGS` is not among the variables cargo sets for a test
//! process, and the full `CARGO*` environment of this test binary is byte-identical
//! under `cargo test -p boyko-engine -p reflect-dogfood --features
//! reflect-dogfood/reflect` versus the plain invocation (empirically diffed: NO-DIFF).
//! An outer feature selection is therefore *unobservable from inside this process*, and
//! a guard claiming to detect it would be a gate that cannot fail.
//!
//! So the portable form lands: the harness **spawns its own `cargo tree` with an
//! explicitly constructed argv**, and [`assert_ship_invocation_purity`] refuses any
//! feature-selecting token in that argv before the ship reading is taken. The outer
//! exposure is thereby closed *by construction* — a fresh `cargo tree` process resolves
//! from the manifests, and the parent's `--features` cannot leak into it (same
//! measurement) — so the number this gate reports is always a ship closure. The guard's
//! RED is the harness-side mutation: route a feature flag into the ship reading and the
//! purity assertion reds with the plan's sentence.

use std::process::Command;

/// The ship targets (GATES D2 owns this list; `boyko_app` is `[lib]`-only and is
/// deliberately not here).
const SHIP_TARGETS: &[&str] = &["boyko_demo", "boyko-engine"];

/// The package under gate, spelled as `cargo tree` prints packages (hyphenated PACKAGE
/// name, not the underscored LIB name — the second RED's subject).
const NEEDLE: &str = "boyko-reflect";

/// One parsed `cargo tree` line.
enum Line {
    /// A package node: name + the features `{f}` reported enabled on it.
    Package { name: String, features: Vec<String> },
    /// A feature edge: `<pkg> feature "<name>"`.
    FeatureEdge { package: String, feature: String },
}

/// Parses one output line of `--format "{p} {f}"`, tolerating the tree-drawing prefix
/// and the `(*)` de-duplication marker. Returns `None` for blank lines.
fn parse_line(raw: &str) -> Option<Line> {
    let line = raw
        .trim_start_matches(['│', '├', '└', '─', ' '])
        .trim_end();
    let line = line.strip_suffix("(*)").unwrap_or(line).trim_end();
    if line.is_empty() {
        return None;
    }

    if let Some((pkg, rest)) = line.split_once(" feature \"") {
        let feature = rest.split('"').next().unwrap_or("").to_owned();
        return Some(Line::FeatureEdge { package: pkg.trim().to_owned(), feature });
    }

    let name = line.split_whitespace().next()?.to_owned();
    // `{f}` is one comma-joined token after `{p}`. `{p}` ends with `(path)` for path
    // deps and with the version for registry deps, so the feature list is what follows
    // the last `)` when one exists, else everything after the version token.
    let feature_text = match line.rfind(')') {
        Some(idx) => &line[idx + 1..],
        None => {
            let mut it = line.split_whitespace();
            it.next(); // name
            it.next(); // version
            it.next().unwrap_or("")
        }
    };
    let features = feature_text
        .trim()
        .split(',')
        .filter(|f| !f.is_empty())
        .map(str::to_owned)
        .collect();
    Some(Line::Package { name, features })
}

/// Refuses any feature-selecting token in a SHIP reading's argv. This is the invocation
/// guard in the portable form (see the module header for why the env form is
/// unbuildable): the ship number below it is a ship closure *because* this argv is
/// feature-clean.
fn assert_ship_invocation_purity(args: &[String]) {
    let dirty: Vec<&String> = args
        .iter()
        .filter(|a| {
            a.as_str() == "--features"
                || a.starts_with("--features=")
                || a.as_str() == "-F"
                || a.as_str() == "--all-features"
                || a.as_str() == "--no-default-features"
        })
        .collect();
    assert!(
        dirty.is_empty(),
        "the ship-closure gate was invoked under a feature selection ({dirty:?}); the \
         number below is not a ship closure. A ship reading resolves the manifests as \
         written -- feature-selected readings belong to the positive control, never to a \
         ship clause"
    );
}

/// The plan's exact command line for one package, plus `extra` args.
fn tree_args(package: &str, extra: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = [
        "tree",
        "-p",
        package,
        "-e",
        "features",
        "--edges",
        "normal,build",
        "--format",
        "{p} {f}",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    args.extend(extra.iter().map(|s| (*s).to_owned()));
    args
}

/// Spawns `cargo tree` with exactly `args` and returns stdout. Spawn or exit failure is
/// a panic, never a skip.
fn run_tree(args: &[String]) -> String {
    let out = Command::new(env!("CARGO"))
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap_or_else(|e| panic!("could not spawn cargo tree ({args:?}): {e}"));
    assert!(
        out.status.success(),
        "cargo tree ({args:?}) failed, so the closure gate has no subject:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A feature-selected reading — the positive control's path, never a ship clause's.
fn tree(package: &str, extra: &[&str]) -> String {
    run_tree(&tree_args(package, extra))
}

/// A ship reading: the purity guard runs over THE argv that is spawned — one
/// construction, one assertion, one spawn — so a feature flag smuggled into the ship
/// path cannot dodge it.
fn ship_tree(package: &str) -> String {
    let args = tree_args(package, &[]);
    assert_ship_invocation_purity(&args);
    run_tree(&args)
}

/// **The ship clauses.** For every D2 ship target: no `boyko-reflect` in the closure, no
/// enabled feature named `reflect` anywhere in it.
#[test]
fn ship_closures_contain_no_reflect() {
    for ship in SHIP_TARGETS {
        let output = ship_tree(ship);
        let mut crate_hits = Vec::new();
        let mut feature_hits = Vec::new();

        for raw in output.lines() {
            match parse_line(raw) {
                Some(Line::Package { name, features }) => {
                    if name == NEEDLE {
                        crate_hits.push(raw.trim().to_owned());
                    }
                    if features.iter().any(|f| f == "reflect") {
                        feature_hits.push(raw.trim().to_owned());
                    }
                }
                Some(Line::FeatureEdge { package, feature }) => {
                    if package == NEEDLE {
                        crate_hits.push(raw.trim().to_owned());
                    }
                    if feature == "reflect" {
                        feature_hits.push(raw.trim().to_owned());
                    }
                }
                None => {}
            }
        }

        assert!(
            crate_hits.is_empty(),
            "`{NEEDLE}` is IN the resolved closure of ship target `{ship}` -- the shipped \
             game would build and link the editor's reflection layer. Offending rows:\n  {}",
            crate_hits.join("\n  ")
        );
        assert!(
            feature_hits.is_empty(),
            "a feature named `reflect` is ENABLED inside ship target `{ship}`'s resolved \
             closure -- the manifests may be clean (G1's question) but this invocation \
             resolves it on, which is the half only a resolver read can see:\n  {}",
            feature_hits.join("\n  ")
        );
    }
}

/// **The positive control — runs every time, not once** (GATES G2). The same helper
/// pointed at `-p reflect-fixture --features reflect` must FIND `boyko-reflect`; without
/// this, a typo in the needle, a changed `--format`, or a `cargo tree` that failed and
/// returned empty output all read exactly like a pass. B.6's present-control discipline,
/// applied to the resolver instead of the linker.
#[test]
fn positive_control_finds_the_crate() {
    let output = tree("reflect-fixture", &["--features", "reflect"]);
    let found = output.lines().filter_map(parse_line).any(|l| match l {
        Line::Package { name, .. } => name == NEEDLE,
        Line::FeatureEdge { package, .. } => package == NEEDLE,
    });
    assert!(
        found,
        "NOT RESOLVED (closure census inert): `-p reflect-fixture --features reflect` \
         resolved WITHOUT `{NEEDLE}`, so this gate's needle names nothing and every ship \
         clause above is green for the wrong reason. Suspect the needle's spelling (the \
         PACKAGE name is hyphenated; `boyko_reflect` is the lib name and matches only the \
         on-disk path), the `--format` string, or a silently failing cargo tree."
    );
}
