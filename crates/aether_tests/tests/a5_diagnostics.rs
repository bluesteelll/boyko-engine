//! Rung A5's trybuild goldens — the §3.6 diagnostics as COMPILER-VISIBLE contracts (the §9 gate
//! names two: color arity and the case rule; the unknown-key list is the third because it is the
//! only one of the three whose failure mode is SILENT — a key Aether merely ignored would ship a
//! material whose roughness the author believed they had set).
//!
//! The fourth, `material_duplicate_name`, is here for the opposite reason: it is the one §3.6
//! fault rustc reports with NO user token in it (see that file's header), so only a golden can
//! hold both of Aether's spans in place.
//!
//! The a0/a2/a3/a4 split holds: the `aether-lang` unit tests pin each message's TEXT, and these
//! pin the other half of §2's span rule — that the error surfaces THROUGH rustc at the user's own
//! tokens, in a real downstream crate. A message right in a unit test but anchored at
//! `Span::call_site()` passes there and fails here.
//!
//! Blessing discipline (the repo's trybuild rule): a `.stderr` is re-blessed ONLY after verifying
//! the error KIND is unchanged — the `token_use_after_submit_rejected` lesson (87 commits red
//! because a line moved and nobody re-blessed).

#[test]
fn a5_diagnostics_land_on_the_users_tokens() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/material_color_arity.rs");
    t.compile_fail("tests/ui/material_name_is_lowercase.rs");
    t.compile_fail("tests/ui/material_unknown_key.rs");
    t.compile_fail("tests/ui/material_duplicate_name.rs");
}
