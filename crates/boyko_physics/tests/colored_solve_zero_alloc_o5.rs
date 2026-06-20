//! Phase O5 Gate 5 — ZERO per-step allocation in the colored solve (W1 fix).
//!
//! The W1 review fix eliminated the per-step heap allocation in the colored
//! solver's `build_columns` / canonical-order materialization (the per-manifold
//! base map + canonical index list are now capacity-reused `Vec` scratch on the
//! solver, refilled with `clear()` each step, never `vec!`-allocated). This file
//! proves it with a counting `#[global_allocator]`:
//!
//! 1. **Direct (isolated build + solve):** a `ColoredSoftStepSolver` warmed on a
//!    realistic contact graph, then measured for ONE more `solve_colored` step
//!    under the counting allocator → ZERO allocations (release; ONE cfg-gated
//!    debug-assert scratch in debug, from the SHARED `ConstraintGraph` build, not
//!    the colored solver). This is exactly the `build_columns` + canonical +
//!    substep solve + canonical warm store the production stage drives.
//! 2. **Differential (schedule delta):** a warmed colored-solve step's alloc delta
//!    is bounded by a small per-system dispatch constant over the default step —
//!    i.e. the colored build+solve adds no DATA-DEPENDENT per-step allocation.
//!
//! Gated `cfg(not(miri))`: the counting allocator's System-delegating wrapper is a
//! known Miri harness artifact (it trips a tag/protector diagnostic in the std
//! harness's own shutdown AFTER every test body passes — see `broadphase_grid.rs`
//! / `constraint_graph_o4_world.rs`), and the differential test spins up the
//! threadpool (Miri-intractable).

#![cfg(not(miri))]

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::components::{
    BodyType, Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass,
};
use boyko_physics::manifold::{BodyIndex, ContactPoint, Manifold};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::{add_physics_colored_solve, add_physics_systems};
use boyko_physics::resources::{BodyState, ConstraintGraph, PhysicsConfig, SolverScratch};
use boyko_physics::solver::{ColoredSoftStepSolver, SoftStepSolver};

// ── Helpers ──────────────────────────────────────────────────────────────────

fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; we view its bytes as a read-only
    // slice bounded by the borrow (mirrors `softstep::as_bytes`).
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

fn spawn_body(world: &mut EcsMaster, body: RigidBody, mass: RigidBodyMass, collider: Collider) {
    let archetype = world.bundle_archetype_id_for::<RigidBodyBundle>();
    world
        .create_entity(
            archetype,
            &[
                (RigidBody::component_id(), as_bytes(&body)),
                (RigidBodyMass::component_id(), as_bytes(&mass)),
                (Collider::component_id(), as_bytes(&collider)),
            ],
        )
        .expect("invariant: RigidBodyBundle archetype accepts the three columns");
}

fn sphere_state(position: Vec3, inv_mass: f32, body_type: BodyType) -> BodyState {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: Mat3::IDENTITY,
        inv_mass,
        restitution: 0.0,
        friction: 0.5,
        body_type,
    };
    let collider = Collider {
        shape: ColliderShape::Sphere { radius: 0.5 },
        layer: 1,
        mask: 1,
    };
    BodyState::from_columns(&body, &mass, &collider)
}

fn sphere_components(position: Vec3, radius: f32, inv_mass: f32, body_type: BodyType) -> (RigidBody, RigidBodyMass, Collider) {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: Mat3::IDENTITY,
        inv_mass,
        restitution: 0.3,
        friction: 0.5,
        body_type,
    };
    let collider = Collider {
        shape: ColliderShape::Sphere { radius },
        layer: 1,
        mask: 1,
    };
    (body, mass, collider)
}

/// A single-point penetrating manifold between rows `a` and `b`.
fn manifold(a: u32, b: u32, normal: Vec3, sep: f32, anchor: Vec3) -> Manifold {
    let mut m = Manifold::new(BodyIndex(a), BodyIndex(b));
    m.normal = normal;
    m.points[0] = ContactPoint {
        anchor_a: anchor,
        anchor_b: anchor,
        separation: sep,
        feature_id: 0,
    };
    m.count = 1;
    m
}

// ── Gate 5 (direct): the warmed colored build + solve allocates ZERO ──────────

#[test]
fn colored_solve_does_no_per_step_alloc_in_steady_state() {
    // A realistic resting contact set: a chain of dynamic spheres each contacting
    // its neighbour + a shared static floor (one island, several colors). The same
    // build_columns / canonical / substep solve / canonical warm store the
    // production `physics_solve_colored` stage drives — measured in isolation under
    // the counting allocator (no threadpool, no schedule machinery).
    let n_dyn = 64usize;
    let floor_row = n_dyn as u32;
    let mut bodies: Vec<BodyState> = (0..n_dyn)
        .map(|i| sphere_state(Vec3::new(0.0, 1.0 + i as f32 * 0.99, 0.0), 1.0, BodyType::Dynamic))
        .collect();
    bodies.push(sphere_state(Vec3::new(0.0, -10.0, 0.0), 0.0, BodyType::Static));

    // Build a fixed manifold set: each dyn sphere on the one below + the bottom on
    // the floor (a connected stack → 1 island, alternating colors).
    let build_manifolds = || {
        let mut m = Vec::new();
        m.push(manifold(0, floor_row, Vec3::new(0.0, -1.0, 0.0), -0.1, Vec3::ZERO));
        for i in 1..n_dyn as u32 {
            m.push(manifold(i - 1, i, Vec3::new(0.0, 1.0, 0.0), -0.1, Vec3::ZERO));
        }
        m
    };
    let manifolds = build_manifolds();

    let inv_mass: Vec<f32> = bodies.iter().map(|b| b.inv_mass).collect();
    let mut graph = ConstraintGraph::with_capacity(bodies.len());
    let is_dynamic = {
        let inv_mass = inv_mass.clone();
        move |row: u32| (row as usize) < inv_mass.len() && inv_mass[row as usize] != 0.0
    };

    let cfg = PhysicsConfig { dt: 1.0 / 60.0, ..PhysicsConfig::default() };
    let mut solver = ColoredSoftStepSolver::default();
    let mut scratch = SolverScratch::with_capacity(bodies.len());
    scratch.set_bodies(&bodies);

    // Warm: several full steps so every solver/graph/scratch buffer reaches its
    // steady-state capacity (clear()+refill reuses it thereafter).
    for _ in 0..8 {
        graph.build(&manifolds, scratch.bodies().len(), &is_dynamic);
        scratch.touched.reset(scratch.bodies().len());
        solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
    }

    // Anti-vacuity: a real partition with > 1 color and the contacts dispatched.
    assert!(graph.n_colors() > 1, "anti-vacuity: > 1 color (got {})", graph.n_colors());
    assert!(!manifolds.is_empty(), "anti-vacuity: > 0 manifolds");

    // The graph build (the SHARED ConstraintGraph) carries exactly ONE cfg-gated
    // debug-assert scratch alloc in debug; the colored solver's own build+solve is
    // zero-alloc in both. Measure the colored solve step IN ISOLATION (graph
    // already built + warmed) so the gate is the COLORED solver's per-step alloc.
    graph.build(&manifolds, scratch.bodies().len(), &is_dynamic); // outside the window
    let before = ALLOC.count();
    scratch.touched.reset(scratch.bodies().len());
    solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
    let after = ALLOC.count();
    let allocs = after.wrapping_sub(before);

    assert_eq!(
        allocs, 0,
        "warmed ColoredSoftStepSolver::solve_colored must allocate ZERO in steady state \
         (build_columns / canonical / substep solve / canonical warm store are all \
         capacity-reused), got {allocs}"
    );
}

// ── Gate 5 (differential): the colored step's extra alloc is bounded dispatch ──

#[test]
fn colored_step_extra_alloc_is_bounded_dispatch_overhead() {
    // The colored-solve path runs TWO extra systems over the default path
    // (physics_build_graph + physics_solve_colored replaces physics_solve_step, so
    // net +1 system) AND swaps the solver. The per-step alloc delta over the
    // default must stay within a small per-system dispatch constant — NOT a
    // data-dependent per-contact/per-color allocation (the isolated test above is
    // the exact-zero gate for the colored solver's own work).
    fn spawn_stack(world: &mut EcsMaster) {
        let column = [
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.05, 1.9, 0.02),
            Vec3::new(-0.03, 2.8, -0.01),
            Vec3::new(0.02, 3.7, 0.03),
        ];
        for &p in &column {
            let (b, m, c) = sphere_components(p, 0.5, 1.0, BodyType::Dynamic);
            spawn_body(world, b, m, c);
        }
        let (b, m, c) = sphere_components(Vec3::new(0.0, -10.0, 0.0), 10.0, 0.0, BodyType::Static);
        spawn_body(world, b, m, c);
    }

    fn warmed_step_alloc_delta(colored: bool) -> usize {
        let mut world = EcsMaster::new();
        spawn_stack(&mut world);
        let mut builder = ScheduleBuilder::new(serial_pool());
        if colored {
            let _ = add_physics_colored_solve(&mut builder, &mut world);
        } else {
            let _ = add_physics_systems::<SoftStepSolver>(&mut builder, &mut world);
        }
        world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(1.0 / 60.0)));
        let mut schedule: Schedule = builder.build(&mut world);
        for _ in 0..120 {
            schedule.run(&mut world);
        }
        let before = ALLOC.count();
        schedule.run(&mut world);
        ALLOC.count().wrapping_sub(before)
    }

    let default_delta = warmed_step_alloc_delta(false);
    let colored_delta = warmed_step_alloc_delta(true);
    let extra = colored_delta.saturating_sub(default_delta);

    // The colored path adds one net system (build_graph) plus the colored solve
    // stage swap. The engine's per-system dispatch overhead is a small fixed
    // constant (~a few allocs for SystemParam machinery, plus the cfg-gated
    // debug_assert_coloring scratch in debug). Bound it well below anything a real
    // data-dependent per-step allocation (per-contact/per-color) would cost — on
    // this 4-sphere stack a leaked per-manifold alloc would be dozens.
    let bound = if cfg!(debug_assertions) { 8 } else { 5 };
    assert!(
        extra <= bound,
        "colored-step extra allocs over default = {extra} (expected <= {bound}: per-system \
         dispatch overhead, NOT the colored build+solve which is proven zero-alloc in isolation); \
         colored={colored_delta} default={default_delta}"
    );
}

// ── Counting global allocator (mirrors constraint_graph_o4_world.rs) ───────────

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    /// Per-thread allocation counter — thread-local (not a shared atomic) so other
    /// tests' parallel allocations cannot corrupt the before/after delta.
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

struct CountingAlloc;

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
// same layout; the wrapper only bumps a thread-local counter (via a `try_with` that
// no-ops if TLS is mid-init, so it never re-enters the allocator). `dealloc` is an
// unchanged pass-through, so the allocator contract is exactly `System`'s.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: forwarded verbatim to the system allocator (same layout).
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr`/`layout` originate from `System.alloc` above (this is the
        // process global allocator), so they satisfy `System::dealloc`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `ptr`/`layout` originate from this allocator; `new_size`
        // forwarded verbatim to `System::realloc`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;
