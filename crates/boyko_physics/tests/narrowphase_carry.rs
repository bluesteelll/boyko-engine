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
//! 1520 B align 8 — data-dependent and outside the narrowphase, which holds no heap container. The
//! run is deterministic (one worker, a fixed scene and despawn order), so the counts repeat.
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
//! one acquisition more.
//!
//! One test in this binary, so the process-wide counter sees no other test's traffic.

#![cfg(not(miri))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
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

/// The counting allocator: `System`, plus one relaxed increment per acquisition.
struct Counting;

// SAFETY: every call forwards verbatim to the platform `System` allocator with the same layout
// (and pointer, for `dealloc` / `realloc`); the wrapper only bumps an atomic counter, which never
// allocates, so the allocator contract is exactly `System`'s.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // Relaxed: a counter read after the measured step's join, which orders every worker's
        // increment before the read.
        ACQUISITIONS.fetch_add(1, Ordering::Relaxed);
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
fn counted_step(world: &mut EcsMaster, physics: &mut Schedule) -> (u64, usize, u64) {
    let before = ACQUISITIONS.load(Ordering::Relaxed);
    physics.run(world);
    let acquisitions = ACQUISITIONS.load(Ordering::Relaxed) - before;
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
        let (unmoved, unmoved_hits, unmoved_reused) = counted_step(&mut world, &mut physics);
        let builds = world.resource::<Manifolds>().pair_carry_jumper_builds();
        // A body in the middle of the spawn order: the tail body swap-moves into its row.
        let doomed = bodies.remove(bodies.len() / 2 - cycle);
        assert!(
            world.delete_entity(doomed),
            "construction: a live body is despawnable"
        );
        let (moved, moved_hits, moved_reused) = counted_step(&mut world, &mut physics);
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
