//! Phase 14b — component lifecycle OBSERVER firing-matrix integration tests.
//!
//! Observers are the runtime-mutable sibling of the Phase 14a hooks: a
//! `(kind, component)`-keyed, `add`/`remove`-able list of fn-ptr runners that
//! fire at the same six structural-op sites, AFTER the per-component hook. This
//! file pins the correctness spec from the architect plan §11 (R2 + R3):
//!
//! | case | what                                                              |
//! |------|-------------------------------------------------------------------|
//! | 1    | all 4 kinds fire (add+insert on spawn; replace+remove on despawn)  |
//! | 2    | multiplicity: N=3 observers on `(Add, C)` all fire, in order       |
//! | 3    | component-targeting: an observer on C does NOT fire for D-only     |
//! | 7    | add-Nth: a 2nd observer on `(Add,C)`; bit already set; both fire   |
//! | 8    | remove-non-last: 2 observers, remove one; the other still fires    |
//! | 9    | sibling last-removal (W3): {A,B} both observed; bit clears only    |
//! |      | when BOTH last observers removed                                   |
//! | 10   | remove-last clears OBSERVER bit but the derive HOOK is preserved   |
//!
//! # Why `static` counters
//!
//! An `ObserverFn` is a bare `unsafe fn` pointer — it cannot capture. Each test
//! therefore owns a private set of module-level `static AtomicUsize` counters
//! plus its own component types, so concurrently-running tests never observe one
//! another's fires (the global registries are process-wide in the test binary).
//!
//! # Component-id strategy
//!
//! Observer tests register against a component's `component_id()`; we use
//! `#[derive(Component)]` types whose ids are minted lazily from the global
//! atomic counter (`register_new`) — they never collide with the explicit
//! `register_layout` slots the bench files use, nor with each other.

use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::{ObserverContext, ObserverKind};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

const SEQ: Ordering = Ordering::SeqCst;

// ════════════════════════════════════════════════════════════════════════════
// Case 1 — all four observer kinds fire at their own site
// ════════════════════════════════════════════════════════════════════════════

static C1_ADD: AtomicUsize = AtomicUsize::new(0);
static C1_INSERT: AtomicUsize = AtomicUsize::new(0);
static C1_REPLACE: AtomicUsize = AtomicUsize::new(0);
static C1_REMOVE: AtomicUsize = AtomicUsize::new(0);

unsafe fn c1_add(_w: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    assert_eq!(ctx.kind, ObserverKind::Add, "ctx.kind matches the registered kind");
    C1_ADD.fetch_add(1, SEQ);
}
unsafe fn c1_insert(_w: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    assert_eq!(ctx.kind, ObserverKind::Insert, "ctx.kind matches the registered kind");
    C1_INSERT.fetch_add(1, SEQ);
}
unsafe fn c1_replace(_w: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    assert_eq!(ctx.kind, ObserverKind::Replace, "ctx.kind matches the registered kind");
    C1_REPLACE.fetch_add(1, SEQ);
}
unsafe fn c1_remove(_w: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    assert_eq!(ctx.kind, ObserverKind::Remove, "ctx.kind matches the registered kind");
    C1_REMOVE.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C1Comp(u32);

// Case 1b — in-place replace observers (separate type so case 1 and 1b never
// share counters).
static C1B_ADD: AtomicUsize = AtomicUsize::new(0);
static C1B_REPLACE: AtomicUsize = AtomicUsize::new(0);
static C1B_INSERT: AtomicUsize = AtomicUsize::new(0);

unsafe fn c1b_add(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    C1B_ADD.fetch_add(1, SEQ);
}
unsafe fn c1b_replace(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    C1B_REPLACE.fetch_add(1, SEQ);
}
unsafe fn c1b_insert(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    C1B_INSERT.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C1bComp(u32);

#[derive(Bundle)]
struct C1bBundle {
    c: C1bComp,
}

/// Case 1 — all four kinds fire, exercised over the DIRECT-API structural ops
/// (`spawn_one` fires add+insert; `delete_entity` fires replace+remove per
/// dying component, D1). The deferred-command apply paths (in-place
/// `cmds.insert`, deferred `cmds.spawn`, the migration ops) are covered
/// separately by cases F1/F2/F3 — they currently FAIL (TESTER FINDING: the
/// 14b fire-site widening missed the command-apply sites). This case proves the
/// four kinds CAN fire and route the correct `ObserverContext.kind`.
#[test]
fn all_four_observer_kinds_fire_at_their_site() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_add::<C1Comp>(c1_add);
    ecs.observe_on_insert::<C1Comp>(c1_insert);
    ecs.observe_on_replace::<C1Comp>(c1_replace);
    ecs.observe_on_remove::<C1Comp>(c1_remove);

    let arch = ecs.create_archetype(&[C1Comp::component_id()]);

    // DIRECT spawn ⇒ add + insert
    let e = ecs.spawn_one(arch, C1Comp(1)).expect("spawn");
    assert_eq!(C1_ADD.load(SEQ), 1, "spawn fired on_add observer once");
    assert_eq!(C1_INSERT.load(SEQ), 1, "spawn fired on_insert observer once");
    assert_eq!(C1_REPLACE.load(SEQ), 0, "spawn does not fire replace");
    assert_eq!(C1_REMOVE.load(SEQ), 0, "spawn does not fire remove");

    // DIRECT despawn ⇒ replace + remove (D1 — despawn fires BOTH per dying
    // component; routes through delete_entity -> fire_despawn_hooks, which IS
    // wired for observers).
    assert!(ecs.delete_entity(e), "despawn");
    assert_eq!(C1_REPLACE.load(SEQ), 1, "despawn fired on_replace observer once");
    assert_eq!(C1_REMOVE.load(SEQ), 1, "despawn fired on_remove observer once");
    assert_eq!(C1_ADD.load(SEQ), 1, "despawn did not fire add (still 1)");
    assert_eq!(C1_INSERT.load(SEQ), 1, "despawn did not fire insert (still 1)");
}

/// Case 1b — in-place replace (`cmds.entity(e).insert` of the SAME archetype
/// shape) should fire on_replace (OLD) then on_insert (NEW), no on_add. This is
/// the `apply_replace_in_place` deferred path. TESTER FINDING: it currently
/// does NOT fire observers (only hooks) — `insert_command.rs` gates only
/// `ON_REPLACE_HOOK` / `ON_INSERT_HOOK`. The 14a HOOK analog
/// (phase14a_hooks_firing.rs::insert_in_place_fires_replace_old_then_insert_new_no_add)
/// PASSES.
#[test]
fn in_place_replace_fires_replace_and_insert_observers() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_add::<C1bComp>(c1b_add);
    ecs.observe_on_replace::<C1bComp>(c1b_replace);
    ecs.observe_on_insert::<C1bComp>(c1b_insert);

    let arch = ecs.create_archetype(&[C1bComp::component_id()]);
    let e = ecs.spawn_one(arch, C1bComp(100)).expect("spawn");
    // Reset the spawn's add/insert fires — assert only about the in-place op.
    C1B_ADD.store(0, SEQ);
    C1B_INSERT.store(0, SEQ);
    C1B_REPLACE.store(0, SEQ);

    // Insert the SAME bundle shape ⇒ replace-in-place (target == source).
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(C1bBundle { c: C1bComp(200) });
    });

    assert_eq!(C1B_ADD.load(SEQ), 0, "in-place replace must NOT fire on_add");
    assert_eq!(
        C1B_REPLACE.load(SEQ),
        1,
        "in-place replace must fire on_replace observer (TESTER FINDING: \
         apply_replace_in_place gates only ON_REPLACE_HOOK)"
    );
    assert_eq!(
        C1B_INSERT.load(SEQ),
        1,
        "in-place replace must fire on_insert observer (TESTER FINDING: \
         apply_replace_in_place calls only trigger_on_insert)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Case 2 — multiplicity: N=3 observers on (Add, C) all fire in registration order
// ════════════════════════════════════════════════════════════════════════════

static C2_CLOCK: AtomicUsize = AtomicUsize::new(0);
static C2_AT_0: AtomicUsize = AtomicUsize::new(usize::MAX);
static C2_AT_1: AtomicUsize = AtomicUsize::new(usize::MAX);
static C2_AT_2: AtomicUsize = AtomicUsize::new(usize::MAX);
static C2_FIRES: AtomicUsize = AtomicUsize::new(0);

unsafe fn c2_obs0(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    C2_AT_0.store(C2_CLOCK.fetch_add(1, SEQ), SEQ);
    C2_FIRES.fetch_add(1, SEQ);
}
unsafe fn c2_obs1(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    C2_AT_1.store(C2_CLOCK.fetch_add(1, SEQ), SEQ);
    C2_FIRES.fetch_add(1, SEQ);
}
unsafe fn c2_obs2(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    C2_AT_2.store(C2_CLOCK.fetch_add(1, SEQ), SEQ);
    C2_FIRES.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C2Comp(u32);

#[test]
fn three_observers_on_same_kind_all_fire_in_registration_order() {
    let mut ecs = EcsMaster::new();
    // Register in a defined order; the fire loop walks the Vec in push order.
    ecs.observe_on_add::<C2Comp>(c2_obs0);
    ecs.observe_on_add::<C2Comp>(c2_obs1);
    ecs.observe_on_add::<C2Comp>(c2_obs2);

    let arch = ecs.create_archetype(&[C2Comp::component_id()]);
    let _e = ecs.spawn_one(arch, C2Comp(7)).expect("spawn");

    assert_eq!(C2_FIRES.load(SEQ), 3, "all three observers fired on one spawn");
    let (a0, a1, a2) = (C2_AT_0.load(SEQ), C2_AT_1.load(SEQ), C2_AT_2.load(SEQ));
    assert!(
        a0 < a1 && a1 < a2,
        "observers fire in registration order (clocks {a0} < {a1} < {a2})"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Case 3 — component-targeting (D4): an observer on C does NOT fire for D-only
// ════════════════════════════════════════════════════════════════════════════

static C3_C_FIRES: AtomicUsize = AtomicUsize::new(0);

unsafe fn c3_c_add(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    C3_C_FIRES.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C3CompC(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C3CompD(u32);

#[test]
fn observer_on_c_does_not_fire_for_d_only_entity() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_add::<C3CompC>(c3_c_add);
    // Materialise D's id so its archetype is valid, but D has no observer.
    let arch_d = ecs.create_archetype(&[C3CompD::component_id()]);

    let _e = ecs.spawn_one(arch_d, C3CompD(1)).expect("spawn D-only");
    assert_eq!(
        C3_C_FIRES.load(SEQ),
        0,
        "an observer keyed on C must NOT fire for an entity that has only D (component-targeting)"
    );

    // Sanity: the C observer DOES fire for a C entity (it is actually wired).
    let arch_c = ecs.create_archetype(&[C3CompC::component_id()]);
    let _e2 = ecs.spawn_one(arch_c, C3CompC(2)).expect("spawn C");
    assert_eq!(C3_C_FIRES.load(SEQ), 1, "the C observer fires for a C entity (it is wired)");
}

// ════════════════════════════════════════════════════════════════════════════
// Case 7 — add-Nth: a 2nd observer when an archetype already exists; both fire
// ════════════════════════════════════════════════════════════════════════════

static C7_FIRST: AtomicUsize = AtomicUsize::new(0);
static C7_SECOND: AtomicUsize = AtomicUsize::new(0);

unsafe fn c7_first(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    C7_FIRST.fetch_add(1, SEQ);
}
unsafe fn c7_second(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    C7_SECOND.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C7Comp(u32);

#[test]
fn adding_a_second_observer_keeps_bit_set_and_both_fire() {
    let mut ecs = EcsMaster::new();
    // First observer + an existing archetype (bit gets set by the add-first walk).
    ecs.observe_on_add::<C7Comp>(c7_first);
    let arch = ecs.create_archetype(&[C7Comp::component_id()]);
    let _e0 = ecs.spawn_one(arch, C7Comp(1)).expect("spawn 0");
    assert_eq!(C7_FIRST.load(SEQ), 1, "first observer fires on the first spawn");

    // Add a SECOND observer for the same (Add, C). The bit is already set, so
    // this is the no-walk path (add-Nth). A later spawn must fire BOTH.
    ecs.observe_on_add::<C7Comp>(c7_second);
    let _e1 = ecs.spawn_one(arch, C7Comp(2)).expect("spawn 1");

    assert_eq!(C7_FIRST.load(SEQ), 2, "first observer fired again (bit stayed set)");
    assert_eq!(C7_SECOND.load(SEQ), 1, "the newly-added second observer fired");
}

// ════════════════════════════════════════════════════════════════════════════
// Case 8 — remove-non-last: 2 observers, remove one; the other still fires
// ════════════════════════════════════════════════════════════════════════════

static C8_KEPT: AtomicUsize = AtomicUsize::new(0);
static C8_REMOVED: AtomicUsize = AtomicUsize::new(0);

unsafe fn c8_kept(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    C8_KEPT.fetch_add(1, SEQ);
}
unsafe fn c8_removed(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    C8_REMOVED.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C8Comp(u32);

#[test]
fn removing_non_last_observer_leaves_the_other_firing() {
    let mut ecs = EcsMaster::new();
    let _kept_id = ecs.observe_on_add::<C8Comp>(c8_kept);
    let removed_id = ecs.observe_on_add::<C8Comp>(c8_removed);

    let arch = ecs.create_archetype(&[C8Comp::component_id()]);

    // Remove the second observer (non-last: one remains, so the bit stays set).
    let ok = ecs.remove_observer(removed_id);
    assert!(ok, "remove_observer returns true for a registered id");

    let _e = ecs.spawn_one(arch, C8Comp(1)).expect("spawn");
    assert_eq!(C8_KEPT.load(SEQ), 1, "the surviving observer still fires");
    assert_eq!(C8_REMOVED.load(SEQ), 0, "the removed observer does NOT fire");

    // Removing it again returns false (the id is retired).
    assert!(!ecs.remove_observer(removed_id), "double-remove returns false");
}

// ════════════════════════════════════════════════════════════════════════════
// Case 9 — sibling last-removal (W3): {A,B} both observed; the ON_ADD_OBSERVER
//           bit stays set after removing A's last (B still observes), and clears
//           only after BOTH are removed
// ════════════════════════════════════════════════════════════════════════════

static C9_A: AtomicUsize = AtomicUsize::new(0);
static C9_B: AtomicUsize = AtomicUsize::new(0);

unsafe fn c9_a(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    C9_A.fetch_add(1, SEQ);
}
unsafe fn c9_b(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    C9_B.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C9A(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C9B(u32);

#[test]
fn sibling_last_removal_clears_bit_only_when_all_siblings_removed() {
    let mut ecs = EcsMaster::new();
    // Archetype {A, B}; on_add observers on BOTH components.
    let a_id = ecs.observe_on_add::<C9A>(c9_a);
    let b_id = ecs.observe_on_add::<C9B>(c9_b);
    let arch = ecs.create_archetype(&[C9A::component_id(), C9B::component_id()]);

    // Baseline: both fire on a spawn.
    let _e0 = ecs.spawn_two(arch, C9A(1), C9B(2)).expect("spawn 0");
    assert_eq!(C9_A.load(SEQ), 1, "A observer fires");
    assert_eq!(C9_B.load(SEQ), 1, "B observer fires");

    // Remove A's last observer. B still observes Add, so the archetype's
    // ON_ADD_OBSERVER bit must STAY set — B must keep firing.
    assert!(ecs.remove_observer(a_id), "A observer removed");
    let _e1 = ecs.spawn_two(arch, C9A(3), C9B(4)).expect("spawn 1");
    assert_eq!(C9_A.load(SEQ), 1, "A observer no longer fires (removed)");
    assert_eq!(
        C9_B.load(SEQ),
        2,
        "B observer STILL fires: the bit stays set because a sibling (B) observes Add"
    );

    // Remove B's last observer too. Now NO component in {A,B} observes Add ⇒
    // the bit clears ⇒ a subsequent spawn fires neither.
    assert!(ecs.remove_observer(b_id), "B observer removed");
    let _e2 = ecs.spawn_two(arch, C9A(5), C9B(6)).expect("spawn 2");
    assert_eq!(C9_A.load(SEQ), 1, "A still silent");
    assert_eq!(
        C9_B.load(SEQ),
        2,
        "B observer silent now too: the bit cleared after the last sibling was removed"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Case 10 — remove-last clears the OBSERVER bit but PRESERVES the derive HOOK
// ════════════════════════════════════════════════════════════════════════════

static C10_HOOK: AtomicUsize = AtomicUsize::new(0);
static C10_OBSERVER: AtomicUsize = AtomicUsize::new(0);

unsafe fn c10_hook(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    C10_HOOK.fetch_add(1, SEQ);
}
unsafe fn c10_observer(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    C10_OBSERVER.fetch_add(1, SEQ);
}

/// Carries BOTH a derive `on_add` HOOK and (registered at runtime) an `on_add`
/// OBSERVER. Removing the observer must leave the hook firing (W3 clears only
/// the ON_ADD_OBSERVER bit, never the ON_ADD_HOOK bit).
#[derive(Component)]
#[component(on_add = c10_hook)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C10Comp(u32);

#[test]
fn removing_last_observer_preserves_the_derive_hook() {
    let mut ecs = EcsMaster::new();
    let obs_id = ecs.observe_on_add::<C10Comp>(c10_observer);
    let arch = ecs.create_archetype(&[C10Comp::component_id()]);

    // Both fire on the first spawn.
    let _e0 = ecs.spawn_one(arch, C10Comp(1)).expect("spawn 0");
    assert_eq!(C10_HOOK.load(SEQ), 1, "derive hook fires");
    assert_eq!(C10_OBSERVER.load(SEQ), 1, "runtime observer fires");

    // Remove the (last) observer. The ON_ADD_OBSERVER bit clears, but the
    // ON_ADD_HOOK bit is untouched — the hook must keep firing.
    assert!(ecs.remove_observer(obs_id), "observer removed");
    let _e1 = ecs.spawn_one(arch, C10Comp(2)).expect("spawn 1");
    assert_eq!(
        C10_HOOK.load(SEQ),
        2,
        "the derive HOOK survives observer removal (HOOK bit preserved)"
    );
    assert_eq!(C10_OBSERVER.load(SEQ), 1, "the removed observer no longer fires");
}

// ════════════════════════════════════════════════════════════════════════════
// Case F3 (TESTER FINDING — engine bug, see report) — the DEFERRED-spawn fire
// site (`SpawnAtCommand::apply` in spawn_at_command.rs) does NOT fire on_add /
// on_insert OBSERVERS. It gates only `ON_ADD_HOOK` / `ON_INSERT_HOOK` and calls
// only `trigger_on_add` / `trigger_on_insert`. A DIRECT spawn
// (`ecs.spawn_one` → `create_entity`) DOES fire observers (case 1), so this
// isolates the deferred (`cmds.spawn`) command-apply path. The 14a HOOK analog
// (phase14a_hooks_firing.rs::spawn_deferred_via_commands_fires_add_and_insert)
// PASSES.
// ════════════════════════════════════════════════════════════════════════════

static F3_ADD: AtomicUsize = AtomicUsize::new(0);
static F3_INSERT: AtomicUsize = AtomicUsize::new(0);

unsafe fn f3_add(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    F3_ADD.fetch_add(1, SEQ);
}
unsafe fn f3_insert(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    F3_INSERT.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct F3Comp(u32);

#[derive(Bundle)]
struct F3Bundle {
    c: F3Comp,
}

#[test]
fn deferred_spawn_via_commands_fires_add_and_insert_observers() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_add::<F3Comp>(f3_add);
    ecs.observe_on_insert::<F3Comp>(f3_insert);
    // Prime the id so the archetype is resolvable at apply.
    let _ = F3Comp::component_id();

    // Deferred spawn via Commands → SpawnAtCommand::apply (NOT the direct
    // create_entity path).
    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(F3Bundle { c: F3Comp(42) });
    });

    assert_eq!(ecs.entity_count(), 1, "deferred spawn registered one entity");
    assert_eq!(
        F3_ADD.load(SEQ),
        1,
        "deferred spawn must fire on_add observer (TESTER FINDING: \
         SpawnAtCommand::apply gates only ON_ADD_HOOK)"
    );
    assert_eq!(
        F3_INSERT.load(SEQ),
        1,
        "deferred spawn must fire on_insert observer (TESTER FINDING: \
         SpawnAtCommand::apply calls only trigger_on_insert)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Bonus — no-observer archetype: a plain component never enters an observer fire
// (cheap regression alongside the 0%-gate bench; confirms zero accidental wiring)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C11Plain(u32);

#[test]
fn plain_component_with_no_observer_registered_fires_nothing() {
    let mut ecs = EcsMaster::new();
    let id = C11Plain::component_id();
    // No observer registered, no derive hook ⇒ the HOOKS slot is unset.
    assert!(
        component_registry::get_hooks(id.0).is_none(),
        "a plain #[derive(Component)] leaves its HOOKS slot UNSET"
    );
    let arch = ecs.create_archetype(&[id]);
    let e = ecs.spawn_one(arch, C11Plain(1)).expect("spawn");
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).despawn();
    });
    // The contract is "no panic, no fire entered" — entity lifecycle still works.
    assert_eq!(ecs.entity_count(), 0, "despawn of a no-observer entity still works");
}
