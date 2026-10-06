//! S5 — the parallel tree query (`docs/physics/perf-campaign/levers/scaling/01-DESIGN.md` §6.5):
//! the world-level gates.
//!
//! * [`s5_switch_mirrors_into_the_tree`]: `PhysicsConfig::parallel_tree_query` reaches the tree
//!   broadphase on every Tree step, both ways, through the real schedule.
//! * [`s5_default_world_is_worker_count_invariant`]: S5's {1, N} oracle — G3's box stacks (338
//!   dynamic boxes, 43 active leaf nodes) through the real schedule with the switch on are bit
//!   identical, frame by frame, at W 1/2/3/4/5/8/16 and to the switch-off W1 run, and the query
//!   dispatched on every frame after the first at W ≥ 2 and on none at W1.
//! * [`s5_tail_stays_a_rounding_error`]: the same stacks with the switch on at W 2/4/8/16 — the
//!   serial tail answers at most 1 % of the leaf nodes the dispatched frames queried, so the
//!   per-row budget covers the density the chunks meet (a degenerate budget is value-neutral and
//!   invisible to every pose gate).
//! * [`s5_one_worker_allocates_nothing`]: the W1 path's census proof (the G-L4-1 twin) — a warmed
//!   tree step with the switch on allocates nothing and dispatches nothing on a one-worker pool,
//!   while the same step on four workers dispatches and allocates its scope.
//!
//! The unit gates (the partition, the regions, the tail, the fallback, the history, the one-lane
//! identity, the helper panic and the Miri subset) are `broadphase_tree::tests::s5_*`.
//!
//! Spins real thread pools (intractable under Miri), so `cfg(not(miri))`.

#![cfg(not(miri))]

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::broadphase_tree::{BroadphaseTree, S5_MIN_LEAVES};
use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_systems;
use boyko_physics::resources::{BodyState, BroadphaseKind, ContactPairs, PhysicsConfig};
use boyko_physics::solver::DefaultRigidSolver;

/// Fixed timestep.
const DT: f32 = 1.0 / 60.0;

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; the slice views its `size_of::<T>()` bytes
    // read-only for the duration of the borrow, which is the exact layout the component pool
    // stores.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Spawns one box: dynamic with unit mass when `inv_mass` is 1, static when 0.
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

/// A static floor and `grid × grid` two-box stacks (G3's scene at `grid = 13`), each box
/// overlapping what it rests on by 1 cm so every contact exists from the first frame.
fn spawn_stacks(world: &mut EcsMaster, grid: usize) {
    spawn_box(world, Vec3::new(0.0, -0.5, 0.0), Vec3::new(40.0, 0.5, 40.0), 0.0);
    let half = Vec3::new(0.5, 0.5, 0.5);
    let origin = -0.75 * (grid as f32 - 1.0);
    for i in 0..grid {
        for k in 0..grid {
            let x = origin + 1.5 * i as f32;
            let z = origin + 1.5 * k as f32;
            spawn_box(world, Vec3::new(x, 0.49, z), half, 1.0);
            spawn_box(world, Vec3::new(x, 1.48, z), half, 1.0);
        }
    }
}

/// The default world over `spawn_stacks(grid)` on a `workers`-wide pool.
fn default_world(workers: usize, grid: usize) -> (EcsMaster, Schedule) {
    let mut world = EcsMaster::new();
    spawn_stacks(&mut world, grid);
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut world);
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(DT)));
    let schedule = builder.build(&mut world);
    (world, schedule)
}

/// The switch reaches the tree on every Tree step, both ways: a world whose configuration turns
/// it on reads it on in the tree after one step, and off again after the next.
#[test]
fn s5_switch_mirrors_into_the_tree() {
    let (mut world, mut schedule) = default_world(2, 2);
    assert_eq!(
        world.resource::<PhysicsConfig>().broadphase,
        BroadphaseKind::Tree,
        "construction: the default world runs the tree broadphase"
    );
    assert!(!world.resource::<BroadphaseTree>().parallel_query(), "construction: the tree's own default is off");
    for on in [true, false, true] {
        world.resource_mut::<PhysicsConfig>().parallel_tree_query = on;
        schedule.run(&mut world);
        assert_eq!(
            world.resource::<BroadphaseTree>().parallel_query(),
            on,
            "S5: the tree did not read `parallel_tree_query = {on}` on the step after it was set"
        );
    }
}


/// Frames the worker-count gate steps.
const FRAMES: usize = 60;

/// G3's grid: 13 × 13 two-box stacks, 338 dynamic boxes over 43 active leaf nodes, above
/// `S5_MIN_LEAVES` on every frame with sleeping off.
const GRID: usize = 13;

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

/// What one run of the worker-count gate produced.
struct S5Run {
    /// The state hash after every frame.
    hashes: Vec<u64>,
    /// The state hash before the first frame.
    spawn_hash: u64,
    /// How far `BroadphaseTree::query_dispatches` moved over the frames.
    dispatches: u64,
    /// The fewest active leaf nodes any frame queried.
    min_leaves: u64,
}

/// The default world over G3's stacks on `workers` workers with `parallel_tree_query = on`, for
/// [`FRAMES`] frames through the real schedule.
fn run_s5(workers: usize, on: bool) -> S5Run {
    let (mut world, mut schedule) = default_world(workers, GRID);
    world.resource_mut::<PhysicsConfig>().parallel_tree_query = on;
    let spawn_hash = state_hash(&mut world);
    let before = world.resource::<BroadphaseTree>().query_dispatches();
    let mut diag = world.resource::<BroadphaseTree>().diag();
    let mut hashes = Vec::with_capacity(FRAMES);
    let mut min_leaves = u64::MAX;
    for _ in 0..FRAMES {
        schedule.run(&mut world);
        hashes.push(state_hash(&mut world));
        let now = world.resource::<BroadphaseTree>().diag();
        min_leaves = min_leaves.min(
            now.leaf_list_leaves + now.fallback_leaves - diag.leaf_list_leaves - diag.fallback_leaves,
        );
        diag = now;
    }
    let dispatches = world.resource::<BroadphaseTree>().query_dispatches() - before;
    S5Run { hashes, spawn_hash, dispatches, min_leaves }
}

/// S5's {1, N} oracle on the real schedule: the default world with `parallel_tree_query` on is
/// bit-identical, frame by frame, at W 1/2/3/4/5/8/16 and to the switch-off W1 run; and the
/// switch reaches the dispatch on every frame after the first at W ≥ 2 and on none at W1 (the
/// `lanes < 2` term).
#[test]
fn s5_default_world_is_worker_count_invariant() {
    let reference = run_s5(1, false);
    assert_ne!(reference.hashes.last().copied(), Some(reference.spawn_hash), "non-vacuity: the scene must move");
    assert_eq!(reference.dispatches, 0, "the switch-off reference never dispatches");
    assert!(
        reference.min_leaves >= S5_MIN_LEAVES as u64,
        "non-vacuity: a frame queried {} active leaf nodes, under the inline threshold {S5_MIN_LEAVES}",
        reference.min_leaves
    );
    let mut diverged = Vec::new();
    for workers in [1usize, 2, 3, 4, 5, 8, 16] {
        let r = run_s5(workers, true);
        // The first tree-path step has no history; every later one dispatches at two lanes or more.
        let expected = if workers >= 2 { FRAMES as u64 - 1 } else { 0 };
        assert_eq!(
            r.dispatches, expected,
            "S5: W{workers}: `query_dispatches` moved {} over {FRAMES} frames, expected {expected} (every \
             frame after the first at W >= 2, none at W1)",
            r.dispatches
        );
        if r.hashes != reference.hashes {
            let frame = r
                .hashes
                .iter()
                .zip(&reference.hashes)
                .position(|(a, b)| a != b)
                .map_or(FRAMES, |i| i + 1);
            diverged.push(format!("W{workers}: first differs after frame {frame}"));
        }
    }
    assert!(
        diverged.is_empty(),
        "S5: the default world with parallel_tree_query on is not worker-count invariant against the \
         switch-off W1 run:\n  {}",
        diverged.join("\n  ")
    );
}

/// The bound [`s5_tail_stays_a_rounding_error`] holds the tail to: at most 1 % of the leaf nodes
/// the dispatched frames queried.
const TAIL_PERCENT_MAX: u64 = 1;

/// S5's tail stays a rounding error of the dispatched work (triage r1 G2): G3's stacks through the
/// real schedule with the switch on, at W 2/4/8/16 for [`FRAMES`] frames — every frame after the
/// first dispatches, and the calling thread answers at most [`TAIL_PERCENT_MAX`] % of the leaf
/// nodes the dispatched frames queried in the serial tail after the join. A per-row budget too
/// small for the density the chunks meet is value-neutral (the tail answers with the serial
/// rules), so no pose or pair gate sees it: S5 would run as a serial query plus a scope.
#[test]
fn s5_tail_stays_a_rounding_error() {
    let read = |tree: &BroadphaseTree| {
        let d = tree.diag();
        (tree.query_dispatches(), tree.query_tail_leaves(), d.leaf_list_leaves + d.fallback_leaves)
    };
    for workers in [2usize, 4, 8, 16] {
        let (mut world, mut schedule) = default_world(workers, GRID);
        world.resource_mut::<PhysicsConfig>().parallel_tree_query = true;
        let (mut dispatched, mut tail, mut leaves) = (0u64, 0u64, 0u64);
        for _ in 0..FRAMES {
            let (d0, t0, l0) = read(world.resource::<BroadphaseTree>());
            schedule.run(&mut world);
            let (d1, t1, l1) = read(world.resource::<BroadphaseTree>());
            if d1 > d0 {
                dispatched += d1 - d0;
                tail += t1 - t0;
                leaves += l1 - l0;
            }
        }
        println!("S5 tail W{workers}: {tail} of {leaves} leaf nodes over {dispatched} dispatched frames");
        assert_eq!(
            dispatched,
            FRAMES as u64 - 1,
            "non-vacuity: W{workers}: S5 dispatched on {dispatched} of {FRAMES} frames, want every frame after the first"
        );
        assert!(
            tail * 100 <= TAIL_PERCENT_MAX * leaves,
            "S5: W{workers}: the calling thread answered {tail} of the {leaves} leaf nodes the dispatched frames \
             queried in the serial tail, over {TAIL_PERCENT_MAX} %: the per-row budget no longer covers the density \
             the chunks meet"
        );
    }
}

/// G3's stacks as gathered rows, for the direct-drive tree.
fn stacks_bodies(grid: usize) -> Vec<BodyState> {
    let boxed = |position: Vec3, half: Vec3, inv_mass: f32| BodyState {
        position,
        inv_mass,
        simulated: inv_mass != 0.0,
        shape: ColliderShape::Box { half_extents: half },
        ..BodyState::default()
    };
    let mut v = vec![boxed(Vec3::new(0.0, -0.5, 0.0), Vec3::new(40.0, 0.5, 40.0), 0.0)];
    let half = Vec3::new(0.5, 0.5, 0.5);
    let origin = -0.75 * (grid as f32 - 1.0);
    for i in 0..grid {
        for k in 0..grid {
            let x = origin + 1.5 * i as f32;
            let z = origin + 1.5 * k as f32;
            v.push(boxed(Vec3::new(x, 0.49, z), half, 1.0));
            v.push(boxed(Vec3::new(x, 1.48, z), half, 1.0));
        }
    }
    v
}

/// One warmed direct-drive tree step with S5's switch on, on the calling thread inside a
/// `workers`-wide `install` frame: the allocations it made on this thread and the dispatches.
fn warmed_tree_step(bodies: &[BodyState], workers: usize) -> (usize, u64) {
    let mut tree = BroadphaseTree::with_capacity(bodies.len());
    tree.set_parallel_query(true);
    let mut out = ContactPairs::with_capacity(0);
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    pool.install(|_| {
        // Warm: the history exists and every column reaches its steady length.
        for _ in 0..8 {
            tree.step_direct(bodies, &mut out);
        }
        let dispatches = tree.query_dispatches();
        let before = alloc::count();
        tree.step_direct(bodies, &mut out);
        (alloc::count().wrapping_sub(before), tree.query_dispatches() - dispatches)
    })
}

/// The W1 path's census proof (the G-L4-1 twin): a warmed tree step with S5's switch on allocates
/// nothing and dispatches nothing on a one-worker pool — the `lanes < 2` term keeps it on the serial
/// pass — while the same step on four workers dispatches and allocates its scope.
#[test]
fn s5_one_worker_allocates_nothing() {
    let bodies = stacks_bodies(GRID);
    let (allocs, dispatches) = warmed_tree_step(&bodies, 4);
    assert_eq!(dispatches, 1, "non-vacuity: the warmed step on 4 workers must dispatch S5");
    assert!(allocs > 0, "non-vacuity: a dispatched step allocates its `pool.scope`; it allocated {allocs} times");
    let (allocs, dispatches) = warmed_tree_step(&bodies, 1);
    assert_eq!(
        (allocs, dispatches),
        (0, 0),
        "S5: a warmed tree step on a 1-worker pool with the switch on allocated {allocs} times and \
         dispatched {dispatches} times; one lane must run the serial pass (the `lanes < 2` term)"
    );
}

/// A thread-local counting allocator (the G-L4-1 pattern).
mod alloc {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
    }

    /// Allocations made on this thread so far.
    pub(super) fn count() -> usize {
        ALLOC_COUNT.with(|c| c.get())
    }

    #[inline]
    fn bump() {
        let _ = ALLOC_COUNT.try_with(|c| c.set(c.get() + 1));
    }

    pub(super) struct CountingAlloc;

    // SAFETY: every call forwards verbatim to the platform `System` allocator with the same
    // layout; the wrapper only bumps a thread-local counter (via `try_with`, which no-ops if TLS is
    // mid-init, so it never re-enters the allocator). `dealloc` is an unchanged pass-through, so
    // the allocator contract is exactly `System`'s.
    unsafe impl GlobalAlloc for CountingAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            bump();
            // SAFETY: forwarded verbatim to the system allocator (same layout).
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: `ptr`/`layout` originate from `System.alloc` above.
            unsafe { System.dealloc(ptr, layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            bump();
            // SAFETY: `ptr`/`layout` originate from this allocator; `new_size` forwarded.
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }

    #[global_allocator]
    static ALLOC: CountingAlloc = CountingAlloc;
}
