//! Feature 2 — entity-targeted observers + custom triggers + ChildOf
//! propagation + on_despawn — BEHAVIORAL / integration tests.
//!
//! Mirrors the Phase-14b observer test style (module-level `static AtomicUsize`
//! counters + bare `unsafe fn` runners, since an `ObserverFn`/`TriggerFn` is a
//! non-capturing fn pointer) and pins the `docs/OBSERVERS-PLAN.md` "Tests"
//! section:
//!
//! | item | what                                                                 |
//! |------|----------------------------------------------------------------------|
//! | 1    | entity-targeted lifecycle fires for ITS entity only; **C1 headline** |
//! | 2    | sticky bit survives multi-step migration                             |
//! | 3    | stale-handle recycle guard (despawn + reuse EntityId)                |
//! | 4    | W1 live-entity contract (debug `should_panic`)                       |
//! | 5    | custom triggers: global + entity-targeted, payload read              |
//! | 6    | propagation up ChildOf + `propagate(false)` stop + non-bubble O3     |
//! | 7    | on_despawn before drop; **cascade parent-first + local order**       |
//! | 8    | re-entrancy: deferred drain once; propagate TLS save/restore         |
//! | 9    | `EntityCommands::observe` deferred wrapper attaches + fires           |
//!
//! # Why `static` counters
//!
//! `ObserverFn`/`TriggerFn` are bare `unsafe fn` pointers — they cannot capture.
//! Each test owns a private set of module-level `static AtomicUsize` counters
//! plus its own component / trigger types, so concurrently-running tests never
//! observe one another's fires (the global registries are process-wide in the
//! test binary). A global "sequence clock" (`fetch_add`) records fire ORDER.

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::trigger::{Trigger, TriggerContext};
use boyko_ecs::ecs::core::component::observers::propagate::propagate;
use boyko_ecs::ecs::core::component::observers::traversal::ChildOfTraversal;
use boyko_ecs::ecs::core::component::observers::{ObserverContext, ObserverKind};
use boyko_ecs::ecs::core::hierarchy::ChildOf;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

const SEQ: Ordering = Ordering::SeqCst;

/// A process-global monotonic clock used to record the ORDER fires happen in.
static CLOCK: AtomicUsize = AtomicUsize::new(0);
#[inline]
fn tick() -> usize {
    CLOCK.fetch_add(1, SEQ)
}

// Freshly-spawned `Entity` handles are smuggled out of each (`Send + Sync`)
// system closure via an `Arc<Mutex<Vec<Entity>>>` probe (the established
// Phase-11 / Phase-19 pattern) and read back after the `run_system` apply
// window. The closure is inlined per call site (a generic helper would need the
// captured FnOnce to be `Sync`, which the system-closure bound forces).

// ════════════════════════════════════════════════════════════════════════════
// Item 1 — entity-targeted lifecycle fires for ITS entity ONLY
//          + the C1 regression headline (fire on first migration to a fresh
//          archetype)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct A1(u32);

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct B1(u32);

#[derive(Bundle)]
struct B1Bundle {
    b: B1,
}

static I1_E1_ADD: AtomicUsize = AtomicUsize::new(0);

unsafe fn i1_e1_add(_w: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    assert_eq!(ctx.kind, ObserverKind::Add, "ctx.kind is Add");
    assert_eq!(ctx.component_id, B1::component_id(), "ctx.component_id is B1");
    I1_E1_ADD.fetch_add(1, SEQ);
}

/// **C1 regression — the headline.** `observe_entity(e1, Add, B1)` on a LIVE e1
/// in archetype `{A1}` (no observed member there before), then insert `B1` to
/// migrate e1 into a FRESH `{A1,B1}` archetype → the entity `on_add` observer
/// for `B1` must fire EXACTLY ONCE on the first migration into the new
/// archetype (the bug the sticky-bit-before-flags-read fix closes). A
/// structural op on a SECOND entity in the same archetype must NOT fire e1's
/// observer.
#[test]
fn c1_entity_observer_fires_on_first_migration_into_fresh_archetype() {
    let mut ecs = EcsMaster::new();
    let arch_a = ecs.create_archetype(&[A1::component_id()]);
    // Two LIVE entities in {A1}; the observer is attached to e1 only.
    let e1 = ecs.spawn_one(arch_a, A1(1)).expect("spawn e1");
    let e2 = ecs.spawn_one(arch_a, A1(2)).expect("spawn e2");

    // Materialise B1's id so its archetype dimension is valid (no fresh
    // {A1,B1} archetype exists yet — this is the "fresh archetype" of C1).
    let _ = B1::component_id();
    ecs.observe_entity(e1, ObserverKind::Add, B1::component_id(), i1_e1_add);

    assert_eq!(I1_E1_ADD.load(SEQ), 0, "no fire before any structural op");

    // Insert B1 into e1 → migrate {A1} -> FRESH {A1,B1}.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e1).insert(B1Bundle { b: B1(10) });
    });
    assert_eq!(
        I1_E1_ADD.load(SEQ),
        1,
        "C1: the entity on_add observer for B1 fires exactly once on the FIRST \
         migration into the fresh {{A1,B1}} archetype"
    );

    // Insert B1 into e2 (NOT observed) → same fresh archetype now exists. e1's
    // observer must NOT fire for an op on e2.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e2).insert(B1Bundle { b: B1(20) });
    });
    assert_eq!(
        I1_E1_ADD.load(SEQ),
        1,
        "an op on e2 must NOT fire e1's entity-targeted observer (still 1)"
    );
}

// ── Item 1 (cont.) — on_insert / on_replace / on_remove entity-targeted ──────

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct A1b(u32);

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct B1b(u32);

#[derive(Bundle)]
struct B1bBundle {
    b: B1b,
}

static I1B_INSERT: AtomicUsize = AtomicUsize::new(0);
static I1B_REPLACE: AtomicUsize = AtomicUsize::new(0);
static I1B_REMOVE: AtomicUsize = AtomicUsize::new(0);

unsafe fn i1b_insert(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    I1B_INSERT.fetch_add(1, SEQ);
}
unsafe fn i1b_replace(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    I1B_REPLACE.fetch_add(1, SEQ);
}
unsafe fn i1b_remove(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    I1B_REMOVE.fetch_add(1, SEQ);
}

#[test]
fn entity_observer_insert_replace_remove_fire_via_migration_paths() {
    let mut ecs = EcsMaster::new();
    let arch_a = ecs.create_archetype(&[A1b::component_id()]);
    let e = ecs.spawn_one(arch_a, A1b(1)).expect("spawn");
    let _ = B1b::component_id();

    ecs.observe_entity(e, ObserverKind::Insert, B1b::component_id(), i1b_insert);
    ecs.observe_entity(e, ObserverKind::Replace, B1b::component_id(), i1b_replace);
    ecs.observe_entity(e, ObserverKind::Remove, B1b::component_id(), i1b_remove);

    // 1) Insert B1b → migrate {A1b} -> {A1b,B1b}: fires on_insert (new), not replace/remove.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(B1bBundle { b: B1b(10) });
    });
    assert_eq!(I1B_INSERT.load(SEQ), 1, "on_insert fired on add-via-migration");
    assert_eq!(I1B_REPLACE.load(SEQ), 0, "no replace on a newly-added component");
    assert_eq!(I1B_REMOVE.load(SEQ), 0, "no remove yet");

    // 2) Insert B1b again (same archetype shape) → in-place replace: fires
    //    on_replace (old value) then on_insert (new value), no remove.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(B1bBundle { b: B1b(20) });
    });
    assert_eq!(I1B_REPLACE.load(SEQ), 1, "in-place replace fires on_replace once");
    assert_eq!(I1B_INSERT.load(SEQ), 2, "in-place replace fires on_insert (new value)");
    assert_eq!(I1B_REMOVE.load(SEQ), 0, "in-place replace does not remove");

    // 3) Remove B1b → migrate {A1b,B1b} -> {A1b}: fires on_remove (and replace
    //    for the dying value).
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).remove::<B1b>();
    });
    assert_eq!(I1B_REMOVE.load(SEQ), 1, "on_remove fired on remove-via-migration");
}

// ════════════════════════════════════════════════════════════════════════════
// Item 2 — sticky bit survives multi-step migration: attach on {A2}, insert C2
//          (->{A2,C2}), insert D2 (->{A2,C2,D2}) → observer still fires each step
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct A2(u32);
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct C2(u32);
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct D2(u32);

#[derive(Bundle)]
struct C2Bundle {
    c: C2,
}
#[derive(Bundle)]
struct D2Bundle {
    d: D2,
}

static I2_C2_ADD: AtomicUsize = AtomicUsize::new(0);
static I2_D2_ADD: AtomicUsize = AtomicUsize::new(0);

unsafe fn i2_c2_add(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    I2_C2_ADD.fetch_add(1, SEQ);
}
unsafe fn i2_d2_add(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    I2_D2_ADD.fetch_add(1, SEQ);
}

#[test]
fn sticky_bit_survives_multi_step_migration() {
    let mut ecs = EcsMaster::new();
    let arch_a = ecs.create_archetype(&[A2::component_id()]);
    let e = ecs.spawn_one(arch_a, A2(1)).expect("spawn");
    let _ = C2::component_id();
    let _ = D2::component_id();

    ecs.observe_entity(e, ObserverKind::Add, C2::component_id(), i2_c2_add);
    ecs.observe_entity(e, ObserverKind::Add, D2::component_id(), i2_d2_add);

    // Step 1: insert C2 → {A2} -> {A2,C2}. Fires the C2 on_add.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(C2Bundle { c: C2(2) });
    });
    assert_eq!(I2_C2_ADD.load(SEQ), 1, "C2 on_add fires at the first migration");

    // Step 2: insert D2 → {A2,C2} -> {A2,C2,D2}. The bit must have been
    // re-raised on the {A2,C2} destination so the D2 on_add still fires.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(D2Bundle { d: D2(3) });
    });
    assert_eq!(
        I2_D2_ADD.load(SEQ),
        1,
        "D2 on_add fires at the SECOND migration (sticky bit re-raised on each \
         new destination archetype)"
    );
    assert_eq!(I2_C2_ADD.load(SEQ), 1, "C2 on_add did not re-fire (still 1)");
}

// ════════════════════════════════════════════════════════════════════════════
// Item 3 — stale-handle recycle guard: despawn an observed entity, reuse its
//          EntityId (generation bump) → the NEW entity does NOT inherit the
//          old observer
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct A3(u32);
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct B3(u32);

#[derive(Bundle)]
struct B3Bundle {
    b: B3,
}

static I3_FIRES: AtomicUsize = AtomicUsize::new(0);

unsafe fn i3_add(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    I3_FIRES.fetch_add(1, SEQ);
}

#[test]
fn stale_handle_recycle_guard_no_inherited_observer() {
    let mut ecs = EcsMaster::new();
    let arch_a = ecs.create_archetype(&[A3::component_id()]);
    let _ = B3::component_id();

    // Observe e1's on_add for B3, then despawn e1 (its slot becomes reusable).
    let e1 = ecs.spawn_one(arch_a, A3(1)).expect("spawn e1");
    ecs.observe_entity(e1, ObserverKind::Add, B3::component_id(), i3_add);
    assert!(ecs.delete_entity(e1), "despawn e1");

    // Spawn until a new entity reuses e1's slot (generation bumped). The direct
    // entity_master free-list reuses the most-recently-freed id first.
    let e2 = ecs.spawn_one(arch_a, A3(2)).expect("spawn e2 (recycles e1 slot)");
    assert_eq!(
        e2.id().0,
        e1.id().0,
        "the new entity reuses e1's slot id (free-list recycle)"
    );
    assert_ne!(
        e2.generation(),
        e1.generation(),
        "the recycled handle has a bumped generation"
    );

    // Migrate the RECYCLED entity by inserting B3. The OLD observer (keyed by
    // e1's generation) must NOT fire for e2.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e2).insert(B3Bundle { b: B3(10) });
    });
    assert_eq!(
        I3_FIRES.load(SEQ),
        0,
        "a recycled EntityId does NOT inherit the dead entity's observer"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Item 4 — W1 live-entity contract: observe_entity on a DEAD handle trips the
//          debug_assert! (debug-build only test)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct A4(u32);

unsafe fn i4_noop(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {}

/// The happy path: attaching to a LIVE entity works (no panic, returns an id).
#[test]
fn observe_entity_on_live_entity_succeeds() {
    let mut ecs = EcsMaster::new();
    let arch_a = ecs.create_archetype(&[A4::component_id()]);
    let e = ecs.spawn_one(arch_a, A4(1)).expect("spawn");
    // Returns an id without panic — the live path.
    let _id = ecs.observe_entity(e, ObserverKind::Add, A4::component_id(), i4_noop);
}

/// Debug-only: attaching to a despawned (dead) handle trips the live-entity
/// `debug_assert!`. `cfg(debug_assertions)` so the test is not compiled into a
/// release build (where the assert vanishes and the call silently no-ops).
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "live-entity contract")]
fn observe_entity_on_dead_handle_trips_debug_assert() {
    let mut ecs = EcsMaster::new();
    let arch_a = ecs.create_archetype(&[A4::component_id()]);
    let e = ecs.spawn_one(arch_a, A4(1)).expect("spawn");
    assert!(ecs.delete_entity(e), "despawn so the handle is dead");
    // Dead handle → debug_assert!(is_entity_live) fires.
    let _ = ecs.observe_entity(e, ObserverKind::Add, A4::component_id(), i4_noop);
}

// ════════════════════════════════════════════════════════════════════════════
// Item 5 — custom triggers: trigger::<E> fires global + entity-targeted;
//          trigger_global fires global only; payload is read correctly
// ════════════════════════════════════════════════════════════════════════════

/// A non-bubbling custom trigger carrying a value the observer records.
struct DamageEvent {
    amount: u32,
}
impl Trigger for DamageEvent {
    type Traversal = ChildOfTraversal;
    type Broadcast = ChildOf;
    // AUTO_PROPAGATE defaults to false.
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct A5(u32);

static I5_GLOBAL_SUM: AtomicUsize = AtomicUsize::new(0);
static I5_ENTITY_SUM: AtomicUsize = AtomicUsize::new(0);
static I5_GLOBAL_FIRES: AtomicUsize = AtomicUsize::new(0);
static I5_ENTITY_FIRES: AtomicUsize = AtomicUsize::new(0);

unsafe fn i5_global(_w: DeferredEcsMaster<'_>, _ctx: TriggerContext, event: *const u8) {
    // SAFETY: the trigger walk pins a live `DamageEvent` for the call.
    let ev = unsafe { &*(event as *const DamageEvent) };
    I5_GLOBAL_SUM.fetch_add(ev.amount as usize, SEQ);
    I5_GLOBAL_FIRES.fetch_add(1, SEQ);
}
unsafe fn i5_entity(_w: DeferredEcsMaster<'_>, _ctx: TriggerContext, event: *const u8) {
    let ev = unsafe { &*(event as *const DamageEvent) };
    I5_ENTITY_SUM.fetch_add(ev.amount as usize, SEQ);
    I5_ENTITY_FIRES.fetch_add(1, SEQ);
}

#[test]
fn custom_trigger_fires_global_and_entity_targeted_with_payload() {
    let mut ecs = EcsMaster::new();
    let arch_a = ecs.create_archetype(&[A5::component_id()]);
    let e = ecs.spawn_one(arch_a, A5(1)).expect("spawn");

    ecs.observe::<DamageEvent>(i5_global);
    ecs.observe_entity_event::<DamageEvent>(e, i5_entity);

    // trigger at e → both global and entity-targeted fire; payload read.
    ecs.trigger::<DamageEvent>(e, DamageEvent { amount: 7 });
    assert_eq!(I5_GLOBAL_FIRES.load(SEQ), 1, "global observer fired once");
    assert_eq!(I5_ENTITY_FIRES.load(SEQ), 1, "entity observer fired once");
    assert_eq!(I5_GLOBAL_SUM.load(SEQ), 7, "global observer read amount=7");
    assert_eq!(I5_ENTITY_SUM.load(SEQ), 7, "entity observer read amount=7");
}

#[test]
fn trigger_global_fires_global_only() {
    let mut ecs = EcsMaster::new();
    let arch_a = ecs.create_archetype(&[A5::component_id()]);
    let e = ecs.spawn_one(arch_a, A5(1)).expect("spawn");

    // Use a distinct trigger type so this test's counters never collide with
    // the targeted one above.
    struct G5;
    impl Trigger for G5 {
        type Traversal = ChildOfTraversal;
        type Broadcast = ChildOf;
    }
    static G5_GLOBAL: AtomicUsize = AtomicUsize::new(0);
    static G5_ENTITY: AtomicUsize = AtomicUsize::new(0);
    unsafe fn g5_global(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _e: *const u8) {
        G5_GLOBAL.fetch_add(1, SEQ);
    }
    unsafe fn g5_entity(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _e: *const u8) {
        G5_ENTITY.fetch_add(1, SEQ);
    }

    ecs.observe::<G5>(g5_global);
    ecs.observe_entity_event::<G5>(e, g5_entity);

    ecs.trigger_global::<G5>(G5);
    assert_eq!(G5_GLOBAL.load(SEQ), 1, "trigger_global fires the global observer");
    assert_eq!(
        G5_ENTITY.load(SEQ),
        0,
        "trigger_global does NOT fire any entity-targeted observer"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Item 6 — propagation up ChildOf + propagate(false) stop + non-bubbling O3
// ════════════════════════════════════════════════════════════════════════════

/// A bubbling custom trigger (AUTO_PROPAGATE = true) that walks up ChildOf.
struct BubbleEvent;
impl Trigger for BubbleEvent {
    const AUTO_PROPAGATE: bool = true;
    type Traversal = ChildOfTraversal;
    type Broadcast = ChildOf;
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct A6(u32);

#[derive(Bundle)]
struct A6Bundle {
    a: A6,
}

// Per-entity targeted observers, keyed by which generation each entity is. To
// distinguish child / parent / grandparent fires we register the same runner on
// all three and count total hops; ordering is checked via the global clock.
static I6_FIRES: AtomicUsize = AtomicUsize::new(0);

unsafe fn i6_bubble(_w: DeferredEcsMaster<'_>, _ctx: TriggerContext, _e: *const u8) {
    I6_FIRES.fetch_add(1, SEQ);
}

/// Builds a 3-level chain child -> parent -> grandparent via `add_child`, all
/// live after the apply window. Returns `[grandparent, parent, child]`.
fn build_chain(ecs: &mut EcsMaster) -> [Entity; 3] {
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let mut s = probe.lock().expect("lock");
        s.push(cmds.spawn(A6Bundle { a: A6(0) }).id()); // grandparent
        s.push(cmds.spawn(A6Bundle { a: A6(1) }).id()); // parent
        s.push(cmds.spawn(A6Bundle { a: A6(2) }).id()); // child
    });
    let ents = sink.lock().expect("lock").clone();
    let (gp, p, c) = (ents[0], ents[1], ents[2]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(gp).add_child(p);
        cmds.entity(p).add_child(c);
    });
    [gp, p, c]
}

#[test]
fn bubbling_trigger_walks_up_childof_to_grandparent() {
    let mut ecs = EcsMaster::new();
    let [gp, p, c] = build_chain(&mut ecs);

    ecs.observe_entity_event::<BubbleEvent>(gp, i6_bubble);
    ecs.observe_entity_event::<BubbleEvent>(p, i6_bubble);
    ecs.observe_entity_event::<BubbleEvent>(c, i6_bubble);

    // Trigger at the child → bubbles c -> p -> gp = 3 entity-targeted fires.
    ecs.trigger::<BubbleEvent>(c, BubbleEvent);
    assert_eq!(
        I6_FIRES.load(SEQ),
        3,
        "an AUTO_PROPAGATE event fired at the child bubbles to parent and grandparent (3 hops)"
    );
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct A6b(u32);
#[derive(Bundle)]
struct A6bBundle {
    a: A6b,
}

static I6B_CHILD: AtomicUsize = AtomicUsize::new(0);
static I6B_PARENT: AtomicUsize = AtomicUsize::new(0);

/// Child observer requests STOP propagation.
unsafe fn i6b_child_stop(_w: DeferredEcsMaster<'_>, _ctx: TriggerContext, _e: *const u8) {
    I6B_CHILD.fetch_add(1, SEQ);
    propagate(false);
}
unsafe fn i6b_parent(_w: DeferredEcsMaster<'_>, _ctx: TriggerContext, _e: *const u8) {
    I6B_PARENT.fetch_add(1, SEQ);
}

/// A NON-bubbling event whose observers may opt into propagation at runtime.
struct OptInEvent;
impl Trigger for OptInEvent {
    // AUTO_PROPAGATE defaults to false — runtime opt-in only.
    type Traversal = ChildOfTraversal;
    type Broadcast = ChildOf;
}

static I6_OPTIN_CHILD: AtomicUsize = AtomicUsize::new(0);
static I6_OPTIN_PARENT: AtomicUsize = AtomicUsize::new(0);

unsafe fn i6_optin_child(_w: DeferredEcsMaster<'_>, _ctx: TriggerContext, _e: *const u8) {
    I6_OPTIN_CHILD.fetch_add(1, SEQ);
    propagate(true);
}
unsafe fn i6_optin_parent(_w: DeferredEcsMaster<'_>, _ctx: TriggerContext, _e: *const u8) {
    I6_OPTIN_PARENT.fetch_add(1, SEQ);
}

/// TESTER FINDING (F1 — engine bug, see report). `propagate(false)` from an
/// observer must STOP an `AUTO_PROPAGATE = true` event's bubble before the next
/// hop (the documented contract: `docs/OBSERVERS-PLAN.md` line 40 / 184 + the
/// design doc `design_observers.md` line 259-273 / 509). The design's loop
/// condition is `if !get_propagate()` (the TLS seeded with `AUTO_PROPAGATE` by
/// `PropagateGuard`). The IMPLEMENTATION instead wrote
/// `if !(const { E::AUTO_PROPAGATE } || get_propagate())`
/// (`ecs_master.rs::trigger_walk`, line ~2455), whose
/// `const { AUTO_PROPAGATE } ||` short-circuit ELIDES the `get_propagate()`
/// read for an auto-propagate event — so `propagate(false)` is a silent no-op
/// and the bubble cannot be stopped. The companion test
/// `propagate_true_opt_in_bubbles_one_runtime_hop` proves the runtime-opt-in
/// `propagate(true)` half DOES work, isolating the defect to the
/// auto-propagate STOP case.
#[test]
fn propagate_false_from_observer_stops_the_bubble() {
    let mut ecs = EcsMaster::new();
    // Build a child -> parent chain via A6b.
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let mut s = probe.lock().expect("lock");
        s.push(cmds.spawn(A6bBundle { a: A6b(0) }).id()); // parent
        s.push(cmds.spawn(A6bBundle { a: A6b(1) }).id()); // child
    });
    let ents = sink.lock().expect("lock").clone();
    let (parent, child) = (ents[0], ents[1]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(child);
    });

    ecs.observe_entity_event::<BubbleEvent>(child, i6b_child_stop);
    ecs.observe_entity_event::<BubbleEvent>(parent, i6b_parent);

    // Trigger at the child; the child's observer calls propagate(false).
    ecs.trigger::<BubbleEvent>(child, BubbleEvent);
    assert_eq!(I6B_CHILD.load(SEQ), 1, "child observer fired once");
    assert_eq!(
        I6B_PARENT.load(SEQ),
        0,
        "propagate(false) from the child observer stops the bubble before the parent \
         (TESTER FINDING F1: trigger_walk's `const {{ AUTO_PROPAGATE }} ||` elides the \
         get_propagate() read for an auto-propagate event, so propagate(false) is a no-op)"
    );
}

/// Companion to F1: the runtime opt-in half DOES work. A NON-bubbling event
/// (`A6cNonBubble::AUTO_PROPAGATE == false`) whose target observer calls
/// `propagate(true)` bubbles exactly one runtime hop to the parent. This proves
/// the `get_propagate()` read is honoured when `AUTO_PROPAGATE == false` (the
/// const branch folds away), isolating F1 to the auto-propagate STOP case.
#[test]
fn propagate_true_opt_in_bubbles_one_runtime_hop() {
    let mut ecs = EcsMaster::new();
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let mut s = probe.lock().expect("lock");
        s.push(cmds.spawn(A6bBundle { a: A6b(0) }).id()); // parent
        s.push(cmds.spawn(A6bBundle { a: A6b(1) }).id()); // child
    });
    let ents = sink.lock().expect("lock").clone();
    let (parent, child) = (ents[0], ents[1]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(child);
    });

    // OptInEvent has AUTO_PROPAGATE = false; the child observer opts in.
    ecs.observe_entity_event::<OptInEvent>(child, i6_optin_child);
    ecs.observe_entity_event::<OptInEvent>(parent, i6_optin_parent);

    ecs.trigger::<OptInEvent>(child, OptInEvent);
    assert_eq!(I6_OPTIN_CHILD.load(SEQ), 1, "child observer fired and called propagate(true)");
    assert_eq!(
        I6_OPTIN_PARENT.load(SEQ),
        1,
        "a runtime propagate(true) opt-in bubbles one hop to the parent (the \
         non-auto-propagate path honours get_propagate())"
    );
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct A6c(u32);
#[derive(Bundle)]
struct A6cBundle {
    a: A6c,
}

static I6C_CHILD: AtomicUsize = AtomicUsize::new(0);
static I6C_PARENT: AtomicUsize = AtomicUsize::new(0);

unsafe fn i6c_child(_w: DeferredEcsMaster<'_>, _ctx: TriggerContext, _e: *const u8) {
    I6C_CHILD.fetch_add(1, SEQ);
}
unsafe fn i6c_parent(_w: DeferredEcsMaster<'_>, _ctx: TriggerContext, _e: *const u8) {
    I6C_PARENT.fetch_add(1, SEQ);
}

/// O3 — a non-bubbling trigger (AUTO_PROPAGATE = false, the `DamageEvent`-shape)
/// with NO observer-set `propagate(true)` fires ONLY at the target (one hop).
#[test]
fn non_bubbling_trigger_fires_only_at_target_one_hop() {
    let mut ecs = EcsMaster::new();
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let mut s = probe.lock().expect("lock");
        s.push(cmds.spawn(A6cBundle { a: A6c(0) }).id()); // parent
        s.push(cmds.spawn(A6cBundle { a: A6c(1) }).id()); // child
    });
    let ents = sink.lock().expect("lock").clone();
    let (parent, child) = (ents[0], ents[1]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(child);
    });

    // DamageEvent has AUTO_PROPAGATE = false.
    ecs.observe_entity_event::<DamageEvent>(child, i6c_child);
    ecs.observe_entity_event::<DamageEvent>(parent, i6c_parent);

    ecs.trigger::<DamageEvent>(child, DamageEvent { amount: 1 });
    assert_eq!(I6C_CHILD.load(SEQ), 1, "non-bubbling trigger fires at the target exactly once");
    assert_eq!(
        I6C_PARENT.load(SEQ),
        0,
        "O3: a non-bubbling trigger does NOT reach the parent (one hop only)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Item 9 — EntityCommands::observe deferred wrapper attaches + fires
// ════════════════════════════════════════════════════════════════════════════

struct CmdTrigger(u32);
impl Trigger for CmdTrigger {
    type Traversal = ChildOfTraversal;
    type Broadcast = ChildOf;
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct A9(u32);

static I9_FIRES: AtomicUsize = AtomicUsize::new(0);
static I9_SUM: AtomicUsize = AtomicUsize::new(0);

unsafe fn i9_obs(_w: DeferredEcsMaster<'_>, _ctx: TriggerContext, event: *const u8) {
    let ev = unsafe { &*(event as *const CmdTrigger) };
    I9_SUM.fetch_add(ev.0 as usize, SEQ);
    I9_FIRES.fetch_add(1, SEQ);
}

#[test]
fn entity_commands_observe_deferred_wrapper_attaches_and_fires() {
    let mut ecs = EcsMaster::new();
    let arch_a = ecs.create_archetype(&[A9::component_id()]);
    let e = ecs.spawn_one(arch_a, A9(1)).expect("spawn");

    // Defer-attach via EntityCommands::observe (applied at the next drain).
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).observe::<CmdTrigger>(i9_obs);
    });

    // Now trigger at e — the deferred attach is applied, so it fires.
    ecs.trigger::<CmdTrigger>(e, CmdTrigger(42));
    assert_eq!(I9_FIRES.load(SEQ), 1, "the deferred-attached entity observer fires");
    assert_eq!(I9_SUM.load(SEQ), 42, "it reads the trigger payload");
}

// A keep-alive use of `tick()` so the helper is exercised (the ordering tests
// in the despawn/reentrancy file use it; this asserts it is monotonic here).
#[test]
fn clock_is_monotonic() {
    let a = tick();
    let b = tick();
    assert!(b > a, "the shared sequence clock advances");
}
