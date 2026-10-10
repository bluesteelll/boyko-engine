//! Rung A3's trybuild goldens — the §3.5 machine diagnostics as COMPILER-VISIBLE contracts,
//! spans on the user's own tokens (the a0/a2 split: unit tests pin the TEXT, trybuild pins the
//! surfacing-through-rustc half — including the duplicate-handler note's SECOND span).

#[test]
fn a3_diagnostics_land_on_the_users_tokens() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/machine_composite_target_without_initial.rs");
    t.compile_fail("tests/ui/machine_duplicate_handler.rs");
    t.compile_fail("tests/ui/machine_unknown_initial_did_you_mean.rs");
}
