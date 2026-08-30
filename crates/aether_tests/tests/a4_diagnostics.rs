//! Rung A4's trybuild goldens — the hierarchy's own hard faults as COMPILER-VISIBLE contracts.
//!
//! All seven are faults that ONLY exist because the chart is flattened at expansion time.
//! Three shapes, and each was measured to expand SILENTLY (or to emit a duplicate definition on
//! generated tokens) before its check landed:
//!
//! * **name collision** — flattening concatenates the path and the generated fn/predicate names
//!   are its snake_case collapse, so two chart positions can collide at either level
//!   (`A.BC`/`AB.C` → variant `ABC`; `AB`/`Ab` → `__state_chart_m__ab` and `in_a_b`). rustc
//!   reports these as "defined multiple times" pointing at code the user never wrote;
//! * **reachability-independent naming** — an `initial` no transition retargets through, and a
//!   handler an inner state shadows, are both skipped by the lazy per-leaf walk. Their names
//!   were never resolved at all, so a typo expanded clean;
//! * **dead states** (R2) — a state nothing targets compiles, registers a system, and can never
//!   run it. This is the only one of the three that is not a NAMING fault: nothing about it is
//!   ill-formed, which is exactly why it survived every check that came before.
//!
//! The R2 rung moved the flattening itself into `boyko_macros::state_chart!`, so these messages
//! are now raised there and reach the user through Aether's lowering. That the spans still land on
//! the author's own tokens — no macro-expansion backtrace, no caret on the `aether!` token — is
//! what these goldens measure, and MEASURED, **eight of the nine `machine_*` goldens that existed
//! before the move passed it byte-identical** (six here, three under `a3_diagnostics`, minus the
//! one overlap). The ninth is `machine_snake_collapse_collision`, and it changed for a reason the
//! move dictates rather than for a rendering artifact: the generated name lost its per-event
//! segment, and the caret moved onto the colliding STATE, which is the token actually at fault.

#[test]
fn a4_hierarchy_diagnostics_land_on_the_users_tokens() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/machine_flattened_name_collision.rs");
    t.compile_fail("tests/ui/machine_duplicate_sibling_state.rs");
    t.compile_fail("tests/ui/machine_snake_collapse_collision.rs");
    t.compile_fail("tests/ui/machine_initial_on_a_leaf.rs");
    t.compile_fail("tests/ui/machine_unreferenced_composite_initial.rs");
    t.compile_fail("tests/ui/machine_shadowed_handler_target.rs");
    t.compile_fail("tests/ui/machine_unreachable_state.rs");
}
