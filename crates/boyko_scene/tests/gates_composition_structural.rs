//! S2 propagation GATES — WORLD COMPOSITION (moved-parent re-propagation, wide
//! trees) and STRUCTURAL change (add / remove a child), driven through a
//! HAND-BUILT [`Schedule`] that runs `propagate_transforms`.
//!
//! # Why a hand-built `Schedule` (mirrors `boyko_ui` / `boyko_demo`)
//!
//! `propagate_transforms` is dirty-gated against a `(last_run, this_run]` change
//! window held in its scratch resource. Driving it through a real
//! [`ScheduleBuilder`]→[`Schedule`] keeps the change-detection window advancing
//! frame-to-frame exactly the way a host drives the system (the established
//! `boyko_ui` `common::Ui` / `boyko_demo` `SimRunner` schedule-build pattern):
//! `schedule.run(&mut world)` bumps the world tick and runs the exclusive system,
//! so a mutation issued before a `run()` is seen exactly once on that run.
//!
//! Spawns and structural edits go through `Commands` (so the `ChildOf`/`Children`
//! hooks maintain the reverse collection the descent reads) and are harvested out
//! of the `Send + Sync` system closure via an `Arc<Mutex<…>>` probe — the
//! established Phase-11/19 deferred-spawn pattern.

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::Bundle;
use boyko_math::{Affine3A, Quat, Vec3};
use boyko_threadpool::ThreadPoolBuilder;

use boyko_scene::propagation::compute_global_transform;
use boyko_scene::{GlobalTransform, Transform, propagate_transforms};

// ───────────────────────── harness ─────────────────────────

#[derive(Bundle)]
struct SpatialBundle {
    transform: Transform,
    global: GlobalTransform,
}

#[inline]
fn spatial(transform: Transform) -> SpatialBundle {
    SpatialBundle { transform, global: GlobalTransform::IDENTITY }
}

/// A no-op exclusive system: a tick-advance vehicle that does NOT touch the
/// propagation scratch's `last_run`.
fn noop_exclusive(_w: &mut EcsMaster) {}

/// A test world plus the single-system propagation [`Schedule`] (the hand-built
/// schedule vehicle). `run()` advances the change-detection window and runs the
/// propagation once.
///
/// A separate single-noop `ticker` schedule is run once in [`Scene::new`] to lift
/// the world change tick off the `Tick::ZERO` sentinel BEFORE any spawn — the
/// `(last_run, this_run]` lower bound excludes `Tick::ZERO`, so a component
/// stamped at tick 0 would never be seen dirty by the first propagation run (the
/// established `advance_tick` discipline from the in-crate `propagation.rs`
/// suite, here expressed through a schedule to satisfy the hand-built-schedule
/// vehicle). The ticker is a SEPARATE schedule so advancing the tick does not run
/// (and thus does not advance the `last_run` of) the propagation system.
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
        // Lift the world tick off ZERO before any spawn (see the struct doc).
        ticker.run(&mut world);

        Self { world, schedule, ticker }
    }

    /// Spawns one spatial entity, optionally under `parent`, returning its live
    /// handle (live after the one apply window `run_system` drives).
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

    /// Runs `propagate_transforms` once through the schedule.
    fn run(&mut self) {
        self.schedule.run(&mut self.world);
    }

    /// Advances the world change tick (without running propagation) so a mutation
    /// issued AFTER a propagation run lands strictly inside the next run's
    /// `(last_run, this_run]` window. `run_system` alone does NOT advance the
    /// world tick (only a `Schedule::run` does), so a post-run mutation must be
    /// preceded by a tick advance or it would stamp at `<= last_run` and be missed
    /// — the `advance_tick` discipline from the in-crate `propagation.rs` suite.
    fn tick(&mut self) {
        self.ticker.run(&mut self.world);
    }

    fn global(&self, e: Entity) -> Affine3A {
        self.world
            .get_component::<GlobalTransform>(e)
            .expect("entity has GlobalTransform")
            .affine()
    }

    /// Reparents `child` under `new_parent` (overwrites `ChildOf` in place;
    /// stamps the child's structural tick). Advances the tick first so the edit
    /// is observed on the next propagation run.
    fn reparent(&mut self, child: Entity, new_parent: Entity) {
        self.tick();
        self.world.run_system(move |mut cmds: Commands| {
            cmds.entity(child).set_parent(new_parent);
        });
    }

    /// Links `child` under `parent` (`set_parent`), advancing the tick first so the
    /// new `ChildOf` is observed on the next propagation run. Unlike [`reparent`]
    /// this carries no semantic intent — it is the raw edge builder the cycle gate
    /// uses to close a loop (`A→B→A`), which the kernel does NOT reject (only the
    /// one-compare self-reference is guarded — `hierarchy/mod.rs`).
    fn link(&mut self, child: Entity, parent: Entity) {
        self.tick();
        self.world.run_system(move |mut cmds: Commands| {
            cmds.entity(child).set_parent(parent);
        });
    }

    /// Detaches `child` from its parent (removes `ChildOf`; the child becomes a
    /// root and keeps its prior LOCAL transform). Uses the parent-side
    /// `remove_children` detach path.
    fn detach(&mut self, parent: Entity, child: Entity) {
        self.tick();
        self.world.run_system(move |mut cmds: Commands| {
            cmds.entity(parent).remove_children(&[child]);
        });
    }

    /// Detaches `child` via the CHILD-side `remove_parent` path (the other detach
    /// API leg — also a full `ChildOf` removal, so it must fire the F1 observer
    /// the same way `remove_children` does).
    fn detach_via_remove_parent(&mut self, child: Entity) {
        self.tick();
        self.world.run_system(move |mut cmds: Commands| {
            cmds.entity(child).remove_parent();
        });
    }

    /// Rewrites `e`'s LOCAL transform (stamps its `Transform` changed tick).
    fn set_local(&mut self, e: Entity, t: Transform) {
        self.tick();
        self.world.run_system(move |mut cmds: Commands| {
            cmds.entity(e).insert(t);
        });
    }
}

const EPS: f32 = 1e-4;

#[track_caller]
fn assert_affine_eq(got: Affine3A, want: Affine3A, ctx: &str) {
    for r in 0..3 {
        let g = got.matrix3.rows[r];
        let w = want.matrix3.rows[r];
        assert!(
            (g.x - w.x).abs() < EPS && (g.y - w.y).abs() < EPS && (g.z - w.z).abs() < EPS,
            "{ctx}: matrix3 row {r} mismatch: got {g:?} want {w:?}"
        );
    }
    let gt = got.translation;
    let wt = want.translation;
    assert!(
        (gt.x - wt.x).abs() < EPS && (gt.y - wt.y).abs() < EPS && (gt.z - wt.z).abs() < EPS,
        "{ctx}: translation mismatch: got {gt:?} want {wt:?}"
    );
}

fn quat_z_90() -> Quat {
    use std::f32::consts::FRAC_1_SQRT_2;
    Quat::new(0.0, 0.0, FRAC_1_SQRT_2, FRAC_1_SQRT_2)
}

// ════════════════════════════════════════════════════════════════════════════
// COMPOSITION — a moved parent re-propagates ALL descendants (value-checked).
// The existing suite checks the COMPOSE COUNT after a grandparent move; this
// checks the resulting VALUES of every descendant after moving an interior node.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn moved_parent_repropagates_all_descendants() {
    let mut s = Scene::new();
    let g_t = Transform::from_translation(Vec3::new(1.0, 0.0, 0.0));
    let p_t = Transform::from_translation(Vec3::new(0.0, 2.0, 0.0));
    let c_t = Transform::from_translation(Vec3::new(0.0, 0.0, 3.0));
    let gc_t = Transform::from_translation(Vec3::new(4.0, 0.0, 0.0));

    let g = s.spawn(g_t, None);
    let p = s.spawn(p_t, Some(g));
    let c = s.spawn(c_t, Some(p));
    let gc = s.spawn(gc_t, Some(c)); // 4-deep chain: g→p→c→gc

    s.run();
    // Baseline: full chain composes.
    let want_g0 = g_t.to_affine();
    let want_p0 = want_g0.mul(p_t.to_affine());
    let want_c0 = want_p0.mul(c_t.to_affine());
    let want_gc0 = want_c0.mul(gc_t.to_affine());
    assert_affine_eq(s.global(gc), want_gc0, "grandchild before move");

    // Move the INTERIOR node `p` (not the root). Its subtree (c, gc) must
    // re-propagate; the root `g` is unchanged.
    let p_t2 = Transform::from_translation(Vec3::new(0.0, 20.0, 0.0));
    s.set_local(p, p_t2);
    s.run();

    let want_g = g_t.to_affine(); // root unmoved
    let want_p = want_g.mul(p_t2.to_affine());
    let want_c = want_p.mul(c_t.to_affine());
    let want_gc = want_c.mul(gc_t.to_affine());
    assert_affine_eq(s.global(g), want_g, "root after interior move");
    assert_affine_eq(s.global(p), want_p, "moved parent");
    assert_affine_eq(s.global(c), want_c, "child re-propagated");
    assert_affine_eq(s.global(gc), want_gc, "grandchild re-propagated through moved ancestor");
}

// ════════════════════════════════════════════════════════════════════════════
// COMPOSITION — a wide tree (one parent, many children) composes every child
// against parent.global ∘ child.local, and a parent move updates the whole fan.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn wide_fan_composes_and_repropagates() {
    let mut s = Scene::new();
    let parent_t = Transform {
        translation: Vec3::new(10.0, 0.0, 0.0),
        rotation: quat_z_90(),
        scale: Vec3::new(2.0, 2.0, 2.0),
    };
    let parent = s.spawn(parent_t, None);

    const FAN: usize = 64;
    let mut kids = Vec::with_capacity(FAN);
    let mut kid_locals = Vec::with_capacity(FAN);
    for i in 0..FAN {
        let t = Transform::from_translation(Vec3::new(i as f32, -(i as f32), (i as f32) * 0.5));
        kid_locals.push(t);
        kids.push(s.spawn(t, Some(parent)));
    }

    s.run();
    let want_parent0 = parent_t.to_affine();
    for (i, &k) in kids.iter().enumerate() {
        let want = want_parent0.mul(kid_locals[i].to_affine());
        assert_affine_eq(s.global(k), want, &format!("fan child {i} before move"));
    }

    // Move the parent: every one of the 64 children re-propagates.
    let parent_t2 = Transform {
        translation: Vec3::new(-5.0, 7.0, 1.0),
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };
    s.set_local(parent, parent_t2);
    s.run();

    let want_parent = parent_t2.to_affine();
    for (i, &k) in kids.iter().enumerate() {
        let want = want_parent.mul(kid_locals[i].to_affine());
        assert_affine_eq(s.global(k), want, &format!("fan child {i} after parent move"));
    }
}

// ════════════════════════════════════════════════════════════════════════════
// STRUCTURAL — ADD a child to an existing root re-propagates it next run.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn add_child_to_existing_root_repropagates() {
    let mut s = Scene::new();
    let root_t = Transform::from_translation(Vec3::new(100.0, 0.0, 0.0));
    let child_t = Transform::from_translation(Vec3::new(1.0, 1.0, 1.0));

    let root = s.spawn(root_t, None);
    let child = s.spawn(child_t, None); // initially an INDEPENDENT root
    s.run();
    // Child starts as its own root: global == its own local.
    assert_affine_eq(s.global(child), child_t.to_affine(), "child as independent root");

    // Attach child under root (a first-attach: stamps the child's ChildOf tick).
    s.reparent(child, root);
    s.run();
    assert_affine_eq(
        s.global(child),
        root_t.to_affine().mul(child_t.to_affine()),
        "child re-propagates after being added under root",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// STRUCTURAL — REMOVE a child (detach) makes it a root again next run, with its
// world pose collapsing back to its own LOCAL transform.
//
// ── FINDING F1 (FIXED) ──────────────────────────────────────────────────────
// A full `ChildOf` REMOVAL (detach to root) is now re-propagated. The defect was
// that `collect_dirty` flags a node only when a PRESENT `Transform` OR a PRESENT
// `ChildOf` has a newer `changed_tick`; a detach removes `ChildOf` entirely, so
// the now-absent link carried no tick and the orphaned child was never in the
// dirty set — it kept its stale parent-relative `GlobalTransform` until its
// `Transform` was next written. The fix routes the unlink through the
// `child_of_on_remove` observer, which queues the orphaned entity on the
// propagation scratch's detach queue; `propagate_transforms` drains it each run
// and recomposes the entity as a root (`GlobalTransform = local.to_affine()`).
// This test now asserts that fixed re-root behavior.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn remove_child_makes_it_a_root_again() {
    let mut s = Scene::new();
    let root_t = Transform::from_translation(Vec3::new(50.0, 0.0, 0.0));
    let child_t = Transform::from_translation(Vec3::new(0.0, 3.0, 0.0));

    let root = s.spawn(root_t, None);
    let child = s.spawn(child_t, Some(root));
    s.run();
    assert_affine_eq(
        s.global(child),
        root_t.to_affine().mul(child_t.to_affine()),
        "child under root before detach",
    );

    // EXPECTED (gate #4): detach removes ChildOf ⇒ the child becomes a root and
    // its world pose collapses to its own local affine on the next run.
    s.detach(root, child);
    s.run();
    assert_affine_eq(
        s.global(child),
        child_t.to_affine(),
        "detached child collapses to its own local pose (now a root)",
    );
    assert_affine_eq(
        s.global(child),
        compute_global_transform(&s.world, child),
        "detached child matches the reference walk",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// STRUCTURAL — F1 via the CHILD-side `remove_parent` detach path. The other
// detach API leg must re-root the orphan the same way `remove_children` does:
// its world pose collapses to its own LOCAL affine on the next run.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn remove_parent_makes_child_a_root_again() {
    let mut s = Scene::new();
    let root_t = Transform::from_translation(Vec3::new(70.0, 0.0, 0.0));
    let child_t = Transform::from_translation(Vec3::new(0.0, 5.0, 0.0));

    let root = s.spawn(root_t, None);
    let child = s.spawn(child_t, Some(root));
    s.run();
    assert_affine_eq(
        s.global(child),
        root_t.to_affine().mul(child_t.to_affine()),
        "child under root before remove_parent",
    );

    // Detach via the child-side leg. The F1 observer fires on the full ChildOf
    // removal and queues the orphan; the next run recomposes it as a root.
    s.detach_via_remove_parent(child);
    s.run();
    assert_affine_eq(
        s.global(child),
        child_t.to_affine(),
        "remove_parent orphan collapses to its own local pose",
    );
    assert_affine_eq(
        s.global(child),
        compute_global_transform(&s.world, child),
        "remove_parent orphan matches the reference walk",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// STRUCTURAL — F1 on an INTERIOR node carrying its own subtree. Detaching a
// middle node makes IT a root (collapses to its local) AND its descendant
// re-propagates from the new root pose (not the stale grandparent-relative one).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn detach_interior_node_reroots_its_subtree() {
    let mut s = Scene::new();
    let g_t = Transform::from_translation(Vec3::new(100.0, 0.0, 0.0));
    let mid_t = Transform::from_translation(Vec3::new(0.0, 10.0, 0.0));
    let leaf_t = Transform::from_translation(Vec3::new(0.0, 0.0, 1.0));

    let g = s.spawn(g_t, None);
    let mid = s.spawn(mid_t, Some(g));
    let leaf = s.spawn(leaf_t, Some(mid)); // g → mid → leaf
    s.run();
    assert_affine_eq(
        s.global(leaf),
        g_t.to_affine().mul(mid_t.to_affine()).mul(leaf_t.to_affine()),
        "leaf under g→mid before detach",
    );

    // Detach `mid` from `g`: mid becomes a root (collapses to mid_t), and leaf —
    // still mid's child — re-propagates as mid.global ∘ leaf.local from the NEW
    // (root) mid pose. leaf is dirtied via the descent through the re-rooted mid.
    s.detach(g, mid);
    s.run();
    assert_affine_eq(
        s.global(mid),
        mid_t.to_affine(),
        "detached interior node collapses to its own local pose (now a root)",
    );
    assert_affine_eq(
        s.global(leaf),
        mid_t.to_affine().mul(leaf_t.to_affine()),
        "leaf re-propagates from the re-rooted mid (not the stale grandparent pose)",
    );
    assert_affine_eq(
        s.global(leaf),
        compute_global_transform(&s.world, leaf),
        "leaf matches the reference walk after interior detach",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// STRUCTURAL — re-parent + add in sequence stays consistent (the consistency
// window): the world pose tracks the CURRENT parent each run. (Detach is covered
// separately above; FINDING F1 keeps it out of this multi-step happy path.)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn sequential_reparent_edits_stay_consistent() {
    let mut s = Scene::new();
    let a_t = Transform::from_translation(Vec3::new(1000.0, 0.0, 0.0));
    let b_t = Transform::from_translation(Vec3::new(0.0, 2000.0, 0.0));
    let child_t = Transform::from_translation(Vec3::new(7.0, 0.0, 0.0));

    let a = s.spawn(a_t, None);
    let b = s.spawn(b_t, None);
    let child = s.spawn(child_t, Some(a));
    s.run();
    assert_affine_eq(
        s.global(child),
        a_t.to_affine().mul(child_t.to_affine()),
        "child under A",
    );

    // A → B (a REPLACE: stamps the new ChildOf tick, so reparent re-propagates).
    s.reparent(child, b);
    s.run();
    assert_affine_eq(
        s.global(child),
        b_t.to_affine().mul(child_t.to_affine()),
        "child under B after reparent",
    );

    // B → A (back).
    s.reparent(child, a);
    s.run();
    assert_affine_eq(
        s.global(child),
        a_t.to_affine().mul(child_t.to_affine()),
        "child re-attached under A",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// CYCLE TERMINATION (FINDING F3) — a malformed deep `ChildOf` cycle the kernel
// does NOT detect (only the one-compare self-reference is guarded —
// `hierarchy/mod.rs`) must TERMINATE the propagation, never hang or read OOB.
//
// `propagate_transforms` carries three release-mode bounds for exactly this:
//   * `root_ancestor`'s ascent cap `MAX_ANCESTOR_DEPTH` (the seeding `ChildOf`
//     walk up from a dirty non-root),
//   * the descent's per-path depth cap `MAX_TRANSFORM_DEPTH` (one `Children`
//     chain), and
//   * the descent's total-visit cap `MAX_DESCENT_STEPS`.
// In DEBUG these caps fire a `debug_assert!` (fail loud); in RELEASE they bail
// out quietly (the cycle subtree is dropped) — never an infinite loop. These
// tests prove BOTH behaviors on a real, kernel-accepted cycle.
//
// NOTE on cost: a dirty cycle node makes `root_ancestor` ascend up to
// `MAX_ANCESTOR_DEPTH` (~1M) hops before bailing, so the release path here is
// intentionally heavy (bounded, but ~millions of iterations) — that is the
// documented `root_ancestor`-on-a-cycle cost the bound makes safe.
// ════════════════════════════════════════════════════════════════════════════

/// Builds a real 2-node `ChildOf` cycle (`A→B→A`) the kernel accepts (neither
/// edge is the guarded self-reference) and asserts the RELEASE-mode bounds
/// terminate the propagation run instead of hanging / reading OOB.
///
/// Debug builds `debug_assert!` inside the caps (fail loud), so this is gated to
/// release; the debug-mode companion `cycle_propagation_debug_asserts` asserts
/// the loud-failure leg.
#[cfg(not(debug_assertions))]
#[test]
fn cycle_propagation_terminates_in_release() {
    let mut s = Scene::new();
    let a = s.spawn(Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)), None);
    let b = s.spawn(Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)), Some(a));
    // Close the loop: A under B ⇒ A→B (existing) + B→A (new) = a 2-node cycle.
    s.link(a, b);

    // The whole point: this RETURNS. A pre-bound build would hang here.
    s.run();
    // Reaching this line is the assertion — the bounded descent / ascent
    // terminated. The cached poses are unspecified for a malformed cycle (the
    // subtree is dropped at the cap), so we assert termination, not values.
    assert!(s.world.has_entity(a) && s.world.has_entity(b), "both cycle nodes survive");

    // A deeper 3-node cycle (A→B→C→A) exercises the same bounds over a longer
    // per-path chain; it too must terminate.
    let mut s3 = Scene::new();
    let a3 = s3.spawn(Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)), None);
    let b3 = s3.spawn(Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)), Some(a3));
    let c3 = s3.spawn(Transform::from_translation(Vec3::new(0.0, 0.0, 1.0)), Some(b3));
    s3.link(a3, c3); // close A→B→C→A
    s3.run();
    assert!(
        s3.world.has_entity(a3) && s3.world.has_entity(b3) && s3.world.has_entity(c3),
        "all three cycle nodes survive a terminating propagation",
    );
}

/// Debug-mode companion: the same kernel-accepted 2-node cycle must trip a
/// `debug_assert!` inside one of the propagation caps (`MAX_ANCESTOR_DEPTH` in
/// `root_ancestor`, or `MAX_TRANSFORM_DEPTH` / `MAX_DESCENT_STEPS` in the
/// descent) — the "fail loud in debug" leg of the cycle guard. `#[should_panic]`
/// asserts the loud failure fires (debug builds only).
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "cycle")]
fn cycle_propagation_debug_asserts() {
    let mut s = Scene::new();
    let a = s.spawn(Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)), None);
    let b = s.spawn(Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)), Some(a));
    s.link(a, b); // close A→B→A
    s.run(); // a cap's `debug_assert!` must fire here (panics in debug)
}

// ════════════════════════════════════════════════════════════════════════════
// DEEP-CYCLE — a LONG ChildOf chain (N nodes) closed into a loop (tail → head)
// the kernel accepts (no edge is the guarded self-reference) exercises the
// descent's per-path depth cap over a genuinely DEEP structure, not just a 2/3
// node loop. It must TERMINATE: never hang, never panic-in-release, never read
// OOB. The release leg asserts return + survival; the debug leg asserts a cap's
// `debug_assert!` fires loud.
// ════════════════════════════════════════════════════════════════════════════

/// Builds a chain `n0 → n1 → … → n_{N-1}` (each `n_{i+1}` a child of `n_i`) and
/// returns the node handles. `n0` is the root, `n_{N-1}` the leaf.
fn build_chain(s: &mut Scene, len: usize) -> Vec<Entity> {
    assert!(len >= 2, "a chain needs at least two nodes");
    let mut nodes = Vec::with_capacity(len);
    let root = s.spawn(Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)), None);
    nodes.push(root);
    for i in 1..len {
        let parent = nodes[i - 1];
        let e = s.spawn(
            Transform::from_translation(Vec3::new(0.0, i as f32 * 0.01, 0.0)),
            Some(parent),
        );
        nodes.push(e);
    }
    nodes
}

/// RELEASE: a deep N-node chain closed into a cycle (`leaf`'s child becomes the
/// `root`, i.e. `root → … → leaf → root`) must terminate the propagation via the
/// bounded caps rather than hang. The point is that this `run()` RETURNS.
#[cfg(not(debug_assertions))]
#[test]
fn deep_chain_cycle_terminates_in_release() {
    const LEN: usize = 64;
    let mut s = Scene::new();
    let nodes = build_chain(&mut s, LEN);
    let root = nodes[0];
    let leaf = nodes[LEN - 1];

    // Close the loop: make the root a child of the leaf ⇒ root→…→leaf→root, a
    // deep N-node cycle the kernel does NOT reject (no edge is a self-reference).
    s.link(root, leaf);

    // The assertion is that this RETURNS (bounded caps terminate the descent /
    // ascent). A pre-bound build would hang here.
    s.run();
    for (i, &n) in nodes.iter().enumerate() {
        assert!(s.world.has_entity(n), "deep-cycle node #{i} survives a terminating run");
    }
}

/// DEBUG: the same deep N-node cycle must trip a `debug_assert!` inside one of the
/// propagation caps (the "fail loud in debug" leg over a DEEP chain).
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "cycle")]
fn deep_chain_cycle_debug_asserts() {
    const LEN: usize = 64;
    let mut s = Scene::new();
    let nodes = build_chain(&mut s, LEN);
    let root = nodes[0];
    let leaf = nodes[LEN - 1];
    s.link(root, leaf); // close root→…→leaf→root
    s.run(); // a cap's `debug_assert!` must fire here (panics in debug)
}
