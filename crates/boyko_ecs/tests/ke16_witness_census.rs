//! KE16 witness census — every measured artifact says which build produced it.
//!
//! ## The obligation
//!
//! `KE16-DESIGN.md` §4: "Every KE16 bench and the two red-first gates print [`ke16_variant()`] on
//! entry and, when the env var `KE16_EXPECT` is set, **panic on mismatch**. The tester's protocol
//! sets `KE16_EXPECT` on every run, so 'features not actually enabled' cannot produce a number."
//! `KE16-DESIGN-MEASUREMENT.md` §5 shape 3 records the same thing from the other side: a number
//! whose banner does not match the baseline name is a refused reading.
//!
//! ## Why a census rather than a review note
//!
//! The obligation is per FILE and is invisible at runtime unless someone sets the env var. A
//! reviewer's `grep` proves it for the checkout it was run on; nothing keeps it true through the
//! next edit of a bench that is not part of the default test run at all. This file is the same
//! mechanical shape the repository already uses for properties that live in source rather than in
//! behaviour (`tests/ignore_reasons_census.rs`, `boyko_physics`'s `w4_rhi_vulkan_has_no_x8`), and
//! it fails loudly if one of the five artifacts loses its witness.
//!
//! It lives in `boyko_ecs` — not in the root package — because the KE16 consumer axis owns four
//! of the five files it reads and the fifth (the pool's bench) is assigned to the same axis; the
//! root package's census slot is for repository-wide properties. It reads sources through
//! `CARGO_MANIFEST_DIR/..`, so it is checkout-relative and needs no fixture.
//!
//! ## What a failure means
//!
//! Not "the code is wrong" — "a measurement taken with this artifact cannot be attributed to a
//! configuration". The whole KE16 tournament is a comparison of numbers across builds; an
//! unlabelled number is not a weak result, it is not a result.
//!
//! The census is deleted with the features when the verdict lands (`KE16-DESIGN.md` §4,
//! "Removal after the verdict").

use std::fs;
use std::path::PathBuf;

/// The five artifacts `KE16-DESIGN.md` §4 names: three criterion benches and the two red-first
/// occupancy gates. Paths are workspace-relative.
const WITNESS_BEARING: &[&str] = &[
    "crates/boyko_threadpool/benches/ke16_nested_scope.rs",
    "crates/boyko_ecs/benches/ke16_par_iter_in_system.rs",
    "crates/boyko_physics/benches/ke16_solve_in_system.rs",
    "crates/boyko_threadpool/tests/ke16_nested_scope_occupancy.rs",
    "crates/boyko_ecs/tests/ke16_occupancy_gate.rs",
];

fn workspace_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `<root>/crates/boyko_ecs`.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Reads a workspace-relative source file, failing the test rather than skipping when it is
/// absent: a missing file is how a census turns into a vacuous pass.
fn read(rel: &str) -> String {
    let path = workspace_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "KE16 witness census cannot read {rel} ({e}). Either the file was renamed — in which \
             case this list is stale and the protocol's file set changed under it — or it was \
             deleted, and a measured artifact vanished with it"
        )
    })
}

/// Lines that are not whole-line comments. The census asks about CODE: every file below also
/// discusses `ke16_check_expected_variant` in its doc comment, and a doc mention is exactly what
/// a file that lost the call would still have.
fn code_lines(src: &str) -> impl Iterator<Item = &str> {
    src.lines().filter(|l| {
        let t = l.trim_start();
        !t.starts_with("//") && !t.starts_with("/*") && !t.starts_with('*')
    })
}

/// The census's own calibration: a file that only TALKS about the witness must not satisfy it.
///
/// Every one of the five artifacts documents `ke16_check_expected_variant` and the banner in its
/// header, so if the scan counted comment lines it would pass on a file that had lost the call —
/// the exact false green this census exists to prevent, and one that would never be noticed,
/// because a census that cannot fail reports the same word as one that cannot be broken. The
/// fixture below is the shape a stripped file would leave behind.
#[test]
fn the_census_scan_does_not_count_comment_only_mentions() {
    let doc_only = concat!(
        "//! This file calls ke16_check_expected_variant() on entry and prints\n",
        "//! println!(\"KE16 variant: ...\") before every row.\n",
        "  // ke16_check_expected_variant();\n",
        "  /* println!(\"KE16 variant: {}\", v); */\n",
        "fn main() {}\n",
    );

    assert_eq!(
        code_lines(doc_only).filter(|l| l.contains("ke16_check_expected_variant(")).count(),
        0,
        "the scan counts commented-out or documented mentions of the expect check, so a file that \
         lost the CALL would still pass the census"
    );
    assert_eq!(
        code_lines(doc_only).filter(|l| l.contains("println!") && l.contains("KE16 variant")).count(),
        0,
        "the scan counts a documented banner, so a file that stopped printing one would still \
         pass the census"
    );
}

/// Every witness-bearing artifact calls the pool crate's `KE16_EXPECT` check.
///
/// The check itself is one function in `boyko_threadpool` (`lib.rs`'s `ke16_check_expected_variant`),
/// so this is the whole of the "panic on mismatch" obligation: a file that calls it cannot produce
/// a number under a mislabelled `--features` line, and a file that does not call it can.
#[test]
fn every_ke16_measured_artifact_calls_the_expect_check() {
    for rel in WITNESS_BEARING {
        let src = read(rel);
        let calls = code_lines(&src).filter(|l| l.contains("ke16_check_expected_variant(")).count();
        assert!(
            calls >= 1,
            "{rel} does not call `ke16_check_expected_variant()` in code (only, at most, in a \
             comment). A run of this artifact under `--features boyko-threadpool/ke16-*` would \
             certify the configuration it was ASKED for instead of the one it was BUILT as \
             (KE16-DESIGN.md §4; KE16-DESIGN-MEASUREMENT.md §5 shape 3)"
        );
    }
}

/// Every witness-bearing artifact prints the variant banner.
///
/// The panic alone is not enough: with `KE16_EXPECT` unset — which is how a casual `cargo bench`
/// runs — the check is silent, and the banner is then the only record tying a criterion median to
/// a build. The spelling is pinned to `KE16 variant` so one `grep` over a protocol log collects
/// every row.
#[test]
fn every_ke16_measured_artifact_prints_the_variant_banner() {
    for rel in WITNESS_BEARING {
        let src = read(rel);
        let banners = code_lines(&src)
            .filter(|l| l.contains("println!") && l.contains("KE16 variant"))
            .count();
        assert!(
            banners >= 1,
            "{rel} never prints the `KE16 variant:` banner in code. With `KE16_EXPECT` unset the \
             expect-check is silent, so this artifact's numbers would carry no record of which \
             build produced them"
        );
    }
}

/// The App-7 rows exist under the name the design gives them.
///
/// `KE16-DESIGN-APP.md` §6 specifies a `ke16_park_timeout` group with rows `park_timeout_50us`,
/// `park_timeout_1ms` and `park_timeout_2ms`; §6b makes those medians load-bearing for how the
/// whole campaign reads its park-driven latencies (measured 15 296 us unguarded against 1 021 us
/// under the host's `timeBeginPeriod(1)`). A row silently renamed is a row the protocol's
/// `--bench`-name filter stops selecting, and criterion reports no error for a filter that
/// matches nothing.
#[test]
fn the_app7_park_timeout_rows_exist_by_name() {
    let rel = "crates/boyko_threadpool/benches/ke16_nested_scope.rs";
    let src = read(rel);
    for row in ["ke16_park_timeout", "park_timeout_50us", "park_timeout_1ms", "park_timeout_2ms"] {
        assert!(
            code_lines(&src).any(|l| l.contains(row)),
            "{rel} carries no `{row}` in code: App-7's timed-wait resolution rows \
             (KE16-DESIGN-APP.md §6) cannot be selected by name"
        );
    }
}

/// The App-10 reference row exists by name.
///
/// `KE16-DESIGN-APP.md` §11 makes `bench_thread_install_Wminus1` MANDATORY and explains why the
/// acceptance line cannot be read without it: the bench-thread route's structural advantage is at
/// most `(W+1)/W`, and comparing against the W+1-lane row would loosen the line by up to 6.25 % at
/// W=16 — "a pass inside that margin is not evidence the fix is complete".
#[test]
fn the_app10_wminus1_reference_row_exists_by_name() {
    let rel = "crates/boyko_physics/benches/ke16_solve_in_system.rs";
    let src = read(rel);
    assert!(
        code_lines(&src).any(|l| l.contains("bench_thread_install_Wminus1")),
        "{rel} carries no `bench_thread_install_Wminus1` row in code: the acceptance line of \
         KE16-DESIGN-APP.md §11 has no W-lane reference to be measured against"
    );
}
