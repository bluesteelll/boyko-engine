//! G-C-3 (L9 C2, `docs/physics/perf-campaign/levers/L9-contact-reuse/02-DESIGN-REV1.md`): the
//! narrowphase's pair carry adds no heap allocation to a step — in particular not to a step whose
//! rows moved, where the carry translates its join through the row identity and rebuilds its
//! jumper bitset. The steady-state frame census (`alloc_frame_census.rs`, S1b / S1c) runs a pile
//! whose rows never move, so this is the arm that reaches the jumper build.
//!
//! A box pile on the default pipeline, one worker, warmed. Each cycle counts the heap acquisitions
//! (allocations and reallocations, every thread) of one step whose rows are unchanged, despawns
//! one body from the middle of the pile — the archetype's tail body swap-moves into its row, a row
//! stage 2 resolves, so its pairs join by the jumper search — and counts the next step, whose rows
//! moved. No moved step may make more acquisitions than the fewest any unmoved step made.
//!
//! The bound is the fewest, not the same cycle's, because the unmoved steps are not all equal.
//! Measured on this scene (2026-09-23, msvc): every step makes two acquisitions in release (256 B
//! align 128 and 4096 B align 64; three in debug), and two of the six unmoved steps make one more,
//! 1520 B align 8 — data-dependent and outside the narrowphase, which holds no heap container: the
//! `Injector` block crossbeam-deque allocates once per 63 pushes, periodic in the push count.
//!
//! The data path is deterministic (one worker, a fixed scene and despawn order), and since ci-fix-1
//! so are the step's acquisitions. Before it, a thread's FIRST crossbeam-epoch pin landed in
//! whichever step it happened to come in: master's push CI (run 38182709148) and 1 in 33 local runs
//! counted a moved step at 6 against a fewest of 3, the +3 being the epoch `Global`, queue sentinel
//! and the driver's `Local`. `ThreadPoolBuilder::build` now registers its builder and every worker
//! before it returns, outside every counted step, and [`EPOCH_SIZED`] names any epoch-sized
//! acquisition a counted step still makes, before the comparison below can misread it as the
//! carry.
//!
//! Witnesses, so the equality cannot hold vacuously: the moved step built the jumper bitset
//! (`Manifolds::pair_carry_jumper_builds` rose by one), the carry was never Reset, and carried
//! separating axes rejected pairs on both steps (`Manifolds::separated_axis_hits`).
//!
//! The census runs twice, on two worlds, each setting the flag: contact reuse off, and on (L9
//! C3), where the pairs also read, copy and rebuild their reuse records — through a row move, a
//! flipped join and a jumper search among them. The reuse-on arm's witness: pairs reused their
//! records on both steps of every cycle.
//!
//! RED-first (C2's log): a `Vec::with_capacity` in the jumper build makes every moved step count
//! one acquisition more. The epoch guard's (ci-fix-1 C4): a 1-worker pool built inside a counted
//! step reds with the guard's message, naming the new worker's `Local`, before any comparison.
//!
//! One test in this binary, so the process-wide counter sees no other test's traffic.

#![cfg(not(miri))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyMass, Simulated};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_systems;
use boyko_physics::resources::{Manifolds, PhysicsConfig};
use boyko_physics::solver::DefaultRigidSolver;

/// Heap acquisitions (`alloc`, `alloc_zeroed` through `alloc`, `realloc`) on every thread.
static ACQUISITIONS: AtomicU64 = AtomicU64::new(0);

/// The crossbeam-epoch object sizes (crossbeam-epoch 0.9.21, x86_64): the collector's `Global`, the
/// global queue's sentinel node (or a later sealed-bag node), and a thread's `Local`.
///
/// 640/128 is not crossbeam's alone (`PoolInner::workers` at W = 4, `PoolInner::injector_local` at
/// W = 2): this test's pool has one worker and no counted step builds a pool, so here it names the
/// collector.
const EPOCH_SIGNATURES: [(usize, usize, &str); 3] = [
    (640, 128, "Global 640/128"),
    (2072, 8, "queue sentinel or sealed-bag node 2072/8"),
    (2304, 128, "Local 2304/128"),
];

/// Acquisitions of an [`EPOCH_SIGNATURES`] size made while [`COUNTING`] was set, every thread.
static EPOCH_SIZED: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

/// Set while a counted step runs.
static COUNTING: AtomicBool = AtomicBool::new(false);

/// [`EPOCH_SIZED`]'s share of one acquisition.
#[inline]
fn note_epoch_sized(size: usize, align: usize) {
    if COUNTING.load(Ordering::Relaxed)
        && let Some(i) = EPOCH_SIGNATURES.iter().position(|&(s, a, _)| (s, a) == (size, align))
    {
        // Relaxed: read after the step's join, as `ACQUISITIONS` is.
        EPOCH_SIZED[i].fetch_add(1, Ordering::Relaxed);
    }
}

/// The counting allocator: `System`, plus one relaxed increment per acquisition.
struct Counting;

// SAFETY: every call forwards verbatim to the platform `System` allocator with the same layout
// (and pointer, for `dealloc` / `realloc`); the wrapper only reads and bumps atomic counters, which
// never allocate, so the allocator contract is exactly `System`'s.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // Relaxed: a counter read after the measured step's join, which orders every worker's
        // increment before the read.
        ACQUISITIONS.fetch_add(1, Ordering::Relaxed);
        note_epoch_sized(layout.size(), layout.align());
        // SAFETY: forwarded verbatim to the system allocator (same layout).
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` / `layout` came from `System.alloc` or `System.realloc` above.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // Relaxed: as in `alloc`.
        ACQUISITIONS.fetch_add(1, Ordering::Relaxed);
        note_epoch_sized(new_size, layout.align());
        // SAFETY: `ptr` / `layout` came from this allocator; `new_size` forwarded verbatim.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Fixed timestep.
const DT: f32 = 1.0 / 60.0;
/// Columns of the pile along x and z.
const COLUMNS: usize = 6;
/// Layers of the pile.
const LAYERS: usize = 3;
/// Horizontal pitch: unit boxes 0.2 apart, whose bounding spheres overlap, so every lateral
/// neighbour is a candidate pair the SAT separates on a face axis.
const PITCH: f32 = 1.2;
/// Steps before the first counted step, so every column and table has grown.
const WARM_STEPS: usize = 60;
/// Counted cycles (one unmoved step, one despawn, one moved step each).
const CYCLES: usize = 6;

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; the slice views its `size_of::<T>()` bytes
    // read-only for the duration of the borrow, which is the exact layout the component pool
    // stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// The pile: a static floor, then `LAYERS` layers of `COLUMNS × COLUMNS` unit boxes, stacked.
fn spawn_pile(world: &mut EcsMaster) -> Vec<Entity> {
    let archetype = world.create_archetype(&[
        RigidBody::component_id(),
        RigidBodyMass::component_id(),
        Collider::component_id(),
    ]);
    let spawn = |world: &mut EcsMaster, position: Vec3, half: Vec3, dynamic: bool| {
        let body = RigidBody {
            position,
            linear_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
        };
        let mass = RigidBodyMass {
            inv_inertia: if dynamic {
                Mat3::from_diagonal(Vec3::new(6.0, 6.0, 6.0))
            } else {
                Mat3::ZERO
            },
            inv_mass: if dynamic { 1.0 } else { 0.0 },
            restitution: 0.0,
            friction: 0.5,
        };
        let collider = Collider {
            shape: ColliderShape::Box { half_extents: half },
            layer: 1,
            mask: 1,
        };
        let e = world
            .create_entity(
                archetype,
                &[
                    (RigidBody::component_id(), as_bytes(&body)),
                    (RigidBodyMass::component_id(), as_bytes(&mass)),
                    (Collider::component_id(), as_bytes(&collider)),
                ],
            )
            .expect("construction: the body archetype accepts its columns");
        if dynamic {
            world.enable::<Simulated>(e);
        }
        e
    };
    spawn(
        world,
        Vec3::new(0.0, -0.5, 0.0),
        Vec3::new(20.0, 0.5, 20.0),
        false,
    );
    let origin = -0.5 * PITCH * (COLUMNS as f32 - 1.0);
    let mut bodies = Vec::with_capacity(COLUMNS * COLUMNS * LAYERS);
    for y in 0..LAYERS {
        for z in 0..COLUMNS {
            for x in 0..COLUMNS {
                let p = Vec3::new(
                    origin + PITCH * x as f32,
                    0.5 + y as f32,
                    origin + PITCH * z as f32,
                );
                bodies.push(spawn(world, p, Vec3::new(0.5, 0.5, 0.5), true));
            }
        }
    }
    bodies
}

/// Heap acquisitions of one step, the step's carried-axis rejections and its reused pairs.
///
/// RED, naming the class, when the step made an acquisition of an [`EPOCH_SIGNATURES`] size: that
/// is a thread's first epoch touch, not the carry, and reading it as the carry is the misreading
/// this guard exists to prevent.
fn counted_step(
    world: &mut EcsMaster,
    physics: &mut Schedule,
    (reuse, cycle, moved): (bool, usize, bool),
) -> (u64, usize, u64) {
    let epoch_before: [u64; 3] = core::array::from_fn(|i| EPOCH_SIZED[i].load(Ordering::Relaxed));
    let before = ACQUISITIONS.load(Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    physics.run(world);
    COUNTING.store(false, Ordering::Relaxed);
    let acquisitions = ACQUISITIONS.load(Ordering::Relaxed) - before;
    let epoch: [u64; 3] =
        core::array::from_fn(|i| EPOCH_SIZED[i].load(Ordering::Relaxed) - epoch_before[i]);
    let n: u64 = epoch.iter().sum();
    if n > 0 {
        let list: Vec<String> = EPOCH_SIGNATURES
            .iter()
            .zip(epoch)
            .filter(|&(_, k)| k > 0)
            .map(|(&(_, _, name), k)| format!("{k} x {name}"))
            .collect();
        panic!(
            "contact_reuse {reuse}, cycle {cycle}, {} step: {n} acquisition(s) of a crossbeam-epoch \
             size ({}) inside the counted step: a thread's first epoch touch (Collector 640/128, \
             queue sentinel or sealed-bag node 2072/8, Local 2304/128), not the carry. The pool \
             registers its participants at build, so this is a thread the pool did not build or a \
             sealed-bag push.",
            if moved { "moved" } else { "unmoved" },
            list.join(", ")
        );
    }
    let m = world.resource::<Manifolds>();
    (acquisitions, m.separated_axis_hits(), m.pair_classes().reused)
}

#[test]
fn a_step_whose_rows_moved_allocates_what_an_unmoved_step_does() {
    census(false);
    census(true);
}

/// One census arm, contact reuse set to `reuse` (module docs).
fn census(reuse: bool) {
    let mut world = EcsMaster::new();
    let mut bodies = spawn_pile(&mut world);
    let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(1).build());
    add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut world);
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    let mut physics = builder.build(&mut world);
    world.resource_mut::<PhysicsConfig>().contact_reuse = reuse;
    for _ in 0..WARM_STEPS {
        physics.run(&mut world);
    }

    let mut rows = Vec::with_capacity(CYCLES);
    for cycle in 0..CYCLES {
        let (unmoved, unmoved_hits, unmoved_reused) =
            counted_step(&mut world, &mut physics, (reuse, cycle, false));
        let builds = world.resource::<Manifolds>().pair_carry_jumper_builds();
        // A body in the middle of the spawn order: the tail body swap-moves into its row.
        let doomed = bodies.remove(bodies.len() / 2 - cycle);
        assert!(
            world.delete_entity(doomed),
            "construction: a live body is despawnable"
        );
        let (moved, moved_hits, moved_reused) =
            counted_step(&mut world, &mut physics, (reuse, cycle, true));
        let m = world.resource::<Manifolds>();
        rows.push((cycle, unmoved, moved, unmoved_hits, moved_hits));
        assert_eq!(
            (unmoved_reused > 0, moved_reused > 0),
            (reuse, reuse),
            "contact_reuse {reuse}, cycle {cycle}: reused pairs ({unmoved_reused}, {moved_reused})"
        );
        assert_eq!(
            m.pair_carry_jumper_builds(),
            builds + 1,
            "cycle {cycle}: the moved step must build the jumper bitset (the witness that it \
             reached the jumper search)"
        );
        assert!(
            unmoved_hits > 0 && moved_hits > 0,
            "cycle {cycle}: carried separating axes must reject pairs on both steps \
             ({unmoved_hits}, {moved_hits})"
        );
    }
    let resets = world.resource::<Manifolds>().pair_carry_resets();
    println!(
        "G-C-3 contact_reuse {reuse} (cycle, unmoved acquisitions, moved acquisitions, unmoved hits, moved hits): {rows:?}; carry resets {resets}"
    );
    assert_eq!(resets, 0, "the pair carry lost its place");
    let fewest = rows
        .iter()
        .map(|&(_, unmoved, _, _, _)| unmoved)
        .min()
        .expect("CYCLES > 0");
    for &(cycle, _, moved, _, _) in &rows {
        assert!(
            moved <= fewest,
            "contact_reuse {reuse}, cycle {cycle}: the step whose rows moved made {moved} heap \
             acquisitions, the fewest of any unmoved step {fewest}: the carry's translated join, \
             jumper build or record copy allocates"
        );
    }
}
