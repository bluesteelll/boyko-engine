//! Phase 14b — observer DEFERRED-mutation, dynamic-archetype-bit (the C1
//! surface, both directions), and re-entrancy (the SOUNDNESS-adjacent) tests.
//!
//! Architect plan §11 (R2 + R3) cases:
//!
//! | case | what                                                               |
//! |------|--------------------------------------------------------------------|
//! | 4    | deferred mutation: an observer enqueues `commands()` ops; they      |
//! |      | apply at the outermost drain, not inline                            |
//! | 5    | add-after-archetype-exists (C1 dir 1): archetype with C exists       |
//! |      | FIRST, then `observe_on_add::<C>()`; a later spawn fires            |
//! | 6    | archetype-created-after-add (C1 dir 2): `observe_on_remove::<C>()`   |
//! |      | FIRST, then the first entity creating an archetype with C; despawn   |
//! |      | fires. PLUS the mid-deferred-drain variant: an observer enqueues an  |
//! |      | insert that migrates an entity into a BRAND-NEW archetype with C —   |
//! |      | that new archetype must carry the observer bit (construction seed).  |
//! | 14   | re-entrancy depth: an observer's deferred command triggers another   |
//! |      | structural op (more observers) — single drain, no double-apply      |
//! |      | (the 14a F1 regression guard).                                       |
//!
//! Observers are bare fn-ptrs, so per-test state lives in module `static`s; a
//! target entity is stashed into a `static` `TargetCell` (id+generation packed
//! into a `u64`), mirroring the 14a deferred harness.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::ObserverContext;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::{Bundle, Component};

const SEQ: Ordering = Ordering::SeqCst;

// ════════════════════════════════════════════════════════════════════════════
// Case 4 — deferred mutation: an observer's enqueued spawn applies at the drain,
//          not inline
// ════════════════════════════════════════════════════════════════════════════

static C4_FIRE_COUNT: AtomicUsize = AtomicUsize::new(0);
static C4_CHILD_VISIBLE_INLINE: AtomicUsize = AtomicUsize::new(0);
static C4_SELF_VISIBLE: AtomicUsize = AtomicUsize::new(0);

unsafe fn c4_on_add(mut w: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    C4_FIRE_COUNT.fetch_add(1, SEQ);
    // Self must already be registered (the observer fires POST-register).
    if w.get_component::<C4Parent>(ctx.entity).is_some() {
        C4_SELF_VISIBLE.fetch_add(1, SEQ);
    }
    // Enqueue a deferred child spawn; capture its (reserved) handle.
    let child = w.commands().spawn(C4ChildBundle { c: C4Child(9) });
    // A deferred spawn must NOT be queryable inline — it materialises at the drain.
    if w.get_component::<C4Child>(child).is_some() {
        C4_CHILD_VISIBLE_INLINE.fetch_add(1, SEQ);
    }
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C4Parent(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C4Child(u32);

#[derive(Bundle)]
struct C4ChildBundle {
    c: C4Child,
}

#[test]
fn observer_deferred_spawn_is_applied_after_drain_not_inline() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_add::<C4Parent>(c4_on_add);
    let arch = ecs.create_archetype(&[C4Parent::component_id()]);
    let _ = C4Child::component_id();

    // Direct-API spawn → the parent's on_add observer fires inline and enqueues
    // a child spawn into the world-resident deferred queue; create_entity's
    // OUTERMOST drain (depth 0) applies it.
    let _p = ecs.spawn_one(arch, C4Parent(7)).expect("spawn parent");

    assert_eq!(C4_FIRE_COUNT.load(SEQ), 1, "parent on_add observer fired exactly once");
    assert_eq!(
        C4_SELF_VISIBLE.load(SEQ),
        1,
        "the observer fires POST-register: it sees its own entity"
    );
    assert_eq!(
        C4_CHILD_VISIBLE_INLINE.load(SEQ),
        0,
        "the observer's deferred spawn must NOT be visible inline (deferred, not inline)"
    );
    assert_eq!(
        ecs.entity_count(),
        2,
        "parent + deferred-spawned child both present AFTER the outermost drain"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Case 5 — add-after-archetype-exists (C1 direction 1): archetype with C is
//          created FIRST, THEN observe_on_add::<C>(); a later spawn fires
//          (the add-first walk set the bit on the pre-existing archetype)
// ════════════════════════════════════════════════════════════════════════════

static C5_FIRES: AtomicUsize = AtomicUsize::new(0);

unsafe fn c5_add(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    C5_FIRES.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C5Comp(u32);

#[test]
fn add_observer_after_archetype_exists_fires_on_next_spawn() {
    let mut ecs = EcsMaster::new();

    // Archetype with C exists FIRST (and an entity spawned before any observer).
    let arch = ecs.create_archetype(&[C5Comp::component_id()]);
    let _e0 = ecs.spawn_one(arch, C5Comp(1)).expect("pre-observer spawn");
    assert_eq!(C5_FIRES.load(SEQ), 0, "no observer yet ⇒ no fire");

    // Register the observer NOW — the add-first dynamic walk must set
    // ON_ADD_OBSERVER on the already-existing archetype.
    ecs.observe_on_add::<C5Comp>(c5_add);

    // A subsequent spawn into the SAME archetype must fire.
    let _e1 = ecs.spawn_one(arch, C5Comp(2)).expect("post-observer spawn");
    assert_eq!(
        C5_FIRES.load(SEQ),
        1,
        "observer registered after the archetype existed fires on the next spawn (add-first walk)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Case 6a — archetype-created-after-add (C1 direction 2): observe_on_remove::<C>()
//           FIRST, THEN the first entity that creates an archetype with C;
//           despawn fires (the construction seed set the bit)
// ════════════════════════════════════════════════════════════════════════════

static C6_REMOVE_FIRES: AtomicUsize = AtomicUsize::new(0);

unsafe fn c6_remove(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    C6_REMOVE_FIRES.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C6Comp(u32);

#[test]
fn add_observer_before_archetype_exists_fires_via_construction_seed() {
    let mut ecs = EcsMaster::new();

    // Observer registered BEFORE any archetype containing C exists. The
    // registry has no archetype to walk yet — the bit must instead be SEEDED
    // when the archetype is later constructed.
    ecs.observe_on_remove::<C6Comp>(c6_remove);

    // NOW create the first archetype containing C, and an entity in it.
    let arch = ecs.create_archetype(&[C6Comp::component_id()]);
    let e = ecs.spawn_one(arch, C6Comp(1)).expect("spawn");

    // Despawn ⇒ on_remove must fire (construction seed set ON_REMOVE_OBSERVER).
    ecs.run_system(move |mut cmds: boyko_ecs::ecs::core::system::Commands| {
        cmds.entity(e).despawn();
    });
    assert_eq!(
        C6_REMOVE_FIRES.load(SEQ),
        1,
        "an archetype created AFTER the observer was registered carries the bit (construction seed)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Case 6b — mid-deferred-drain variant: an observer enqueues an insert that
//           migrates an entity into a BRAND-NEW archetype containing C (created
//           via migration_helpers → create_archetype DURING the drain); that
//           new archetype must carry the observer bit, so C's on_add fires.
// ════════════════════════════════════════════════════════════════════════════

static C6B_TARGET: TargetCell = TargetCell::new();
static C6B_TRIGGER_FIRES: AtomicUsize = AtomicUsize::new(0);
static C6B_NEW_ADD_FIRES: AtomicUsize = AtomicUsize::new(0);

/// The trigger's on_add enqueues an insert of `C6bNew` onto a pre-existing
/// target entity. That insert MIGRATES the target into a brand-new archetype
/// `{C6bBase, C6bNew}` constructed during the drain — the seed must give that
/// new archetype the ON_ADD_OBSERVER bit so `C6bNew`'s on_add fires.
unsafe fn c6b_trigger_add(mut w: DeferredEcsMaster<'_>, _ctx: ObserverContext) {
    C6B_TRIGGER_FIRES.fetch_add(1, SEQ);
    let target = C6B_TARGET.get();
    w.commands().entity(target).insert(C6bNewBundle { c: C6bNew(1) });
}
unsafe fn c6b_new_add(_w: DeferredEcsMaster<'_>, _ctx: ObserverContext) {
    C6B_NEW_ADD_FIRES.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C6bTrigger(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C6bBase(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C6bNew(u32);

#[derive(Bundle)]
struct C6bNewBundle {
    c: C6bNew,
}

#[test]
fn observer_deferred_insert_seeds_observer_bit_on_brand_new_archetype() {
    let mut ecs = EcsMaster::new();
    // Observer on the NEW component, registered up front (no archetype with it
    // exists yet — only the construction seed can give the migration target its
    // bit).
    ecs.observe_on_add::<C6bNew>(c6b_new_add);
    // Observer on the trigger component (fires the deferred insert).
    ecs.observe_on_add::<C6bTrigger>(c6b_trigger_add);

    // A pre-existing target in archetype {C6bBase} — it must NOT yet contain
    // C6bNew (the migration adds it during the drain).
    let base_arch = ecs.create_archetype(&[C6bBase::component_id()]);
    let target = ecs.spawn_one(base_arch, C6bBase(7)).expect("spawn target");
    C6B_TARGET.set(target);

    // Spawn the trigger via the DIRECT api: its on_add enqueues the insert; the
    // outermost drain applies it, creating the {C6bBase, C6bNew} archetype.
    let trigger_arch = ecs.create_archetype(&[C6bTrigger::component_id()]);
    let _trigger = ecs.spawn_one(trigger_arch, C6bTrigger(1)).expect("spawn trigger");

    assert_eq!(C6B_TRIGGER_FIRES.load(SEQ), 1, "trigger on_add fired once");
    assert!(
        ecs.has_component(target, C6bNew::component_id()),
        "the deferred insert migrated the target into {{C6bBase, C6bNew}}"
    );
    assert_eq!(
        C6B_NEW_ADD_FIRES.load(SEQ),
        1,
        "the brand-new archetype created mid-drain carries the observer bit \
         (construction seed) ⇒ C6bNew's on_add fired"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Case F1 (TESTER FINDING — engine bug, see report) — the insert-MIGRATION fire
// site (`migrate_entity_insert` in migration_helpers.rs) does NOT fire on_add /
// on_insert OBSERVERS. It still gates only `ON_ADD_HOOK` / `ON_INSERT_HOOK` and
// calls only `trigger_on_*` (the hook dispatch) — the Phase 14b fire-site
// widening (HOOK->ANY + fire_on_*_observers) missed this site. This is the
// synchronous (non-mid-drain) variant of case 6b, isolating the bug from the
// deferred-drain timing. The 14a HOOK analog
// (phase14a_hooks_firing.rs::insert_migration_fires_add_for_new_insert_for_bundle_only)
// PASSES, proving the migration fire site itself works — only OBSERVERS are
// unwired there.
// ════════════════════════════════════════════════════════════════════════════

static F1_NEW_ADD_FIRES: AtomicUsize = AtomicUsize::new(0);

unsafe fn f1_new_add(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    F1_NEW_ADD_FIRES.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct F1Base(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct F1New(u32);

#[derive(Bundle)]
struct F1NewBundle {
    c: F1New,
}

#[test]
fn insert_migration_fires_on_add_observer_for_new_component() {
    let mut ecs = EcsMaster::new();
    // Observer on the NEW component, registered up front. The migration target
    // archetype {F1Base, F1New} is created on insert and must carry the bit
    // (construction seed) AND its fire site must dispatch observers.
    ecs.observe_on_add::<F1New>(f1_new_add);

    let base_arch = ecs.create_archetype(&[F1Base::component_id()]);
    let target = ecs.spawn_one(base_arch, F1Base(7)).expect("spawn target");

    // Synchronous insert via a command system → migration_helpers
    // ::migrate_entity_insert; creates {F1Base, F1New}.
    ecs.run_system(move |mut cmds: boyko_ecs::ecs::core::system::Commands| {
        cmds.entity(target).insert(F1NewBundle { c: F1New(1) });
    });

    assert!(
        ecs.has_component(target, F1New::component_id()),
        "the insert migrated the target into {{F1Base, F1New}}"
    );
    assert_eq!(
        F1_NEW_ADD_FIRES.load(SEQ),
        1,
        "insert-migration must fire the newly-added component's on_add observer \
         (TESTER FINDING: migration_helpers::migrate_entity_insert gates only \
         ON_ADD_HOOK and calls only trigger_on_add — observers unwired)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Case F2 (TESTER FINDING — engine bug, see report) — the remove-MIGRATION fire
// site (`migrate_entity_remove` in migration_helpers.rs) does NOT fire
// on_replace / on_remove OBSERVERS for a single-component `remove` (the
// non-despawn path). It gates only `ON_REPLACE_HOOK` / `ON_REMOVE_HOOK` and
// calls only `trigger_on_replace` / `trigger_on_remove`. The 14a HOOK analog
// (phase14a_hooks_firing.rs::remove_fires_replace_then_remove_predrop_*) PASSES.
// ════════════════════════════════════════════════════════════════════════════

static F2_REPLACE: AtomicUsize = AtomicUsize::new(0);
static F2_REMOVE: AtomicUsize = AtomicUsize::new(0);

unsafe fn f2_replace(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    F2_REPLACE.fetch_add(1, SEQ);
}
unsafe fn f2_remove(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    F2_REMOVE.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct F2Removed(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct F2Keep(u32);

#[test]
fn remove_migration_fires_replace_and_remove_observers() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_replace::<F2Removed>(f2_replace);
    ecs.observe_on_remove::<F2Removed>(f2_remove);

    let arch = ecs.create_archetype(&[F2Removed::component_id(), F2Keep::component_id()]);
    let e = ecs.spawn_two(arch, F2Removed(55), F2Keep(9)).expect("spawn");

    // Single-component remove ⇒ migrate to {F2Keep} (the remove-migration path,
    // NOT despawn — despawn goes through fire_despawn_hooks which IS wired).
    ecs.run_system(move |mut cmds: boyko_ecs::ecs::core::system::Commands| {
        cmds.entity(e).remove::<F2Removed>();
    });

    assert!(!ecs.has_component(e, F2Removed::component_id()), "Removed gone");
    assert_eq!(
        F2_REPLACE.load(SEQ),
        1,
        "remove-migration must fire on_replace observer (TESTER FINDING: \
         migration_helpers::migrate_entity_remove gates only ON_REPLACE_HOOK)"
    );
    assert_eq!(
        F2_REMOVE.load(SEQ),
        1,
        "remove-migration must fire on_remove observer (TESTER FINDING: \
         migration_helpers::migrate_entity_remove calls only trigger_on_remove)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Case 14 — re-entrancy depth: an observer's deferred command triggers another
//           structural op (firing more observers); single drain, no double-apply
//           (the 14a F1 regression guard, observer-driven)
// ════════════════════════════════════════════════════════════════════════════

static C14_TARGET: TargetCell = TargetCell::new();
static C14_TRIGGER_FIRES: AtomicUsize = AtomicUsize::new(0);
static C14_TARGET_REMOVE_FIRES: AtomicUsize = AtomicUsize::new(0);

/// The trigger's on_add (observer) enqueues a despawn of a pre-existing target.
/// The despawn is deferred and applied at the outermost drain; the target's
/// on_remove OBSERVER fires during that drain. If the drain were not
/// re-entrancy-guarded (14a F1), the target's on_remove would fire twice and/or
/// `DespawnCommand::apply` would trip on a second application.
unsafe fn c14_trigger_add(mut w: DeferredEcsMaster<'_>, _ctx: ObserverContext) {
    C14_TRIGGER_FIRES.fetch_add(1, SEQ);
    w.commands().entity(C14_TARGET.get()).despawn();
}
unsafe fn c14_target_remove(_w: DeferredEcsMaster<'_>, _ctx: ObserverContext) {
    C14_TARGET_REMOVE_FIRES.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C14Trigger(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct C14Target(u32);

#[test]
fn observer_deferred_despawn_applies_exactly_once_no_double_apply() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_add::<C14Trigger>(c14_trigger_add);
    ecs.observe_on_remove::<C14Target>(c14_target_remove);

    let target_arch = ecs.create_archetype(&[C14Target::component_id()]);
    let trigger_arch = ecs.create_archetype(&[C14Trigger::component_id()]);

    let target = ecs.spawn_one(target_arch, C14Target(1)).expect("spawn target");
    C14_TARGET.set(target);
    assert!(ecs.has_entity(target), "target alive before trigger");

    // Direct-API spawn: the trigger's on_add observer fires inline, enqueues the
    // target despawn; the outermost drain applies it ONCE. The target's
    // on_remove observer fires during the (single, re-entrancy-guarded) drain.
    let _trigger = ecs.spawn_one(trigger_arch, C14Trigger(2)).expect("spawn trigger");

    assert!(!ecs.has_entity(target), "deferred despawn removed the target");
    assert_eq!(
        C14_TARGET_REMOVE_FIRES.load(SEQ),
        1,
        "target on_remove observer fired EXACTLY once — no re-entrant double-apply (F1 guard)"
    );
    assert_eq!(C14_TRIGGER_FIRES.load(SEQ), 1, "trigger on_add fired exactly once");
    assert_eq!(ecs.entity_count(), 1, "only the trigger entity remains");
}

// ────────────────────────────────────────────────────────────────────────────
// Helper: Send+Sync cell stashing an Entity into a static, for observers.
// ────────────────────────────────────────────────────────────────────────────

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
