//! S2 propagation GATE — F2 CHANGE DETECTION on `GlobalTransform`.
//!
//! `propagate_transforms` writes every `GlobalTransform` through
//! [`set_global_if_changed`] (`Mut::set_if_neq`), which bumps the row's
//! `changed_tick` ONLY on a real change. This gate proves the two F2 halves a
//! downstream `Changed<GlobalTransform>` consumer (camera `ViewUniform`,
//! GPU-instance upload, lights) depends on:
//!
//! 1. **F2-fires** — moving a parent's `Transform` ADVANCES the moved subtree's
//!    `GlobalTransform` `changed_tick`, so a `Changed<GlobalTransform>` query on
//!    the following frame observes the propagated move. (Earlier the write went
//!    through `get_component_raw_mut`, which BYPASSED the stamp; a propagated move
//!    was then invisible to change-detection — the F2 defect this fix closes.)
//! 2. **F2-quiet** — re-running propagation on an UNCHANGED tree does NOT advance
//!    any `GlobalTransform` `changed_tick` (set-if-changed; the 0%-overhead
//!    property holds for values that did not actually move).
//!
//! The `changed_tick` is read directly off the kernel via
//! [`EcsMaster::get_component_changed_tick`] (the same accessor the dirty scan
//! uses), keyed on `GlobalTransform::component_id()` — this is exactly the per-row
//! tick a `Changed<GlobalTransform>` filter compares.
//!
//! # The deterministic frame vehicle (mirrors `gates_composition_structural`)
//!
//! Propagation is dirty-gated against a `(last_run, this_run]` window held in its
//! scratch. A hand-built single-system [`Schedule`] drives it; a SEPARATE noop
//! "ticker" schedule advances the world change tick between runs so a post-run
//! mutation lands strictly inside the next run's window. The ticker is run once in
//! [`Scene::new`] to lift the world tick off the `Tick::ZERO` sentinel before any
//! spawn.

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::Bundle;
use boyko_math::Vec3;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_scene::{GlobalTransform, Transform, propagate_transforms};

#[derive(Bundle)]
struct SpatialBundle {
    transform: Transform,
    global: GlobalTransform,
}

#[inline]
fn spatial(transform: Transform) -> SpatialBundle {
    SpatialBundle { transform, global: GlobalTransform::IDENTITY }
}

fn noop_exclusive(_w: &mut EcsMaster) {}

struct Scene {
    world: EcsMaster,
    schedule: Schedule,
    ticker: Schedule,
}

impl Scene {
    fn new() -> Self {
        let mut world = EcsMaster::new();
        let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(2).build());
        builder.add_system(propagate_transforms);
        let schedule = builder.build(&mut world);

        let mut tbuilder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(2).build());
        tbuilder.add_system(noop_exclusive);
        let mut ticker = tbuilder.build(&mut world);
        ticker.run(&mut world); // lift the tick off ZERO before any spawn

        Self { world, schedule, ticker }
    }

    fn spawn(&mut self, t: Transform, parent: Option<Entity>) -> Entity {
        let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
        let probe = Arc::clone(&sink);
        self.world.run_system(move |mut cmds: Commands| {
            let mut ec = cmds.spawn(spatial(t));
            if let Some(p) = parent {
                ec.set_parent(p);
            }
            *probe.lock().expect("probe") = Some(ec.id());
        });
        let e = sink.lock().expect("probe").expect("spawn produced a handle");
        assert!(self.world.has_entity(e), "spawned entity live after apply");
        e
    }

    fn run(&mut self) {
        self.schedule.run(&mut self.world);
    }

    /// Advances the world change tick (without running propagation) so a post-run
    /// mutation lands inside the next run's `(last_run, this_run]` window.
    fn tick(&mut self) {
        self.ticker.run(&mut self.world);
    }

    fn set_local(&mut self, e: Entity, t: Transform) {
        self.tick();
        self.world.run_system(move |mut cmds: Commands| {
            cmds.entity(e).insert(t);
        });
    }

    /// The raw `GlobalTransform` `changed_tick` counter for `e` — the exact per-row
    /// tick a `Changed<GlobalTransform>` filter compares. Panics if absent (the
    /// caller spawned the spatial bundle, so the column is present).
    fn global_changed_tick(&self, e: Entity) -> u32 {
        let gid = GlobalTransform::component_id();
        self.world
            .get_component_changed_tick(e, gid)
            .expect("entity has a GlobalTransform changed tick")
            .get()
    }
}

// ════════════════════════════════════════════════════════════════════════════
// F2-FIRES — moving a parent ADVANCES the moved subtree's GlobalTransform tick.
// A Changed<GlobalTransform> query on the next frame would observe the move.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn changed_tick_fires_on_real_move() {
    let mut s = Scene::new();
    let root = s.spawn(Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)), None);
    let child = s.spawn(Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)), Some(root));
    let grandchild =
        s.spawn(Transform::from_translation(Vec3::new(0.0, 0.0, 3.0)), Some(child));

    // First run: the whole tree composes for the first time, stamping ticks.
    s.run();
    let root_t0 = s.global_changed_tick(root);
    let child_t0 = s.global_changed_tick(child);
    let gc_t0 = s.global_changed_tick(grandchild);

    // Move the ROOT. Its whole subtree (root, child, grandchild) re-propagates to
    // genuinely different world poses, so every one's GlobalTransform tick MUST
    // advance — the signal a Changed<GlobalTransform> reader keys on.
    s.set_local(root, Transform::from_translation(Vec3::new(50.0, 0.0, 0.0)));
    s.run();

    let root_t1 = s.global_changed_tick(root);
    let child_t1 = s.global_changed_tick(child);
    let gc_t1 = s.global_changed_tick(grandchild);

    assert!(
        root_t1 > root_t0,
        "moved root's GlobalTransform changed_tick must advance ({root_t0} -> {root_t1})"
    );
    assert!(
        child_t1 > child_t0,
        "moved subtree child's GlobalTransform changed_tick must advance ({child_t0} -> {child_t1})"
    );
    assert!(
        gc_t1 > gc_t0,
        "moved subtree grandchild's GlobalTransform changed_tick must advance ({gc_t0} -> {gc_t1})"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// F2-QUIET — a still frame advances NO GlobalTransform tick (set-if-changed; the
// 0%-overhead property: an unchanged recompose stays tick-silent).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn changed_tick_quiet_on_still_frame() {
    let mut s = Scene::new();
    let root = s.spawn(Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)), None);
    let child = s.spawn(Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)), Some(root));
    let grandchild =
        s.spawn(Transform::from_translation(Vec3::new(0.0, 0.0, 3.0)), Some(child));

    // First run stamps everyone.
    s.run();
    let root_t0 = s.global_changed_tick(root);
    let child_t0 = s.global_changed_tick(child);
    let gc_t0 = s.global_changed_tick(grandchild);

    // Advance the world tick and re-run on a STILL tree (nothing moved). No node's
    // GlobalTransform value changes, so set_if_changed must bump NO tick — a
    // Changed<GlobalTransform> reader sees nothing on a still frame.
    s.tick();
    s.run();
    s.tick();
    s.run();

    assert_eq!(
        s.global_changed_tick(root),
        root_t0,
        "still-frame root GlobalTransform tick must not advance"
    );
    assert_eq!(
        s.global_changed_tick(child),
        child_t0,
        "still-frame child GlobalTransform tick must not advance"
    );
    assert_eq!(
        s.global_changed_tick(grandchild),
        gc_t0,
        "still-frame grandchild GlobalTransform tick must not advance"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// F2 — PRECISION: moving ONE interior node advances ONLY its subtree's
// GlobalTransform ticks; an UNRELATED sibling subtree stays tick-silent.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn changed_tick_fires_only_on_moved_subtree() {
    let mut s = Scene::new();
    let root = s.spawn(Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)), None);
    // Two independent subtrees under the root.
    let a = s.spawn(Transform::from_translation(Vec3::new(0.0, 10.0, 0.0)), Some(root));
    let a_kid = s.spawn(Transform::from_translation(Vec3::new(0.0, 0.0, 1.0)), Some(a));
    let b = s.spawn(Transform::from_translation(Vec3::new(0.0, -10.0, 0.0)), Some(root));
    let b_kid = s.spawn(Transform::from_translation(Vec3::new(0.0, 0.0, 2.0)), Some(b));

    s.run();
    let root_t0 = s.global_changed_tick(root);
    let a_t0 = s.global_changed_tick(a);
    let a_kid_t0 = s.global_changed_tick(a_kid);
    let b_t0 = s.global_changed_tick(b);
    let b_kid_t0 = s.global_changed_tick(b_kid);

    // Move only subtree A's interior node.
    s.set_local(a, Transform::from_translation(Vec3::new(0.0, 99.0, 0.0)));
    s.run();

    // A and its child advance.
    assert!(
        s.global_changed_tick(a) > a_t0,
        "moved interior node A's tick must advance"
    );
    assert!(
        s.global_changed_tick(a_kid) > a_kid_t0,
        "A's child tick must advance (re-propagated through moved A)"
    );
    // The root did not move (only A's local changed) ⇒ its recomposed value is
    // identical ⇒ tick silent. B and B's child are an unrelated subtree ⇒ silent.
    assert_eq!(
        s.global_changed_tick(root),
        root_t0,
        "unmoved root must stay tick-silent (value unchanged)"
    );
    assert_eq!(
        s.global_changed_tick(b),
        b_t0,
        "unrelated sibling B must stay tick-silent"
    );
    assert_eq!(
        s.global_changed_tick(b_kid),
        b_kid_t0,
        "unrelated sibling B's child must stay tick-silent"
    );
}
