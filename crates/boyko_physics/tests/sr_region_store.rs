//! SR phase B — the warm store as the solve region's last stage (the cut's C4 gate): the records the
//! store writes, read back as the next step's seeds, equal the one-worker run's on every frame.
//!
//! The default world runs at W 1, 4 and 8 (the solve region is the default since SR's flip). On
//! every frame the next step's seed of every point of every logical manifold — the stream's, whose
//! records the store just wrote, and the held store's (`ColoredSoftStepSolver::for_each_warm_seed`)
//! — and the step's `WarmSeedStats` are folded with the bodies' bits into one FNV-1a hash, which
//! must equal W1's (W1 never opens a region). Arms:
//!
//! | arm | the store | what it covers |
//! |---|---|---|
//! | `rest` | in the region (no row can bounce) | pass 1's impulse writes per cohort range, pass 2's empty records |
//! | `bouncy` | serial, after the restitution pass | restitution 0.5 on every row (the region ends before the store), and a row of boxes dropped [`DROP`] onto the static floor so the restitution pass applies impulses the next step's seeds carry |
//! | `sleeping` | in the region | islands freeze: pass 2's carry (B1) per manifold range, L10's held path (`solve_colored_held`, the sleep-skip's `Sets` mode); [`SLIDERS`] frictionless boxes slide on a frictionless floor and never sleep, so a wide colour stays awake and the region keeps opening (and storing) while the stacks are frozen |
//!
//! # Non-vacuity
//!
//! - Every region-step arm ran its store in the region on every region step of W ≥ 2
//!   (`ColoredSoftStepSolver::region_store_dispatches`), and the bouncy arm on none.
//! - The sleeping arm carried a frozen manifold's points on some frame (`carry_points > 0`) and held
//!   a row in L10's sets on some frame; and at every W ≥ 2 some frame's store ran IN THE REGION with
//!   a frozen manifold to carry — without the sliders every frozen step lays out too few points to
//!   open a region, and the region's pass 2 would never carry (SR C4's red-first found it so).
//! - Some seed is a hit (the store wrote impulses that the next step reads).
//! - The bouncy arm's restitution acts: some box rebounds upward faster than [`REBOUND`] on some
//!   frame of the one-worker run, and the same drop at restitution 0 never does — a resting pile
//!   never reaches the restitution threshold, and then a store taken before the restitution pass
//!   would store the same impulses.
//!
//! The seeds are compared before the store counts, so a store that runs in the region on a step
//! where a row can bounce reds the identity, not only the count.
//!
//! Spins real thread pools (intractable under Miri), so `cfg(not(miri))`.

#![cfg(not(miri))]

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_systems;
use boyko_physics::resources::{Manifolds, PhysicsConfig};
use boyko_physics::sleep_sets::SleepSets;
use boyko_physics::solver::DefaultRigidSolver;

/// Fixed timestep.
const DT: f32 = 1.0 / 60.0;

/// Stacks per side: 9 × 9 = 81 two-box stacks, 162 dynamic boxes; the floor colour holds about 324
/// slots, over the default wide floor.
const GRID: usize = 9;

/// The bouncy arm's drop: a row of `GRID` boxes beside the stacks starts this far above its rest on
/// the static floor and strikes it at about `sqrt(2 g DROP)` ≈ 3.4 m/s, over the restitution
/// threshold (1 m/s). Against a STATIC row on purpose: `vn0` is a relative velocity.
const DROP: f32 = 0.6;

/// The upward speed (m/s) only a restitution rebound off the floor reaches in the drop.
const REBOUND: f32 = 0.5;

/// The sleeping arm's sliders: frictionless boxes sliding at [`SLIDE_SPEED`] on a frictionless
/// floor, never asleep; their floor contacts are body-disjoint, so they make one colour of
/// `4 × SLIDERS` points, over the default wide floor (256).
const SLIDERS: usize = 80;

/// The sliders' speed (m/s), along +x: far over the sleep threshold, and the same for every slider,
/// so none meets another.
const SLIDE_SPEED: f32 = 1.0;

/// One arm.
#[derive(Clone, Copy, Debug)]
struct Arm {
    name: &'static str,
    restitution: f32,
    /// [`SLIDERS`] sliders on a frictionless floor, spawned before the stacks.
    sliders: bool,
    /// How far above its rest the dropped row starts (m); 0 drops nothing.
    drop: f32,
    sleeping: bool,
    frames: usize,
    /// Whether the store runs in the region on a region step.
    region_store: bool,
}

const ARMS: [Arm; 3] = [
    Arm { name: "rest", restitution: 0.0, sliders: false, drop: 0.0, sleeping: false, frames: 30, region_store: true },
    Arm { name: "bouncy", restitution: 0.5, sliders: false, drop: DROP, sleeping: false, frames: 30, region_store: false },
    Arm { name: "sleeping", restitution: 0.0, sliders: true, drop: 0.0, sleeping: true, frames: 160, region_store: true },
];

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; the slice views its `size_of::<T>()` bytes
    //   read-only for the duration of the borrow, which is the exact layout the component pool
    //   stores.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// One box's material and start velocity.
#[derive(Clone, Copy)]
struct Mat {
    restitution: f32,
    friction: f32,
    velocity: Vec3,
}

fn spawn_box(world: &mut EcsMaster, position: Vec3, half_extents: Vec3, inv_mass: f32, mat: Mat) {
    let body = RigidBody { position, linear_velocity: mat.velocity, rotation: Quat::IDENTITY, angular_velocity: Vec3::ZERO };
    let mass = RigidBodyMass {
        inv_inertia: if inv_mass == 0.0 { Mat3::ZERO } else { Mat3::from_diagonal(Vec3::new(6.0, 6.0, 6.0)) },
        inv_mass,
        restitution: mat.restitution,
        friction: mat.friction,
    };
    let collider = Collider { shape: ColliderShape::Box { half_extents }, layer: 1, mask: 1 };
    let archetype = world.bundle_archetype_id_for::<RigidBodyBundle>();
    let e = world
        .create_entity(
            archetype,
            &[
                (RigidBody::component_id(), as_bytes(&body)),
                (RigidBodyMass::component_id(), as_bytes(&mass)),
                (Collider::component_id(), as_bytes(&collider)),
            ],
        )
        .expect("invariant: RigidBodyBundle archetype accepts the three columns");
    world.enable::<Simulated>(e);
}

/// A static floor and a `GRID × GRID` field of two-box stacks that settle within a few dozen
/// frames, so a sleeping run freezes its islands; with `arm.drop > 0` a row of `GRID` boxes beside
/// the field starts that far above its rest and falls onto the floor; with `arm.sliders` the floor
/// is frictionless and [`SLIDERS`] frictionless boxes slide beside the field (spawned before the
/// stacks, so the stacks' manifolds come after theirs in the stream and sit in the store's last
/// manifold range). The stacks keep friction 0.5 (a contact's friction is the max of its rows').
fn spawn_scene(world: &mut EcsMaster, arm: Arm) {
    let restitution = arm.restitution;
    let still = |friction: f32| Mat { restitution, friction, velocity: Vec3::ZERO };
    let floor_friction = if arm.sliders { 0.0 } else { 0.5 };
    spawn_box(world, Vec3::new(0.0, -0.5, 0.0), Vec3::new(40.0, 0.5, 40.0), 0.0, still(floor_friction));
    let half = Vec3::new(0.5, 0.5, 0.5);
    let origin = -0.75 * (GRID as f32 - 1.0);
    if arm.sliders {
        let slide = Mat { restitution, friction: 0.0, velocity: Vec3::new(SLIDE_SPEED, 0.0, 0.0) };
        for s in 0..SLIDERS {
            let (i, k) = (s / 10, s % 10);
            spawn_box(world, Vec3::new(-origin + 4.0 + 1.5 * i as f32, 0.49, -7.0 + 1.5 * k as f32), half, 1.0, slide);
        }
    }
    for i in 0..GRID {
        for k in 0..GRID {
            let (x, z) = (origin + 1.5 * i as f32, origin + 1.5 * k as f32);
            spawn_box(world, Vec3::new(x, 0.49, z), half, 1.0, still(0.5));
            spawn_box(world, Vec3::new(x, 1.48, z), half, 1.0, still(0.5));
        }
    }
    if arm.drop > 0.0 {
        let x = -origin + 3.0;
        for k in 0..GRID {
            spawn_box(world, Vec3::new(x, 0.5 + arm.drop, origin + 1.5 * k as f32), half, 1.0, still(0.5));
        }
    }
}

/// FNV-1a 64 of one word.
fn fold(h: u64, word: u64) -> u64 {
    let mut h = h;
    for byte in word.to_le_bytes() {
        h ^= u64::from(byte);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// One frame's observation: the bodies, the next step's seeds and the step's warm statistics.
fn frame_hash(world: &mut EcsMaster) -> (u64, u64) {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    {
        let q = world.query::<&RigidBody, ()>();
        for b in q.iter() {
            for w in [
                b.position.x,
                b.position.y,
                b.position.z,
                b.linear_velocity.x,
                b.linear_velocity.y,
                b.linear_velocity.z,
                b.rotation.x,
                b.rotation.y,
                b.rotation.z,
                b.rotation.w,
                b.angular_velocity.x,
                b.angular_velocity.y,
                b.angular_velocity.z,
            ] {
                h = fold(h, u64::from(w.to_bits()));
            }
        }
    }
    let solver = world.resource::<DefaultRigidSolver>();
    let manifolds = world.resource::<Manifolds>();
    let sets = world.resource::<SleepSets>();
    let mut hits = 0u64;
    solver.for_each_warm_seed(manifolds, sets, |key, fid, seed| {
        h = fold(h, key);
        h = fold(h, u64::from(fid));
        match seed {
            Some(s) => {
                hits += 1;
                for v in s {
                    h = fold(h, u64::from(v.to_bits()));
                }
            }
            None => h = fold(h, u64::MAX),
        }
    });
    let s = solver.warm_seed_stats();
    for v in [s.manifolds, s.translated, s.points, s.point_hits, s.carry_points, s.carry_hits] {
        h = fold(h, u64::from(v));
    }
    (fold(h, s.remap_resets), hits)
}

/// What one run produced.
struct Run {
    hashes: Vec<u64>,
    seed_hits: u64,
    regions: u64,
    region_stores: u64,
    carried_frames: usize,
    held_frames: usize,
    /// Frames whose store ran in the region with a frozen manifold's points to carry.
    region_carried_frames: usize,
    /// The fastest upward body velocity over every frame.
    max_up: f32,
}

fn run(arm: Arm, workers: usize) -> Run {
    let mut world = EcsMaster::new();
    spawn_scene(&mut world, arm);
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    let keys = add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut world);
    assert!(keys.build_graph.is_some(), "construction: the default world is the colored solve");
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(DT)));
    let mut schedule = builder.build(&mut world);
    {
        let cfg = world.resource_mut::<PhysicsConfig>();
        cfg.parallel_solve = true;
        cfg.sleeping = arm.sleeping;
    }
    let (r0, s0) = {
        let solver = world.resource::<DefaultRigidSolver>();
        (solver.region_dispatches(), solver.region_store_dispatches())
    };
    let mut out = Run {
        hashes: Vec::with_capacity(arm.frames),
        seed_hits: 0,
        regions: 0,
        region_stores: 0,
        carried_frames: 0,
        held_frames: 0,
        region_carried_frames: 0,
        max_up: f32::NEG_INFINITY,
    };
    let mut stores = s0;
    for _ in 0..arm.frames {
        schedule.run(&mut world);
        let (h, hits) = frame_hash(&mut world);
        out.hashes.push(h);
        out.seed_hits += hits;
        let (carrying, stores_now) = {
            let solver = world.resource::<DefaultRigidSolver>();
            (solver.warm_seed_stats().carry_points > 0, solver.region_store_dispatches())
        };
        out.carried_frames += usize::from(carrying);
        out.region_carried_frames += usize::from(carrying && stores_now > stores);
        stores = stores_now;
        out.held_frames += usize::from(world.resource::<SleepSets>().stats().held_rows > 0);
        let q = world.query::<&RigidBody, ()>();
        out.max_up = q.iter().map(|b| b.linear_velocity.y).fold(out.max_up, f32::max);
    }
    let solver = world.resource::<DefaultRigidSolver>();
    out.regions = solver.region_dispatches() - r0;
    out.region_stores = solver.region_store_dispatches() - s0;
    out
}

#[test]
fn region_store_seeds_equal_the_one_worker_run() {
    for arm in ARMS {
        let reference = run(arm, 1);
        assert_eq!(reference.regions, 0, "{}: one worker opens no region", arm.name);
        assert!(reference.seed_hits > 0, "{}: non-vacuity: some next-step seed is a hit", arm.name);
        if arm.sleeping {
            assert!(
                reference.carried_frames > 0 && reference.held_frames > 0,
                "{}: non-vacuity: a frozen manifold was carried ({} frames) and a row held ({} frames)",
                arm.name,
                reference.carried_frames,
                reference.held_frames
            );
        }
        if arm.restitution > 0.0 {
            let control = run(Arm { name: "bouncy_control", restitution: 0.0, ..arm }, 1);
            eprintln!(
                "[SR] {}: fastest upward speed {} m/s, the same drop at restitution 0 {} m/s",
                arm.name, reference.max_up, control.max_up
            );
            assert!(
                reference.max_up > REBOUND && control.max_up < REBOUND,
                "{}: non-vacuity: the restitution pass must act — a rebound faster than {REBOUND} m/s \
                 (got {}), which the same drop at restitution 0 never reaches (got {})",
                arm.name,
                reference.max_up,
                control.max_up
            );
        }
        for workers in [4usize, 8] {
            let got = run(arm, workers);
            assert!(got.regions > 0, "{} at W{workers}: the region opened", arm.name);
            if arm.sleeping {
                eprintln!(
                    "[SR] {} at W{workers}: {} region steps, {} of them stored in the region with a frozen manifold to carry",
                    arm.name, got.regions, got.region_carried_frames
                );
                assert!(
                    got.region_carried_frames > 0,
                    "{} at W{workers}: non-vacuity: some region step's store carried a frozen manifold",
                    arm.name
                );
            }
            for (frame, (a, b)) in reference.hashes.iter().zip(&got.hashes).enumerate() {
                assert_eq!(
                    a, b,
                    "{} at W{workers}: frame {frame}'s bodies, next-step seeds and warm statistics differ from W1's",
                    arm.name
                );
            }
            let want_stores = if arm.region_store { got.regions } else { 0 };
            assert_eq!(
                got.region_stores, want_stores,
                "{} at W{workers}: the store ran in the region on {} of {} region steps",
                arm.name, got.region_stores, got.regions
            );
        }
    }
}
