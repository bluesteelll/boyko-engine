//! Feature 2 — on_despawn (pre-drop, intact-value read) + the parent-first
//! cascade + re-entrancy (deferred drain once; propagate-TLS save/restore).
//!
//! Split out from `feature2_observers_behavioral.rs` so the despawn-ordering
//! statics (sequence clock slots) never alias the trigger / migration counters.
//!
//! Items covered (`docs/OBSERVERS-PLAN.md` "Tests"):
//!
//! * **7a** on_despawn fires BEFORE drop — the handler reads intact component
//!   values via the read-only view.
//! * **7b** within-entity order Despawn -> Replace -> Remove.
//! * **7c** cascade: a 3-level subtree fires on_despawn once per entity,
//!   PARENT-first (FIX W10 — parent's on_despawn before its children's).
//! * **8a** re-entrancy: an observer that `commands().spawn()/despawn()` →
//!   deferred drain applies once at the outermost boundary (depth counter).
//! * **8b** re-entrancy: an observer firing another `trigger` → the propagate
//!   TLS is saved/restored (no cross-contamination of the outer walk).
//!
//! # Hook registration is PROCESS-GLOBAL + staleness-gated
//!
//! `register_component_hooks::<C>()` writes the process-global `HOOKS` table
//! (write-once per `ComponentId`) and PANICS if `C` was ever placed in a live
//! archetype of ANY world in this process. Every test therefore uses a FRESH
//! component type and registers its hook BEFORE creating any archetype that
//! holds it. The fresh types also keep the process-global table collision-free
//! across the concurrently-running tests in this binary.

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::trigger::{Trigger, TriggerContext};
use boyko_ecs::ecs::core::component::observers::propagate::propagate;
use boyko_ecs::ecs::core::component::observers::traversal::ChildOfTraversal;
use boyko_ecs::ecs::core::component::observers::ObserverContext;
use boyko_ecs::ecs::core::hierarchy::ChildOf;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

const SEQ: Ordering = Ordering::SeqCst;

/// Process-global monotonic clock recording the ORDER of fires.
static CLOCK: AtomicUsize = AtomicUsize::new(0);
#[inline]
fn tick() -> usize {
    CLOCK.fetch_add(1, SEQ)
}

// ════════════════════════════════════════════════════════════════════════════
// Item 7a — on_despawn fires BEFORE drop; handler reads the intact value
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Hp(u32);

static D7A_FIRES: AtomicUsize = AtomicUsize::new(0);
static D7A_SEEN_HP: AtomicUsize = AtomicUsize::new(usize::MAX);

/// on_despawn hook — reads the dying entity's STILL-INTACT `Hp` value through
/// the read-only view (proves it fires pre-drop).
unsafe fn d7a_on_despawn(w: DeferredEcsMaster<'_>, ctx: HookContext) {
    if let Some(hp) = w.get_component::<Hp>(ctx.entity) {
        D7A_SEEN_HP.store(hp.0 as usize, SEQ);
    }
    D7A_FIRES.fetch_add(1, SEQ);
}

#[test]
fn on_despawn_fires_before_drop_and_reads_intact_value() {
    let mut ecs = EcsMaster::new();
    // Register the hook BEFORE any archetype holding Hp exists.
    ecs.register_component_hooks::<Hp>().on_despawn(d7a_on_despawn).finish();

    let arch = ecs.create_archetype(&[Hp::component_id()]);
    let e = ecs.spawn_one(arch, Hp(99)).expect("spawn");
    assert_eq!(D7A_FIRES.load(SEQ), 0, "no despawn fire before delete");

    assert!(ecs.delete_entity(e), "despawn");
    assert_eq!(D7A_FIRES.load(SEQ), 1, "on_despawn fired exactly once");
    assert_eq!(
        D7A_SEEN_HP.load(SEQ),
        99,
        "on_despawn read the fully-intact Hp value (fired pre-drop, before remove)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Item 7b — within-entity order: Despawn -> Replace -> Remove (all pre-drop)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Ord1(u32);

static D7B_DESPAWN_AT: AtomicUsize = AtomicUsize::new(usize::MAX);
static D7B_REPLACE_AT: AtomicUsize = AtomicUsize::new(usize::MAX);
static D7B_REMOVE_AT: AtomicUsize = AtomicUsize::new(usize::MAX);

unsafe fn d7b_despawn(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    D7B_DESPAWN_AT.store(tick(), SEQ);
}
unsafe fn d7b_replace(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    D7B_REPLACE_AT.store(tick(), SEQ);
}
unsafe fn d7b_remove(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    D7B_REMOVE_AT.store(tick(), SEQ);
}

#[test]
fn despawn_within_entity_order_is_despawn_replace_remove() {
    let mut ecs = EcsMaster::new();
    ecs.register_component_hooks::<Ord1>()
        .on_despawn(d7b_despawn)
        .on_replace(d7b_replace)
        .on_remove(d7b_remove)
        .finish();

    let arch = ecs.create_archetype(&[Ord1::component_id()]);
    let e = ecs.spawn_one(arch, Ord1(1)).expect("spawn");
    assert!(ecs.delete_entity(e), "despawn");

    let (d, rep, rem) = (
        D7B_DESPAWN_AT.load(SEQ),
        D7B_REPLACE_AT.load(SEQ),
        D7B_REMOVE_AT.load(SEQ),
    );
    assert!(d != usize::MAX && rep != usize::MAX && rem != usize::MAX, "all three fired");
    assert!(
        d < rep && rep < rem,
        "within-entity order is Despawn({d}) -> Replace({rep}) -> Remove({rem})"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Item 7c — cascade: a 3-level subtree fires on_despawn PARENT-first (W10)
// ════════════════════════════════════════════════════════════════════════════

/// A marker carried by every subtree entity; its on_despawn records the fire
/// order keyed by a per-entity "depth" value smuggled in the component itself.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Node(u32);

#[derive(Bundle)]
struct NodeBundle {
    n: Node,
}

// Fire-order slots indexed by the Node's depth (0 = grandparent/root, 1 =
// parent, 2 = child).
static D7C_AT: [AtomicUsize; 3] = [
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
];
static D7C_FIRES: AtomicUsize = AtomicUsize::new(0);

unsafe fn d7c_on_despawn(w: DeferredEcsMaster<'_>, ctx: HookContext) {
    // Read the still-intact Node depth via the view (also exercises the
    // pre-drop read on the cascade path).
    if let Some(node) = w.get_component::<Node>(ctx.entity) {
        let depth = node.0 as usize;
        if depth < 3 {
            D7C_AT[depth].store(tick(), SEQ);
        }
    }
    D7C_FIRES.fetch_add(1, SEQ);
}

#[test]
fn cascade_despawn_fires_on_despawn_parent_first() {
    let mut ecs = EcsMaster::new();
    ecs.register_component_hooks::<Node>().on_despawn(d7c_on_despawn).finish();

    // Spawn root(depth0) -> mid(depth1) -> leaf(depth2), linked via add_child.
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let mut s = probe.lock().expect("lock");
        s.push(cmds.spawn(NodeBundle { n: Node(0) }).id()); // root
        s.push(cmds.spawn(NodeBundle { n: Node(1) }).id()); // mid
        s.push(cmds.spawn(NodeBundle { n: Node(2) }).id()); // leaf
    });
    let ents = sink.lock().expect("lock").clone();
    let (root, mid, leaf) = (ents[0], ents[1], ents[2]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(root).add_child(mid);
        cmds.entity(mid).add_child(leaf);
    });

    // Default-recursive despawn of the root → cascades the whole subtree.
    assert!(ecs.delete_entity(root), "despawn root cascades the subtree");
    assert!(!ecs.has_entity(mid), "mid was cascaded");
    assert!(!ecs.has_entity(leaf), "leaf was cascaded");

    assert_eq!(D7C_FIRES.load(SEQ), 3, "on_despawn fired once per subtree entity");
    let (a0, a1, a2) = (D7C_AT[0].load(SEQ), D7C_AT[1].load(SEQ), D7C_AT[2].load(SEQ));
    assert!(a0 != usize::MAX && a1 != usize::MAX && a2 != usize::MAX, "all three fired");
    assert!(
        a0 < a1 && a1 < a2,
        "PARENT-first cascade (W10): root({a0}) on_despawn before mid({a1}) before leaf({a2})"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Item 8a — re-entrancy: an observer that spawns/despawns → the deferred drain
//           applies once at the OUTERMOST boundary (depth counter)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Trig8(u32);

/// The component the re-entrant deferred spawn creates — its own on_add must
/// fire exactly once (proving the deferred command applied, and applied once).
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Spawned8(u32);

#[derive(Bundle)]
struct Spawned8Bundle {
    s: Spawned8,
}

static D8A_OBS_FIRES: AtomicUsize = AtomicUsize::new(0);
static D8A_SPAWNED_ADD: AtomicUsize = AtomicUsize::new(0);

unsafe fn d8a_on_remove_spawns(mut w: DeferredEcsMaster<'_>, _c: HookContext) {
    // Re-entrantly enqueue a deferred spawn. It must NOT apply inline; the
    // outermost owner drains it exactly once.
    D8A_OBS_FIRES.fetch_add(1, SEQ);
    w.commands().spawn(Spawned8Bundle { s: Spawned8(7) });
}
unsafe fn d8a_spawned_on_add(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    D8A_SPAWNED_ADD.fetch_add(1, SEQ);
}

#[test]
fn reentrant_deferred_spawn_from_observer_applies_once() {
    let mut ecs = EcsMaster::new();
    // Trig8's on_remove (fired at despawn) enqueues a deferred spawn.
    ecs.register_component_hooks::<Trig8>().on_remove(d8a_on_remove_spawns).finish();
    // Observe the SPAWNED component's on_add so we can count how many times the
    // deferred spawn applied.
    ecs.observe_on_add::<Spawned8>(d8a_spawned_on_add);
    let _ = Spawned8::component_id();

    let arch = ecs.create_archetype(&[Trig8::component_id()]);
    let e = ecs.spawn_one(arch, Trig8(1)).expect("spawn");

    // Despawn e → its on_remove re-entrantly enqueues a deferred spawn; the
    // outermost delete_entity drain applies it exactly once.
    assert!(ecs.delete_entity(e), "despawn");

    assert_eq!(D8A_OBS_FIRES.load(SEQ), 1, "the on_remove handler fired once");
    assert_eq!(
        D8A_SPAWNED_ADD.load(SEQ),
        1,
        "the re-entrant deferred spawn applied EXACTLY once at the outermost drain"
    );
    assert_eq!(ecs.entity_count(), 1, "exactly one entity remains (the deferred spawn)");
}

// ════════════════════════════════════════════════════════════════════════════
// Item 8b — propagation isolation across SEQUENTIAL trigger walks
//
// The plan's "observer fires another `trigger` re-entrantly → propagate TLS
// saved/restored" describes the `PropagateGuard` invariant. The read-only
// `DeferredEcsMaster` view handed to a `TriggerFn` intentionally exposes NO
// `trigger` (and no `&mut EcsMaster`), so a genuinely NESTED `trigger` from
// INSIDE an observer is not reachable through the public, safe API. The genuine
// NESTED `PropagateGuard::enter`/`drop` save+restore is therefore covered by an
// in-crate unit test in `observers/propagate.rs::tests` (added by the tester);
// here we pin the OBSERVABLE half through the public API with a SINGLE trigger
// type (so the test is not confounded by FINDING F2's TriggerId collapse): a
// walk whose observer leaves the TLS dirty must NOT contaminate a SUBSEQUENT
// walk (the per-walk `PropagateGuard::enter(AUTO_PROPAGATE)` re-seed).
// ════════════════════════════════════════════════════════════════════════════

/// A single non-bubbling event; observers may opt into the bubble at runtime.
struct LeakEvent;
impl Trigger for LeakEvent {
    // AUTO_PROPAGATE defaults to false — runtime opt-in only (so this test
    // avoids FINDING F1: the auto-propagate STOP path).
    type Traversal = ChildOfTraversal;
    type Broadcast = ChildOf;
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct A8b(u32);
#[derive(Bundle)]
struct A8bBundle {
    a: A8b,
}

static D8B_CHILD: AtomicUsize = AtomicUsize::new(0);
static D8B_PARENT: AtomicUsize = AtomicUsize::new(0);
static D8B_WALK: AtomicUsize = AtomicUsize::new(0);

/// The child observer's behaviour depends on which walk is running:
/// * walk 1: leave the TLS DIRTY by calling `propagate(true)` (and the parent
///   has no observer, so the dirty flag would, if leaked, make walk 2 bubble
///   spuriously);
/// * walk 2: call NOTHING — if the dirty `true` from walk 1 leaked, the bubble
///   would reach the parent; if `PropagateGuard` re-seeds (false) it stops.
unsafe fn d8b_child(_w: DeferredEcsMaster<'_>, _ctx: TriggerContext, _e: *const u8) {
    D8B_CHILD.fetch_add(1, SEQ);
    if D8B_WALK.load(SEQ) == 1 {
        propagate(true); // leave the TLS dirty after walk 1
    }
    // walk 2: deliberately do NOT touch the TLS.
}
unsafe fn d8b_parent(_w: DeferredEcsMaster<'_>, _ctx: TriggerContext, _e: *const u8) {
    D8B_PARENT.fetch_add(1, SEQ);
}

#[test]
fn propagate_does_not_leak_across_sequential_walks() {
    let mut ecs = EcsMaster::new();
    let ents = {
        let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
        let probe = Arc::clone(&sink);
        ecs.run_system(move |mut cmds: Commands| {
            let mut s = probe.lock().expect("lock");
            s.push(cmds.spawn(A8bBundle { a: A8b(0) }).id()); // parent
            s.push(cmds.spawn(A8bBundle { a: A8b(1) }).id()); // child
        });
        sink.lock().expect("lock").clone()
    };
    let (parent, child) = (ents[0], ents[1]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(child);
    });

    ecs.observe_entity_event::<LeakEvent>(child, d8b_child);
    ecs.observe_entity_event::<LeakEvent>(parent, d8b_parent);

    // Walk 1: the child opts in (propagate(true)), so the bubble reaches the
    // parent AND the TLS is left dirty (`true`).
    D8B_WALK.store(1, SEQ);
    ecs.trigger::<LeakEvent>(child, LeakEvent);
    assert_eq!(D8B_CHILD.load(SEQ), 1, "walk 1 fired at the child");
    assert_eq!(D8B_PARENT.load(SEQ), 1, "walk 1 opted in -> bubbled to the parent");

    // Walk 2: the child touches NOTHING. If walk 1's dirty `true` leaked, the
    // bubble would reach the parent again; `PropagateGuard::enter(false)` must
    // re-seed so the walk stops at the child.
    D8B_WALK.store(2, SEQ);
    ecs.trigger::<LeakEvent>(child, LeakEvent);
    assert_eq!(D8B_CHILD.load(SEQ), 2, "walk 2 fired at the child");
    assert_eq!(
        D8B_PARENT.load(SEQ),
        1,
        "walk 2 did NOT bubble (parent still 1): walk 1's propagate(true) did not leak \
         — each walk re-seeds the TLS via PropagateGuard"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Item 5/F2 — TESTER FINDING (engine bug): an entity observing TWO distinct
// custom-trigger types fires BOTH observers for EITHER trigger, because
// `static_trigger_id::<E>` collapses all `Trigger` types to one `TriggerId`
// (the function-local `static CACHE` in a generic fn is shared across
// monomorphisations — the Phase-12.5 collapse class). Behavioral repro at the
// public-API level. The clean unit-level proof is
// `observers::trigger::tests::distinct_trigger_types_get_distinct_ids`.
// ════════════════════════════════════════════════════════════════════════════

struct EventX;
impl Trigger for EventX {
    type Traversal = ChildOfTraversal;
    type Broadcast = ChildOf;
}
struct EventY;
impl Trigger for EventY {
    type Traversal = ChildOfTraversal;
    type Broadcast = ChildOf;
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Af2(u32);

static F2_X: AtomicUsize = AtomicUsize::new(0);
static F2_Y: AtomicUsize = AtomicUsize::new(0);

unsafe fn f2_x(_w: DeferredEcsMaster<'_>, _ctx: TriggerContext, _e: *const u8) {
    F2_X.fetch_add(1, SEQ);
}
unsafe fn f2_y(_w: DeferredEcsMaster<'_>, _ctx: TriggerContext, _e: *const u8) {
    F2_Y.fetch_add(1, SEQ);
}

#[test]
fn distinct_trigger_types_do_not_cross_fire_on_one_entity() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[Af2::component_id()]);
    let e = ecs.spawn_one(arch, Af2(1)).expect("spawn");

    // Observe TWO distinct trigger types on the SAME entity.
    ecs.observe_entity_event::<EventX>(e, f2_x);
    ecs.observe_entity_event::<EventY>(e, f2_y);

    // Trigger ONLY EventX. Only f2_x must fire; f2_y must stay 0.
    ecs.trigger::<EventX>(e, EventX);
    assert_eq!(F2_X.load(SEQ), 1, "EventX's observer fired");
    assert_eq!(
        F2_Y.load(SEQ),
        0,
        "EventY's observer must NOT fire for an EventX trigger (TESTER FINDING F2: \
         static_trigger_id collapses all Trigger types to one TriggerId, so both \
         entity observers share one DispatchKey and cross-fire)"
    );
}
