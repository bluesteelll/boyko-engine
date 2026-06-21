//! S2 propagation GATE — DETERMINISM: a `GlobalTransform` value is invariant to
//! the order in which siblings are visited.
//!
//! A child's world pose depends ONLY on its own LOCAL `Transform` and its
//! parent's (already-finalized) `GlobalTransform` — never on a sibling. So the
//! final per-entity values must be identical no matter the (unspecified)
//! `Children` sibling order, which `swap_remove` actively perturbs on every
//! detach.
//!
//! The gate builds the SAME logical tree in two worlds with DIFFERENT sibling
//! orders — `world_a` attaches children in index order; `world_b` attaches them
//! in a non-trivial permutation (reverse + a mid-swap), so the parent's
//! `Children` Vec stores siblings in a genuinely different visit order. Children
//! are spawned BEFORE any attach so entity ids do not encode the attach order;
//! the only difference the descent sees is the stored sibling order. The test
//! then asserts every child's final propagated value (keyed by its per-index
//! local) is BIT-IDENTICAL across the two worlds. The propagation runs through a
//! hand-built single-system [`Schedule`].
//!
//! `Children::swap_remove` order-perturbation on REMOVAL is deliberately NOT used
//! here: a full `ChildOf` removal (detach) is not re-propagated (FINDING F1, see
//! `gates_composition_structural`), which would confound this value comparison.

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::Children;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::Bundle;
use boyko_math::{Affine3A, Quat, Vec3};
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

/// A tick-advance no-op (see the `gates_composition_structural` harness doc for
/// why the world tick must be lifted off `Tick::ZERO` before any spawn).
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

    fn spawn(&mut self, t: Transform) -> Entity {
        let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
        let probe = Arc::clone(&sink);
        self.world.run_system(move |mut cmds: Commands| {
            *probe.lock().expect("probe") = Some(cmds.spawn(spatial(t)).id());
        });
        sink.lock().expect("probe").expect("handle")
    }

    fn attach(&mut self, parent: Entity, child: Entity) {
        self.tick();
        self.world.run_system(move |mut cmds: Commands| {
            cmds.entity(parent).add_child(child);
        });
    }

    /// Advances the world change tick (without running propagation) so a post-run
    /// structural edit lands in the next run's `(last_run, this_run]` window.
    fn tick(&mut self) {
        self.ticker.run(&mut self.world);
    }

    fn run(&mut self) {
        self.schedule.run(&mut self.world);
    }

    fn global(&self, e: Entity) -> Affine3A {
        self.world
            .get_component::<GlobalTransform>(e)
            .expect("has GlobalTransform")
            .affine()
    }

    fn children_order(&self, parent: Entity) -> Vec<Entity> {
        self.world
            .get_component::<Children>(parent)
            .map(|c| c.as_slice().to_vec())
            .unwrap_or_default()
    }
}

/// A deterministic per-index LOCAL transform (varied translation + rotation +
/// non-uniform scale, so order-sensitivity in any matrix lane would diverge).
fn local_for(i: usize) -> Transform {
    let a = (i as f32) * 0.37;
    Transform {
        translation: Vec3::new(i as f32, -(i as f32) * 0.5, (i as f32) * 0.25),
        rotation: Quat::new(0.0, 0.0, a.sin(), a.cos()).normalize(),
        scale: Vec3::new(1.0 + (i as f32) * 0.1, 1.0, 1.0 + (i as f32) * 0.05),
    }
}

fn assert_affine_bit_eq(a: Affine3A, b: Affine3A, ctx: &str) {
    // Same scalar ops, same inputs ⇒ identical f32 bits regardless of traversal
    // order. Use exact equality: any difference is a real order-dependence bug.
    for r in 0..3 {
        assert_eq!(a.matrix3.rows[r].x, b.matrix3.rows[r].x, "{ctx}: m3[{r}].x");
        assert_eq!(a.matrix3.rows[r].y, b.matrix3.rows[r].y, "{ctx}: m3[{r}].y");
        assert_eq!(a.matrix3.rows[r].z, b.matrix3.rows[r].z, "{ctx}: m3[{r}].z");
    }
    assert_eq!(a.translation.x, b.translation.x, "{ctx}: t.x");
    assert_eq!(a.translation.y, b.translation.y, "{ctx}: t.y");
    assert_eq!(a.translation.z, b.translation.z, "{ctx}: t.z");
}

// ════════════════════════════════════════════════════════════════════════════
// DETERMINISM — different sibling visit order, identical final values.
// ════════════════════════════════════════════════════════════════════════════

/// Builds a parent + `FAN` children (child `i` carries `local_for(i)`), attaching
/// the children in the order given by `attach_order` (a permutation of `0..FAN`).
/// Returns `(parent, kids)` where `kids[i]` is the handle carrying `local_for(i)`.
/// All children are spawned FIRST (so entity ids do not encode the attach order),
/// then attached in the requested order, so the parent's `Children` Vec stores
/// siblings in `attach_order` — the lever that perturbs the descent's visit order.
fn build_fan(s: &mut Scene, fan: usize, attach_order: &[usize]) -> (Entity, Vec<Entity>) {
    let parent = s.spawn(local_for(100));
    let kids: Vec<Entity> = (0..fan).map(|i| s.spawn(local_for(i))).collect();
    for &i in attach_order {
        s.attach(parent, kids[i]);
    }
    (parent, kids)
}

#[test]
fn global_transform_invariant_to_sibling_order() {
    const FAN: usize = 16;

    // World A: attach in INDEX order 0,1,…,15.
    let order_a: Vec<usize> = (0..FAN).collect();
    let mut a = Scene::new();
    let (a_parent, a_kids) = build_fan(&mut a, FAN, &order_a);
    a.run();

    // World B: attach in a NON-TRIVIAL permutation (reverse + a mid-swap), so the
    // parent's `Children` stores siblings in a genuinely different order. Entity
    // ids are sequential-by-spawn in BOTH worlds (children spawned before any
    // attach), so a different STORED order is the only difference the descent sees.
    let mut order_b: Vec<usize> = (0..FAN).rev().collect();
    order_b.swap(3, 11); // ensure it is not merely the exact reverse of A
    let mut b = Scene::new();
    let (b_parent, b_kids) = build_fan(&mut b, FAN, &order_b);
    b.run();

    // Precondition: the two worlds MUST store siblings in different order. Map each
    // stored child handle back to its per-index local key (the spawn id offset from
    // the first child) so the comparison is meaningful across worlds.
    let stored_a = a.children_order(a_parent);
    let stored_b = b.children_order(b_parent);
    let key_a: Vec<usize> = stored_a
        .iter()
        .map(|e| a_kids.iter().position(|k| k == e).expect("child of A"))
        .collect();
    let key_b: Vec<usize> = stored_b
        .iter()
        .map(|e| b_kids.iter().position(|k| k == e).expect("child of B"))
        .collect();
    assert_ne!(
        key_a, key_b,
        "test precondition: the two worlds must store siblings in DIFFERENT visit \
         order (a={key_a:?} b={key_b:?})"
    );

    // Parent: same local ⇒ same world pose, bit-for-bit.
    assert_affine_bit_eq(a.global(a_parent), b.global(b_parent), "parent");

    // Every child, KEYED BY its per-index local: the propagated value must be
    // bit-identical across the two differently-ordered worlds (a child's world
    // pose depends only on its parent's finalized global + its own local — never
    // on a sibling or the visit order).
    for i in 0..FAN {
        assert_affine_bit_eq(
            a.global(a_kids[i]),
            b.global(b_kids[i]),
            &format!("child #{i} (order-invariant)"),
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// DETERMINISM — re-running propagation on an UNCHANGED tree yields byte-identical
// values (no drift across runs; idempotent recompute).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn repeated_runs_are_idempotent() {
    const FAN: usize = 8;
    let mut s = Scene::new();
    let parent = s.spawn(local_for(100));
    let mut kids = Vec::with_capacity(FAN);
    for i in 0..FAN {
        let k = s.spawn(local_for(i));
        s.attach(parent, k);
        kids.push(k);
    }
    s.run();
    let snap: Vec<Affine3A> = kids.iter().map(|&k| s.global(k)).collect();

    // Run several more times (a still tree). Values must not drift by a single
    // bit (the recompute is a pure function of unchanged inputs).
    for _ in 0..5 {
        s.run();
    }
    for (i, &k) in kids.iter().enumerate() {
        assert_affine_bit_eq(s.global(k), snap[i], &format!("child #{i} across re-runs"));
    }
}
