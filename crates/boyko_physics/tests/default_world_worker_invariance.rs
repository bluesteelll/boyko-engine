//! G3 — the default physics world is worker-count invariant, bit for bit.
//!
//! The default world (`add_physics_systems::<DefaultRigidSolver>`, default
//! `PhysicsConfig`: the colored solve with the O7 AVX2 cohort kernel) runs a box-stack
//! scene for [`FRAMES`] frames through the real schedule on pools of 1, 2, 4 and 8
//! workers, each with `parallel_solve` off and on and `parallel_narrowphase` off and on,
//! plus one scalar run (`simd_solve = false`, 1 worker, both off). The per-frame FNV-1a
//! hash of every `RigidBody` bit must be identical across all seventeen runs: the pool
//! size may change WHERE a color is solved or a pair is collided, never which kernel,
//! color set, chunk partition or order produces the numbers.
//!
//! The design listed a "serial" pool beside the 1-worker one. In this engine they are
//! the same configuration — `ThreadPoolBuilder` clamps to at least one worker and
//! `Schedule::run` always installs its pool — so the 1-worker pool is the serial arm.
//!
//! # Non-vacuity
//!
//! - Every frame's widest color, summed from `ConstraintGraph::color` and each
//!   manifold's point count, holds at least `2 × MIN_PARALLEL_SLOTS_PER_COLOR` (512)
//!   slots, so `parallel_solve` has a color to dispatch on every frame.
//! - The dispatch witness of `large_island_gate_p2.rs`: the same scene's warmed
//!   parallel step, replayed on the calling thread inside a 4-worker `install` frame,
//!   allocates (a `pool.scope` was dispatched), under a thread-local counting
//!   allocator.
//! - The scene moves: the final hash differs from the spawn state's.
//! - The SIMD arm is compiled in, and `simd_solve` is on in the `PhysicsConfig` resource of
//!   the world `add_physics_systems` built, read the same way as G-L4-2 below, so a plugin-side
//!   override of the flag reds here. Its twin asserts `PhysicsConfig::default()` itself (G1
//!   pins the build configuration and that default).
//! - G-L4-2: `parallel_solve` is on in the `PhysicsConfig` resource of the world
//!   `add_physics_systems` built (L4), read before [`run`] applies its own flags, so the
//!   default world is the parallel arm; reverting the default or overriding the flag in the
//!   plugin reds here, not only in a census. Its twin asserts `PhysicsConfig::default()`
//!   itself, the public default a host inherits when it builds its own config from it.
//! - G-L5-4: `parallel_narrowphase` is on in the same built resource (L5 C4), with the same
//!   twin on `PhysicsConfig::default()`. And the flag reaches the dispatch: on every
//!   flag-on arm of two or more workers `Manifolds::narrowphase_dispatches` rises by one per
//!   frame — the scene has [`MIN_PAIRS`] candidate pairs or more on every frame, two chunks of
//!   `NP_MIN_PAIRS_PER_CHUNK` at any lane count — and on every flag-off arm and on the
//!   one-worker flag-on arm it does not move (the `lanes < 2` term; the twin of G-L4-1 for
//!   the narrowphase is `narrowphase_parallel_equivalence.rs`'s
//!   `one_worker_parallel_narrowphase_runs_the_serial_loop`).
//!
//! # G-L4-1 — one lane never opens a scope
//!
//! [`one_worker_parallel_solve_takes_the_inline_path`]: the same warmed step on a ONE-worker
//! pool with `parallel_solve` on allocates nothing, because the whole-step gate's `lanes >= 2`
//! term sends it down the inline path. With L4's default flip every W=1 default world takes
//! that arm, so without the term each wide color of every pass would pay a `pool.scope` for no
//! parallelism. Its non-vacuity is the 4-worker witness above on the same step: the width term
//! passes there, so a zero at one worker can only come from the lanes term.
//!
//! Spins real thread pools (intractable under Miri), so `cfg(not(miri))`.

#![cfg(not(miri))]
// clippy 1.98.0 false positive: this file's `thread_local!` initialiser already uses
// the `const { … }` form the lint asks for (all of them in the workspace do). See this
// crate's lib.rs for the full account and the delete condition.
#![allow(clippy::missing_const_for_thread_local)]

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::manifold::Manifold;
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::narrowphase::NP_MIN_PAIRS_PER_CHUNK;
use boyko_physics::plugin::add_physics_systems;
use boyko_physics::resources::{
    BodyState, ConstraintGraph, ContactPairs, Manifolds, PhysicsConfig, SolverScratch,
};
use boyko_physics::solver::DefaultRigidSolver;

/// Frames each run steps.
const FRAMES: usize = 60;

/// Fixed timestep.
const DT: f32 = 1.0 / 60.0;

/// Stacks per side of the square grid: 13 × 13 = 169 two-box stacks, 338 dynamic
/// boxes. Every box-floor face contact carries 4 points, so the floor color holds
/// about 676 slots.
const GRID: usize = 13;

/// The solver's per-color dispatch floor (`MIN_PARALLEL_SLOTS_PER_COLOR`, private to
/// `solver/colored.rs`). The scene must clear it twice over on every frame.
const MIN_PARALLEL_SLOTS_PER_COLOR: usize = 256;

/// The fewest candidate pairs any frame may have: two chunks of the narrowphase's per-chunk
/// floor, so `parallel_narrowphase` dispatches on every frame at every lane count of two or
/// more (`chunk_count` is `min(lanes x NP_CHUNKS_PER_LANE, pairs / NP_MIN_PAIRS_PER_CHUNK)`,
/// and two lanes give twelve by the lane term). The scene holds 1,131 on every frame (measured
/// 2026-09-21 with the count printed per run): the 169 stacks' box-floor and box-box pairs plus
/// the AABB overlaps between neighbouring stacks, 4.4x this floor.
const MIN_PAIRS: usize = 2 * NP_MIN_PAIRS_PER_CHUNK;

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; the slice views its
    // `size_of::<T>()` bytes read-only for the duration of the borrow, which is
    // the exact layout the component pool stores.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

fn spawn_box(world: &mut EcsMaster, position: Vec3, half_extents: Vec3, inv_mass: f32) {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        // A unit cube of mass 1 has I = 1/6 on the diagonal.
        inv_inertia: if inv_mass == 0.0 {
            Mat3::ZERO
        } else {
            Mat3::from_diagonal(Vec3::new(6.0, 6.0, 6.0))
        },
        inv_mass,
        restitution: 0.0,
        friction: 0.5,
    };
    let collider = Collider {
        shape: ColliderShape::Box { half_extents },
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
    world.enable::<Simulated>(e);
}

/// A static floor and a `GRID × GRID` field of two-box stacks, each box overlapping
/// what it rests on by 1 cm so every contact exists from the first frame.
fn spawn_scene(world: &mut EcsMaster) {
    spawn_box(world, Vec3::new(0.0, -0.5, 0.0), Vec3::new(40.0, 0.5, 40.0), 0.0);
    let half = Vec3::new(0.5, 0.5, 0.5);
    let origin = -0.75 * (GRID as f32 - 1.0);
    for i in 0..GRID {
        for k in 0..GRID {
            let x = origin + 1.5 * i as f32;
            let z = origin + 1.5 * k as f32;
            spawn_box(world, Vec3::new(x, 0.49, z), half, 1.0);
            spawn_box(world, Vec3::new(x, 1.48, z), half, 1.0);
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

/// The slot count of the widest color of the last step: per color, the sum of its
/// manifolds' point counts.
fn widest_color_slots(world: &EcsMaster) -> usize {
    let graph = world.resource::<ConstraintGraph>();
    let manifolds = world.resource::<Manifolds>().manifolds();
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
    /// State hash after every frame.
    hashes: Vec<u64>,
    /// The narrowest per-frame widest color, in slots.
    min_widest: usize,
    /// The fewest candidate pairs any frame had.
    min_pairs: usize,
    /// How far `Manifolds::narrowphase_dispatches` moved over the [`FRAMES`] frames.
    np_dispatches: u64,
    /// The state hash of the spawn state, before any frame.
    spawn_hash: u64,
    /// The last frame's gathered snapshot and manifolds, for the dispatch witness.
    last: (Vec<BodyState>, Vec<Manifold>),
    /// The `PhysicsConfig` resource exactly as `add_physics_systems` and the schedule
    /// build left it in this world, read before [`run`] applies its own flags.
    built_config: PhysicsConfig,
}

/// Runs the default world on a `workers`-wide pool with the given flags.
fn run(workers: usize, parallel_solve: bool, parallel_narrowphase: bool, simd_solve: bool) -> Run {
    let mut world = EcsMaster::new();
    spawn_scene(&mut world);
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    let keys = add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut world);
    assert!(keys.build_graph.is_some(), "construction: the default world is the colored solve");
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(DT)));
    let mut schedule = builder.build(&mut world);
    let built_config = *world.resource::<PhysicsConfig>();
    {
        let cfg = world.resource_mut::<PhysicsConfig>();
        cfg.parallel_solve = parallel_solve;
        cfg.parallel_narrowphase = parallel_narrowphase;
        cfg.simd_solve = simd_solve;
    }

    let spawn_hash = state_hash(&mut world);
    let np_before = world.resource::<Manifolds>().narrowphase_dispatches();
    let mut hashes = Vec::with_capacity(FRAMES);
    let mut min_widest = usize::MAX;
    let mut min_pairs = usize::MAX;
    for _ in 0..FRAMES {
        schedule.run(&mut world);
        hashes.push(state_hash(&mut world));
        min_widest = min_widest.min(widest_color_slots(&world));
        min_pairs = min_pairs.min(world.resource::<ContactPairs>().pairs().len());
    }
    let np_dispatches = world.resource::<Manifolds>().narrowphase_dispatches() - np_before;
    let last = (
        world.resource::<SolverScratch>().bodies().to_vec(),
        world.resource::<Manifolds>().manifolds().to_vec(),
    );
    Run {
        hashes,
        min_widest,
        min_pairs,
        np_dispatches,
        spawn_hash,
        last,
        built_config,
    }
}

#[test]
// `assertions_on_constants`: the `cfg!` is constant per build, and asserting it is the
// point — it is the build-configuration witness that the SIMD arm under test exists.
#[allow(clippy::assertions_on_constants)]
fn default_world_is_worker_count_invariant() {
    assert!(
        cfg!(all(target_arch = "x86_64", target_feature = "avx2")),
        "non-vacuity: without x86_64 + avx2 the SIMD arm is compiled out"
    );
    // The twin of the SIMD check on the built world below, for the reason given at G-L4-2's
    // twin: a host that builds its config from the default inherits this flag.
    assert!(
        PhysicsConfig::default().simd_solve,
        "non-vacuity (twin): `PhysicsConfig::default()` must carry `simd_solve`; a host that \
         builds its config from the default inherits this value, not the plugin's"
    );
    // The twin of G-L4-2 below. `PhysicsConfig::default()` is public API: a host that tunes
    // the resource by re-inserting `PhysicsConfig { .., ..PhysicsConfig::default() }`
    // inherits this flag, not the plugin's, so a plugin that forced the flag on over a
    // reverted default would pass G-L4-2 and still hand such a host the serial arm.
    assert!(
        PhysicsConfig::default().parallel_solve,
        "G-L4-2 (twin): `PhysicsConfig::default()` must carry `parallel_solve` (L4); a host \
         that builds its config from the default inherits this value, not the plugin's"
    );
    assert!(
        PhysicsConfig::default().parallel_narrowphase,
        "G-L5-4 (twin): `PhysicsConfig::default()` must carry `parallel_narrowphase` (L5 C4); \
         a host that builds its config from the default inherits this value, not the plugin's"
    );

    let reference = run(1, false, false, true);
    // G-L4-2: the config resource of the world the plugin built, which this test steps, so a
    // plugin-side override of the flag reds here, not only a change to the default.
    assert!(
        reference.built_config.parallel_solve,
        "G-L4-2: the world `add_physics_systems` built must dispatch its wide colors (L4: \
         `parallel_solve` on by default); its `PhysicsConfig` resource has it off"
    );
    assert!(
        reference.built_config.parallel_narrowphase,
        "G-L5-4: the world `add_physics_systems` built must dispatch its narrowphase (L5 C4: \
         `parallel_narrowphase` on by default); its `PhysicsConfig` resource has it off"
    );
    assert!(
        reference.min_pairs >= MIN_PAIRS,
        "non-vacuity: the fewest candidate pairs on a frame is {}, under {MIN_PAIRS} — \
         parallel_narrowphase would run the serial loop on some frame",
        reference.min_pairs
    );
    // The SIMD arm, read from the same resource: `run` sets `simd_solve` itself, so only the
    // built config says whether the world the plugin hands a host runs the arm under test.
    assert!(
        reference.built_config.simd_solve,
        "non-vacuity: the world `add_physics_systems` built must run the SIMD arm; its \
         `PhysicsConfig` resource has `simd_solve` off"
    );
    assert!(
        reference.min_widest >= 2 * MIN_PARALLEL_SLOTS_PER_COLOR,
        "non-vacuity: the narrowest per-frame widest color holds {} slots, under 2 × {} — \
         parallel_solve would have nothing to dispatch on some frame",
        reference.min_widest,
        MIN_PARALLEL_SLOTS_PER_COLOR
    );
    assert_ne!(
        reference.hashes.last().copied(),
        Some(reference.spawn_hash),
        "non-vacuity: the scene must move"
    );
    let (bodies, manifolds) = &reference.last;
    let allocs = dispatch::warmed_parallel_step_allocs(bodies, manifolds, 4);
    assert!(
        allocs > 0,
        "non-vacuity: a warmed parallel step on this scene must dispatch a `pool.scope`; it \
         allocated {allocs} times"
    );

    let mut runs = Vec::new();
    for workers in [1usize, 2, 4, 8] {
        for parallel_solve in [false, true] {
            for parallel_narrowphase in [false, true] {
                let label = format!(
                    "{workers}w parallel_solve={parallel_solve} \
                     parallel_narrowphase={parallel_narrowphase}"
                );
                // The narrowphase dispatches on every frame of a flag-on arm of two or
                // more workers, and on no frame otherwise (the `lanes < 2` term).
                let dispatches = parallel_narrowphase && workers >= 2;
                let r = run(workers, parallel_solve, parallel_narrowphase, true);
                runs.push((label, dispatches, r));
            }
        }
    }
    runs.push(("1w scalar (simd_solve=false)".to_owned(), false, run(1, false, false, false)));

    // G-L5-4's dispatch-counter non-vacuity: the flag reaches the dispatch on every frame of
    // every arm that can dispatch, and nowhere else. Checked before the hash comparison, so
    // an arm that silently ran the serial loop cannot pass as "invariant".
    for (label, dispatches, r) in &runs {
        let expected = if *dispatches { FRAMES as u64 } else { 0 };
        assert_eq!(
            r.np_dispatches, expected,
            "G-L5-4: {label}: `narrowphase_dispatches` moved {} over {FRAMES} frames, expected \
             {expected} (one per frame on a flag-on arm of two or more workers, none on a \
             flag-off arm or at one worker)",
            r.np_dispatches
        );
    }

    let mut diverged = Vec::new();
    for (label, _, r) in &runs {
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
        "the default world is not worker-count invariant against 1w parallel_solve=false \
         parallel_narrowphase=false (SIMD):\n  {}",
        diverged.join("\n  ")
    );
}

/// G-L4-1 (module docs): with `parallel_solve` on, a one-worker pool takes the inline path
/// and allocates nothing, while the same warmed step on four workers dispatches.
#[test]
fn one_worker_parallel_solve_takes_the_inline_path() {
    let reference = run(1, false, false, true);
    assert!(
        reference.min_widest >= 2 * MIN_PARALLEL_SLOTS_PER_COLOR,
        "non-vacuity: the narrowest per-frame widest color holds {} slots, under 2 × {} — the \
         width term, not the lanes term, would decide the inline path",
        reference.min_widest,
        MIN_PARALLEL_SLOTS_PER_COLOR
    );
    let (bodies, manifolds) = &reference.last;
    let at_four = dispatch::warmed_parallel_step_allocs(bodies, manifolds, 4);
    assert!(
        at_four > 0,
        "non-vacuity: the same warmed step on 4 workers must dispatch a `pool.scope`; it \
         allocated {at_four} times"
    );
    let at_one = dispatch::warmed_parallel_step_allocs(bodies, manifolds, 1);
    assert_eq!(
        at_one, 0,
        "G-L4-1: a warmed colored step on a 1-worker pool with `parallel_solve` on allocated \
         {at_one} times; one lane must take the inline path (the whole-step gate's `lanes >= 2` \
         term), never a `pool.scope`"
    );
}

/// The dispatch witness of `large_island_gate_p2.rs`, replayed on this scene: one
/// warmed colored step with `parallel_solve` on, on the calling thread inside a pool
/// `install` frame, under a thread-local counting allocator. A `pool.scope` dispatch
/// allocates; the inline path does not.
mod dispatch {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    use boyko_threadpool::ThreadPoolBuilder;

    use boyko_physics::manifold::Manifold;
    use boyko_physics::resources::{BodyState, ConstraintGraph, PhysicsConfig, SolverScratch};
    use boyko_physics::solver::ColoredSoftStepSolver;

    use super::DT;

    pub(super) fn warmed_parallel_step_allocs(
        bodies: &[BodyState],
        manifolds: &[Manifold],
        workers: usize,
    ) -> usize {
        let cfg = PhysicsConfig {
            dt: DT,
            parallel_solve: true,
            ..PhysicsConfig::default()
        };
        let inv_mass: Vec<f32> = bodies.iter().map(|b| b.inv_mass).collect();
        let mut graph = ConstraintGraph::with_capacity(bodies.len());
        graph.build(manifolds, bodies.len(), |row| {
            (row as usize) < inv_mass.len() && inv_mass[row as usize] != 0.0
        });
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(bodies.len());
        scratch.set_bodies(bodies);

        let pool = ThreadPoolBuilder::new().num_threads(workers).build();
        pool.install(|_scope| {
            // Warm so every solver and scratch buffer reaches steady capacity.
            for _ in 0..8 {
                scratch.touched.reset(scratch.bodies().len());
                solver.solve_colored(&cfg, manifolds, &graph, &mut scratch);
            }
            scratch.touched.reset(scratch.bodies().len());
            let before = ALLOC.count();
            solver.solve_colored(&cfg, manifolds, &graph, &mut scratch);
            ALLOC.count().wrapping_sub(before)
        })
    }

    thread_local! {
        static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
    }

    pub(super) struct CountingAlloc;

    impl CountingAlloc {
        fn count(&self) -> usize {
            ALLOC_COUNT.with(|c| c.get())
        }
    }

    #[inline]
    fn bump_alloc_count() {
        let _ = ALLOC_COUNT.try_with(|c| c.set(c.get() + 1));
    }

    // SAFETY: every call forwards verbatim to the platform `System` allocator with the
    // same layout; the wrapper only bumps a thread-local counter (via `try_with`, which
    // no-ops if TLS is mid-init, so it never re-enters the allocator). `dealloc` is an
    // unchanged pass-through, so the allocator contract is exactly `System`'s.
    unsafe impl GlobalAlloc for CountingAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            bump_alloc_count();
            // SAFETY: forwarded verbatim to the system allocator (same layout).
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: `ptr`/`layout` originate from `System.alloc` above.
            unsafe { System.dealloc(ptr, layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            bump_alloc_count();
            // SAFETY: `ptr`/`layout` originate from this allocator; `new_size` forwarded.
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }

    #[global_allocator]
    static ALLOC: CountingAlloc = CountingAlloc;
}
