//! Rung A2's trybuild goldens — the §3.3 diagnostics as COMPILER-VISIBLE contracts, spans on
//! the user's own tokens (the same split as a0_diagnostics: unit tests pin the TEXT, trybuild
//! pins the surfacing-through-rustc half).

#[test]
fn a2_diagnostics_land_on_the_users_tokens() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/query_takes_angle_brackets.rs");
    t.compile_fail("tests/ui/clauses_need_a_plugin.rs");
    t.compile_fail("tests/ui/duplicate_on_schedule.rs");
}
