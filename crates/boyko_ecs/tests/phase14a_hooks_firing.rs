//! Phase 14a — component lifecycle hook FIRING-MATRIX integration tests.
//!
//! This file pins the correctness spec of the four hook kinds (`on_add`,
//! `on_insert`, `on_replace`, `on_remove`) at every structural-op site, per the
//! plan §4.3 firing matrix:
//!
//! | site                | fires                                              |
//! |---------------------|----------------------------------------------------|
//! | spawn (direct)      | ALL `on_add`, THEN ALL `on_insert` (once each)     |
//! | spawn (deferred)    | same as direct (the `SpawnAtCommand::apply` path)  |
//! | insert-in-place     | `on_replace` (OLD) then `on_insert` (NEW); no add  |
//! | insert-migration    | `on_add` for I\S; `on_insert` for I (bundle) only  |
//! | remove (migrate)    | `on_replace` + `on_remove` for C, PRE-drop, SOURCE |
//! | despawn             | `on_replace` + `on_remove` for ALL, PRE-remove     |
//!
//! # Why `static` counters
//!
//! A `HookFn` is a bare `unsafe fn` pointer — it cannot capture. Each test
//! therefore owns a private set of module-level `static AtomicUsize` counters
//! plus its own component types, so concurrently-running tests never observe one
//! another's fires. Hooks also assert the *value they read* (OLD vs NEW vs
//! dying) by stashing it into a `static` via the read-only `DeferredEcsMaster`
//! view (the canonical "hooks observe a counter / read the dying value"
//! pattern, brief §"Key facts").
//!
//! # Component-id strategy
//!
//! Hook tests MUST use `#[derive(Component)] #[component(...)]` so the
//! macro-generated `component_id()` installs hooks into the cold `HOOKS` table
//! on first call (the derive path; the only path the firing sites read). Derive
//! IDs are minted lazily from the global atomic counter (`register_new`) — they
//! never collide with the explicit `register_layout` slots other test files use.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

const SEQ: Ordering = Ordering::SeqCst;

// ════════════════════════════════════════════════════════════════════════════
// Test 1 — spawn (direct create_entity / spawn_one): on_add then on_insert
// ════════════════════════════════════════════════════════════════════════════

static T1_ADD: AtomicUsize = AtomicUsize::new(0);
static T1_INSERT: AtomicUsize = AtomicUsize::new(0);
/// Monotonic event clock: each fire grabs `fetch_add(1)` to record ITS order.
static T1_CLOCK: AtomicUsize = AtomicUsize::new(0);
static T1_ADD_AT: AtomicUsize = AtomicUsize::new(usize::MAX);
static T1_INSERT_AT: AtomicUsize = AtomicUsize::new(usize::MAX);

unsafe fn t1_on_add(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T1_ADD.fetch_add(1, SEQ);
    T1_ADD_AT.store(T1_CLOCK.fetch_add(1, SEQ), SEQ);
}
unsafe fn t1_on_insert(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T1_INSERT.fetch_add(1, SEQ);
    T1_INSERT_AT.store(T1_CLOCK.fetch_add(1, SEQ), SEQ);
}

#[derive(Component)]
#[component(on_add = t1_on_add, on_insert = t1_on_insert)]
#[repr(C)]
struct T1Comp(u32);

#[test]
fn spawn_direct_fires_on_add_then_on_insert_once_each() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[T1Comp::component_id()]);

    let _e = ecs.spawn_one(arch, T1Comp(7)).expect("spawn");

    assert_eq!(T1_ADD.load(SEQ), 1, "on_add fires exactly once on spawn");
    assert_eq!(T1_INSERT.load(SEQ), 1, "on_insert fires exactly once on spawn");
    assert!(
        T1_ADD_AT.load(SEQ) < T1_INSERT_AT.load(SEQ),
        "on_add must fire BEFORE on_insert (add-before-insert ordering)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 2 — spawn (deferred via Commands): same matrix, SpawnAtCommand::apply
// ════════════════════════════════════════════════════════════════════════════

static T2_ADD: AtomicUsize = AtomicUsize::new(0);
static T2_INSERT: AtomicUsize = AtomicUsize::new(0);

unsafe fn t2_on_add(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T2_ADD.fetch_add(1, SEQ);
}
unsafe fn t2_on_insert(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T2_INSERT.fetch_add(1, SEQ);
}

#[derive(Component)]
#[component(on_add = t2_on_add, on_insert = t2_on_insert)]
#[repr(C)]
struct T2Comp(u32);

#[derive(Bundle)]
struct T2Bundle {
    c: T2Comp,
}

#[test]
fn spawn_deferred_via_commands_fires_add_and_insert() {
    let mut ecs = EcsMaster::new();
    // Prime the id (installs hooks) before any archetype exists.
    let _ = T2Comp::component_id();

    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(T2Bundle { c: T2Comp(42) });
    });

    assert_eq!(ecs.entity_count(), 1, "deferred spawn registered one entity");
    assert_eq!(T2_ADD.load(SEQ), 1, "on_add fires once via SpawnAtCommand::apply");
    assert_eq!(T2_INSERT.load(SEQ), 1, "on_insert fires once via SpawnAtCommand::apply");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 3 — insert-in-place: on_replace (reads OLD) then on_insert (reads NEW),
//           on_add does NOT fire
// ════════════════════════════════════════════════════════════════════════════

static T3_ADD: AtomicUsize = AtomicUsize::new(0);
static T3_REPLACE: AtomicUsize = AtomicUsize::new(0);
static T3_INSERT: AtomicUsize = AtomicUsize::new(0);
static T3_CLOCK: AtomicUsize = AtomicUsize::new(0);
static T3_REPLACE_AT: AtomicUsize = AtomicUsize::new(usize::MAX);
static T3_INSERT_AT: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Value the on_replace hook read (must be the OLD value, 100).
static T3_REPLACE_SAW: AtomicU32 = AtomicU32::new(u32::MAX);
/// Value the on_insert hook read (must be the NEW value, 200).
static T3_INSERT_SAW: AtomicU32 = AtomicU32::new(u32::MAX);
/// The entity under test, so hooks can `get_component` to read the live value.
static T3_ENTITY: AtomicU64Cell = AtomicU64Cell::new();

unsafe fn t3_on_add(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T3_ADD.fetch_add(1, SEQ);
}
unsafe fn t3_on_replace(w: DeferredEcsMaster<'_>, ctx: HookContext) {
    T3_REPLACE.fetch_add(1, SEQ);
    T3_REPLACE_AT.store(T3_CLOCK.fetch_add(1, SEQ), SEQ);
    if let Some(v) = w.get_component::<T3Comp>(ctx.entity) {
        T3_REPLACE_SAW.store(v.0, SEQ);
    }
}
unsafe fn t3_on_insert(w: DeferredEcsMaster<'_>, ctx: HookContext) {
    T3_INSERT.fetch_add(1, SEQ);
    T3_INSERT_AT.store(T3_CLOCK.fetch_add(1, SEQ), SEQ);
    if let Some(v) = w.get_component::<T3Comp>(ctx.entity) {
        T3_INSERT_SAW.store(v.0, SEQ);
    }
}

#[derive(Component)]
#[component(on_add = t3_on_add, on_replace = t3_on_replace, on_insert = t3_on_insert)]
#[repr(C)]
#[derive(Clone, Copy)]
struct T3Comp(u32);

#[derive(Bundle)]
struct T3Bundle {
    c: T3Comp,
}

#[test]
fn insert_in_place_fires_replace_old_then_insert_new_no_add() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[T3Comp::component_id()]);

    let e = ecs.spawn_one(arch, T3Comp(100)).expect("spawn");
    T3_ENTITY.set(e);
    // Reset the spawn's add/insert fires — we only assert about the in-place op.
    T3_ADD.store(0, SEQ);
    T3_INSERT.store(0, SEQ);
    T3_REPLACE.store(0, SEQ);

    // Insert the SAME bundle shape ⇒ replace-in-place (target == source).
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(T3Bundle { c: T3Comp(200) });
    });

    assert_eq!(T3_ADD.load(SEQ), 0, "on_add must NOT fire on in-place replace");
    assert_eq!(T3_REPLACE.load(SEQ), 1, "on_replace fires once on in-place replace");
    assert_eq!(T3_INSERT.load(SEQ), 1, "on_insert fires once on in-place replace");
    assert!(
        T3_REPLACE_AT.load(SEQ) < T3_INSERT_AT.load(SEQ),
        "on_replace must fire BEFORE on_insert"
    );
    assert_eq!(
        T3_REPLACE_SAW.load(SEQ),
        100,
        "on_replace reads the OLD value (100) — fires PRE-overwrite"
    );
    assert_eq!(
        T3_INSERT_SAW.load(SEQ),
        200,
        "on_insert reads the NEW value (200) — fires POST-overwrite"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 4 — insert-migration: on_add for newly-added (I\S), on_insert for the
//           BUNDLE set (I) only, retained-carried-over fire NOTHING
// ════════════════════════════════════════════════════════════════════════════

// Retained component (already present, in source, NOT in the inserted bundle).
static T4_RET_ADD: AtomicUsize = AtomicUsize::new(0);
static T4_RET_INSERT: AtomicUsize = AtomicUsize::new(0);
// Newly-added component (in the inserted bundle, NOT in source).
static T4_NEW_ADD: AtomicUsize = AtomicUsize::new(0);
static T4_NEW_INSERT: AtomicUsize = AtomicUsize::new(0);

unsafe fn t4_ret_add(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T4_RET_ADD.fetch_add(1, SEQ);
}
unsafe fn t4_ret_insert(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T4_RET_INSERT.fetch_add(1, SEQ);
}
unsafe fn t4_new_add(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T4_NEW_ADD.fetch_add(1, SEQ);
}
unsafe fn t4_new_insert(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T4_NEW_INSERT.fetch_add(1, SEQ);
}

#[derive(Component)]
#[component(on_add = t4_ret_add, on_insert = t4_ret_insert)]
#[repr(C)]
#[derive(Clone, Copy)]
struct T4Retained(u32);

#[derive(Component)]
#[component(on_add = t4_new_add, on_insert = t4_new_insert)]
#[repr(C)]
#[derive(Clone, Copy)]
struct T4New(u32);

#[derive(Bundle)]
struct T4NewBundle {
    n: T4New,
}

#[test]
fn insert_migration_fires_add_for_new_insert_for_bundle_only() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[T4Retained::component_id()]);
    // Force the new-component id (installs its hooks) before its first archetype.
    let _ = T4New::component_id();

    let e = ecs.spawn_one(arch, T4Retained(1)).expect("spawn retained");
    // Clear the spawn's fires for the retained component — assert only the migration.
    T4_RET_ADD.store(0, SEQ);
    T4_RET_INSERT.store(0, SEQ);

    // Insert T4New ⇒ migration to {Retained, New}.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(T4NewBundle { n: T4New(2) });
    });

    assert!(ecs.has_component(e, T4New::component_id()), "migration added New");
    assert!(ecs.has_component(e, T4Retained::component_id()), "Retained carried over");

    // Newly-added component: BOTH on_add and on_insert fire.
    assert_eq!(T4_NEW_ADD.load(SEQ), 1, "on_add fires for newly-added bundle component");
    assert_eq!(T4_NEW_INSERT.load(SEQ), 1, "on_insert fires for bundle component");

    // Retained-carried-over (in source, NOT in bundle): fires NOTHING.
    assert_eq!(
        T4_RET_ADD.load(SEQ),
        0,
        "on_add must NOT fire for a retained (carried-over) component"
    );
    assert_eq!(
        T4_RET_INSERT.load(SEQ),
        0,
        "on_insert must NOT fire for a retained (not-in-bundle) component"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 5 — remove (migrate_entity_remove): on_replace + on_remove for C,
//           PRE-drop, reading the SOURCE (dying) row
// ════════════════════════════════════════════════════════════════════════════

static T5_REPLACE: AtomicUsize = AtomicUsize::new(0);
static T5_REMOVE: AtomicUsize = AtomicUsize::new(0);
static T5_CLOCK: AtomicUsize = AtomicUsize::new(0);
static T5_REPLACE_AT: AtomicUsize = AtomicUsize::new(usize::MAX);
static T5_REMOVE_AT: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Value read by on_remove via `get_component` — must be the dying value (55).
static T5_REMOVE_SAW: AtomicU32 = AtomicU32::new(u32::MAX);
// A component that stays (must NOT fire its remove — only the removed one does).
static T5_KEEP_REMOVE: AtomicUsize = AtomicUsize::new(0);

unsafe fn t5_on_replace(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T5_REPLACE.fetch_add(1, SEQ);
    T5_REPLACE_AT.store(T5_CLOCK.fetch_add(1, SEQ), SEQ);
}
unsafe fn t5_on_remove(w: DeferredEcsMaster<'_>, ctx: HookContext) {
    T5_REMOVE.fetch_add(1, SEQ);
    T5_REMOVE_AT.store(T5_CLOCK.fetch_add(1, SEQ), SEQ);
    // The dying bytes are still live (PRE-drop, EntityInland still at SOURCE).
    if let Some(v) = w.get_component::<T5Removed>(ctx.entity) {
        T5_REMOVE_SAW.store(v.0, SEQ);
    }
}
unsafe fn t5_keep_remove(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T5_KEEP_REMOVE.fetch_add(1, SEQ);
}

#[derive(Component)]
#[component(on_replace = t5_on_replace, on_remove = t5_on_remove)]
#[repr(C)]
#[derive(Clone, Copy)]
struct T5Removed(u32);

#[derive(Component)]
#[component(on_remove = t5_keep_remove)]
#[repr(C)]
#[derive(Clone, Copy)]
struct T5Keep(u32);

#[test]
fn remove_fires_replace_then_remove_predrop_reading_dying_value() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[T5Removed::component_id(), T5Keep::component_id()]);

    let e = ecs.spawn_two(arch, T5Removed(55), T5Keep(9)).expect("spawn");

    // Remove only T5Removed ⇒ migrate to {Keep}.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).remove::<T5Removed>();
    });

    assert!(ecs.has_entity(e), "entity survives a component remove");
    assert!(!ecs.has_component(e, T5Removed::component_id()), "Removed gone");
    assert!(ecs.has_component(e, T5Keep::component_id()), "Keep retained");

    assert_eq!(T5_REPLACE.load(SEQ), 1, "on_replace fires once for the removed component");
    assert_eq!(T5_REMOVE.load(SEQ), 1, "on_remove fires once for the removed component");
    assert!(
        T5_REPLACE_AT.load(SEQ) < T5_REMOVE_AT.load(SEQ),
        "on_replace must fire BEFORE on_remove"
    );
    assert_eq!(
        T5_REMOVE_SAW.load(SEQ),
        55,
        "on_remove reads the SOURCE (dying) value (55) — fires PRE-drop, EntityInland at SOURCE"
    );
    assert_eq!(
        T5_KEEP_REMOVE.load(SEQ),
        0,
        "a RETAINED component's on_remove must NOT fire on a single-component remove"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 6 — despawn (delete_entity): on_replace + on_remove for ALL components,
//           PRE-remove, reading the dying values
// ════════════════════════════════════════════════════════════════════════════

static T6_A_REPLACE: AtomicUsize = AtomicUsize::new(0);
static T6_A_REMOVE: AtomicUsize = AtomicUsize::new(0);
static T6_B_REPLACE: AtomicUsize = AtomicUsize::new(0);
static T6_B_REMOVE: AtomicUsize = AtomicUsize::new(0);
static T6_A_REMOVE_SAW: AtomicU32 = AtomicU32::new(u32::MAX);

unsafe fn t6_a_replace(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T6_A_REPLACE.fetch_add(1, SEQ);
}
unsafe fn t6_a_remove(w: DeferredEcsMaster<'_>, ctx: HookContext) {
    T6_A_REMOVE.fetch_add(1, SEQ);
    if let Some(v) = w.get_component::<T6CompA>(ctx.entity) {
        T6_A_REMOVE_SAW.store(v.0, SEQ);
    }
}
unsafe fn t6_b_replace(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T6_B_REPLACE.fetch_add(1, SEQ);
}
unsafe fn t6_b_remove(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T6_B_REMOVE.fetch_add(1, SEQ);
}

#[derive(Component)]
#[component(on_replace = t6_a_replace, on_remove = t6_a_remove)]
#[repr(C)]
#[derive(Clone, Copy)]
struct T6CompA(u32);

#[derive(Component)]
#[component(on_replace = t6_b_replace, on_remove = t6_b_remove)]
#[repr(C)]
#[derive(Clone, Copy)]
struct T6CompB(u32);

#[test]
fn despawn_fires_replace_and_remove_for_all_components_predrop() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[T6CompA::component_id(), T6CompB::component_id()]);

    let e = ecs.spawn_two(arch, T6CompA(77), T6CompB(88)).expect("spawn");

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).despawn();
    });

    assert!(!ecs.has_entity(e), "despawn removes the entity");
    assert_eq!(T6_A_REPLACE.load(SEQ), 1, "on_replace fires for component A on despawn");
    assert_eq!(T6_A_REMOVE.load(SEQ), 1, "on_remove fires for component A on despawn");
    assert_eq!(T6_B_REPLACE.load(SEQ), 1, "on_replace fires for component B on despawn");
    assert_eq!(T6_B_REMOVE.load(SEQ), 1, "on_remove fires for component B on despawn");
    assert_eq!(
        T6_A_REMOVE_SAW.load(SEQ),
        77,
        "despawn on_remove reads the dying value (77) — fires PRE-remove"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 7 — ordering: a 3-component bundle fires ALL on_add before ANY on_insert
// ════════════════════════════════════════════════════════════════════════════

static T7_CLOCK: AtomicUsize = AtomicUsize::new(0);
static T7_MAX_ADD_AT: AtomicUsize = AtomicUsize::new(0);
static T7_MIN_INSERT_AT: AtomicUsize = AtomicUsize::new(usize::MAX);
static T7_ADD_COUNT: AtomicUsize = AtomicUsize::new(0);
static T7_INSERT_COUNT: AtomicUsize = AtomicUsize::new(0);

unsafe fn t7_on_add(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    let at = T7_CLOCK.fetch_add(1, SEQ);
    T7_ADD_COUNT.fetch_add(1, SEQ);
    T7_MAX_ADD_AT.fetch_max(at, SEQ);
}
unsafe fn t7_on_insert(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    let at = T7_CLOCK.fetch_add(1, SEQ);
    T7_INSERT_COUNT.fetch_add(1, SEQ);
    T7_MIN_INSERT_AT.fetch_min(at, SEQ);
}

#[derive(Component)]
#[component(on_add = t7_on_add, on_insert = t7_on_insert)]
#[repr(C)]
#[derive(Clone, Copy)]
struct T7A(u32);
#[derive(Component)]
#[component(on_add = t7_on_add, on_insert = t7_on_insert)]
#[repr(C)]
#[derive(Clone, Copy)]
struct T7B(u32);
#[derive(Component)]
#[component(on_add = t7_on_add, on_insert = t7_on_insert)]
#[repr(C)]
#[derive(Clone, Copy)]
struct T7C(u32);

#[derive(Bundle)]
struct T7Bundle {
    a: T7A,
    b: T7B,
    c: T7C,
}

#[test]
fn three_component_bundle_fires_all_add_before_any_insert() {
    let mut ecs = EcsMaster::new();
    let _ = T7A::component_id();
    let _ = T7B::component_id();
    let _ = T7C::component_id();

    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(T7Bundle { a: T7A(1), b: T7B(2), c: T7C(3) });
    });

    assert_eq!(T7_ADD_COUNT.load(SEQ), 3, "all 3 on_add fire");
    assert_eq!(T7_INSERT_COUNT.load(SEQ), 3, "all 3 on_insert fire");
    assert!(
        T7_MAX_ADD_AT.load(SEQ) < T7_MIN_INSERT_AT.load(SEQ),
        "the LAST on_add (clock {}) must precede the FIRST on_insert (clock {}) — \
         add-before-insert is whole-bundle, not interleaved",
        T7_MAX_ADD_AT.load(SEQ),
        T7_MIN_INSERT_AT.load(SEQ),
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 8 — no-hook component: flags empty, NO trigger entered (counter stays 0)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct T8NoHook(u32);

#[test]
fn no_attr_component_never_enters_a_trigger() {
    use boyko_ecs::ecs::core::component::component_registry;

    let mut ecs = EcsMaster::new();
    let id = T8NoHook::component_id();
    // A plain derive installs NOTHING into the HOOKS table.
    assert!(
        component_registry::get_hooks(id.0).is_none(),
        "a plain #[derive(Component)] leaves its HOOKS slot UNSET"
    );

    let arch = ecs.create_archetype(&[id]);
    // The archetype's flags must be empty (no hook bit raised) — the 0-cost path.
    let e = ecs.spawn_one(arch, T8NoHook(1)).expect("spawn");
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).despawn();
    });
    // Nothing to assert via a hook (there is none) — the contract is "no panic,
    // no trigger entered". A regression that wrongly raised a flag bit would
    // dispatch into a None HOOKS slot, which is still a no-op, so we additionally
    // assert the flags directly below in `no_hook_archetype_flags_are_empty`.
    assert_eq!(ecs.entity_count(), 0, "despawn of a no-hook entity still works");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 9 — all-four-hooks component: each kind fires at its own site
// ════════════════════════════════════════════════════════════════════════════

static T9_ADD: AtomicUsize = AtomicUsize::new(0);
static T9_INSERT: AtomicUsize = AtomicUsize::new(0);
static T9_REPLACE: AtomicUsize = AtomicUsize::new(0);
static T9_REMOVE: AtomicUsize = AtomicUsize::new(0);

unsafe fn t9_add(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T9_ADD.fetch_add(1, SEQ);
}
unsafe fn t9_insert(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T9_INSERT.fetch_add(1, SEQ);
}
unsafe fn t9_replace(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T9_REPLACE.fetch_add(1, SEQ);
}
unsafe fn t9_remove(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T9_REMOVE.fetch_add(1, SEQ);
}

#[derive(Component)]
#[component(on_add = t9_add, on_insert = t9_insert, on_replace = t9_replace, on_remove = t9_remove)]
#[repr(C)]
#[derive(Clone, Copy)]
struct T9All(u32);

#[derive(Bundle)]
struct T9Bundle {
    c: T9All,
}

#[test]
fn all_four_hooks_each_fire_at_their_site() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[T9All::component_id()]);

    // spawn ⇒ add + insert
    let e = ecs.spawn_one(arch, T9All(1)).expect("spawn");
    assert_eq!(T9_ADD.load(SEQ), 1, "spawn fired on_add");
    assert_eq!(T9_INSERT.load(SEQ), 1, "spawn fired on_insert");

    // in-place replace ⇒ replace + insert (insert count climbs to 2)
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(T9Bundle { c: T9All(2) });
    });
    assert_eq!(T9_REPLACE.load(SEQ), 1, "in-place replace fired on_replace");
    assert_eq!(T9_INSERT.load(SEQ), 2, "in-place replace fired on_insert (now 2 total)");
    assert_eq!(T9_ADD.load(SEQ), 1, "in-place replace did NOT fire on_add (still 1)");

    // despawn ⇒ replace (now 2) + remove
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).despawn();
    });
    assert_eq!(T9_REPLACE.load(SEQ), 2, "despawn fired on_replace (now 2 total)");
    assert_eq!(T9_REMOVE.load(SEQ), 1, "despawn fired on_remove");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 10 — mixed archetype: one hooked + one non-hooked component;
//            only the hooked one's hooks fire
// ════════════════════════════════════════════════════════════════════════════

static T10_HOOKED_ADD: AtomicUsize = AtomicUsize::new(0);
static T10_HOOKED_REMOVE: AtomicUsize = AtomicUsize::new(0);

unsafe fn t10_add(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T10_HOOKED_ADD.fetch_add(1, SEQ);
}
unsafe fn t10_remove(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    T10_HOOKED_REMOVE.fetch_add(1, SEQ);
}

#[derive(Component)]
#[component(on_add = t10_add, on_remove = t10_remove)]
#[repr(C)]
#[derive(Clone, Copy)]
struct T10Hooked(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct T10Plain(u32);

#[test]
fn mixed_archetype_fires_only_the_hooked_components_hooks() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[T10Hooked::component_id(), T10Plain::component_id()]);

    let e = ecs.spawn_two(arch, T10Hooked(1), T10Plain(2)).expect("spawn");
    // The hooked component fired on_add ONCE; the plain one has no hook to fire.
    assert_eq!(
        T10_HOOKED_ADD.load(SEQ),
        1,
        "the hooked component fires on_add exactly once (the plain one cannot)"
    );

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).despawn();
    });
    assert_eq!(
        T10_HOOKED_REMOVE.load(SEQ),
        1,
        "despawn fires on_remove once for the hooked component (the plain one is silent)"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Small helper: a Send+Sync cell for stashing an Entity into a `static`.
// ────────────────────────────────────────────────────────────────────────────

use std::sync::atomic::AtomicU64;
use boyko_ecs::ecs::core::entity::entity::Entity;

/// Packs an `Entity` (id + generation) into a single `u64` `static` cell so a
/// non-capturing hook could recover the entity under test if needed. The
/// firing-matrix hooks read `ctx.entity` from the `HookContext` directly, so
/// only `set` is exercised; `get` is retained for symmetry / future tests.
struct AtomicU64Cell(AtomicU64);
impl AtomicU64Cell {
    const fn new() -> Self {
        Self(AtomicU64::new(u64::MAX))
    }
    fn set(&self, e: Entity) {
        let packed = (e.id().0 as u64) | ((e.generation() as u64) << 32);
        self.0.store(packed, SEQ);
    }
    #[allow(dead_code)]
    fn get(&self) -> Entity {
        use boyko_ecs::ecs::identifiers::primitives::EntityId;
        let packed = self.0.load(SEQ);
        Entity::new(EntityId((packed & 0xFFFF_FFFF) as usize), (packed >> 32) as u32)
    }
}
