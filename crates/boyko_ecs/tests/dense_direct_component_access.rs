//! Follow-up #14 — the direct component-access API (`get_component` /
//! `get_component_mut` / `has_component` / `set_component_raw`, plus the raw
//! siblings `get_component_raw` / `get_component_raw_mut`) was TABLE-ONLY: a
//! component registered as `#[component(storage = "dense")]` is not in any
//! archetype's `columns` table, so these accessors silently returned
//! `None` / `false` / no-op even when the entity was a live member of the
//! component's `DenseStore`.
//!
//! This file gates the fix: every accessor now routes on
//! `component_registry::storage_kind`, and a live dense member reads/writes
//! correctly through the same `DenseStore` a dense query would use. Mirrors the
//! spawn / fixture conventions of `dense_d2_routing.rs` / `dense_d4_change_detection.rs`.

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_threadpool::ThreadPoolBuilder;
use boyko_macros::{Bundle, Component};

/// 8-byte POD dense payload.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[repr(C)]
struct DVal {
    x: f32,
    y: f32,
}

/// A plain TABLE component so the dense member rides alongside a real archetype.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct TTag(u32);

#[derive(Bundle)]
struct WithDense {
    t: TTag,
    d: DVal,
}

#[derive(Bundle)]
struct TagOnly {
    t: TTag,
}

#[inline]
fn dval_bytes(v: &DVal) -> &[u8] {
    // SAFETY: `DVal` is `#[repr(C)]` POD; its own byte span is a valid
    // representation of the registered type.
    unsafe { std::slice::from_raw_parts((v as *const DVal).cast::<u8>(), std::mem::size_of::<DVal>()) }
}

/// Advances the world's change-detection tick by one frame via a trivial
/// one-system `Schedule`. `EcsMaster::bump_change_tick` is `pub(crate)` — only
/// `Schedule::run` (the sanctioned frame-start bump site) is reachable from an
/// integration test, mirroring `dense_d4_change_detection.rs`.
fn advance_tick(world: &mut EcsMaster) {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|_q: boyko_ecs::ecs::core::iters::query::Query<&TTag>| {});
    let mut schedule = builder.build(&mut *world);
    schedule.run(&mut *world);
}

// ════════════════════════════════════════════════════════════════════════════
// get_component / get_component_raw: Some(correct value) for a dense member,
// None for an entity lacking the dense component.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn get_component_dense_member_some_absent_none() {
    let mut ecs = EcsMaster::new();
    let val = DVal { x: 1.0, y: 2.0 };
    let with = ecs.run_system(move |mut cmds: Commands| {
        cmds.spawn(WithDense { t: TTag(1), d: val }).id()
    });
    let without = ecs.run_system(|mut cmds: Commands| cmds.spawn(TagOnly { t: TTag(2) }).id());

    assert_eq!(
        ecs.get_component::<DVal>(with),
        Some(&val),
        "get_component must read a live dense member through the DenseStore"
    );
    assert_eq!(
        ecs.get_component::<DVal>(without),
        None,
        "get_component must return None for an entity lacking the dense component"
    );

    // Raw sibling (component_api.rs ~L173): same routing, byte-level.
    let raw = ecs
        .get_component_raw(with, DVal::component_id())
        .expect("get_component_raw must resolve a live dense member");
    // SAFETY: `raw` points at the live `DVal` value for the read's duration.
    assert_eq!(unsafe { *(raw as *const DVal) }, val);
    assert!(
        ecs.get_component_raw(without, DVal::component_id()).is_none(),
        "get_component_raw must return None for a non-member entity"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// has_component / dense_contains parity.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn has_component_dense_member_true_absent_false() {
    let mut ecs = EcsMaster::new();
    let with = ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(WithDense { t: TTag(1), d: DVal { x: 0.0, y: 0.0 } }).id()
    });
    let without = ecs.run_system(|mut cmds: Commands| cmds.spawn(TagOnly { t: TTag(2) }).id());

    let cid: ComponentId = DVal::component_id();
    assert!(ecs.has_component(with, cid), "has_component must see a live dense member");
    assert!(
        !ecs.has_component(without, cid),
        "has_component must return false for an entity lacking the dense component"
    );
    assert_eq!(
        ecs.has_component(with, cid),
        ecs.dense_contains(with, cid),
        "has_component must agree with the dense_contains oracle"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// set_component_raw: updates the value in place (round-trips through a
// subsequent get); returns false and does NOT create a membership for an
// entity that never had the component.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn set_component_raw_dense_round_trips_and_rejects_absent() {
    let mut ecs = EcsMaster::new();
    let with = ecs.run_system(move |mut cmds: Commands| {
        cmds.spawn(WithDense { t: TTag(1), d: DVal { x: 1.0, y: 1.0 } }).id()
    });
    let without = ecs.run_system(|mut cmds: Commands| cmds.spawn(TagOnly { t: TTag(2) }).id());
    let cid = DVal::component_id();

    let new_val = DVal { x: 9.0, y: 8.0 };
    assert!(
        ecs.set_component_raw(with, cid, dval_bytes(&new_val)),
        "set_component_raw must succeed for a live dense member"
    );
    assert_eq!(
        ecs.get_component::<DVal>(with),
        Some(&new_val),
        "the write must round-trip through a subsequent get"
    );

    assert!(
        !ecs.set_component_raw(without, cid, dval_bytes(&new_val)),
        "set_component_raw must return false for an entity that never had the component"
    );
    assert!(
        !ecs.has_component(without, cid),
        "set_component_raw must NOT silently create a new dense membership"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// get_component_mut: returns a working &mut, and its deref-guard bumps the
// row's changed tick (verified via Mut::is_changed(), the change-detection
// API), just like a dense query's Mut<T> would. None for an absent member.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn get_component_mut_dense_writes_and_bumps_changed_tick() {
    let mut ecs = EcsMaster::new();
    let with = ecs.run_system(move |mut cmds: Commands| {
        cmds.spawn(WithDense { t: TTag(1), d: DVal { x: 1.0, y: 1.0 } }).id()
    });
    let without = ecs.run_system(|mut cmds: Commands| cmds.spawn(TagOnly { t: TTag(2) }).id());

    assert!(
        ecs.get_component_mut::<DVal>(without).is_none(),
        "get_component_mut must return None for an entity lacking the dense component"
    );

    // Advance the tick so the insert's changed-tick stamp ages out of the
    // direct-API guard's (last_run, this_run] window (last_run == this_run ==
    // current_tick() for the system-less path — O4 semantics).
    advance_tick(&mut ecs);

    {
        let mut m = ecs.get_component_mut::<DVal>(with).expect("live dense member");
        assert!(
            !m.is_changed(),
            "before any deref, an untouched-since-last-tick dense row is not Changed"
        );
        m.x += 1.0; // DerefMut → bumps the dense slot's changed tick
        assert!(m.is_changed(), "the deref-guard write must bump the changed tick");
        assert_eq!(m.x, 2.0, "the write is visible through the returned &mut");
    }

    // The bump persisted in the DenseStore (not just guard-local state): a
    // FRESH Mut fetched afterward (same tick — no schedule advance in between)
    // still observes Changed, and the value round-trips.
    let m2 = ecs.get_component_mut::<DVal>(with).expect("live dense member");
    assert!(m2.is_changed(), "the changed tick bump must persist in the DenseStore");
    assert_eq!(*m2, DVal { x: 2.0, y: 1.0 });
}
