//! Phase X.A Wave 7 Step 7B — `compile_fail` acceptance tests for the
//! [`Query::for_each_chunk`] / [`Query::par_for_each_chunk`] trait gates.
//!
//! Each `.rs` file in `tests/compile_fail_chunk/` is compiled in isolation;
//! the matching `.stderr` baseline records the expected compiler diagnostic.
//! Regenerate the baselines after a rustc point release that shifts diagnostic
//! wording via:
//!
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo test -p boyko-ecs --test compile_fail_chunk
//! ```
//!
//! # Covered cases (per plan §11.2)
//!
//! Each case maps to a specific gate in the chunked-iter API:
//!
//! | File                                | Gate                                       |
//! |-------------------------------------|--------------------------------------------|
//! | `changed_filter_rejected.rs`        | `Changed<C>: !ArchetypalQueryFilter` (§3)  |
//! | `added_filter_rejected.rs`          | `Added<C>: !ArchetypalQueryFilter` (§3)    |
//! | `ref_data_rejected.rs`              | `Ref<'_, T>: !ChunkedQueryData` (§4.3)     |
//! | `mut_data_rejected.rs`              | `Mut<'_, T>: !ChunkedQueryData` (§4.3)     |
//! | `or_with_changed_rejected.rs`       | `Or<(W, Changed<C>)>: !Archetypal` (§5.2)  |
//!
//! Plus one `#[should_panic]` runtime test below for the SystemParam aliasing
//! path (§11.2 row 6 — `Query<(&mut T, &mut T), ()>::for_each_chunk` inside a
//! `Schedule`). That case is a **runtime** `boyko-B0002` panic from
//! `FilteredAccessSet::add_component_write`, not a compile error: the direct
//! `EcsMaster::query::<(&mut T, &mut T)>()` API bypasses `FilteredAccessSet`
//! entirely (verified per plan §11.2 / critic Round 1 N4), so a trybuild
//! `.rs` file cannot exercise the gate. The test MUST be a `Schedule`-driven
//! `#[should_panic]` runtime assertion to validate the gate.
//!
//! Gated behind `#[cfg(not(miri))]` because Miri does not have the trybuild
//! driver wired and the test would not compile under Miri's restricted env.
//! Mirrors the pattern in `tests/par_iter_captures_commands_fails.rs`.

#![cfg(not(miri))]

#[test]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail_chunk/*.rs");
}

// ── Runtime aliasing test (plan §11.2 row 6) ────────────────────────────────
//
// The aliasing gate is enforced at **runtime** by `FilteredAccessSet`'s
// `add_component_write` returning `Err(ConflictKind::ComponentWriteVsWrite)`
// on the second `&mut T` registration. The error surfaces as a B0002
// `intra_system_conflict_panic` BEFORE the system body runs.
//
// Per plan §11.2 critic Round 1 N4: the direct `EcsMaster::query::<(&mut T,
// &mut T), ()>()` API NEVER calls `init_access`; it calls only
// `QueryDataState::new` → `init_state`. The aliasing check therefore only
// fires when the query is driven through the SystemParam pipeline — i.e.,
// inside a system body invoked via `EcsMaster::run_closure_once` or
// `Schedule::run`. This test uses `run_closure_once` (mirrors the pattern
// in `tests/query_dsl_smoke.rs::intra_system_conflict_query_mut_query_ref_*`).

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

/// Distinct component type per integration-test file (the global registry
/// asserts equal `Layout` on each `register_layout` call — collisions across
/// integration tests would panic on the SECOND test's first call).
///
/// Slot 481 is between the existing Wave 6 / Wave 7 chunk-test allocations
/// (≤ 479) and the archetype_bundle miri tests (480, 482). The Wave 7
/// `parallel_*` tests reserve up to 472; the proptest harness reserves
/// 473-479. Slot 481 is the closest free slot above and is verified free
/// against the existing crate-wide allocation map.
const COMP_ALIAS_T: ComponentId = ComponentId(481);

#[repr(C)]
#[derive(Clone, Copy)]
struct AliasT(u32);

impl Component for AliasT {
    fn component_id() -> ComponentId {
        COMP_ALIAS_T
    }
}

/// `Query<(&mut AliasT, &mut AliasT)>::for_each_chunk(...)` must panic at
/// `boyko-B0002` (intra-system component write-vs-write conflict). The
/// `for_each_chunk` body is irrelevant — the panic fires inside
/// `FilteredAccessSet::add_component_write` during the system's
/// `init_access` walk, before the closure body runs.
///
/// # Why `run_closure_once` (not direct API)
///
/// Direct `EcsMaster::query::<(&mut AliasT, &mut AliasT), ()>()` bypasses
/// `FilteredAccessSet` and DOES NOT panic — see plan §11.2 critic Round 1
/// N4. The test MUST use the SystemParam pipeline to validate the gate;
/// otherwise a regression that silently lets `(&mut T, &mut T)` through the
/// chunk path could not be detected.
///
/// # Why this isn't a `compile_fail/` file
///
/// `boyko-B0002` is a RUNTIME panic — `compile_fail/` requires a compiler
/// diagnostic. Bevy's equivalent gate is also runtime; the plan §11.2 table
/// row 6 reflects that explicitly. The 5 `.rs` files under
/// `compile_fail_chunk/` cover the 5 compile-time gates (Changed, Added,
/// Ref, Mut, Or-with-Changed); this `#[should_panic]` test covers the 6th
/// runtime gate.
#[test]
#[should_panic(expected = "boyko-B0002")]
fn aliasing_query_mut_t_mut_t_rejected_in_systemparam_path() {
    // Register `AliasT`'s `Layout` in the global registry — required by the
    // `archetype_bundle` precondition asserted at `component_pool_bundle.rs:62`
    // ("Component ID ... not registered in layout registry"). Without this,
    // the `create_archetype` call below would panic on layout-missing BEFORE
    // the aliasing gate is reached.
    component_registry::register_layout::<AliasT>(COMP_ALIAS_T.0);

    let mut ecs = EcsMaster::new();
    // Lazily register AliasT via create_archetype — the QueryData state
    // would also register it, but priming here matches the pattern in
    // `query_dsl_smoke.rs::intra_system_conflict_query_mut_query_ref_*`.
    let _arch = ecs.create_archetype(&[AliasT::component_id()]);

    // `(Query<&mut AliasT>, Query<&mut AliasT>)` is the tuple SystemParam
    // under test. Same component twice ⇒ two `add_component_write` calls
    // on the same `ComponentId` during the tuple `init_access` walk; the
    // second call returns `Err(ConflictKind::ComponentWriteVsWrite)` and
    // the param wrapper panics with `boyko-B0002` before the closure body
    // runs. Mirrors the gate validation pattern from
    // `query_dsl_smoke.rs::intra_system_conflict_query_mut_query_ref_same_component_panics`.
    ecs.run_closure_once(
        |(mut q1, _q2): (
            Query<'_, '_, &mut AliasT>,
            Query<'_, '_, &mut AliasT>,
        )| {
            // Unreachable — `init_access` panics before any closure body runs.
            q1.for_each_chunk(|_slice: &mut [AliasT]| {});
        },
    );
}
