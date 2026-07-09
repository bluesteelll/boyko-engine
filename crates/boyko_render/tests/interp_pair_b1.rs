//! Pillar B increment B1 — the interpolation-DATA side, end to end.
//!
//! Exercises the FIRST production dense component
//! ([`GpuTransform3D`](boyko_render::GpuTransform3D)) through the REAL derive /
//! registration path plus the single-site per-substep pack
//! ([`pack_gpu_transforms`](boyko_render::pack_gpu_transforms)):
//!
//! * `seed` — a fresh spawn has `prev == curr` bitwise BEFORE any pack (the
//!   no-teleport rule, mirroring `boyko_demo`'s T6);
//! * `shuffle` — over 3 substeps of an authored `Transform` walk, `curr` tracks the
//!   current `Transform` and `prev` equals the PRIOR substep's `curr` BITWISE (the
//!   D3 single-shuffle discipline, mirroring `boyko_demo`'s T3);
//! * `physics` — a falling `DynamicBody` under the real physics fixed step: after N
//!   updates the pair's `curr` tracks the integrated `Transform` and `prev` lags it
//!   by exactly one substep.
//!
//! The dense component lands in `boyko_render`, so this crate's own test target is
//! the "real production consumer" gate for the fresh dense kernel: the pair is
//! spawned via `Commands::spawn` (the dense insert routing), read back via the mixed
//! `(&Transform, &GpuTransform3D)` query (the D3 mixed path), and mutated by the pack
//! system (the dense `&mut` write-through).

use std::sync::Arc;
use std::time::Duration;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_macros::Bundle;

use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::bundles::DynamicBody;
use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyMass, Simulated};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_systems_with_scene_sync;
use boyko_physics::resources::PhysicsConfig;
use boyko_physics::solver::NoopSolver;

use boyko_render::{GpuTransform3D, pack_gpu_transforms};

use boyko_scene::render_caps::{MaterialHandle, MeshHandle, Visibility};
use boyko_scene::transform::{GlobalTransform, Transform};

use bytemuck::bytes_of;

/// A deterministic single-threaded pool.
fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// A `(Transform, GpuTransform3D)` spawn bundle — the table `Transform` rides
/// alongside the dense `GpuTransform3D` pair (tuples are not `Bundle`s; a named
/// `#[derive(Bundle)]` struct is the spawn payload, like the D3 `TransformBody`).
#[derive(Bundle)]
struct PairBundle {
    transform: Transform,
    pair: GpuTransform3D,
}

/// A dense-ONLY bundle carrying just the `GpuTransform3D` pair, used to attach the
/// pair to an already-spawned entity (e.g. a physics `DynamicBody`).
///
/// A dense component's `#[component(storage = "dense")]` derive DELIBERATELY
/// suppresses its single-component self-`Bundle` (the D0 rationale, `component.rs:315`),
/// so a bare `.insert(pair)` does not compile. A one-field `#[derive(Bundle)]` struct
/// is the supported attach path (the D3 dense-spawn discipline): the derive emits the
/// bundle whose only id is the dense id, and the D2 structural-op routing writes it
/// into the dense store. This is the render-side consumer, so the render-side test
/// owns the bundle newtype.
#[derive(Bundle)]
struct PairOnly {
    pair: GpuTransform3D,
}

/// Bitwise equality over the whole 96-byte pair record (NaN-proof, rounding-proof —
/// the G7 bar, lifted from `boyko_demo/tests/interpolation.rs`).
fn trs_bits_eq(a: &boyko_render::TrsPacked, b: &boyko_render::TrsPacked) -> bool {
    bytes_of(a) == bytes_of(b)
}

/// A distinct decomposed pose per substep so a stale `prev`/`curr` is caught by
/// value: translation walks along +x, rotation stays identity (the pack copies it
/// verbatim), scale stays unit.
fn walking_transform(step: u32) -> Transform {
    Transform {
        translation: Vec3::new(step as f32 * 2.5, -1.0, 3.0),
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    }
}

/// Spawns one entity carrying a table `Transform` + the dense `GpuTransform3D`,
/// seeded from that transform (`prev == curr`). Returns its `Entity`.
fn spawn_pair(world: &mut EcsMaster, t: Transform) -> Entity {
    let sink: Arc<std::sync::Mutex<Option<Entity>>> = Arc::new(std::sync::Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let e = cmds
            .spawn(PairBundle {
                transform: t,
                pair: GpuTransform3D::from_transform(&t),
            })
            .id();
        *probe.lock().expect("probe") = Some(e);
    });
    sink.lock().expect("probe").expect("pair spawn handle")
}

/// Reads back the SOLE dense pair via `dense_iter` (a dense component lives in the
/// global `DenseStore`, NOT the archetype column — `get_component` would read the
/// null archetype column and return `None`; the dense query is the read path). Every
/// test spawns exactly one pair-bearing entity, so the first row is the one under
/// test.
fn read_pair(world: &mut EcsMaster) -> GpuTransform3D {
    let view = world.query::<&GpuTransform3D, ()>();
    let mut it = view.dense_iter();
    let (_e, pair) = it.next().expect("exactly one dense pair exists");
    *pair
}

/// Counts the live dense pairs (the dense-presence oracle — a dense component is
/// signature-excluded, so `has_component` returns `false`; the store count is the
/// real membership witness).
fn pair_count(world: &mut EcsMaster) -> usize {
    world.query::<&GpuTransform3D, ()>().dense_iter().count()
}

// ════════════════════════════════════════════════════════════════════════════════
// seed — a fresh spawn has prev == curr bitwise BEFORE any pack.
// ════════════════════════════════════════════════════════════════════════════════

#[test]
fn spawn_seeds_prev_equal_curr_bitwise() {
    let mut world = EcsMaster::new();
    let t = walking_transform(0);
    let _entity = spawn_pair(&mut world, t);
    assert_eq!(pair_count(&mut world), 1, "the dense pair spawned into the store");

    let g = read_pair(&mut world);
    assert!(
        trs_bits_eq(&g.prev, &g.curr),
        "a freshly spawned pair must have prev == curr bitwise (the seed rule)"
    );
    // And curr is genuinely the spawn pose (not a zeroed default).
    let want = boyko_render::TrsPacked::from_transform(&t);
    assert!(trs_bits_eq(&g.curr, &want), "curr mirrors the spawn Transform");
}

// ════════════════════════════════════════════════════════════════════════════════
// shuffle — over 3 substeps, prev == prior curr bitwise (D3 single-shuffle).
// ════════════════════════════════════════════════════════════════════════════════

#[test]
fn pack_shuffles_prev_to_prior_curr_over_three_substeps() {
    let mut world = EcsMaster::new();
    let entity = spawn_pair(&mut world, walking_transform(0));

    let mut prior_curr = read_pair(&mut world).curr;

    // Three substeps: author a fresh Transform, then run the pack. The pack must
    // shuffle prev = old curr, then write curr = from(the new Transform).
    for step in 1..=3u32 {
        // Author the new pose on the table Transform (the source of truth).
        {
            let mut t = world
                .get_component_mut::<Transform>(entity)
                .expect("transform lives");
            *t = walking_transform(step);
        }

        world.run_system(pack_gpu_transforms);

        let g = read_pair(&mut world);

        // D3 binding assertion: prev is the PRIOR substep's curr, bitwise.
        assert!(
            trs_bits_eq(&g.prev, &prior_curr),
            "substep {step}: prev must be the prior substep's curr bitwise"
        );
        // curr tracks the freshly authored Transform.
        let want = boyko_render::TrsPacked::from_transform(&walking_transform(step));
        assert!(
            trs_bits_eq(&g.curr, &want),
            "substep {step}: curr must track the current Transform"
        );
        // The row actually moved (independent witness) ⇒ prev != curr.
        assert!(
            !trs_bits_eq(&g.prev, &g.curr),
            "substep {step}: a moved row must have prev != curr"
        );

        prior_curr = g.curr;
    }
}

// ════════════════════════════════════════════════════════════════════════════════
// physics — a falling DynamicBody: curr tracks Transform, prev lags one substep.
// ════════════════════════════════════════════════════════════════════════════════

/// The starting pose: high up so gravity has room to integrate downward.
const START: Vec3 = Vec3::new(0.0, 50.0, 0.0);

/// Builds a `DynamicBody` at [`START`] under unit dynamic mass + a unit sphere.
fn dynamic_body_at_start() -> DynamicBody {
    DynamicBody {
        transform: Transform::from_translation(START),
        global: GlobalTransform::default(),
        mesh: MeshHandle(1),
        material: MaterialHandle(0x00CD),
        body: RigidBody {
            position: START,
            linear_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
        },
        mass: RigidBodyMass {
            inv_inertia: Mat3::IDENTITY,
            inv_mass: 1.0,
            restitution: 0.5,
            friction: 0.3,
        },
        collider: Collider {
            shape: ColliderShape::Sphere { radius: 0.5 },
            layer: 1,
            mask: 1,
        },
        visibility: Visibility::Visible,
    }
}

/// Spawns the `DynamicBody` + attaches the dense `GpuTransform3D` pair (seeded from
/// the spawn pose) + enables the `Simulated` bit so the body integrates.
fn spawn_falling_body_with_pair(world: &mut EcsMaster) -> Entity {
    let sink: Arc<std::sync::Mutex<Option<Entity>>> = Arc::new(std::sync::Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let seed = GpuTransform3D::from_transform(&Transform::from_translation(START));
        let e = cmds
            .spawn(dynamic_body_at_start())
            .insert(PairOnly { pair: seed })
            .enable::<Simulated>()
            .id();
        *probe.lock().expect("probe") = Some(e);
    });
    sink.lock().expect("probe").expect("body spawn handle")
}

/// Wires the physics pipeline + S5 scene sync for the `NoopSolver` (so the Foundation
/// integrator runs and gravity moves the body), with a fixed `dt`.
fn build_physics(world: &mut EcsMaster, dt: f32) -> Schedule {
    let mut builder = ScheduleBuilder::new(serial_pool());
    let _ = add_physics_systems_with_scene_sync::<NoopSolver>(&mut builder, world);
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(dt)));
    builder.build(world)
}

#[test]
fn falling_body_pair_tracks_transform_and_prev_lags_one_substep() {
    let mut world = EcsMaster::new();
    let entity = spawn_falling_body_with_pair(&mut world);

    // Sanity: the dense pair routed into the store (dense insert routed correctly for
    // the first production dense consumer). A dense component is signature-excluded, so
    // `has_component` returns `false`; the store count is the membership oracle.
    assert_eq!(
        pair_count(&mut world),
        1,
        "the dense GpuTransform3D pair is attached to the body"
    );

    let dt = 1.0 / 64.0;
    let mut schedule = build_physics(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -10.0, 0.0);

    // Per substep: run the physics fixed step (which integrates + `sync_body_to_transform`
    // mirrors the pose into Transform), THEN the pack (ordered after the scene-sync tail —
    // here explicit sequencing gives the deterministic order the wiring fn's `.after`
    // edge produces in a combined schedule).
    const SUBSTEPS: u32 = 6;
    let mut prior_curr = read_pair(&mut world).curr;
    let mut prior_transform = *world.get_component::<Transform>(entity).expect("transform");

    let mut fell = false;
    for substep in 0..SUBSTEPS {
        schedule.run(&mut world);
        world.run_system(pack_gpu_transforms);

        let t = *world.get_component::<Transform>(entity).expect("transform");
        let g = read_pair(&mut world);

        // curr tracks the post-solve Transform for THIS substep, bitwise.
        let want_curr = boyko_render::TrsPacked::from_transform(&t);
        assert!(
            trs_bits_eq(&g.curr, &want_curr),
            "substep {substep}: pair.curr must track the integrated Transform"
        );
        // prev lags by exactly one substep: it is the PRIOR substep's curr, which is
        // the prior substep's Transform packed.
        assert!(
            trs_bits_eq(&g.prev, &prior_curr),
            "substep {substep}: pair.prev must be the prior substep's curr (one-substep lag)"
        );
        let want_prev = boyko_render::TrsPacked::from_transform(&prior_transform);
        assert!(
            trs_bits_eq(&g.prev, &want_prev),
            "substep {substep}: pair.prev packs the prior substep's Transform"
        );

        if t.translation.y < START.y {
            fell = true;
        }
        prior_curr = g.curr;
        prior_transform = t;
    }

    assert!(fell, "the body must have fallen under gravity over {SUBSTEPS} substeps");
    // After the fall the current pose is below the start (curr tracked it down).
    let final_t = *world.get_component::<Transform>(entity).expect("transform");
    assert!(
        final_t.translation.y < START.y,
        "final Transform.y {} < start {}",
        final_t.translation.y,
        START.y
    );
}
