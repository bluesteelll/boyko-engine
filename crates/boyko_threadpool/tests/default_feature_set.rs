//! The pool ships with **no** cargo feature on: `default = []`, pinned mechanically.
//!
//! This assertion used to live in `tests/ke16_feature_scheme_census.rs::the_default_feature_set_is_empty`,
//! alongside the census of the eleven `ke16-*` campaign switches. Step App removed those switches,
//! and the census went with them — but this row did not belong to the campaign. It guards a
//! property of the SHIPPED crate: an untouched `cargo build` measures and runs the configuration
//! the engine actually ships, and every switch is opt-in.
//!
//! Two rows survive the campaign, and the pin is not redundant for either:
//!
//! * `tb-neg-m2w` is a deliberate Tree-Borrows **negative control** — it swaps in a body that
//!   commits UB on purpose. Putting it in `default` is caught loudly today, but by `src/lib.rs`'s
//!   `#[cfg(all(feature = "tb-neg-m2w", not(miri)))] compile_error!`, i.e. by a REFUSAL TO BUILD
//!   rather than by a test. That refusal is a property of that one feature; it does not generalise.
//! * `scheduler-trace` has no such refusal. Defaulting it on would arm tracing in every build and
//!   every measurement, and nothing else in the tree would say so.
//!
//! Hand-rolled parse rather than a TOML dependency: the crate has none, and the property is one
//! line of one table.

use std::fs;
use std::path::{Path, PathBuf};

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The dependency list of one row of the `[features]` table, by row name.
///
/// Comment lines and blank lines are skipped; a trailing `# comment` on a row is not part of its
/// value. Returns `None` when the table has no such row, which the caller reports as its own
/// failure rather than silently passing.
fn feature_row_deps(manifest: &str, want: &str) -> Option<Vec<String>> {
    let mut in_features = false;

    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_features = trimmed == "[features]";
            continue;
        }
        if !in_features || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((name, value)) = trimmed.split_once('=') else {
            continue;
        };
        if name.trim() != want {
            continue;
        }
        let value = value.split('#').next().unwrap_or("").trim();
        return Some(
            value
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .map(|d| d.trim().trim_matches('"').to_string())
                .filter(|d| !d.is_empty())
                .collect(),
        );
    }
    None
}

#[test]
fn the_default_feature_set_is_empty() {
    let manifest = read(&crate_root().join("Cargo.toml"));
    let deps = feature_row_deps(&manifest, "default")
        .expect("invariant: Cargo.toml declares a `default` feature row");

    assert!(
        deps.is_empty(),
        "`default` enables {deps:?}; the pool ships with every switch opt-in, so an untouched \
         build is the configuration that is measured and shipped"
    );
}

/// The parser must be able to SEE a non-empty default, or the pin above is green from emptiness.
///
/// The row it reads is the real manifest's, so a rename or a reformat that made `default`
/// unfindable would fail `the_default_feature_set_is_empty` at its `expect` rather than pass it —
/// but nothing there proves a POPULATED row would be reported as populated. This does.
#[test]
fn the_parser_reports_a_populated_default_row() {
    let manifest = "[features]\ndefault = [\"scheduler-trace\"]\nscheduler-trace = []\n";
    assert_eq!(
        feature_row_deps(manifest, "default"),
        Some(vec!["scheduler-trace".to_string()])
    );
    assert_eq!(feature_row_deps("[features]\n", "default"), None);
}
