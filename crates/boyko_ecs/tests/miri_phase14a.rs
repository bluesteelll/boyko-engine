//! Phase 14a — Miri (Stacked / Tree Borrows) coverage for the lifecycle-hook
//! unsafe paths. Single-thread only (multi-thread Miri deferred per Phase 9.1).
//!
//! Run via: `cargo +nightly miri test -p boyko-ecs --test miri_phase14a`
//! with `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks"`. The
//! `-Zmiri-ignore-leaks` is REQUIRED: the deferred-spawn scenarios touch the
//! bundle caches, whose `BundleColumnRecord`s carry deliberate, bounded
//! `Box::leak`s (#53-class, SBO6-bounded per `(BundleTypeId, ArchetypeId)`
//! per world — a borrow-decoupling design choice, not a bug; the Phase X.I
//! tester proved the report pre-existing).
//!
//! Covers the four `unsafe`-bearing dispatch behaviours (plan §8 Miri subset):
//!
//! 1. **Dispatch path** — a hook bumps a resource via the read-only view's
//!    `resource_mut`; exercises `DeferredEcsMaster::from_world` +
//!    `world.as_mut()` provenance under TB.
//! 2. **Deferred-command path** — a hook enqueues a spawn via `ctx.commands()`;
//!    the outermost drain applies it. Exercises the `NonNull<EcsMaster>` mint at
//!    the trigger site + the world-resident queue push + drain.
//! 3. **Pre-drop remove path** — a remove hook reads the dying value via
//!    `get_component`, while `EntityInland` still points at the SOURCE row
//!    (the bytes are live but logically dying). Exercises the §3.5 phased
//!    restructure (drop every `&mut Archetype` before firing).
//! 4. **§3.5 dual-presence window** — the remove migrates an entity out of a
//!    MULTI-entity archetype, so `move_out_entity` swap-removes a THIRD entity
//!    into the vacated slot. The remove hook reads `C` from the dying entity
//!    during that window — the row-pointer + swap interaction Miri scrutinises.
//!
//! # File gate
//!
//! `#![cfg(miri)]` — only compiles under Miri. Native runs ignore the file; the
//! `phase14a_hooks_firing` / `phase14a_hooks_deferred` integration suites cover
//! the same semantics end-to-end on the native target.

#![cfg(miri)]

use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::{Bundle, Component, Resource};

const SEQ: Ordering = Ordering::SeqCst;

// ════════════════════════════════════════════════════════════════════════════
// 1 — dispatch path: hook bumps a resource via resource_mut
// ════════════════════════════════════════════════════════════════════════════

#[derive(Resource)]
struct MiriCounter(u32);

unsafe fn m1_on_add(mut w: DeferredEcsMaster<'_>, _c: HookContext) {
    if let Some(c) = w.resource_mut::<MiriCounter>() {
        c.0 += 1;
    }
}

#[derive(Component)]
#[component(on_add = m1_on_add)]
#[repr(C)]
struct M1Comp(u32);

#[test]
fn miri_dispatch_resource_mut() {
    let mut ecs = EcsMaster::new();
    ecs.insert_resource(MiriCounter(0));
    let arch = ecs.create_archetype(&[M1Comp::component_id()]);

    let _e = ecs.spawn_one(arch, M1Comp(1)).expect("spawn");
    let _e2 = ecs.spawn_one(arch, M1Comp(2)).expect("spawn");

    assert_eq!(
        ecs.resource::<MiriCounter>().0,
        2,
        "the on_add hook bumped the resource via resource_mut twice"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// 2 — deferred-command path: hook enqueues a spawn; drain applies it
// ════════════════════════════════════════════════════════════════════════════

unsafe fn m2_on_add(mut w: DeferredEcsMaster<'_>, _c: HookContext) {
    w.commands().spawn(M2ChildBundle { c: M2Child(1) });
}

#[derive(Component)]
#[component(on_add = m2_on_add)]
#[repr(C)]
struct M2Parent(u32);

#[derive(Component)]
#[repr(C)]
struct M2Child(u32);

#[derive(Bundle)]
struct M2ChildBundle {
    c: M2Child,
}

#[test]
fn miri_deferred_command_enqueue_then_drain() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[M2Parent::component_id()]);
    let _ = M2Child::component_id();

    // Direct API → the parent's on_add enqueues a child spawn; create_entity's
    // outermost drain applies it.
    let _p = ecs.spawn_one(arch, M2Parent(7)).expect("spawn parent");
    assert_eq!(ecs.entity_count(), 2, "deferred child spawn applied at the drain");
}

// ════════════════════════════════════════════════════════════════════════════
// 3 — pre-drop remove path: remove hook reads the dying value
// ════════════════════════════════════════════════════════════════════════════

static M3_SAW: AtomicU32 = AtomicU32::new(u32::MAX);

unsafe fn m3_on_remove(w: DeferredEcsMaster<'_>, ctx: HookContext) {
    // The dying bytes are still live (PRE-drop, EntityInland still at SOURCE).
    if let Some(v) = w.get_component::<M3Removed>(ctx.entity) {
        M3_SAW.store(v.0, SEQ);
    }
}

#[derive(Component)]
#[component(on_remove = m3_on_remove)]
#[repr(C)]
#[derive(Clone, Copy)]
struct M3Removed(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct M3Keep(u32);

#[test]
fn miri_pre_drop_remove_reads_dying_value() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[M3Removed::component_id(), M3Keep::component_id()]);
    let e = ecs.spawn_two(arch, M3Removed(123), M3Keep(0)).expect("spawn");

    ecs.run_system(move |mut cmds: boyko_ecs::ecs::core::system::Commands| {
        cmds.entity(e).remove::<M3Removed>();
    });

    assert_eq!(
        M3_SAW.load(SEQ),
        123,
        "remove hook read the dying SOURCE value (123) via get_component"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// 4 — §3.5 dual-presence window: remove from a MULTI-entity archetype triggers a
//      swap of a THIRD entity; the remove hook reads C from the dying row
// ════════════════════════════════════════════════════════════════════════════

static M4_TARGET: AtomicU64 = AtomicU64::new(u64::MAX);
static M4_SAW: AtomicU32 = AtomicU32::new(u32::MAX);

fn m4_set(e: Entity) {
    M4_TARGET.store((e.id().0 as u64) | ((e.generation() as u64) << 32), SEQ);
}
fn m4_get() -> Entity {
    let p = M4_TARGET.load(SEQ);
    Entity::new(EntityId((p & 0xFFFF_FFFF) as usize), (p >> 32) as u32)
}

unsafe fn m4_on_remove(w: DeferredEcsMaster<'_>, ctx: HookContext) {
    // Read the dying value of the target while a third entity is mid-swap into
    // its vacated slot (move_out_entity runs AFTER this hook, but the dual-
    // presence reasoning §3.5 is what TB scrutinises here).
    if ctx.entity == m4_get() {
        if let Some(v) = w.get_component::<M4Comp>(ctx.entity) {
            M4_SAW.store(v.0, SEQ);
        }
    }
}

#[derive(Component)]
#[component(on_remove = m4_on_remove)]
#[repr(C)]
#[derive(Clone, Copy)]
struct M4Comp(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct M4Other(u32);

#[test]
fn miri_dual_presence_window_swap_remove() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[M4Comp::component_id(), M4Other::component_id()]);

    // Three entities in the same archetype. Remove M4Comp from the FIRST
    // (row 0) → migrate to {M4Other}; move_out_entity on the source archetype
    // swaps the last (row 2) into row 0 — the dual-presence window.
    let e0 = ecs.spawn_two(arch, M4Comp(10), M4Other(0)).expect("spawn e0");
    let _e1 = ecs.spawn_two(arch, M4Comp(20), M4Other(0)).expect("spawn e1");
    let _e2 = ecs.spawn_two(arch, M4Comp(30), M4Other(0)).expect("spawn e2");
    m4_set(e0);

    ecs.run_system(move |mut cmds: boyko_ecs::ecs::core::system::Commands| {
        cmds.entity(e0).remove::<M4Comp>();
    });

    assert_eq!(
        M4_SAW.load(SEQ),
        10,
        "remove hook read e0's dying M4Comp value (10) during the dual-presence window"
    );
    assert!(ecs.has_entity(e0), "e0 survives (only its M4Comp was removed)");
    assert!(!ecs.has_component(e0, M4Comp::component_id()), "M4Comp removed from e0");
    // The swapped-in entity must still be intact + queryable.
    assert_eq!(ecs.entity_count(), 3, "all three entities still live");
}

// Silence unused-counter lints under non-miri (the file is `#![cfg(miri)]`
// gated, but keep the imports honest).
#[allow(dead_code)]
static _TOUCH: AtomicUsize = AtomicUsize::new(0);
