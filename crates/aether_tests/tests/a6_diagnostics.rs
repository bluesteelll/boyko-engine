//! Rung A6's trybuild goldens — the §3.7 diagnostics as COMPILER-VISIBLE contracts.
//!
//! §9's A6 gate names two (unknown material, unknown mesh); the other four are here for reasons
//! the two cannot cover:
//!
//! * `scene_casts_shadow_on_sky` is §3.7's third published message, and the only one of the three
//!   whose fault is a MISPLACED prop rather than a misspelled name — a different parse path.
//! * `scene_name_is_lowercase` pins §2's case rule for the newest value-producing construct; the
//!   `material` golden pins the same rule for a construct whose expansion is a one-liner, and a
//!   scene's is not.
//! * `scene_collides_with_a_material_fn` is the one §4 fault that rustc reports with NO user token
//!   (see that file's header) — the A5 material×material measurement, now widened across kinds
//!   because a `scene` also expands to a bare `pub fn`.
//! * `no_planned_construct_remains` is the successor to `planned_construct_names_its_rung`, which
//!   A6 obsoleted by landing the last planned construct.
//!
//! The a0/a2/a3/a4/a5 split holds: the `aether-lang` unit tests pin each message's TEXT, and these
//! pin the other half of §2's span rule — that the error surfaces THROUGH rustc at the user's own
//! tokens, in a real downstream crate. A message right in a unit test but anchored at
//! `Span::call_site()` passes there and fails here.
//!
//! Blessing discipline (the repo's trybuild rule): a `.stderr` is re-blessed ONLY after verifying
//! the error KIND is unchanged — the `token_use_after_submit_rejected` lesson (87 commits red
//! because a line moved and nobody re-blessed).

#[test]
fn a6_diagnostics_land_on_the_users_tokens() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/scene_unknown_material.rs");
    t.compile_fail("tests/ui/scene_unknown_mesh_binding.rs");
    t.compile_fail("tests/ui/scene_casts_shadow_on_sky.rs");
    t.compile_fail("tests/ui/scene_name_is_lowercase.rs");
    t.compile_fail("tests/ui/scene_collides_with_a_material_fn.rs");
    t.compile_fail("tests/ui/no_planned_construct_remains.rs");
}
