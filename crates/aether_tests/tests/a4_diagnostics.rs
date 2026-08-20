//! Rung A4's trybuild goldens — the hierarchy's own hard faults as COMPILER-VISIBLE contracts.
//!
//! All six are faults that ONLY exist because §5.4 flattens the chart at expansion time.
//! Two shapes, and each was measured to expand SILENTLY (or to emit a duplicate definition on
//! generated tokens) before its check landed:
//!
//! * **name collision** — flattening concatenates the path and the generated fn/predicate names
//!   are its snake_case collapse, so two chart positions can collide at either level
//!   (`A.BC`/`AB.C` → variant `ABC`; `AB`/`A_b` → `__aether_m__a_b__e` and `in_a_b`). rustc
//!   reports these as "defined multiple times" pointing at code the user never wrote;
//! * **reachability-independent naming** — an `initial` no transition retargets through, and a
//!   handler an inner state shadows, are both skipped by the lazy per-leaf walk. Their names
//!   were never resolved at all, so a typo expanded clean.

#[test]
fn a4_hierarchy_diagnostics_land_on_the_users_tokens() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/machine_flattened_name_collision.rs");
    t.compile_fail("tests/ui/machine_duplicate_sibling_state.rs");
    t.compile_fail("tests/ui/machine_snake_collapse_collision.rs");
    t.compile_fail("tests/ui/machine_initial_on_a_leaf.rs");
    t.compile_fail("tests/ui/machine_unreferenced_composite_initial.rs");
    t.compile_fail("tests/ui/machine_shadowed_handler_target.rs");
}
