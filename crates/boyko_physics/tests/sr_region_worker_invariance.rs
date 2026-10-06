//! SR phase B — the solve region is worker-count invariant, bit for bit (`levers/scaling/
//! 02-SR-DESIGN.md` §2; the cut's C2 gate).
//!
//! The default world runs with the solver's region switch on (`ColoredSoftStepSolver::set_region`),
//! so every step whose parallel gate holds — `parallel_solve`, a pool of two workers or more, a
//! colour at least the grain's wide floor — runs its substeps as ONE region: gravity, the warm start
//! per colour, the biased and relax sweeps per colour and integrate + inertia, each a stage of the
//! region's table. The per-frame FNV-1a hash of every `RigidBody` bit and of the step's
//! `WarmSeedStats` (the fill's reduce, which a region step runs after its END: SR C3's warm-stats
//! {1,N} comparison) must equal the one-worker run's (which never opens a region: one lane is the
//! inline path) at W 2, 3, 4, 5, 8 and 16, on every arm:
//!
//! | arm | what it varies |
//! |---|---|
//! | `default` | the shipped configuration: `simd_solve`, contact reuse, sleeping off, V2 on |
//! | `scalar` | `simd_solve` off: the scalar oracle sweeps on group-granular cuts |
//! | `reuse_off` | contact reuse off |
//! | `sleeping` | sleeping on (the freeze capture and restore run around the region) |
//! | `d0` | V2's speculative contacts off (the d = 0 arm, the pre-V2 kernels) |
//! | `bouncy` | restitution 0.5 on every box, and a row of `GRID` boxes dropped [`DROP`] onto the static floor beside the stacks: they strike at about 3.4 m/s, over the restitution threshold, so the restitution pass applies impulses after the region and reads the fill's `vn0` |
//! | `grain` | every grain term lowered: every colour, gravity and integrate entry is multi-block |
//!
//! # Non-vacuity
//!
//! - Every frame's widest colour holds at least `2 × MIN_PARALLEL_SLOTS_PER_COLOR` slots, so the
//!   default grain's gate holds on every frame.
//! - On every arm of two or more workers the region opened on every frame
//!   (`ColoredSoftStepSolver::region_dispatches` rises by one per frame), and on the one-worker
//!   reference it never did.
//! - The `grain` arm's regions published every kind it lowers: the last region's report shows at
//!   least as many published items as the substeps' body and colour items need.
//! - The scene moves: the final hash differs from the spawn state's.
//! - The `bouncy` arm's restitution acts: some box rebounds upward faster than [`REBOUND`] on some
//!   frame of the one-worker run, and the same drop at restitution 0 never does (the witness is the
//!   restitution pass's, not the drop's).
//!
//! The switch is off by default until SR's flip; this file is the identity gate of the switch-on
//! path. Spins real thread pools (intractable under Miri), so `cfg(not(miri))`.

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
use boyko_physics::resources::{ConstraintGraph, Manifolds, PhysicsConfig};
use boyko_physics::solver::DefaultRigidSolver;
use boyko_physics::solver::colored::RegionGrain;

/// Frames each run steps.
const FRAMES: usize = 30;

/// Fixed timestep.
const DT: f32 = 1.0 / 60.0;

/// Stacks per side: 13 × 13 = 169 two-box stacks, 338 dynamic boxes, so the floor colour holds
/// about 676 slots (the scene of `default_world_worker_invariance.rs`).
const GRID: usize = 13;

/// The solver's inline floor (`MIN_PARALLEL_SLOTS_PER_COLOR`, the grain's default wide floor).
const WIDE_FLOOR: usize = 256;

/// The `bouncy` arm's drop: a row of boxes beside the stacks starts this far above its rest on the
/// static floor, so each strikes the floor at about `sqrt(2 g DROP)` ≈ 3.4 m/s, over the
/// restitution threshold (1 m/s), within the run's frames. A resting pile never reaches the
/// threshold, and then the restitution pass and the fill's `vn0` (read only there, D9) would be
/// invisible to this gate. The strike is against a STATIC row on purpose: `vn0` is a relative
/// velocity, and a box striking a dynamic box that gravity accelerates by the same `g·h` keeps it
/// unchanged, which hid a fill scheduled after the first gravity stage (SR C3's red-first).
const DROP: f32 = 0.6;

/// The upward speed (m/s) only a restitution rebound off the floor reaches in the `bouncy` arm's
/// drop (about 1.7 m/s at restitution 0.5); the same drop at restitution 0 stays near zero.
const REBOUND: f32 = 0.5;

/// The worker counts every arm is compared at, against W1.
const WORKERS: [usize; 6] = [2, 3, 4, 5, 8, 16];

/// The `grain` arm's terms: every floor at its minimum, so a colour of two groups, a body range of
/// two 8-row groups and a fill of two cohorts are all cut into blocks.
const LOWERED: RegionGrain = RegionGrain {
    wide_floor: 1,
    colour_min_points: 1,
    max_bpp: 6,
    body_bpp: 8,
    body_rows: 8,
    fill_points: 1,
};

/// One arm of the gate.
#[derive(Clone, Copy, Debug)]
struct Arm {
    name: &'static str,
    simd_solve: bool,
    contact_reuse: bool,
    sleeping: bool,
    speculative: bool,
    restitution: f32,
    /// How far above its rest each top box starts (m).
    drop: f32,
    grain: RegionGrain,
}

const DEFAULT_ARM: Arm = Arm {
    name: "default",
    simd_solve: true,
    contact_reuse: true,
    sleeping: false,
    speculative: true,
    restitution: 0.0,
    drop: 0.0,
    grain: RegionGrain::DEFAULT,
};

const ARMS: [Arm; 7] = [
    DEFAULT_ARM,
    Arm { name: "scalar", simd_solve: false, ..DEFAULT_ARM },
    Arm { name: "reuse_off", contact_reuse: false, ..DEFAULT_ARM },
    Arm { name: "sleeping", sleeping: true, ..DEFAULT_ARM },
    Arm { name: "d0", speculative: false, ..DEFAULT_ARM },
    Arm { name: "bouncy", restitution: 0.5, drop: DROP, ..DEFAULT_ARM },
    Arm { name: "grain", grain: LOWERED, ..DEFAULT_ARM },
];

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; the slice views its `size_of::<T>()` bytes
    //   read-only for the duration of the borrow, which is the exact layout the component pool
    //   stores.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

fn spawn_box(world: &mut EcsMaster, position: Vec3, half_extents: Vec3, inv_mass: f32, restitution: f32) {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: if inv_mass == 0.0 { Mat3::ZERO } else { Mat3::from_diagonal(Vec3::new(6.0, 6.0, 6.0)) },
        inv_mass,
        restitution,
        friction: 0.5,
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

/// A static floor and a `GRID × GRID` field of two-box stacks, each box overlapping what it rests
/// on by 1 cm so every contact exists from the first frame (the overlap pushes the stacks apart, so
/// the sweeps have work); with `drop > 0` a row of `GRID` boxes beside the field starts that far
/// above its rest and falls onto the floor.
fn spawn_scene(world: &mut EcsMaster, restitution: f32, drop: f32) {
    spawn_box(world, Vec3::new(0.0, -0.5, 0.0), Vec3::new(40.0, 0.5, 40.0), 0.0, restitution);
    let half = Vec3::new(0.5, 0.5, 0.5);
    let origin = -0.75 * (GRID as f32 - 1.0);
    for i in 0..GRID {
        for k in 0..GRID {
            let x = origin + 1.5 * i as f32;
            let z = origin + 1.5 * k as f32;
            spawn_box(world, Vec3::new(x, 0.49, z), half, 1.0, restitution);
            spawn_box(world, Vec3::new(x, 1.48, z), half, 1.0, restitution);
        }
    }
    if drop > 0.0 {
        let x = -origin + 3.0;
        for k in 0..GRID {
            spawn_box(world, Vec3::new(x, 0.5 + drop, origin + 1.5 * k as f32), half, 1.0, restitution);
        }
    }
}

/// FNV-1a over every bit of every `RigidBody`, in query order.
fn state_hash(world: &mut EcsMaster) -> u64 {
    let q = world.query::<&RigidBody, ()>();
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for b in q.iter() {
        let words = [
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
        ];
        for w in words {
            for byte in w.to_bits().to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    hash
}

/// [`state_hash`] with the step's `WarmSeedStats` folded in: the frame's observation.
fn frame_hash(world: &mut EcsMaster) -> u64 {
    let mut hash = state_hash(world);
    let s = world.resource::<DefaultRigidSolver>().warm_seed_stats();
    let words = [
        u64::from(s.manifolds),
        u64::from(s.translated),
        u64::from(s.points),
        u64::from(s.point_hits),
        u64::from(s.carry_points),
        u64::from(s.carry_hits),
        s.remap_resets,
    ];
    for w in words {
        for byte in w.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// The slot count of the widest colour of the last step.
fn widest_color_slots(world: &EcsMaster) -> usize {
    let graph = world.resource::<ConstraintGraph>();
    let manifolds = world.resource::<Manifolds>().solver_manifolds();
    (0..graph.n_colors())
        .map(|c| graph.color(c).iter().map(|&m| usize::from(manifolds[m as usize].count)).sum::<usize>())
        .max()
        .unwrap_or(0)
}

/// The fastest upward linear velocity of any body.
fn max_upward_speed(world: &mut EcsMaster) -> f32 {
    let q = world.query::<&RigidBody, ()>();
    q.iter().map(|b| b.linear_velocity.y).fold(f32::NEG_INFINITY, f32::max)
}

/// What one run produced.
struct Run {
    hashes: Vec<u64>,
    spawn_hash: u64,
    /// The bodies' hash after the last frame (the "scene moved" witness compares it with the spawn).
    last_state: u64,
    min_widest: usize,
    regions: u64,
    last_published: u32,
    /// The fastest upward body velocity over every frame.
    max_up: f32,
}

/// Runs the default world with `arm`'s configuration and the region switch on, on a
/// `workers`-wide pool.
fn run(arm: Arm, workers: usize) -> Run {
    let mut world = EcsMaster::new();
    spawn_scene(&mut world, arm.restitution, arm.drop);
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    let keys = add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut world);
    assert!(keys.build_graph.is_some(), "construction: the default world is the colored solve");
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(DT)));
    let mut schedule = builder.build(&mut world);
    {
        let cfg = world.resource_mut::<PhysicsConfig>();
        cfg.parallel_solve = true;
        cfg.simd_solve = arm.simd_solve;
        cfg.contact_reuse = arm.contact_reuse;
        cfg.sleeping = arm.sleeping;
        if !arm.speculative {
            cfg.speculative_distance = 0.0;
            cfg.speculative_velocity_cap = 0.0;
        }
    }
    {
        let solver = world.resource_mut::<DefaultRigidSolver>();
        solver.set_region(true);
        assert!(solver.set_region_grain(arm.grain), "construction: the arm's grain is valid");
    }
    let spawn_hash = state_hash(&mut world);
    let before = world.resource::<DefaultRigidSolver>().region_dispatches();
    let mut hashes = Vec::with_capacity(FRAMES);
    let mut min_widest = usize::MAX;
    let mut max_up = f32::NEG_INFINITY;
    for _ in 0..FRAMES {
        schedule.run(&mut world);
        hashes.push(frame_hash(&mut world));
        min_widest = min_widest.min(widest_color_slots(&world));
        max_up = max_up.max(max_upward_speed(&mut world));
    }
    let last_state = state_hash(&mut world);
    let solver = world.resource::<DefaultRigidSolver>();
    Run {
        hashes,
        spawn_hash,
        last_state,
        min_widest,
        regions: solver.region_dispatches() - before,
        last_published: solver.last_region_report().published,
        max_up,
    }
}

#[test]
fn region_is_worker_count_invariant_on_every_arm() {
    assert!(
        PhysicsConfig::default().speculative_contacts(),
        "non-vacuity: the default arm is V2's shipped default (speculative contacts on)"
    );
    for arm in ARMS {
        let reference = run(arm, 1);
        assert_eq!(reference.regions, 0, "{}: one worker is the inline path, it opens no region", arm.name);
        assert!(
            reference.min_widest >= 2 * WIDE_FLOOR,
            "{}: non-vacuity: every frame's widest colour clears the default wide floor twice \
             (narrowest {})",
            arm.name,
            reference.min_widest
        );
        assert_ne!(
            reference.last_state, reference.spawn_hash,
            "{}: non-vacuity: the scene moved",
            arm.name
        );
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
        for workers in WORKERS {
            let got = run(arm, workers);
            assert_eq!(
                got.regions, FRAMES as u64,
                "{} at W{workers}: the region opened on every frame (the premise of the gate)",
                arm.name
            );
            if arm.name == "grain" {
                // Per substep at least the gravity and integrate entries and every colour's
                // biased and relax sweeps are cut into blocks under the lowered grain.
                assert!(
                    got.last_published >= 4 * 4,
                    "grain at W{workers}: the lowered grain published {} items, too few for its \
                     body and colour stages",
                    got.last_published
                );
            }
            for (frame, (a, b)) in reference.hashes.iter().zip(&got.hashes).enumerate() {
                assert_eq!(
                    a, b,
                    "{} at W{workers}: frame {frame}'s hash (bodies and warm statistics) differs from W1's",
                    arm.name
                );
            }
        }
    }
}
