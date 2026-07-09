//! Dense plan D2 — structural-op routing + lifecycle-fire integration tests.
//!
//! This file is the W2 gate, reconciled to the REAL structural paths (the prior
//! D2 attempt stopped because the plan's W2 site list cited `EcsMaster::create_entity`
//! but `Commands::spawn` actually flows through `SpawnAtCommand` → the
//! `BundleColumnCache`). Per the 4 resolved decisions it pins:
//!
//! * dense on_add/on_insert/on_remove/on_replace/on_despawn fire EXACTLY right
//!   per API: `Commands::spawn`, `create_entity`, `create_entity_at`, insert,
//!   remove, despawn, clone/materialize, hierarchy-cascade;
//! * `spawn_batch` STORES dense but fires ZERO per-row hooks (decision 4 — the
//!   bulk-no-hooks policy, no table/dense asymmetry);
//! * dense insert/remove does NOT change the entity's archetype id (no-migration);
//! * a dense round-trip via Commands (`DenseStore` has it at a consistent slot,
//!   data correct) + despawn tombstones it.
//!
//! # Why OBSERVERS (not derive hooks)
//!
//! The W2 gate counts "observer/hook". The `#[derive(Component)]` hook attrs do
//! not expose `on_despawn` (deferred at the derive layer), so this file uses the
//! public, runtime-registered OBSERVER API (`observe_on_*` / `add_observer`),
//! which covers ALL five kinds. The dense routing fires component observers via
//! the same `fire_*_observers` dispatch as table components, so this exercises
//! the exact W2 fire sites. Dense storage is selected by
//! `#[component(storage = "dense")]` (no hook attrs).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::observers::{ObserverContext, ObserverKind};
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::bundle::Bundle as BundleTrait;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::{Bundle, Component};

const SEQ: Ordering = Ordering::SeqCst;

/// 16-byte POD dense payload shared by the structural tests.
#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct DPos {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

/// A plain table component so dense can ride alongside a real archetype.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct TTag(u32);

#[inline]
fn dpos_bytes(p: &DPos) -> &[u8] {
    // SAFETY: `DPos` is `#[repr(C)]` POD; its own byte span is a valid
    // representation of the registered type.
    unsafe {
        std::slice::from_raw_parts((p as *const DPos).cast::<u8>(), std::mem::size_of::<DPos>())
    }
}

#[inline]
fn tag_bytes(t: &TTag) -> &[u8] {
    // SAFETY: `TTag` is `#[repr(C)]` POD.
    unsafe { std::slice::from_raw_parts((t as *const TTag).cast::<u8>(), std::mem::size_of::<TTag>()) }
}

// ── per-test fire counters (one DISTINCT set per test → no cross-test race) ───

macro_rules! observer_counter {
    ($add:ident, $insert:ident, $replace:ident, $remove:ident, $despawn:ident,
     $f_add:ident, $f_insert:ident, $f_replace:ident, $f_remove:ident, $f_despawn:ident) => {
        static $add: AtomicUsize = AtomicUsize::new(0);
        static $insert: AtomicUsize = AtomicUsize::new(0);
        static $replace: AtomicUsize = AtomicUsize::new(0);
        static $remove: AtomicUsize = AtomicUsize::new(0);
        static $despawn: AtomicUsize = AtomicUsize::new(0);
        unsafe fn $f_add(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
            $add.fetch_add(1, SEQ);
        }
        unsafe fn $f_insert(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
            $insert.fetch_add(1, SEQ);
        }
        unsafe fn $f_replace(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
            $replace.fetch_add(1, SEQ);
        }
        unsafe fn $f_remove(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
            $remove.fetch_add(1, SEQ);
        }
        unsafe fn $f_despawn(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
            $despawn.fetch_add(1, SEQ);
        }
    };
}

/// Registers all five kinds of observer for dense component `C` and zeroes the
/// counters. Call BEFORE spawning so the observer is live when the fire happens.
#[allow(clippy::too_many_arguments)]
fn install_observers<C: Component>(
    ecs: &mut EcsMaster,
    add: unsafe fn(DeferredEcsMaster<'_>, ObserverContext),
    insert: unsafe fn(DeferredEcsMaster<'_>, ObserverContext),
    replace: unsafe fn(DeferredEcsMaster<'_>, ObserverContext),
    remove: unsafe fn(DeferredEcsMaster<'_>, ObserverContext),
    despawn: unsafe fn(DeferredEcsMaster<'_>, ObserverContext),
) {
    let cid = C::component_id();
    ecs.add_observer(ObserverKind::Add, cid, add);
    ecs.add_observer(ObserverKind::Insert, cid, insert);
    ecs.add_observer(ObserverKind::Replace, cid, replace);
    ecs.add_observer(ObserverKind::Remove, cid, remove);
    ecs.add_observer(ObserverKind::Despawn, cid, despawn);
}

/// Spawns `(TTag, D)` via `Commands::spawn`, returning the live handle.
fn spawn_table_dense<D: BundleTrait + Send + Sync>(
    ecs: &mut EcsMaster,
    make: impl Fn() -> D + Send + Sync + 'static,
) -> Entity {
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        probe.lock().expect("lock").push(cmds.spawn(make()).id());
    });
    sink.lock().expect("lock")[0]
}

// ════════════════════════════════════════════════════════════════════════════
// Test 1 — Commands::spawn(Table, Dense): dense on_add + on_insert fire once
//          each; round-trip stored; no migration.
// ════════════════════════════════════════════════════════════════════════════

observer_counter!(
    T1_ADD, T1_INSERT, T1_REPLACE, T1_REMOVE, T1_DESPAWN,
    t1_add, t1_insert, t1_replace, t1_remove, t1_despawn
);

#[derive(Component)]
#[component(storage = "dense")]
#[repr(C)]
struct T1Dense(DPos);

#[derive(Bundle)]
struct T1Bundle {
    t: TTag,
    d: T1Dense,
}

#[test]
fn commands_spawn_table_dense_fires_add_insert_once_and_stores() {
    let mut ecs = EcsMaster::new();
    install_observers::<T1Dense>(&mut ecs, t1_add, t1_insert, t1_replace, t1_remove, t1_despawn);
    T1_ADD.store(0, SEQ);
    T1_INSERT.store(0, SEQ);

    let pos = DPos { x: 1.0, y: 2.0, z: 3.0, w: 4.0 };
    let e = spawn_table_dense(&mut ecs, move || T1Bundle { t: TTag(7), d: T1Dense(pos) });

    assert_eq!(T1_ADD.load(SEQ), 1, "dense on_add fires exactly once on Commands::spawn");
    assert_eq!(T1_INSERT.load(SEQ), 1, "dense on_insert fires exactly once on Commands::spawn");
    assert_eq!(T1_REMOVE.load(SEQ), 0, "no on_remove on spawn");
    assert_eq!(T1_REPLACE.load(SEQ), 0, "no on_replace on spawn");

    // Round-trip: the DenseStore has the entity at a slot with correct data.
    assert!(ecs.dense_contains(e, T1Dense::component_id()), "dense membership recorded");
    let raw = ecs.dense_get_raw(e, T1Dense::component_id()).expect("raw");
    // SAFETY: `raw` points at the live `T1Dense` value for the read's duration.
    let got = unsafe { *(raw as *const DPos) };
    assert_eq!(got, pos, "dense value round-trips through the store");

    // No migration: the entity's archetype hosts only the TABLE component.
    assert!(ecs.get_component_raw(e, TTag::component_id()).is_some(), "table component present");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 2 — direct create_entity(Table, Dense): same dense add+insert matrix.
// ════════════════════════════════════════════════════════════════════════════

observer_counter!(
    T2_ADD, T2_INSERT, T2_REPLACE, T2_REMOVE, T2_DESPAWN,
    t2_add, t2_insert, t2_replace, t2_remove, t2_despawn
);

#[derive(Component)]
#[component(storage = "dense")]
#[repr(C)]
struct T2Dense(DPos);

#[test]
fn create_entity_table_dense_fires_add_insert_once() {
    let mut ecs = EcsMaster::new();
    install_observers::<T2Dense>(&mut ecs, t2_add, t2_insert, t2_replace, t2_remove, t2_despawn);
    T2_ADD.store(0, SEQ);
    T2_INSERT.store(0, SEQ);

    let arch = ecs.create_archetype(&[TTag::component_id()]);
    let tag = TTag(5);
    let pos = DPos { x: 9.0, y: 8.0, z: 7.0, w: 6.0 };
    let e = ecs
        .create_entity(
            arch,
            &[
                (TTag::component_id(), tag_bytes(&tag)),
                (T2Dense::component_id(), dpos_bytes(&pos)),
            ],
        )
        .expect("create_entity");

    assert_eq!(T2_ADD.load(SEQ), 1, "dense on_add fires once via create_entity");
    assert_eq!(T2_INSERT.load(SEQ), 1, "dense on_insert fires once via create_entity");
    assert!(ecs.dense_contains(e, T2Dense::component_id()), "dense stored via direct create_entity");
    let raw = ecs.dense_get_raw(e, T2Dense::component_id()).expect("raw");
    assert_eq!(unsafe { *(raw as *const DPos) }, pos);
}

// ════════════════════════════════════════════════════════════════════════════
// Test 3 — direct create_entity_at(Table, Dense): same matrix at a reserved id.
// ════════════════════════════════════════════════════════════════════════════

observer_counter!(
    T3_ADD, T3_INSERT, T3_REPLACE, T3_REMOVE, T3_DESPAWN,
    t3_add, t3_insert, t3_replace, t3_remove, t3_despawn
);

#[derive(Component)]
#[component(storage = "dense")]
#[repr(C)]
struct T3Dense(DPos);

#[test]
fn create_entity_at_table_dense_fires_add_insert_once() {
    let mut ecs = EcsMaster::new();
    install_observers::<T3Dense>(&mut ecs, t3_add, t3_insert, t3_replace, t3_remove, t3_despawn);
    T3_ADD.store(0, SEQ);
    T3_INSERT.store(0, SEQ);

    let arch = ecs.create_archetype(&[TTag::component_id()]);
    // A fresh, never-registered id (gen 0): satisfies the NULL-slot precondition.
    let entity = Entity::new(EntityId(900_001), 0);
    let tag = TTag(1);
    let pos = DPos { x: 0.5, y: 1.5, z: 2.5, w: 3.5 };
    ecs.create_entity_at(
        entity,
        arch,
        &[
            (TTag::component_id(), tag_bytes(&tag)),
            (T3Dense::component_id(), dpos_bytes(&pos)),
        ],
    )
    .expect("create_entity_at");

    assert_eq!(T3_ADD.load(SEQ), 1, "dense on_add fires once via create_entity_at");
    assert_eq!(T3_INSERT.load(SEQ), 1, "dense on_insert fires once via create_entity_at");
    assert!(ecs.dense_contains(entity, T3Dense::component_id()));
}

// ════════════════════════════════════════════════════════════════════════════
// Test 4 — Commands insert(Dense) onto an existing entity: add + insert; no
//          migration (archetype id unchanged).
// ════════════════════════════════════════════════════════════════════════════

observer_counter!(
    T4_ADD, T4_INSERT, T4_REPLACE, T4_REMOVE, T4_DESPAWN,
    t4_add, t4_insert, t4_replace, t4_remove, t4_despawn
);

#[derive(Component)]
#[component(storage = "dense")]
#[repr(C)]
struct T4Dense(DPos);

#[derive(Bundle)]
struct T4DenseBundle {
    d: T4Dense,
}

#[derive(Bundle)]
struct TagOnly {
    t: TTag,
}

#[test]
fn commands_insert_dense_no_migration() {
    let mut ecs = EcsMaster::new();
    install_observers::<T4Dense>(&mut ecs, t4_add, t4_insert, t4_replace, t4_remove, t4_despawn);
    T4_ADD.store(0, SEQ);
    T4_INSERT.store(0, SEQ);

    // Spawn a table-only entity.
    let e = spawn_table_dense(&mut ecs, || TagOnly { t: TTag(3) });
    let arch_id_before = ecs.entity_archetype_id(e).expect("live").get();

    // Insert a dense component (no table change).
    let pos = DPos { x: 11.0, y: 12.0, z: 13.0, w: 14.0 };
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(T4DenseBundle { d: T4Dense(pos) });
    });

    assert_eq!(T4_ADD.load(SEQ), 1, "dense insert fires on_add once (was absent)");
    assert_eq!(T4_INSERT.load(SEQ), 1, "dense insert fires on_insert once");
    assert_eq!(T4_REPLACE.load(SEQ), 0, "absent-before dense insert fires NO on_replace");
    assert!(ecs.dense_contains(e, T4Dense::component_id()), "dense stored via insert");

    // No migration: archetype id is unchanged.
    let arch_id_after = ecs.entity_archetype_id(e).expect("live").get();
    assert_eq!(arch_id_before, arch_id_after, "dense insert must NOT migrate the archetype");
    assert!(ecs.get_component_raw(e, TTag::component_id()).is_some(), "table comp untouched");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 5 — Commands remove(Dense): on_replace + on_remove fire once; tombstoned;
//          no migration.
// ════════════════════════════════════════════════════════════════════════════

observer_counter!(
    T5_ADD, T5_INSERT, T5_REPLACE, T5_REMOVE, T5_DESPAWN,
    t5_add, t5_insert, t5_replace, t5_remove, t5_despawn
);

#[derive(Component)]
#[component(storage = "dense")]
#[repr(C)]
struct T5Dense(DPos);

#[derive(Bundle)]
struct T5Bundle {
    t: TTag,
    d: T5Dense,
}

#[test]
fn commands_remove_dense_fires_replace_remove_and_tombstones() {
    let mut ecs = EcsMaster::new();
    install_observers::<T5Dense>(&mut ecs, t5_add, t5_insert, t5_replace, t5_remove, t5_despawn);

    let pos = DPos { x: 1.0, y: 1.0, z: 1.0, w: 1.0 };
    let e = spawn_table_dense(&mut ecs, move || T5Bundle { t: TTag(1), d: T5Dense(pos) });
    assert!(ecs.dense_contains(e, T5Dense::component_id()), "dense present after spawn");
    let arch_before = ecs.entity_archetype_id(e).expect("live").get();

    // Reset AFTER spawn so we count only the remove's fires.
    T5_REPLACE.store(0, SEQ);
    T5_REMOVE.store(0, SEQ);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).remove::<T5Dense>();
    });

    assert_eq!(T5_REPLACE.load(SEQ), 1, "dense remove fires on_replace once");
    assert_eq!(T5_REMOVE.load(SEQ), 1, "dense remove fires on_remove once");
    assert!(!ecs.dense_contains(e, T5Dense::component_id()), "dense tombstoned after remove");
    assert_eq!(
        arch_before,
        ecs.entity_archetype_id(e).expect("live").get(),
        "dense remove must NOT migrate the archetype"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 6 — despawn: dense on_despawn + on_replace + on_remove fire once each;
//          tombstoned.
// ════════════════════════════════════════════════════════════════════════════

observer_counter!(
    T6_ADD, T6_INSERT, T6_REPLACE, T6_REMOVE, T6_DESPAWN,
    t6_add, t6_insert, t6_replace, t6_remove, t6_despawn
);

#[derive(Component)]
#[component(storage = "dense")]
#[repr(C)]
struct T6Dense(DPos);

#[derive(Bundle)]
struct T6Bundle {
    t: TTag,
    d: T6Dense,
}

#[test]
fn despawn_fires_dense_despawn_replace_remove_and_tombstones() {
    let mut ecs = EcsMaster::new();
    install_observers::<T6Dense>(&mut ecs, t6_add, t6_insert, t6_replace, t6_remove, t6_despawn);

    let e = spawn_table_dense(&mut ecs, || T6Bundle {
        t: TTag(1),
        d: T6Dense(DPos { x: 2.0, y: 2.0, z: 2.0, w: 2.0 }),
    });
    assert!(ecs.dense_contains(e, T6Dense::component_id()));

    // Reset AFTER spawn.
    T6_DESPAWN.store(0, SEQ);
    T6_REPLACE.store(0, SEQ);
    T6_REMOVE.store(0, SEQ);

    ecs.delete_entity(e);

    assert_eq!(T6_DESPAWN.load(SEQ), 1, "dense on_despawn fires once on despawn");
    assert_eq!(T6_REPLACE.load(SEQ), 1, "dense on_replace fires once on despawn");
    assert_eq!(T6_REMOVE.load(SEQ), 1, "dense on_remove fires once on despawn");
    assert!(!ecs.dense_contains(e, T6Dense::component_id()), "dense tombstoned on despawn");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 7 — clone / materialize: dense membership materialized into the clone +
//          dense on_add/on_insert fire for the clone.
// ════════════════════════════════════════════════════════════════════════════

observer_counter!(
    T7_ADD, T7_INSERT, T7_REPLACE, T7_REMOVE, T7_DESPAWN,
    t7_add, t7_insert, t7_replace, t7_remove, t7_despawn
);

#[derive(Component, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct T7Dense(DPos);

#[derive(Bundle)]
struct T7Bundle {
    t: TTag,
    d: T7Dense,
}

#[test]
fn clone_materializes_dense_membership_and_fires() {
    let mut ecs = EcsMaster::new();
    install_observers::<T7Dense>(&mut ecs, t7_add, t7_insert, t7_replace, t7_remove, t7_despawn);

    let pos = DPos { x: 5.0, y: 6.0, z: 7.0, w: 8.0 };
    let source = spawn_table_dense(&mut ecs, move || T7Bundle { t: TTag(1), d: T7Dense(pos) });
    assert!(ecs.dense_contains(source, T7Dense::component_id()), "source has dense");

    // Reset AFTER the source spawn so we count only the clone's fires.
    T7_ADD.store(0, SEQ);
    T7_INSERT.store(0, SEQ);

    let clone = ecs.clone_and_spawn(source);

    assert!(ecs.dense_contains(clone, T7Dense::component_id()), "clone got the dense membership");
    let raw = ecs.dense_get_raw(clone, T7Dense::component_id()).expect("raw");
    assert_eq!(unsafe { *(raw as *const DPos) }, pos, "clone dense value matches source");
    assert_eq!(T7_ADD.load(SEQ), 1, "clone fires dense on_add once");
    assert_eq!(T7_INSERT.load(SEQ), 1, "clone fires dense on_insert once");
    // Source membership untouched (distinct slot).
    assert!(ecs.dense_contains(source, T7Dense::component_id()));
}

// ════════════════════════════════════════════════════════════════════════════
// Test 8 — spawn_batch(Table, Dense): dense is STORED but fires ZERO per-row
//          hooks (decision 4 — bulk-no-hooks policy, no table/dense asymmetry).
// ════════════════════════════════════════════════════════════════════════════

observer_counter!(
    T8_ADD, T8_INSERT, T8_REPLACE, T8_REMOVE, T8_DESPAWN,
    t8_add, t8_insert, t8_replace, t8_remove, t8_despawn
);

#[derive(Component, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct T8Dense(DPos);

#[derive(Bundle, Clone, Copy)]
struct T8Bundle {
    t: TTag,
    d: T8Dense,
}

#[test]
fn spawn_batch_stores_dense_but_fires_no_per_row_hooks() {
    let mut ecs = EcsMaster::new();
    install_observers::<T8Dense>(&mut ecs, t8_add, t8_insert, t8_replace, t8_remove, t8_despawn);
    T8_ADD.store(0, SEQ);
    T8_INSERT.store(0, SEQ);

    let n = 5usize;
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let bundles = (0..n).map(|i| T8Bundle {
            t: TTag(i as u32),
            d: T8Dense(DPos { x: i as f32, y: 0.0, z: 0.0, w: 0.0 }),
        });
        let handles: Vec<Entity> = cmds.spawn_batch(bundles).expect("spawn_batch").collect();
        *probe.lock().expect("lock") = handles;
    });
    let handles = sink.lock().expect("lock").clone();
    assert_eq!(handles.len(), n);

    // Decision 4: dense is STORED for every batched row...
    for (i, &e) in handles.iter().enumerate() {
        assert!(
            ecs.dense_contains(e, T8Dense::component_id()),
            "batched dense row {i} stored"
        );
        let raw = ecs.dense_get_raw(e, T8Dense::component_id()).expect("raw");
        assert_eq!(unsafe { *(raw as *const DPos) }.x, i as f32, "batched dense value {i}");
    }
    // ...but ZERO per-row hooks/observers fire (consistent with table-in-batch).
    assert_eq!(T8_ADD.load(SEQ), 0, "spawn_batch fires NO dense on_add (bulk-no-hooks)");
    assert_eq!(T8_INSERT.load(SEQ), 0, "spawn_batch fires NO dense on_insert (bulk-no-hooks)");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 9 — hierarchy despawn-cascade: each cascaded child's dense membership is
//          tombstoned + its on_despawn fires (rides delete_entity_core).
// ════════════════════════════════════════════════════════════════════════════

observer_counter!(
    T9_ADD, T9_INSERT, T9_REPLACE, T9_REMOVE, T9_DESPAWN,
    t9_add, t9_insert, t9_replace, t9_remove, t9_despawn
);

#[derive(Component)]
#[component(storage = "dense")]
#[repr(C)]
struct T9Dense(DPos);

#[derive(Bundle)]
struct T9Bundle {
    t: TTag,
    d: T9Dense,
}

#[test]
fn hierarchy_despawn_cascade_tombstones_and_fires_dense() {
    let mut ecs = EcsMaster::new();
    install_observers::<T9Dense>(&mut ecs, t9_add, t9_insert, t9_replace, t9_remove, t9_despawn);

    // Spawn a parent + child, each with a dense component.
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let p = cmds
            .spawn(T9Bundle { t: TTag(0), d: T9Dense(DPos { x: 0.0, y: 0.0, z: 0.0, w: 0.0 }) })
            .id();
        let c = cmds
            .spawn(T9Bundle { t: TTag(1), d: T9Dense(DPos { x: 1.0, y: 0.0, z: 0.0, w: 0.0 }) })
            .id();
        let mut probe = probe.lock().expect("lock");
        probe.push(p);
        probe.push(c);
    });
    let (parent, child) = {
        let s = sink.lock().expect("lock");
        (s[0], s[1])
    };
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(child);
    });
    assert!(ecs.dense_contains(parent, T9Dense::component_id()));
    assert!(ecs.dense_contains(child, T9Dense::component_id()));

    // Reset AFTER setup.
    T9_DESPAWN.store(0, SEQ);

    // Despawn the parent: the default-recursive cascade despawns the child too.
    ecs.delete_entity(parent);

    // Both the parent's and the cascaded child's dense on_despawn fired, and both
    // memberships are tombstoned.
    assert_eq!(
        T9_DESPAWN.load(SEQ),
        2,
        "dense on_despawn fires once for the parent AND once for the cascaded child"
    );
    assert!(!ecs.dense_contains(parent, T9Dense::component_id()), "parent dense tombstoned");
    assert!(!ecs.dense_contains(child, T9Dense::component_id()), "child dense tombstoned");
}
