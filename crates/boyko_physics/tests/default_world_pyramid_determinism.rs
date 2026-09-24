//! The default physics world is deterministic on the Jolt-parity pyramid: run to run,
//! across worker counts, and against the scalar colored oracle.
//!
//! # What it asserts
//!
//! The default world (`add_physics_systems::<DefaultRigidSolver>`, default
//! `PhysicsConfig`: the colored solve with the O7 AVX2 cohort kernel on, `AllPairs`
//! broadphase, sleeping off) runs Jolt's `PyramidScene` through the real schedule. The
//! per-frame FNV-1a hash of every `RigidBody` bit must be identical across:
//!
//! - **run to run**: the reference configuration (1 worker, `parallel_solve` off) run
//!   twice in fresh worlds, and the most parallel configuration (8 workers,
//!   `parallel_solve` on) run twice;
//! - **worker count**: pools of 1, 2 and 8 workers, each with `parallel_solve` off and
//!   on — the pool may change WHERE a color is solved, never the bits;
//! - **the scalar colored oracle**: `simd_solve = false` at 1 worker and at 8 workers
//!   with `parallel_solve` on. `simd_solve` is a speed path over `solve_color`, so a
//!   difference here is a kernel defect.
//!
//! The trajectory is also pinned across processes and commits: the reference run's final
//! hash must equal [`PINNED_FINAL_HASH`], one value per profile (the scene differs by
//! profile, below). This makes "nothing moved" mechanical. A commit that claims bit
//! identity and moves the default world's pyramid by one bit turns this red even when
//! every arm still agrees with every other. Until 2026-09-23 the hash was only printed,
//! and a scratch mutation that turned contact reuse on by default moved it (release
//! `0xa38620b38cbca8d3` -> `0xb583189fa681f3a6`) while the test stayed green. The pin's
//! doc carries its re-pin rule.
//!
//! # Scene size per profile
//!
//! Release runs Jolt's full pyramid (height 15, 1240 dynamic boxes) for 120 frames —
//! the collapse of every 0.5 m layer gap onto the layer below, then settling. Debug runs
//! height 10 (385 boxes) for 60 frames so the file stays in the ordinary debug run: the
//! default `AllPairs` broadphase is O(n²), and an unoptimised 1240-box world is minutes.
//!
//! # Non-vacuity
//!
//! - The build has x86_64 + AVX2 and the default config has `simd_solve` on, so the
//!   AVX2 arm is the one the reference runs (G1 in `default_world_colored_simd.rs` pins
//!   the same two inputs of the dispatch fork).
//! - The scene moves: the final hash differs from the spawn state's.
//! - Contacts exist on the last frame.
//! - At least one frame's widest color (per color, the sum of its manifolds' point
//!   counts) reaches the colored solver's per-color dispatch floor
//!   (`MIN_PARALLEL_SLOTS_PER_COLOR`, 256), so the `parallel_solve` arms have a color
//!   to dispatch. The count of such frames is printed.
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
use boyko_physics::resources::{ConstraintGraph, Manifolds, PhysicsConfig};
use boyko_physics::solver::DefaultRigidSolver;

/// Jolt's `cBoxSize`: the pitch between neighbouring boxes in a layer.
const BOX_SIZE: f32 = 2.0;
/// Jolt's `cBoxSeparation`: the vertical gap added on top of `BOX_SIZE` per layer.
const BOX_SEPARATION: f32 = 0.5;
/// Jolt's `cHalfBoxSize`.
const HALF_BOX: f32 = 0.5 * BOX_SIZE;
/// Jolt's floor half-extents.
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(50.0, 1.0, 50.0);

/// Pyramid layers: Jolt's 15 (1240 boxes) in release, 10 (385 boxes) in debug.
const PYRAMID_HEIGHT: i32 = if cfg!(debug_assertions) { 10 } else { 15 };

/// Frames each run steps.
const FRAMES: usize = if cfg!(debug_assertions) { 60 } else { 120 };

/// Fixed timestep (Jolt's `cDeltaTime`).
const DT: f32 = 1.0 / 60.0;

/// The reference run's final hash, per profile: release (height 15, 120 frames)
/// `0xa386_20b3_8cbc_a8d3`, debug (height 10, 60 frames) `0xc7eb_531b_1e1a_c19b`. Both were
/// read on msvc at L9 C3 (`aef7dda4`), where contact reuse is off by default.
///
/// **Re-pin rule.** The value moves only with a commit that changes values by design (a
/// value-changing lever). That commit re-reads BOTH profiles from this test's own
/// `reference final hash` line, re-pins both here in the same commit, and names the lever
/// and the old values in this doc. In a commit that claims bit identity, a change is a
/// defect, never a re-pin. L9 C4 (contact reuse on by default) is such a lever and
/// re-pins it.
const PINNED_FINAL_HASH: u64 = if cfg!(debug_assertions) {
    0xc7eb_531b_1e1a_c19b
} else {
    0xa386_20b3_8cbc_a8d3
};

/// The colored solver's per-color dispatch floor (`MIN_PARALLEL_SLOTS_PER_COLOR`,
/// private to `solver/colored.rs`), mirrored for the non-vacuity witness only.
const MIN_PARALLEL_SLOTS_PER_COLOR: usize = 256;

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; the slice views its
    // `size_of::<T>()` bytes read-only for the duration of the borrow, which is
    // the exact layout the component pool stores.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

fn spawn_body(world: &mut EcsMaster, position: Vec3, mass: RigidBodyMass, half: Vec3, dynamic: bool) {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let collider = Collider {
        shape: ColliderShape::Box { half_extents: half },
        layer: 1,
        mask: 1,
    };
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
    if dynamic {
        world.enable::<Simulated>(e);
    }
}

/// Jolt's pyramid, index for index (the placement loop of
/// `benches/jolt_parity_pyramid.rs`), on a static floor. Returns the dynamic count.
fn spawn_pyramid(world: &mut EcsMaster) -> usize {
    spawn_body(
        world,
        Vec3::new(0.0, -1.0, 0.0),
        RigidBodyMass {
            inv_inertia: Mat3::ZERO,
            inv_mass: 0.0,
            restitution: 0.0,
            friction: 0.5,
        },
        FLOOR_HALF_EXTENTS,
        false,
    );
    let mut n = 0;
    for i in 0..PYRAMID_HEIGHT {
        let lo = i / 2;
        let hi = PYRAMID_HEIGHT - (i + 1) / 2;
        for j in lo..hi {
            for k in lo..hi {
                let odd = if i & 1 != 0 { HALF_BOX } else { 0.0 };
                let position = Vec3::new(
                    -(PYRAMID_HEIGHT as f32) + BOX_SIZE * j as f32 + odd,
                    1.0 + (BOX_SIZE + BOX_SEPARATION) * i as f32,
                    -(PYRAMID_HEIGHT as f32) + BOX_SIZE * k as f32 + odd,
                );
                spawn_body(
                    world,
                    position,
                    RigidBodyMass {
                        // Unit-density cube of half-extent 1: m = 8, I = 8/12 * 8 per axis.
                        inv_inertia: Mat3::from_diagonal(Vec3::new(0.1875, 0.1875, 0.1875)),
                        inv_mass: 0.125,
                        restitution: 0.0,
                        friction: 0.5,
                    },
                    Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX),
                    true,
                );
                n += 1;
            }
        }
    }
    n
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

/// The widest color of the last step, in slots (per color, its manifolds' point counts).
fn widest_color_slots(world: &EcsMaster) -> usize {
    let graph = world.resource::<ConstraintGraph>();
    let manifolds = world.resource::<Manifolds>().solver_manifolds();
    (0..graph.n_colors())
        .map(|c| {
            graph
                .color(c)
                .iter()
                .map(|&m| usize::from(manifolds[m as usize].count))
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0)
}

/// What one run produced.
struct Run {
    hashes: Vec<u64>,
    spawn_hash: u64,
    /// Frames whose widest color reached the per-color dispatch floor.
    dispatchable_frames: usize,
    /// The widest color seen over the run, in slots.
    max_widest: usize,
    /// Manifolds on the last frame.
    last_manifolds: usize,
}

/// Runs the default world on a `workers`-wide pool with the given flags.
fn run(workers: usize, parallel_solve: bool, simd_solve: bool) -> Run {
    let mut world = EcsMaster::new();
    let n = spawn_pyramid(&mut world);
    assert!(n > 0, "construction: the pyramid has dynamic boxes");
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    let keys = add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut world);
    assert!(keys.build_graph.is_some(), "construction: the default world is the colored solve");
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(DT)));
    let mut schedule = builder.build(&mut world);
    {
        let cfg = world.resource_mut::<PhysicsConfig>();
        cfg.dt = DT;
        cfg.parallel_solve = parallel_solve;
        cfg.simd_solve = simd_solve;
    }

    let spawn_hash = state_hash(&mut world);
    let mut hashes = Vec::with_capacity(FRAMES);
    let mut dispatchable_frames = 0;
    let mut max_widest = 0;
    for _ in 0..FRAMES {
        schedule.run(&mut world);
        hashes.push(state_hash(&mut world));
        let widest = widest_color_slots(&world);
        max_widest = max_widest.max(widest);
        if widest >= MIN_PARALLEL_SLOTS_PER_COLOR {
            dispatchable_frames += 1;
        }
    }
    let last_manifolds = world.resource::<Manifolds>().manifolds().len();
    Run {
        hashes,
        spawn_hash,
        dispatchable_frames,
        max_widest,
        last_manifolds,
    }
}

#[test]
// `assertions_on_constants`: the `cfg!` is constant per build, and asserting it is the
// point — it is the build-configuration witness that the SIMD arm under test exists.
#[allow(clippy::assertions_on_constants)]
fn default_world_pyramid_is_run_to_run_worker_count_and_scalar_identical() {
    assert!(
        cfg!(all(target_arch = "x86_64", target_feature = "avx2")),
        "non-vacuity: without x86_64 + avx2 the SIMD arm is compiled out"
    );
    assert!(
        PhysicsConfig::default().simd_solve,
        "non-vacuity: the default world must run the SIMD arm"
    );

    let reference = run(1, false, true);
    println!(
        "pyramid height {PYRAMID_HEIGHT}, {FRAMES} frames: reference final hash {:#018x}, \
         widest color {} slots, {} of {FRAMES} frames dispatchable, {} manifolds on the last frame",
        reference.hashes.last().copied().unwrap_or(0),
        reference.max_widest,
        reference.dispatchable_frames,
        reference.last_manifolds
    );
    assert_ne!(
        reference.hashes.last().copied(),
        Some(reference.spawn_hash),
        "non-vacuity: the pyramid must move"
    );
    assert!(reference.last_manifolds > 0, "non-vacuity: contacts exist on the last frame");
    assert!(
        reference.dispatchable_frames > 0,
        "non-vacuity: no frame's widest color reached {MIN_PARALLEL_SLOTS_PER_COLOR} slots (max \
         {}), so parallel_solve never had a color to dispatch",
        reference.max_widest
    );

    let mut arms: Vec<(String, Run)> = Vec::new();
    arms.push(("run-to-run: 1w parallel_solve=false (repeat)".to_owned(), run(1, false, true)));
    for workers in [1usize, 2, 8] {
        arms.push((format!("{workers}w parallel_solve=true"), run(workers, true, true)));
        if workers != 1 {
            arms.push((format!("{workers}w parallel_solve=false"), run(workers, false, true)));
        }
    }
    arms.push(("run-to-run: 8w parallel_solve=true (repeat)".to_owned(), run(8, true, true)));
    arms.push(("scalar oracle: 1w simd_solve=false".to_owned(), run(1, false, false)));
    arms.push((
        "scalar oracle: 8w parallel_solve=true simd_solve=false".to_owned(),
        run(8, true, false),
    ));

    let mut diverged = Vec::new();
    for (label, r) in &arms {
        println!("  {label}: final hash {:#018x}", r.hashes.last().copied().unwrap_or(0));
        if r.hashes != reference.hashes {
            let frame = r
                .hashes
                .iter()
                .zip(&reference.hashes)
                .position(|(a, b)| a != b)
                .map_or(FRAMES, |i| i + 1);
            diverged.push(format!("{label}: first differs after frame {frame}"));
        }
    }
    assert!(
        diverged.is_empty(),
        "the default world diverged from 1w parallel_solve=false (SIMD) on the pyramid:\n  {}",
        diverged.join("\n  ")
    );
    let reference_final = reference.hashes.last().copied().unwrap_or(0);
    assert_eq!(
        reference_final, PINNED_FINAL_HASH,
        "the default world's pyramid trajectory moved: reference final hash {reference_final:#018x}, \
         pinned {PINNED_FINAL_HASH:#018x} (height {PYRAMID_HEIGHT}, {FRAMES} frames). Every arm \
         still agrees with the reference, so this is a value change, not a determinism defect. In \
         a commit that claims bit identity it is a defect; only a value-changing lever re-pins, \
         under `PINNED_FINAL_HASH`'s rule"
    );
}
