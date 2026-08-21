//! **Reflection GATES G4 — non-vacuity inside each leg, not only around it.**
//!
//! The CI `reflect-on` job sets `BOYKO_REFLECT_LEG=reflect-on` beside its `--features
//! reflect-fixture/reflect`. This test is the pairing's enforcement: whenever the
//! variable names a leg, the binary it runs in must have been built with the feature —
//! and it reds NAMING THE LEG if not, so dropping `--features` from the job cannot turn
//! the leg's reflect tests into no-ops that report green (`gj1_flag_cost`'s `flags_on()`
//! precedent: *"leg A asked for the profiler and did not get it … every number below
//! would be the logger's alone under a joint name"*).
//!
//! # Why an env marker and not a plain `cfg!` assertion — measured
//!
//! A test that unconditionally asserts `cfg!(feature = "reflect")` reds every plain
//! workspace sweep (`cargo test --workspace` builds this package feature-off, which is
//! the SHIP configuration and correct). And the reverse — detecting the job's feature
//! selection from inside the process — is impossible: an outer `--features` selection is
//! unobservable from a test binary (measured at G2, 2026-08-21: `CARGO_ENCODED_ARGS`
//! does not exist, and the test-process env is byte-identical under the selection). So
//! the leg must NAME itself, and the name travels in `BOYKO_REFLECT_LEG`.
//! `tests/reflect_ci_coverage.rs` (root package) pins the variable and the flag together
//! in the workflow file, so the pair cannot drift apart silently.

/// The leg self-check. Plain sweeps (variable unset) assert nothing — feature-off is the
/// ship configuration there, not a defect.
#[test]
fn the_leg_that_names_itself_carries_the_feature() {
    let Some(leg) = std::env::var_os("BOYKO_REFLECT_LEG") else {
        return;
    };
    let leg = leg.to_string_lossy().into_owned();
    // The compile-time constancy of `cfg!` here is the DESIGN, not an accident the lint
    // caught: the runtime half of the check is the env marker above, and the pair
    // (constant build configuration) x (runtime leg name) is exactly what makes a
    // dropped `--features` visible. The binding keeps the assertion's subject a value.
    let leg_built_with_reflect = cfg!(feature = "reflect");
    assert!(
        leg_built_with_reflect,
        "leg `{leg}` asked for the reflect feature and did not get it: BOYKO_REFLECT_LEG \
         is set, so this binary belongs to a reflect-ON leg, but \
         cfg!(feature = \"reflect\") is false -- every reflect test in this selection is \
         currently a no-op wearing a green name. Restore `--features \
         reflect-fixture/reflect` on the job (GATES G4 item 1)."
    );
}
