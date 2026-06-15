//! Phase 12.5 Track B integration tests for the direct query API
//! (`EcsMaster::query<D, F>()`) and the `QueryView<'w, D, F>` handle.
//!
//! See `docs/PHASE-12.5-QUERY-OPTIMIZATIONS-PLAN.md` §11 for the test
//! catalogue. This file covers the integration / smoke surface; the
//! per-module unit tests live alongside the impls in
//! `crates/boyko_ecs/src/ecs/core/iters/query/`.
//!
//! # Component-slot range
//!
//! 320..=339 — disjoint from existing Phase 8b query tests (480-509),
//! Phase 8.5 (290-309), Phase 10 (260-289), and Phase 11 (411-413).

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::With;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

// ── Test component fixtures ──────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
struct Position {
    x: f32,
    #[allow(dead_code)]
    y: f32,
    #[allow(dead_code)]
    z: f32,
}

impl Component for Position {
    fn component_id() -> ComponentId {
        ComponentId(320)
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Velocity {
    #[allow(dead_code)]
    x: f32,
}

impl Component for Velocity {
    fn component_id() -> ComponentId {
        ComponentId(321)
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Tag(#[allow(dead_code)] u8);

impl Component for Tag {
    fn component_id() -> ComponentId {
        ComponentId(322)
    }
}

fn register_test_components() {
    register_layout::<Position>(Position::component_id().0);
    register_layout::<Velocity>(Velocity::component_id().0);
    register_layout::<Tag>(Tag::component_id().0);
}

fn spawn_pos(ecs: &mut EcsMaster, arch_id: boyko_ecs::ecs::identifiers::primitives::ArchetypeId, x: f32) {
    let p = Position { x, y: 0.0, z: 0.0 };
    // SAFETY: `Position` is `#[repr(C)]` POD; reading its bytes produces
    //   a valid byte slice for the call's duration.
    let bytes = unsafe {
        std::slice::from_raw_parts(
            &p as *const Position as *const u8,
            std::mem::size_of::<Position>(),
        )
    };
    ecs.create_entity(arch_id, &[(Position::component_id(), bytes)])
        .expect("spawn must succeed");
}

// ── Wave B2 — direct query API smoke tests ──────────────────────────────

/// `query_view_iter_smoke` (plan §11.1) — `EcsMaster::query::<&P, ()>().iter()`
/// yields every entity in every matched archetype.
#[test]
fn query_view_iter_smoke() {
    register_test_components();
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[Position::component_id()]);
    spawn_pos(&mut ecs, arch, 1.0);
    spawn_pos(&mut ecs, arch, 2.0);
    spawn_pos(&mut ecs, arch, 4.0);

    let view = ecs.query::<&Position, ()>();
    let sum: f32 = view.iter().map(|p: &Position| p.x).sum();
    assert!(
        (sum - 7.0).abs() < f32::EPSILON,
        "expected sum=7.0, got {}",
        sum,
    );
}

/// `query_view_iter_mut_smoke` — `iter_mut` writes are observable across
/// subsequent queries.
#[test]
fn query_view_iter_mut_smoke() {
    register_test_components();
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[Position::component_id()]);
    spawn_pos(&mut ecs, arch, 1.0);
    spawn_pos(&mut ecs, arch, 2.0);

    {
        let mut view = ecs.query::<&mut Position, ()>();
        for p in view.iter_mut() {
            p.x *= 10.0;
        }
    }

    let view = ecs.query::<&Position, ()>();
    let collected: Vec<f32> = view.iter().map(|p: &Position| p.x).collect();
    collected.iter().for_each(|x| assert!(*x == 10.0 || *x == 20.0));
    assert_eq!(collected.len(), 2);
}

/// `query_warm_path_cache_hit` (plan §11.1) — two successive `query::<D, F>()`
/// calls share the same cached `QueryDataState` pointer.
#[test]
fn query_warm_path_cache_hit() {
    register_test_components();
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[Position::component_id()]);
    spawn_pos(&mut ecs, arch, 1.0);

    // First call — cold init, populates the slot.
    let count1 = ecs.query::<&Position, ()>().archetype_count();
    assert_eq!(count1, 1);

    // Second call — must hit the cache. Pointer identity is not observable
    // through the public API; the smoke check verifies the cached state
    // returns the same archetype set as the cold-init call.
    let count2 = ecs.query::<&Position, ()>().archetype_count();
    assert_eq!(count2, count1);
}

/// Tuple data smoke — `Query<(&Position, &Velocity), ()>` against a
/// (Position, Velocity) archetype.
#[test]
fn query_view_tuple_data_smoke() {
    register_test_components();
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[Position::component_id(), Velocity::component_id()]);

    let p = Position { x: 7.0, y: 0.0, z: 0.0 };
    let v = Velocity { x: 0.0 };
    let p_bytes = unsafe {
        std::slice::from_raw_parts(
            &p as *const Position as *const u8,
            std::mem::size_of::<Position>(),
        )
    };
    let v_bytes = unsafe {
        std::slice::from_raw_parts(
            &v as *const Velocity as *const u8,
            std::mem::size_of::<Velocity>(),
        )
    };
    ecs.create_entity(arch, &[
        (Position::component_id(), p_bytes),
        (Velocity::component_id(), v_bytes),
    ]).expect("spawn must succeed");

    let view = ecs.query::<(&Position, &Velocity), ()>();
    let collected: Vec<f32> = view.iter().map(|(p, _v): (&Position, &Velocity)| p.x).collect();
    assert_eq!(collected, vec![7.0]);
}

/// Filter smoke — `Query<&Position, With<Tag>>` matches only Tag-bearing
/// archetypes.
#[test]
fn query_view_with_filter_smoke() {
    register_test_components();
    let mut ecs = EcsMaster::new();
    let arch_p = ecs.create_archetype(&[Position::component_id()]);
    let arch_pt = ecs.create_archetype(&[Position::component_id(), Tag::component_id()]);

    spawn_pos(&mut ecs, arch_p, 1.0);
    // Spawn a (Position, Tag) row.
    let p = Position { x: 42.0, y: 0.0, z: 0.0 };
    let t = Tag(0);
    let p_bytes = unsafe {
        std::slice::from_raw_parts(
            &p as *const Position as *const u8,
            std::mem::size_of::<Position>(),
        )
    };
    let t_bytes = unsafe {
        std::slice::from_raw_parts(
            &t as *const Tag as *const u8,
            std::mem::size_of::<Tag>(),
        )
    };
    ecs.create_entity(arch_pt, &[
        (Position::component_id(), p_bytes),
        (Tag::component_id(), t_bytes),
    ]).expect("spawn must succeed");

    let view = ecs.query::<&Position, With<Tag>>();
    let collected: Vec<f32> = view.iter().map(|p: &Position| p.x).collect();
    assert_eq!(collected, vec![42.0], "With<Tag> must match only the Tag-bearing row");
}

// ── Wave B4 — change-detection reject surface ────────────────────────────
//
// W4 / I-NEW-4 / QV11: `EcsMaster::query<D, F>()` with a change-detection
// `D`/`F` is now a COMPILE error (the W4 `const`-assert), not a runtime panic.
// The former `#[should_panic]` smoke tests can no longer compile, so they were
// converted to `trybuild` `compile_fail` fixtures in
// `tests/query_change_detection_compile_fail/` (harness
// `tests/query_change_detection_compile_fail.rs`).

// ── NCD const surface — compile-only assertions ─────────────────────────

/// `query_data_needs_change_detection_const_correct_for_leaves` —
/// every leaf `QueryData` impl declares the correct NCD value.
#[test]
fn query_data_needs_change_detection_const_correct_for_leaves() {
    use boyko_ecs::ecs::core::iters::query::data::QueryData;
    use boyko_ecs::ecs::core::iters::query::{Mut, Ref};

    const { assert!(!<&Position as QueryData>::NEEDS_CHANGE_DETECTION) };
    const { assert!(!<&mut Position as QueryData>::NEEDS_CHANGE_DETECTION) };
    const { assert!(<Ref<'_, Position> as QueryData>::NEEDS_CHANGE_DETECTION) };
    const { assert!(<Mut<'_, Position> as QueryData>::NEEDS_CHANGE_DETECTION) };
    const { assert!(!<() as QueryData>::NEEDS_CHANGE_DETECTION) };
}

/// `query_filter_needs_change_detection_const_correct_for_leaves` —
/// every leaf `QueryFilter` impl declares the correct NCD value.
#[test]
fn query_filter_needs_change_detection_const_correct_for_leaves() {
    use boyko_ecs::ecs::core::iters::query::filter::QueryFilter;
    use boyko_ecs::ecs::core::iters::query::{Added, Changed, With, Without};

    const { assert!(!<() as QueryFilter>::NEEDS_CHANGE_DETECTION) };
    const { assert!(!<With<Position> as QueryFilter>::NEEDS_CHANGE_DETECTION) };
    const { assert!(!<Without<Position> as QueryFilter>::NEEDS_CHANGE_DETECTION) };
    const { assert!(<Added<Position> as QueryFilter>::NEEDS_CHANGE_DETECTION) };
    const { assert!(<Changed<Position> as QueryFilter>::NEEDS_CHANGE_DETECTION) };
}

/// Tuple propagation (NCD3) — any tuple element with NCD = true propagates.
#[test]
fn query_data_needs_change_detection_tuple_propagation() {
    use boyko_ecs::ecs::core::iters::query::data::QueryData;
    use boyko_ecs::ecs::core::iters::query::Ref;

    const { assert!(!<(&Position, &Velocity) as QueryData>::NEEDS_CHANGE_DETECTION) };
    const { assert!(<(Ref<'_, Position>, &Velocity) as QueryData>::NEEDS_CHANGE_DETECTION) };
    const { assert!(<(&Velocity, Ref<'_, Position>) as QueryData>::NEEDS_CHANGE_DETECTION) };
}

#[test]
fn query_filter_needs_change_detection_tuple_propagation() {
    use boyko_ecs::ecs::core::iters::query::filter::QueryFilter;
    use boyko_ecs::ecs::core::iters::query::{Added, With};

    const { assert!(!<(With<Position>, With<Velocity>) as QueryFilter>::NEEDS_CHANGE_DETECTION) };
    const { assert!(<(With<Position>, Added<Velocity>) as QueryFilter>::NEEDS_CHANGE_DETECTION) };
}

// ── Wave A — QueryTypeId distinct-pair test (I2) ──────────────────────────

/// `query_type_id_distinct_for_distinct_DF_pairs` (plan §11.1 I2) — two
/// distinct `(D, F)` pairs receive distinct `QueryTypeId`s. Phase 8.5's
/// `BundleTypeId` pattern is verbatim, so the LTO regression risk is the
/// same; this is the per-(D, F) confirmation.
#[test]
fn query_type_id_distinct_for_distinct_df_pairs() {
    use boyko_ecs::ecs::core::iters::query::query_type_registry::QueryTypeKey;

    let id_pos_unit = <(&Position, ()) as QueryTypeKey>::query_type_id();
    let id_vel_unit = <(&Velocity, ()) as QueryTypeKey>::query_type_id();
    let id_pos_with = <(&Position, With<Tag>) as QueryTypeKey>::query_type_id();

    assert_ne!(
        id_pos_unit, id_vel_unit,
        "distinct D types must yield distinct QueryTypeIds"
    );
    assert_ne!(
        id_pos_unit, id_pos_with,
        "distinct F types must yield distinct QueryTypeIds"
    );

    // Same (D, F) pair must return the same id (cache hit on the
    // per-impl OnceLock cell).
    let id_pos_unit_again = <(&Position, ()) as QueryTypeKey>::query_type_id();
    assert_eq!(id_pos_unit, id_pos_unit_again);
}

// ── Wave B3 — SystemMeta::dummy() sequential pointer-stability (W2) ─────

/// `miri_system_meta_dummy_lazy_init` (plan §11.4 / W2) — single-threaded
/// 1000-iteration loop asserting pointer stability across repeated calls.
///
/// Cross-thread CAS soundness of `OnceLock` is covered by stdlib's loom
/// tests; Track B's invariant here is pointer stability, which the
/// sequential test exercises faithfully (avoids the Phase 9.1
/// `Scope::spawn` Tree-Borrows trip that multi-thread Miri would hit).
#[test]
fn system_meta_dummy_lazy_init_pointer_stability() {
    use boyko_ecs::ecs::core::system::system_meta::SystemMeta;

    let p0 = SystemMeta::dummy() as *const SystemMeta;
    for i in 0..1000 {
        let pn = SystemMeta::dummy() as *const SystemMeta;
        assert_eq!(
            p0, pn,
            "SystemMeta::dummy() pointer must be stable across calls (iteration {})",
            i
        );
    }
}

/// W3 compile-time tripwire — the BSS footprint of
/// `OnceLock<SystemMeta>` is ≤ 320 B. The `const _: () = assert!(...);`
/// at module scope in `system_meta.rs` fires at compile if violated; this
/// test is the runtime mirror so a CI workflow that only runs `cargo test`
/// still flags a regression.
#[test]
fn system_meta_dummy_bss_size_within_budget() {
    use boyko_ecs::ecs::core::system::system_meta::SystemMeta;
    use std::sync::OnceLock;

    let observed = std::mem::size_of::<OnceLock<SystemMeta>>();
    assert!(
        observed <= 320,
        "OnceLock<SystemMeta> footprint {} B exceeds 320 B plan budget; \
         revisit PHASE-12.5-QUERY-OPTIMIZATIONS-PLAN.md §10.5",
        observed,
    );
}

/// `query_view_send_sync_compile` (plan §11.1 / W1) — the module-scope
/// `assert_impl_all!` at `query_view.rs` already pins the Send/Sync
/// surface at compile time (fires every build, debug + release + test).
/// This test mirrors that pin at runtime so a regression that somehow
/// slipped past the compile-time gate would still show up in `cargo test`.
#[test]
fn query_view_send_sync_compile() {
    fn assert_send_sync<T: Send + Sync>() {}
    use boyko_ecs::ecs::core::iters::query::QueryView;
    assert_send_sync::<QueryView<'static, (), ()>>();
}

// ── Wave D3 — single / get / get_mut surface smoke ───────────────────────

/// `query_view_single_smoke` — a query that matches exactly one row
/// returns that row through `single`.
#[test]
fn query_view_single_smoke() {
    register_test_components();
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[Position::component_id()]);
    spawn_pos(&mut ecs, arch, 99.0);

    let view = ecs.query::<&Position, ()>();
    let p: &Position = view.single();
    assert_eq!(p.x, 99.0);
}

#[test]
#[should_panic(expected = "yielded zero rows")]
fn query_view_single_panics_on_zero_rows() {
    register_test_components();
    let mut ecs = EcsMaster::new();
    let _arch = ecs.create_archetype(&[Position::component_id()]);
    // No rows spawned.

    let view = ecs.query::<&Position, ()>();
    let _ = view.single();
}

#[test]
#[should_panic(expected = "yielded more than one row")]
fn query_view_single_panics_on_many_rows() {
    register_test_components();
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[Position::component_id()]);
    spawn_pos(&mut ecs, arch, 1.0);
    spawn_pos(&mut ecs, arch, 2.0);

    let view = ecs.query::<&Position, ()>();
    let _ = view.single();
}

#[test]
fn query_view_get_smoke() {
    register_test_components();
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[Position::component_id()]);
    let p = Position { x: 33.0, y: 0.0, z: 0.0 };
    let bytes = unsafe {
        std::slice::from_raw_parts(
            &p as *const Position as *const u8,
            std::mem::size_of::<Position>(),
        )
    };
    let entity = ecs
        .create_entity(arch, &[(Position::component_id(), bytes)])
        .expect("spawn must succeed");

    let view = ecs.query::<&Position, ()>();
    let p_ref: &Position = view.get(entity).expect("entity must be in the matched set");
    assert_eq!(p_ref.x, 33.0);
}

#[test]
fn query_view_get_returns_none_for_unmatched_entity() {
    register_test_components();
    let mut ecs = EcsMaster::new();
    // Archetype A: only Velocity. The Position query won't match it.
    let arch_v = ecs.create_archetype(&[Velocity::component_id()]);
    let v = Velocity { x: 0.0 };
    let v_bytes = unsafe {
        std::slice::from_raw_parts(
            &v as *const Velocity as *const u8,
            std::mem::size_of::<Velocity>(),
        )
    };
    let entity = ecs
        .create_entity(arch_v, &[(Velocity::component_id(), v_bytes)])
        .expect("spawn must succeed");

    let view = ecs.query::<&Position, ()>();
    assert!(
        view.get(entity).is_none(),
        "Velocity-only entity must not match a Position query"
    );
}
