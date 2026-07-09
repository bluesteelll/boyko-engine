//! Phase 14b — Miri (Tree Borrows) coverage for the observer unsafe paths.
//! Single-thread only (multi-thread Miri deferred per Phase 9.1).
//!
//! Run via:
//! ```powershell
//! $env:MIRIFLAGS="-Zmiri-tree-borrows"
//! cargo +nightly miri test -p boyko-ecs --test miri_phase14b
//! ```
//!
//! Per the 14a lesson — Miri-TB caught two soundness bugs (F1/F2) that critic +
//! code-review APPROVED — **Miri-TB is the authoritative soundness oracle** for
//! the observer raw-pointer plumbing. The three critical targets (plan §11.16):
//!
//! 1. **The fire loop (OBS-FIRE-LOOP, the F2-class hazard).** No registry `&`
//!    (nor any `world`-derived `&`) may be live across the
//!    `DeferredEcsMaster::from_world` view mint or the runner call. Cases 1-3,
//!    7-9 drive every fire kind + the multiplicity walk + the dynamic
//!    add-first/remove-last walks.
//! 2. **The seed borrow-split in `create_archetype`** (`&self.observer_registry`
//!    read, copied into a `Copy` `ArchetypeFlags`, then `&mut self.archetypes`
//!    write). Exercised whenever an archetype is constructed with a pre-
//!    registered observer (cases 6 / the seed path).
//! 3. **`get_component_mut`'s `Mut`** (OBS-MUT exclusivity, the system-less
//!    `&mut self` path): the `&mut *archetype_ptr` deref + per-row tick-slot
//!    offset + `&mut T` mint.
//!
//! # File gate
//!
//! `#![cfg(miri)]` — only compiles under Miri. Native runs ignore the file; the
//! `phase14b_observers_*` integration suites cover the same semantics
//! end-to-end on the native target. Entity counts are kept tiny (Miri is slow).

#![cfg(miri)]

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::ObserverContext;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

const SEQ: Ordering = Ordering::SeqCst;

// ════════════════════════════════════════════════════════════════════════════
// Target 1 — the fire loop: all 4 kinds + multiplicity + dynamic walks
// ════════════════════════════════════════════════════════════════════════════

static M1_ADD: AtomicUsize = AtomicUsize::new(0);
static M1_INSERT: AtomicUsize = AtomicUsize::new(0);
static M1_REPLACE: AtomicUsize = AtomicUsize::new(0);
static M1_REMOVE: AtomicUsize = AtomicUsize::new(0);

unsafe fn m1_add(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    M1_ADD.fetch_add(1, SEQ);
}
unsafe fn m1_insert(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    M1_INSERT.fetch_add(1, SEQ);
}
unsafe fn m1_replace(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    M1_REPLACE.fetch_add(1, SEQ);
}
unsafe fn m1_remove(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    M1_REMOVE.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct M1Comp(u32);

/// Exercises the fire-loop unsafe dispatch for all four kinds over the
/// DIRECT-API ops (spawn fires add+insert; despawn fires replace+remove). The
/// deferred-command apply paths are unwired for observers (TESTER FINDING — a
/// correctness bug, not a soundness one), so this Miri soundness test uses only
/// the wired direct paths; the native `phase14b_observers_*` suites cover the
/// unwired sites as documented failing cases.
#[test]
fn miri_fire_loop_all_kinds() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_add::<M1Comp>(m1_add);
    ecs.observe_on_insert::<M1Comp>(m1_insert);
    ecs.observe_on_replace::<M1Comp>(m1_replace);
    ecs.observe_on_remove::<M1Comp>(m1_remove);
    let arch = ecs.create_archetype(&[M1Comp::component_id()]);

    // DIRECT spawn ⇒ add + insert (create_entity — wired).
    let e = ecs.spawn_one(arch, M1Comp(1)).expect("spawn");
    // DIRECT despawn ⇒ replace + remove (delete_entity -> fire_despawn_hooks — wired).
    assert!(ecs.delete_entity(e), "despawn");

    assert_eq!(M1_ADD.load(SEQ), 1, "add fired once on spawn");
    assert_eq!(M1_INSERT.load(SEQ), 1, "insert fired once on spawn");
    assert_eq!(M1_REPLACE.load(SEQ), 1, "replace fired once on despawn");
    assert_eq!(M1_REMOVE.load(SEQ), 1, "remove fired once on despawn");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 1b — multiplicity: 3 observers on (Add, C) (the walk copies entries by
//             value per turn — the OBS-FIRE-LOOP discipline)
// ════════════════════════════════════════════════════════════════════════════

static M1B_FIRES: AtomicUsize = AtomicUsize::new(0);

unsafe fn m1b(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    M1B_FIRES.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct M1bComp(u32);

#[test]
fn miri_fire_loop_multiplicity() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_add::<M1bComp>(m1b);
    ecs.observe_on_add::<M1bComp>(m1b);
    ecs.observe_on_add::<M1bComp>(m1b);
    let arch = ecs.create_archetype(&[M1bComp::component_id()]);
    let _e = ecs.spawn_one(arch, M1bComp(1)).expect("spawn");
    assert_eq!(M1B_FIRES.load(SEQ), 3, "all three observers fired (per-turn copy walk)");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 1c — dynamic walks: add-first (after archetype exists), remove-last
//             (sibling recompute) — the iter_archetypes_mut flag RMW under TB
// ════════════════════════════════════════════════════════════════════════════

static M1C_A: AtomicUsize = AtomicUsize::new(0);
static M1C_B: AtomicUsize = AtomicUsize::new(0);

unsafe fn m1c_a(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    M1C_A.fetch_add(1, SEQ);
}
unsafe fn m1c_b(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    M1C_B.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct M1cA(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct M1cB(u32);

#[test]
fn miri_dynamic_add_first_and_remove_last_walks() {
    let mut ecs = EcsMaster::new();
    // Archetype {A, B} exists before any observer (forces the add-first walk).
    let arch = ecs.create_archetype(&[M1cA::component_id(), M1cB::component_id()]);

    let a_id = ecs.observe_on_add::<M1cA>(m1c_a); // add-first walk over existing arch
    let b_id = ecs.observe_on_add::<M1cB>(m1c_b); // add-first walk (bit already set)

    let _e0 = ecs.spawn_two(arch, M1cA(1), M1cB(2)).expect("spawn 0");
    assert_eq!(M1C_A.load(SEQ), 1, "A fires after add-first walk");
    assert_eq!(M1C_B.load(SEQ), 1, "B fires");

    // Remove-last for A — sibling recompute walk (B keeps the bit).
    assert!(ecs.remove_observer(a_id));
    let _e1 = ecs.spawn_two(arch, M1cA(3), M1cB(4)).expect("spawn 1");
    assert_eq!(M1C_A.load(SEQ), 1, "A silent after removal");
    assert_eq!(M1C_B.load(SEQ), 2, "B still fires (sibling keeps the bit)");

    // Remove-last for B — bit clears.
    assert!(ecs.remove_observer(b_id));
    let _e2 = ecs.spawn_two(arch, M1cA(5), M1cB(6)).expect("spawn 2");
    assert_eq!(M1C_B.load(SEQ), 2, "B silent after the last sibling removed");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 2 — the construction seed borrow-split: observer registered BEFORE the
//            archetype, so create_archetype seeds the bit (the §4 split)
// ════════════════════════════════════════════════════════════════════════════

static M2_FIRES: AtomicUsize = AtomicUsize::new(0);

unsafe fn m2_remove(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    M2_FIRES.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct M2Comp(u32);

#[test]
fn miri_construction_seed_borrow_split() {
    let mut ecs = EcsMaster::new();
    // Register the observer FIRST — only the construction seed can set the bit.
    ecs.observe_on_remove::<M2Comp>(m2_remove);
    let arch = ecs.create_archetype(&[M2Comp::component_id()]); // seed runs here
    let e = ecs.spawn_one(arch, M2Comp(1)).expect("spawn");
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).despawn();
    });
    assert_eq!(M2_FIRES.load(SEQ), 1, "the construction-seeded bit made on_remove fire");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 3 — get_component_mut's Mut (OBS-MUT exclusivity, the &mut self path):
//            &mut *archetype_ptr deref + per-row tick offset + &mut T mint
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct M3Health(u32);

#[test]
fn miri_get_component_mut_writes_through_mut() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[M3Health::component_id()]);
    let e = ecs.spawn_one(arch, M3Health(10)).expect("spawn");

    {
        let mut m = ecs.get_component_mut::<M3Health>(e).expect("has M3Health");
        m.0 = 42; // DerefMut ⇒ &mut T into the pool + tick bump (the unsafe core)
        assert!(m.is_changed(), "is_changed true after a current-tick write");
    }
    assert_eq!(
        ecs.get_component::<M3Health>(e).expect("alive").0,
        42,
        "the write persisted through the Mut"
    );

    // None paths exercise the early-out branches under TB.
    assert!(ecs.delete_entity(e), "despawn");
    assert!(
        ecs.get_component_mut::<M3Health>(e).is_none(),
        "None for a despawned entity"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Target 1d — deferred mutation from inside a fire (the view's commands() +
//             outermost drain re-entrancy, observer-driven)
// ════════════════════════════════════════════════════════════════════════════

static M4_FIRES: AtomicUsize = AtomicUsize::new(0);

unsafe fn m4_on_add(mut w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    M4_FIRES.fetch_add(1, SEQ);
    w.commands().spawn(M4ChildBundle { c: M4Child(1) });
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct M4Parent(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct M4Child(u32);

#[derive(Bundle)]
struct M4ChildBundle {
    c: M4Child,
}

#[test]
fn miri_observer_deferred_spawn_drain() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_add::<M4Parent>(m4_on_add);
    let arch = ecs.create_archetype(&[M4Parent::component_id()]);
    let _ = M4Child::component_id();

    let _p = ecs.spawn_one(arch, M4Parent(7)).expect("spawn parent");
    assert_eq!(M4_FIRES.load(SEQ), 1, "parent on_add observer fired once");
    assert_eq!(ecs.entity_count(), 2, "deferred child spawn applied at the drain");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 5 — the COMMAND-APPLY observer fire paths (Phase 14b fix-wave NEW
//            soundness surface). The four sites below mint `world_ptr` inside a
//            DIFFERENT borrow choreography than the direct-API sites covered by
//            Targets 1-4: `world_ptr = NonNull::from(&mut *world)` is taken
//            INSIDE `SpawnAtCommand::apply` / `migrate_entity_insert` /
//            `migrate_entity_remove` — AFTER the per-site `&mut Archetype`
//            reborrow(s) drop, but reached via the deferred `CommandQueue::apply`
//            bracket rather than a direct `create_entity` / `delete_entity` call.
//            The OBS-FIRE-LOOP discipline (no `world`-derived `&` live across the
//            `from_world` mint or the runner call) must hold identically here.
//
// Run under `-Zmiri-tree-borrows`; the headline is TB-clean (zero aliasing UB)
// on these command-apply observer paths. If the project's exit-leak check trips
// the pre-existing 8 B `Vec<InlandPoolId>` leak (NOT a 14b regression), isolate
// the TB signal with `-Zmiri-ignore-leaks`.
// ════════════════════════════════════════════════════════════════════════════

// ── M5: deferred `cmds.spawn(bundle)` → SpawnAtCommand::apply fires add+insert ─

static M5_ADD: AtomicUsize = AtomicUsize::new(0);
static M5_INSERT: AtomicUsize = AtomicUsize::new(0);

unsafe fn m5_add(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    M5_ADD.fetch_add(1, SEQ);
}
unsafe fn m5_insert(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    M5_INSERT.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct M5Comp(u32);

#[derive(Bundle)]
struct M5Bundle {
    c: M5Comp,
}

/// `SpawnAtCommand::apply`'s on_add / on_insert observer fire loop, reached via
/// the deferred-command apply bracket. `world_ptr` is minted inside `apply`
/// after Step-5's per-component `&mut *archetype_ptr` and the Step-6 field
/// writes dropped — the OBS-FIRE-LOOP must be TB-clean from this context.
#[test]
fn miri_command_apply_deferred_spawn_fires_add_insert() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_add::<M5Comp>(m5_add);
    ecs.observe_on_insert::<M5Comp>(m5_insert);
    let _ = M5Comp::component_id();

    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(M5Bundle { c: M5Comp(1) });
    });

    assert_eq!(ecs.entity_count(), 1, "deferred spawn registered one entity");
    assert_eq!(M5_ADD.load(SEQ), 1, "SpawnAtCommand::apply fired on_add once");
    assert_eq!(M5_INSERT.load(SEQ), 1, "SpawnAtCommand::apply fired on_insert once");
}

// ── M6: `cmds.entity(e).insert(New)` migration → migrate_entity_insert fires ──

static M6_ADD: AtomicUsize = AtomicUsize::new(0);
static M6_INSERT: AtomicUsize = AtomicUsize::new(0);

unsafe fn m6_new_add(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    M6_ADD.fetch_add(1, SEQ);
}
unsafe fn m6_new_insert(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    M6_INSERT.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct M6Base(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct M6New(u32);

#[derive(Bundle)]
struct M6NewBundle {
    c: M6New,
}

/// `migrate_entity_insert`'s Phase-2 fire loop: the entity moves {M6Base} →
/// {M6Base, M6New}. `world_ptr` is minted in Phase 2 AFTER the Phase-1 block's
/// `source` / `target` `&mut Archetype` reborrows dropped at the block close.
/// The on_add observer fires only for the newly-added M6New (the
/// `bundle_added[i]` filter); both fire from the command-apply context.
///
/// FIXED (NEW-1, fix wave): the prior shape collected the bundle's `&[u8]`
/// slices into a stack array INSIDE `for_each_component_bytes`'s closure and read
/// them back AFTER the closure returned — those slices borrowed the bundle's
/// `ManuallyDrop` locals (valid only for that function's stack frame), so the
/// read-back was a dangling-reference UAF that Miri-TB aborted on at
/// `migration_helpers.rs` Step 3, BEFORE the Phase-2 fire ever ran. The fix
/// (mirrors `SpawnAtCommand::apply` + `apply_replace_in_place`) consumes every
/// bundle `&[u8]` AT THE POINT IT IS LIVE — single-pass inside the closure
/// (`migration_helpers.rs:313-365`), so no slice from the closure survives the
/// call. This test now both reaches AND clears the Phase-2 observer fire loop
/// Tree-Borrows-clean; it is the confirmation NEW-1 is fixed.
#[test]
fn miri_command_apply_insert_migration_fires_add_insert() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_add::<M6New>(m6_new_add);
    ecs.observe_on_insert::<M6New>(m6_new_insert);

    let base_arch = ecs.create_archetype(&[M6Base::component_id()]);
    let e = ecs.spawn_one(base_arch, M6Base(7)).expect("spawn target");

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(M6NewBundle { c: M6New(1) });
    });

    assert!(
        ecs.has_component(e, M6New::component_id()),
        "the insert migrated the entity into {{M6Base, M6New}}"
    );
    assert_eq!(M6_ADD.load(SEQ), 1, "migrate_entity_insert fired on_add for M6New once");
    assert_eq!(M6_INSERT.load(SEQ), 1, "migrate_entity_insert fired on_insert for M6New once");
}

// ── M7: `cmds.entity(e).remove::<C>()` migration → migrate_entity_remove ──────

static M7_REPLACE: AtomicUsize = AtomicUsize::new(0);
static M7_REMOVE: AtomicUsize = AtomicUsize::new(0);

unsafe fn m7_replace(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    M7_REPLACE.fetch_add(1, SEQ);
}
unsafe fn m7_remove(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    M7_REMOVE.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct M7Removed(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct M7Keep(u32);

/// `migrate_entity_remove`'s Phase-2 fire loop: the entity moves {M7Removed,
/// M7Keep} → {M7Keep}. `world_ptr` is minted in Phase 2 AFTER the Phase-1
/// block's `source` / `target` `&mut Archetype` dropped; the fire reads the
/// SOURCE (dying) row (the §0 asymmetry) — `EntityInland` is repointed only in
/// Phase 3, AFTER the fire. on_replace then on_remove fire from the
/// command-apply context for the single removed id.
#[test]
fn miri_command_apply_remove_migration_fires_replace_remove() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_replace::<M7Removed>(m7_replace);
    ecs.observe_on_remove::<M7Removed>(m7_remove);

    let arch = ecs.create_archetype(&[M7Removed::component_id(), M7Keep::component_id()]);
    let e = ecs.spawn_two(arch, M7Removed(55), M7Keep(9)).expect("spawn");

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).remove::<M7Removed>();
    });

    assert!(!ecs.has_component(e, M7Removed::component_id()), "M7Removed gone");
    assert_eq!(M7_REPLACE.load(SEQ), 1, "migrate_entity_remove fired on_replace once");
    assert_eq!(M7_REMOVE.load(SEQ), 1, "migrate_entity_remove fired on_remove once");
}

// ── M8: re-entrancy THROUGH the command-apply fire path ──────────────────────
// An observer that itself fires from a command-apply context (SpawnAtCommand::
// apply) enqueues a FURTHER deferred command. The view's `commands()` routes
// into the world-resident queue; the outermost drain applies it. This drives
// the OBS-FIRE-LOOP `from_world` mint AND the re-entrant view `commands()` mint
// nested inside the deferred-apply bracket — the hardest borrow choreography.

static M8_PARENT_FIRES: AtomicUsize = AtomicUsize::new(0);
static M8_CHILD_FIRES: AtomicUsize = AtomicUsize::new(0);

unsafe fn m8_parent_add(mut w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    M8_PARENT_FIRES.fetch_add(1, SEQ);
    // Enqueue a further deferred spawn from inside the command-apply fire path.
    w.commands().spawn(M8ChildBundle { c: M8Child(2) });
}
unsafe fn m8_child_add(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    M8_CHILD_FIRES.fetch_add(1, SEQ);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct M8Parent(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct M8Child(u32);

#[derive(Bundle)]
struct M8ParentBundle {
    c: M8Parent,
}

#[derive(Bundle)]
struct M8ChildBundle {
    c: M8Child,
}

/// The parent is spawned via a DEFERRED `cmds.spawn` (so its on_add fires from
/// `SpawnAtCommand::apply`, the command-apply context). That observer's runner
/// itself enqueues a child spawn; the child's on_add fires when the queued
/// child command applies at the outermost drain. Re-entrancy through the
/// command-apply fire path must be TB-clean and apply exactly once.
///
/// FIXED (NEW-2, fix wave): previously a deferred command enqueued by a callback
/// fired from a COMMAND-APPLY context (here `SpawnAtCommand::apply`) landed in
/// `world.deferred_hook_queue` but was never drained — neither the per-system
/// `CommandQueue::apply` walk nor `run_cached_system` drained it afterward, so
/// the child command silently vanished (`M8_CHILD_FIRES == 0`). It was a plain
/// LOGIC gap, NOT a Tree-Borrows failure (Miri ran to completion with zero
/// aliasing UB). The fix adds `self.drain_deferred_hook_queue()` at the tail of
/// `run_cached_system` (`ecs_master.rs:1862`, depth-0-gated), mirroring
/// `Schedule::run`'s apply-window barrier drain, so the re-entrant child command
/// now drains and applies exactly once. This test now asserts both the TB-clean
/// re-entrant borrow choreography AND the drain semantics.
#[test]
fn miri_command_apply_reentrant_observer_enqueues_command() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_add::<M8Parent>(m8_parent_add);
    ecs.observe_on_add::<M8Child>(m8_child_add);
    let _ = M8Parent::component_id();
    let _ = M8Child::component_id();

    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(M8ParentBundle { c: M8Parent(1) });
    });

    assert_eq!(M8_PARENT_FIRES.load(SEQ), 1, "parent on_add fired once (command-apply path)");
    assert_eq!(
        M8_CHILD_FIRES.load(SEQ),
        1,
        "the re-entrant child on_add fired once at the outermost drain"
    );
    assert_eq!(ecs.entity_count(), 2, "parent + re-entrant child both present after the drain");
}

// Silence unused-import lints under the `#![cfg(miri)]` gate.
#[allow(dead_code)]
static _TOUCH: AtomicU32 = AtomicU32::new(0);
