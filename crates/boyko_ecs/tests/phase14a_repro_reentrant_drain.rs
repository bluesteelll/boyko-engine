//! Phase 14a — focused reproduction of the deferred-hook re-entrancy hazard
//! (TESTER FINDING). See the firing/deferred suites; this file isolates the
//! single behaviour so the developer can pin the root cause.
//!
//! HYPOTHESIS: `EcsMaster::drain_deferred_hook_queue` does NOT raise
//! `hook_drain_depth` while it walks the queue via `apply_via_raw_twin`. When a
//! deferred command resolves to a direct-API method that itself drains
//! (`delete_entity` / `create_entity` / `create_entity_at`, each of which calls
//! `drain_deferred_hook_queue` at the end of its body), that inner call sees
//! `depth == 0` and RE-ENTERS the drain on the SAME in-flight queue —
//! re-applying the command currently being walked. The second application of a
//! `DespawnCommand` finds the entity already removed → the `delete_entity`
//! `false` return trips `DespawnCommand::apply`'s debug_assert.
//!
//! Both the SCHEDULE apply-window drive and the direct-API drive reach
//! `drain_deferred_hook_queue`, so this is exercised on the production path, not
//! only the lightweight `run_system` helper.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::{Bundle, Component};
use boyko_threadpool::ThreadPoolBuilder;

const SEQ: Ordering = Ordering::SeqCst;

static TARGET: AtomicU64 = AtomicU64::new(u64::MAX);
static REMOVE_FIRES: AtomicUsize = AtomicUsize::new(0);

fn set_target(e: Entity) {
    TARGET.store((e.id().0 as u64) | ((e.generation() as u64) << 32), SEQ);
}
fn get_target() -> Entity {
    let p = TARGET.load(SEQ);
    Entity::new(EntityId((p & 0xFFFF_FFFF) as usize), (p >> 32) as u32)
}

/// Trigger's on_add enqueues a despawn of the pre-existing target.
unsafe fn trigger_on_add(mut w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    w.commands().entity(get_target()).despawn();
}
/// Target's on_remove — counts fires. Should be EXACTLY 1 (no double-apply).
unsafe fn target_on_remove(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    REMOVE_FIRES.fetch_add(1, SEQ);
}

#[derive(Component)]
#[component(on_add = trigger_on_add)]
#[repr(C)]
#[derive(Clone, Copy)]
struct ReproTrigger(u32);

#[derive(Component)]
#[component(on_remove = target_on_remove)]
#[repr(C)]
#[derive(Clone, Copy)]
struct ReproTarget(u32);

#[derive(Bundle)]
struct ReproTriggerBundle {
    c: ReproTrigger,
}

/// Production-path (schedule) reproduction. The deferred `SpawnAtCommand` for
/// the trigger fires `on_add` inside `CommandQueue::apply` (depth >= 1); the
/// hook enqueues a despawn into `deferred_hook_queue`; the schedule's
/// outermost `drain_deferred_hook_queue` then applies it. If the drain is not
/// re-entrancy-guarded, `delete_entity`'s own end-of-body drain re-applies the
/// despawn and the debug_assert in `DespawnCommand::apply` fires.
#[test]
#[cfg_attr(miri, ignore)] // thread pool — Phase 9.1 precedent
fn schedule_hook_enqueued_despawn_applies_exactly_once() {
    REMOVE_FIRES.store(0, SEQ);
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    let target_arch = world.create_archetype(&[ReproTarget::component_id()]);
    let _trigger_arch = world.create_archetype(&[ReproTrigger::component_id()]);

    let target = world.spawn_one(target_arch, ReproTarget(1)).expect("spawn target");
    set_target(target);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|mut cmds: Commands| {
        cmds.spawn(ReproTriggerBundle { c: ReproTrigger(2) });
    });
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    assert_eq!(
        REMOVE_FIRES.load(SEQ),
        1,
        "target on_remove must fire EXACTLY once (no re-entrant double-apply)"
    );
    assert!(!world.has_entity(target), "deferred despawn removed the target");
    assert_eq!(world.entity_count(), 1, "only the trigger remains");
}
