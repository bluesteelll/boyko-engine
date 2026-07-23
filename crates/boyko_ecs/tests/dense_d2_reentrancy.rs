//! Dense plan D2 — the PINNED re-entrancy soundness gate (code-reviewer's ask;
//! the Phase-19 / 14a lesson: "an APPROVED-by-review-rounds plan still hid two
//! soundness bugs that ONLY Miri-TB caught").
//!
//! # The case being pinned
//!
//! A dense component's lifecycle observer, while firing SYNCHRONOUSLY inside a
//! structural op that is mutating its OWN dense store, enqueues a structural
//! mutation of a *different* dense-bearing entity:
//!
//! * `on_remove`/`on_despawn` observer ⇒ enqueue a DESPAWN / a dense REMOVE of a
//!   different dense-bearing entity;
//! * `on_add` observer ⇒ enqueue an INSERT of a dense component onto a different
//!   entity.
//!
//! # Why it MUST be sound (the model under test)
//!
//! The view handed to an observer is [`DeferredEcsMaster`] — a read-only world
//! view whose ONLY structural surface is `commands()`, which pushes into the
//! WORLD-RESIDENT `deferred_hook_queue`. That queue drains ONLY at the OUTERMOST
//! apply boundary (depth 0). So no observer can perform a SYNCHRONOUS re-entrant
//! dense mutation: the in-fire `DenseStore` borrow / `solve_view` snapshot held
//! by the structural op above can never be invalidated mid-fire. The deferred
//! op is applied exactly once, strictly AFTER the current op completes.
//!
//! Expected result: **PASS** under Miri-TB — no UB, no double-tombstone, no
//! double-fire, the deferred op applies exactly once, and the e2s/s2e dense
//! invariant holds for every surviving entity (verified at the public boundary
//! via `dense_slot_of` + `dense_get_raw` round-trips — the externally observable
//! consequence of `DenseStore::check_invariant`).
//!
//! If this ever observes a synchronous re-entrant dense mutation (a value that
//! reads back wrong, a slot collision, a double-fire, or Miri flags UB), that is
//! a REAL defect — STOP and report, do not "adjust" the assertion.
//!
//! # Running under Miri (Tree Borrows)
//!
//! ```powershell
//! $env:MIRIFLAGS = "-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! cargo +nightly miri test -p boyko-ecs --test dense_d2_reentrancy
//! ```
//!
//! `-Zmiri-ignore-leaks` is required for the SAME reason as every other
//! `EcsMaster`-constructing Miri test in this crate (`multi_world.rs`,
//! `miri_pool_growth.rs`, `dense_d1_miri.rs`, …): an `EcsMaster`'s VM-reserved /
//! arena-style allocations are by-design not freed at `Drop` (the pre-existing
//! by-design spawn-cache / reservation leak, issue #53). Those leaks are NOT a
//! soundness signal — the load-bearing result is the absence of Undefined
//! Behavior and the `test result: ok` line.

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::{ObserverContext, ObserverKind};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::{Bundle, Component};

const SEQ: Ordering = Ordering::SeqCst;

/// 16-byte POD dense payload.
#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct DPos {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

/// A plain table component so dense rides alongside a real archetype.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct RTag(u32);

/// Send+Sync cell stashing an `Entity` into a `static` for bare-fn observers.
struct TargetCell(AtomicU64);
impl TargetCell {
    const fn new() -> Self {
        Self(AtomicU64::new(u64::MAX))
    }
    fn set(&self, e: Entity) {
        self.0.store((e.id().0 as u64) | ((e.generation() as u64) << 32), SEQ);
    }
    fn get(&self) -> Entity {
        let packed = self.0.load(SEQ);
        Entity::new(EntityId((packed & 0xFFFF_FFFF) as usize), (packed >> 32) as u32)
    }
}

/// Reads a dense value back through the PUBLIC accessor round-trip, asserting the
/// e2s/s2e invariant holds at the boundary: the entity maps to a slot whose value
/// equals `expected`. (The externally observable face of `check_invariant`.)
fn assert_dense_value(ecs: &EcsMaster, e: Entity, cid: boyko_ecs::ecs::identifiers::primitives::ComponentId, expected: DPos, ctx: &str) {
    assert!(ecs.dense_contains(e, cid), "{ctx}: entity must be a live dense member");
    let slot = ecs.dense_slot_of(e, cid).expect("live member has a slot");
    let raw = ecs.dense_get_raw(e, cid).expect("live member has a raw value");
    // SAFETY: `raw` points at the live dense value for the read's duration; the
    //   `&EcsMaster` borrow keeps the column alive and no mutation runs here.
    let got = unsafe { *(raw as *const DPos) };
    assert_eq!(got, expected, "{ctx}: dense value at slot {slot} round-trips correctly");
}

/// Spawns `(RTag, D)` via `Commands::spawn`, returning the live handle.
fn spawn<D>(ecs: &mut EcsMaster, make: impl Fn() -> D + Send + Sync + 'static) -> Entity
where
    D: boyko_ecs::ecs::core::bundle::Bundle + Send + Sync,
{
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        probe.lock().expect("lock").push(cmds.spawn(make()).id());
    });
    sink.lock().expect("lock")[0]
}

// ════════════════════════════════════════════════════════════════════════════
// PINNED CASE A — dense on_remove observer enqueues a DESPAWN of a DIFFERENT
//                 dense-bearing entity. The despawn defers to depth-0 drain.
// ════════════════════════════════════════════════════════════════════════════

static A_TRIGGER_REMOVE_FIRES: AtomicUsize = AtomicUsize::new(0);
static A_VICTIM_DESPAWN_FIRES: AtomicUsize = AtomicUsize::new(0);
static A_VICTIM: TargetCell = TargetCell::new();
/// Was the victim STILL a live dense member at the instant the trigger's
/// on_remove fired? It MUST be — the enqueued despawn is deferred, never inline.
static A_VICTIM_ALIVE_DURING_FIRE: AtomicUsize = AtomicUsize::new(0);

#[derive(Component)]
#[component(storage = "dense")]
#[repr(C)]
struct ATrigger(DPos);

#[derive(Component)]
#[component(storage = "dense")]
#[repr(C)]
struct AVictim(DPos);

#[derive(Bundle)]
struct ATriggerBundle {
    t: RTag,
    d: ATrigger,
}
#[derive(Bundle)]
struct AVictimBundle {
    t: RTag,
    d: AVictim,
}

/// Fires while `dense_remove_and_fire` (or the despawn walk) is mid-mutating the
/// `ATrigger` store. Enqueues a despawn of a DIFFERENT dense-bearing entity.
unsafe fn a_trigger_on_remove(mut w: DeferredEcsMaster<'_>, _ctx: ObserverContext) {
    A_TRIGGER_REMOVE_FIRES.fetch_add(1, SEQ);
    let victim = A_VICTIM.get();
    // The victim's dense membership must STILL be live here — the despawn we are
    // about to enqueue is deferred, so nothing has touched the victim yet.
    if w.get_component::<RTag>(victim).is_some() {
        A_VICTIM_ALIVE_DURING_FIRE.fetch_add(1, SEQ);
    }
    w.commands().despawn(victim);
}
unsafe fn a_victim_on_despawn(_w: DeferredEcsMaster<'_>, _ctx: ObserverContext) {
    A_VICTIM_DESPAWN_FIRES.fetch_add(1, SEQ);
}

#[test]
fn dense_on_remove_observer_deferred_despawn_of_other_dense_entity_is_sound() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_remove::<ATrigger>(a_trigger_on_remove);
    ecs.add_observer(ObserverKind::Despawn, AVictim::component_id(), a_victim_on_despawn);

    let trig_pos = DPos { x: 1.0, y: 2.0, z: 3.0, w: 4.0 };
    let vic_pos = DPos { x: 5.0, y: 6.0, z: 7.0, w: 8.0 };
    let trigger = spawn(&mut ecs, move || ATriggerBundle { t: RTag(1), d: ATrigger(trig_pos) });
    let victim = spawn(&mut ecs, move || AVictimBundle { t: RTag(2), d: AVictim(vic_pos) });
    A_VICTIM.set(victim);

    // Sanity: both live with correct dense values BEFORE the trigger.
    assert_dense_value(&ecs, trigger, ATrigger::component_id(), trig_pos, "pre: trigger");
    assert_dense_value(&ecs, victim, AVictim::component_id(), vic_pos, "pre: victim");

    A_TRIGGER_REMOVE_FIRES.store(0, SEQ);
    A_VICTIM_DESPAWN_FIRES.store(0, SEQ);
    A_VICTIM_ALIVE_DURING_FIRE.store(0, SEQ);

    // Remove the trigger's dense component. Its on_remove observer fires inline
    // (PRE-tombstone, reading the dying value), enqueues the victim despawn; the
    // outermost drain applies that despawn ONCE, firing the victim's on_despawn.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(trigger).remove::<ATrigger>();
    });

    // Trigger's on_remove fired exactly once.
    assert_eq!(A_TRIGGER_REMOVE_FIRES.load(SEQ), 1, "trigger on_remove fires exactly once");
    // The victim was provably still LIVE at the moment the observer fired — proof
    // the despawn was DEFERRED, not a synchronous re-entrant dense mutation.
    assert_eq!(
        A_VICTIM_ALIVE_DURING_FIRE.load(SEQ),
        1,
        "victim still live during the in-fire observer (despawn was deferred, not inline)"
    );
    // The deferred despawn applied EXACTLY once: victim's on_despawn fired once.
    assert_eq!(
        A_VICTIM_DESPAWN_FIRES.load(SEQ),
        1,
        "victim on_despawn fires EXACTLY once — deferred despawn applied once, no double-apply"
    );

    // Final state: trigger dense tombstoned, victim fully gone, no migration of
    // trigger's table archetype. The trigger's table comp survives.
    assert!(!ecs.dense_contains(trigger, ATrigger::component_id()), "trigger dense tombstoned");
    assert!(ecs.get_component_raw(trigger, RTag::component_id()).is_some(), "trigger table survives");
    assert!(!ecs.dense_contains(victim, AVictim::component_id()), "victim dense tombstoned by despawn");
    assert!(!ecs.has_entity(victim), "victim entity fully despawned");
    assert!(ecs.has_entity(trigger), "trigger entity still live (only its dense comp removed)");
}

// ════════════════════════════════════════════════════════════════════════════
// PINNED CASE B — dense on_despawn observer enqueues a dense REMOVE of a
//                 DIFFERENT dense-bearing entity. Defers to depth-0 drain.
// ════════════════════════════════════════════════════════════════════════════

static B_TRIGGER_DESPAWN_FIRES: AtomicUsize = AtomicUsize::new(0);
static B_VICTIM_REMOVE_FIRES: AtomicUsize = AtomicUsize::new(0);
static B_VICTIM: TargetCell = TargetCell::new();
static B_VICTIM_DENSE_ALIVE_DURING_FIRE: AtomicUsize = AtomicUsize::new(0);

#[derive(Component)]
#[component(storage = "dense")]
#[repr(C)]
struct BTrigger(DPos);

#[derive(Component, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct BVictim(DPos);

#[derive(Bundle)]
struct BTriggerBundle {
    t: RTag,
    d: BTrigger,
}
#[derive(Bundle)]
struct BVictimBundle {
    t: RTag,
    d: BVictim,
}
#[derive(Bundle)]
struct BVictimDenseOnly {
    d: BVictim,
}

/// Fires during the trigger's despawn walk (mid-mutating the trigger's dense
/// stores). Enqueues a dense REMOVE of BVictim from a DIFFERENT entity.
unsafe fn b_trigger_on_despawn(mut w: DeferredEcsMaster<'_>, _ctx: ObserverContext) {
    B_TRIGGER_DESPAWN_FIRES.fetch_add(1, SEQ);
    let victim = B_VICTIM.get();
    // The victim's dense membership must STILL be live — the remove is deferred.
    if w.get_component::<RTag>(victim).is_some() {
        B_VICTIM_DENSE_ALIVE_DURING_FIRE.fetch_add(1, SEQ);
    }
    w.commands().entity(victim).remove::<BVictim>();
}
unsafe fn b_victim_on_remove(_w: DeferredEcsMaster<'_>, _ctx: ObserverContext) {
    B_VICTIM_REMOVE_FIRES.fetch_add(1, SEQ);
}

#[test]
fn dense_on_despawn_observer_deferred_remove_of_other_dense_entity_is_sound() {
    let mut ecs = EcsMaster::new();
    ecs.add_observer(ObserverKind::Despawn, BTrigger::component_id(), b_trigger_on_despawn);
    ecs.observe_on_remove::<BVictim>(b_victim_on_remove);

    let trig_pos = DPos { x: 9.0, y: 9.0, z: 9.0, w: 9.0 };
    let vic_pos = DPos { x: 3.0, y: 1.0, z: 4.0, w: 1.0 };
    let trigger = spawn(&mut ecs, move || BTriggerBundle { t: RTag(1), d: BTrigger(trig_pos) });
    let victim = spawn(&mut ecs, move || BVictimBundle { t: RTag(2), d: BVictim(vic_pos) });
    B_VICTIM.set(victim);

    assert_dense_value(&ecs, victim, BVictim::component_id(), vic_pos, "pre: victim");

    B_TRIGGER_DESPAWN_FIRES.store(0, SEQ);
    B_VICTIM_REMOVE_FIRES.store(0, SEQ);
    B_VICTIM_DENSE_ALIVE_DURING_FIRE.store(0, SEQ);

    // Despawn the trigger. Its on_despawn observer fires during the despawn walk,
    // enqueues the victim's dense remove; the outermost drain applies it ONCE.
    ecs.delete_entity(trigger);

    assert_eq!(B_TRIGGER_DESPAWN_FIRES.load(SEQ), 1, "trigger on_despawn fires exactly once");
    assert_eq!(
        B_VICTIM_DENSE_ALIVE_DURING_FIRE.load(SEQ),
        1,
        "victim dense still live during the in-fire observer (remove was deferred)"
    );
    assert_eq!(
        B_VICTIM_REMOVE_FIRES.load(SEQ),
        1,
        "victim on_remove fires EXACTLY once — deferred dense remove applied once, no double-apply"
    );

    // Final state: trigger gone; victim entity still alive but its dense comp
    // tombstoned. The victim's other state (RTag) is intact, no migration crash.
    assert!(!ecs.has_entity(trigger), "trigger fully despawned");
    assert!(!ecs.dense_contains(trigger, BTrigger::component_id()), "trigger dense tombstoned");
    assert!(ecs.has_entity(victim), "victim entity still live");
    assert!(!ecs.dense_contains(victim, BVictim::component_id()), "victim dense removed by deferred op");
    assert!(ecs.get_component_raw(victim, RTag::component_id()).is_some(), "victim table survives");

    // The dense store stays consistent: re-inserting a fresh BVictim member must
    // round-trip (proves the store's free-list / e2s / s2e survived the deferred
    // tombstone — the check_invariant face). Reuse the just-freed slot is fine.
    let other_pos = DPos { x: 2.0, y: 7.0, z: 1.0, w: 8.0 };
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(victim).insert(BVictimDenseOnly { d: BVictim(other_pos) });
    });
    assert_dense_value(&ecs, victim, BVictim::component_id(), other_pos, "post: re-inserted victim");
}

// ════════════════════════════════════════════════════════════════════════════
// PINNED CASE C — dense on_add observer enqueues an INSERT of a dense component
//                 onto a DIFFERENT entity. Defers to depth-0 drain.
// ════════════════════════════════════════════════════════════════════════════

static C_TRIGGER_ADD_FIRES: AtomicUsize = AtomicUsize::new(0);
static C_VICTIM_ADD_FIRES: AtomicUsize = AtomicUsize::new(0);
static C_VICTIM: TargetCell = TargetCell::new();
/// Was the victim's dense membership ABSENT at the instant the trigger's on_add
/// fired? It MUST be — the enqueued insert is deferred, never inline.
static C_VICTIM_ABSENT_DURING_FIRE: AtomicUsize = AtomicUsize::new(0);

#[derive(Component)]
#[component(storage = "dense")]
#[repr(C)]
struct CTrigger(DPos);

#[derive(Component, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct CVictim(DPos);

#[derive(Bundle)]
struct CTriggerBundle {
    t: RTag,
    d: CTrigger,
}
#[derive(Bundle)]
struct CVictimInsert {
    d: CVictim,
}

const C_VICTIM_POS: DPos = DPos { x: 4.0, y: 2.0, z: 4.0, w: 2.0 };

/// Fires during the trigger's `dense_insert_and_fire` (mid-mutating the CTrigger
/// store). Enqueues an INSERT of CVictim onto a DIFFERENT, pre-existing entity.
unsafe fn c_trigger_on_add(mut w: DeferredEcsMaster<'_>, _ctx: ObserverContext) {
    C_TRIGGER_ADD_FIRES.fetch_add(1, SEQ);
    let victim = C_VICTIM.get();
    // The victim is alive (its table comp is visible) but its CVictim dense
    // membership must be ABSENT at this instant — the insert we are about to
    // enqueue is deferred, so nothing has materialised the membership inline.
    if w.has_parent(victim) && w.get_component::<RTag>(victim).is_some() {
        C_VICTIM_ABSENT_DURING_FIRE.fetch_add(1, SEQ);
    }
    w.commands().entity(victim).insert(CVictimInsert { d: CVictim(C_VICTIM_POS) });
}
unsafe fn c_victim_on_add(_w: DeferredEcsMaster<'_>, _ctx: ObserverContext) {
    C_VICTIM_ADD_FIRES.fetch_add(1, SEQ);
}

#[test]
fn dense_on_add_observer_deferred_insert_onto_other_entity_is_sound() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_add::<CTrigger>(c_trigger_on_add);
    ecs.observe_on_add::<CVictim>(c_victim_on_add);
    // Pre-register CVictim so its dense store / observer bit exist before the
    // deferred insert constructs the membership.
    let _ = CVictim::component_id();

    // A pre-existing table-only victim (no CVictim yet).
    let victim = spawn(&mut ecs, || RTag(2));
    C_VICTIM.set(victim);
    assert!(!ecs.dense_contains(victim, CVictim::component_id()), "victim has no dense pre-trigger");

    C_TRIGGER_ADD_FIRES.store(0, SEQ);
    C_VICTIM_ADD_FIRES.store(0, SEQ);
    C_VICTIM_ABSENT_DURING_FIRE.store(0, SEQ);

    // Spawn the trigger. Its on_add observer fires during dense_insert_and_fire,
    // enqueues the victim insert; the outermost drain applies it ONCE.
    let trig_pos = DPos { x: 1.0, y: 1.0, z: 2.0, w: 3.0 };
    let trigger = spawn(&mut ecs, move || CTriggerBundle { t: RTag(1), d: CTrigger(trig_pos) });

    assert_eq!(C_TRIGGER_ADD_FIRES.load(SEQ), 1, "trigger on_add fires exactly once");
    assert_eq!(
        C_VICTIM_ABSENT_DURING_FIRE.load(SEQ),
        1,
        "victim observed pre-insert during the in-fire observer (insert was deferred)"
    );
    assert_eq!(
        C_VICTIM_ADD_FIRES.load(SEQ),
        1,
        "victim on_add fires EXACTLY once — deferred insert applied once, no double-apply"
    );

    // Final state: the deferred insert materialised the victim's dense membership
    // with the correct value (e2s/s2e round-trip), and the trigger's own dense is
    // intact at its own slot — distinct from the victim's (no slot collision).
    assert_dense_value(&ecs, victim, CVictim::component_id(), C_VICTIM_POS, "post: victim insert");
    assert_dense_value(&ecs, trigger, CTrigger::component_id(), trig_pos, "post: trigger intact");
    assert_ne!(
        ecs.dense_slot_of(victim, CVictim::component_id()),
        None,
        "victim got a dense slot"
    );
}
