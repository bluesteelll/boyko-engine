//! Rung A0's trybuild goldens — the §3.1 diagnostics as COMPILER-VISIBLE contracts.
//!
//! The `aether-lang` unit tests already assert every message's TEXT (the tighter pin); what only
//! trybuild can assert is the other half of the §2 span rule: that the error surfaces THROUGH
//! rustc at the user's own tokens, in a real downstream crate, with the derive and the engine in
//! scope. A message that is right in a unit test but anchored at `Span::call_site()` would pass
//! there and fail here.
//!
//! Blessing discipline (the repo's trybuild rule): a `.stderr` is re-blessed ONLY after verifying
//! the error KIND is unchanged — the `token_use_after_submit_rejected` lesson (87 commits red
//! because a line moved and nobody re-blessed).

#[test]
fn a0_diagnostics_land_on_the_users_tokens() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/unknown_construct.rs");
    t.compile_fail("tests/ui/planned_construct_names_its_rung.rs");
    t.compile_fail("tests/ui/lowercase_component.rs");
    t.compile_fail("tests/ui/duplicate_hook.rs");
    t.compile_fail("tests/ui/bad_tag_modifier.rs");
    t.compile_fail("tests/ui/tag_missing_semicolon.rs");
    // Rung A1's two goldens (§9 table: arity cap, participant syntax).
    t.compile_fail("tests/ui/bundle_arity_cap.rs");
    t.compile_fail("tests/ui/participant_without_context.rs");
}
