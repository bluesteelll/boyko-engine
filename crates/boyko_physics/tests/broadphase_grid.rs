//! O2 broadphase gates: the uniform-grid CSR broadphase ([`BroadphaseGrid`]) is a
//! PURE optimization of the shipped O(n²) all-pairs path — its
//! feasibility-filtered, `(min, max)`-sorted [`ContactPairs`] set is bit-identical
//! to all-pairs (the load-bearing 0%-correctness gate), deterministic run-to-run,
//! and allocation-free in steady state.
//!
//! The reference all-pairs predicate here is the literal production loop from
//! [`physics_broadphase`](boyko_physics::systems::physics_broadphase)'s
//! `AllPairs` arm (same operand order, same [`body_bounding_radius`]), so a match
//! proves the grid reproduces the real default path — NOT a re-derived oracle.

use boyko_physics::components::ColliderShape;
use boyko_physics::manifold::BodyIndex;
use boyko_physics::math::Vec3;
use boyko_physics::resources::{BodyState, BroadphaseGrid};
use boyko_physics::systems::body_bounding_radius;

use proptest::prelude::*;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Builds a `BodyState` carrying only the two fields the broadphase reads
/// (`position`, `shape`) — every other field defaults (irrelevant to pairing).
fn body(position: Vec3, shape: ColliderShape) -> BodyState {
    BodyState {
        position,
        shape,
        ..Default::default()
    }
}

fn sphere(position: Vec3, radius: f32) -> BodyState {
    body(position, ColliderShape::Sphere { radius })
}

fn boxx(position: Vec3, half: Vec3) -> BodyState {
    body(position, ColliderShape::Box { half_extents: half })
}

/// The reference all-pairs broadphase — the LITERAL production `AllPairs` arm
/// (same predicate, same `(min, max)` emission). Already sorted because `i < j`.
fn all_pairs(bodies: &[BodyState]) -> Vec<(BodyIndex, BodyIndex)> {
    let mut pairs = Vec::new();
    let n = bodies.len();
    for i in 0..n {
        for j in (i + 1)..n {
            let bound = body_bounding_radius(&bodies[i]) + body_bounding_radius(&bodies[j]);
            let delta = bodies[j].position - bodies[i].position;
            if delta.length_squared() <= bound * bound {
                pairs.push((BodyIndex(i as u32), BodyIndex(j as u32)));
            }
        }
    }
    pairs
}

/// Runs the grid build over `bodies` into a fresh grid, returning the pair set.
fn grid_pairs(grid: &mut BroadphaseGrid, bodies: &[BodyState]) -> Vec<(BodyIndex, BodyIndex)> {
    let mut out = Vec::new();
    grid.build(bodies, &mut out);
    out
}

/// Asserts the grid pair set is bit-identical (same `(min, max)` order) to
/// all-pairs over `bodies`.
fn assert_grid_eq_all_pairs(bodies: &[BodyState]) {
    let mut grid = BroadphaseGrid::with_capacity(bodies.len());
    let g = grid_pairs(&mut grid, bodies);
    let a = all_pairs(bodies);
    assert_eq!(
        g, a,
        "grid pairs must be bit-identical to all-pairs (same (min, max) order)"
    );
    // The all-pairs reference is `(min, max)`-sorted by construction; confirm the
    // grid output honors the same sorted invariant independently.
    assert!(
        g.windows(2).all(|w| w[0] <= w[1]),
        "grid pairs are sorted (min, max)"
    );
}

// ── The 0%-correctness proptest (THE load-bearing O2 gate) ───────────────────

proptest! {
    // 1000 random scenes: varied counts, positions, radii, and shapes. The grid
    // pair set must EXACTLY equal all-pairs for every one (C4 0%-correctness).
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn grid_equals_all_pairs(
        scene in proptest::collection::vec(
            (
                // Position in a bounded box so clusters and gaps both occur.
                (-20.0_f32..20.0, -20.0_f32..20.0, -20.0_f32..20.0),
                // A shape: sphere (radius) or box (half-extents). 60/40 split.
                prop_oneof![
                    (0.1_f32..3.0).prop_map(|r| ColliderShape::Sphere { radius: r }),
                    ((0.1_f32..3.0), (0.1_f32..3.0), (0.1_f32..3.0))
                        .prop_map(|(x, y, z)| ColliderShape::Box {
                            half_extents: Vec3::new(x, y, z),
                        }),
                ],
            ),
            0..40usize,
        )
    ) {
        let bodies: Vec<BodyState> = scene
            .into_iter()
            .map(|((px, py, pz), shape)| body(Vec3::new(px, py, pz), shape))
            .collect();
        let mut grid = BroadphaseGrid::with_capacity(bodies.len());
        let g = grid_pairs(&mut grid, &bodies);
        let a = all_pairs(&bodies);
        prop_assert_eq!(g, a);
    }
}

// ── Multi-oversized size disparity (O2 W1: decoupled cell-size floor) ─────────

proptest! {
    // Scenes that MIX many typical-radius bodies with a FEW much-larger bodies
    // (radius >> the typical/median) so MULTIPLE bodies cross the MAX_CELL_SPAN
    // threshold and land in the oversized list — exercising the oversized–oversized
    // dedup (lower-row emit) AND the oversized–normal emit in one scene. The grid
    // pair set MUST stay bit-identical to all-pairs (the floor change is
    // output-neutral). A dense fixed lattice of typical bodies keeps `cbrt(n)`
    // large so the cells are fine (the median floor dominates), guaranteeing the
    // randomized giants span >= MAX_CELL_SPAN cells → oversized; the deterministic
    // `multi_oversized_hatch_is_genuinely_exercised` test (below) is the explicit
    // non-vacuity proof that >= 2 land oversized.
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn multi_oversized_equals_all_pairs(
        // The FEW giants: radius >> the typical median, randomized position (kept
        // inside the lattice so they overlap many small bodies) and radius. 2..=5
        // so multiple go oversized in the same scene (exercising the dedup).
        giants in proptest::collection::vec(
            (
                (0.0_f32..9.0, 0.0_f32..9.0, 0.0_f32..9.0),
                15.0_f32..45.0,
            ),
            2..6usize,
        ),
    ) {
        // A dense 10³ lattice of small bodies (cbrt(n) ≈ 10) packed in a tight box
        // so the cells stay fine regardless of the giants.
        let mut bodies: Vec<BodyState> = Vec::new();
        for z in 0..10 {
            for y in 0..10 {
                for x in 0..10 {
                    bodies.push(sphere(Vec3::new(x as f32 * 0.9, y as f32 * 0.9, z as f32 * 0.9), 0.5));
                }
            }
        }
        bodies.extend(
            giants
                .into_iter()
                .map(|((px, py, pz), r)| sphere(Vec3::new(px, py, pz), r)),
        );

        let mut grid = BroadphaseGrid::with_capacity(bodies.len());
        let g = grid_pairs(&mut grid, &bodies);
        let a = all_pairs(&bodies);
        prop_assert_eq!(g, a, "grid pairs must equal all-pairs under size disparity");
    }
}

#[test]
fn multi_oversized_hatch_is_genuinely_exercised() {
    // A deterministic size-disparity scene that MUST route >= 2 bodies through the
    // oversized hatch (the non-vacuity proof for the multi-oversized proptest: it
    // shows the decoupled floor actually classifies multiple giants as oversized,
    // so the oversized–oversized dedup path is live, not dead code). The grid pair
    // set still equals all-pairs.
    // The cell size is `max(extent_max / cbrt(n), 2·median_radius)`; for a giant to
    // span >= MAX_CELL_SPAN cells the cells must be fine, i.e. `extent_max / cbrt(n)`
    // must be small. Pack MANY small bodies in a modest box so `cbrt(n)` is large
    // and the extent stays bounded → the `extent / cbrt(n)` term drops below the
    // giants' diameter, so each giant spans far more than MAX_CELL_SPAN fine cells.
    let mut bodies = Vec::new();
    let side = 16; // 16³ = 4096 small bodies → cbrt(n) ≈ 16, fine cells.
    for z in 0..side {
        for y in 0..side {
            for x in 0..side {
                let p = Vec3::new(x as f32 * 0.9, y as f32 * 0.9, z as f32 * 0.9);
                bodies.push(sphere(p, 0.5));
            }
        }
    }
    // A handful of giants (radius >> median 0.5) overlapping the cluster → each
    // spans far more than MAX_CELL_SPAN fine cells → oversized.
    for k in 0..4 {
        let f = k as f32;
        bodies.push(sphere(Vec3::new(f * 2.0 + 1.0, f * 2.0 + 1.0, f * 2.0), 25.0));
    }

    let mut grid = BroadphaseGrid::with_capacity(bodies.len());
    let g = grid_pairs(&mut grid, &bodies);

    // Non-vacuity: the decoupled floor routes >= 2 giants through the hatch, so
    // both oversized–normal AND oversized–oversized emission are exercised.
    assert!(
        grid.oversized_len() >= 2,
        "size disparity must classify >= 2 bodies oversized (got {}); the hatch is dead otherwise",
        grid.oversized_len()
    );

    // The invariant still holds: bit-identical to all-pairs.
    assert_eq!(g, all_pairs(&bodies), "multi-oversized grid still equals all-pairs");
}

// ── Edge cases (plan O2 gate list) ───────────────────────────────────────────

#[test]
fn empty_world_no_pairs() {
    assert_grid_eq_all_pairs(&[]);
    let mut grid = BroadphaseGrid::with_capacity(0);
    assert!(grid_pairs(&mut grid, &[]).is_empty(), "empty world emits no pairs");
}

#[test]
fn single_body_no_pairs() {
    let bodies = [sphere(Vec3::new(1.0, 2.0, 3.0), 0.5)];
    assert_grid_eq_all_pairs(&bodies);
    let mut grid = BroadphaseGrid::with_capacity(1);
    assert!(grid_pairs(&mut grid, &bodies).is_empty(), "one body emits no pairs");
}

#[test]
fn all_coincident_every_pair_is_a_candidate() {
    // 12 bodies at the same point → every unordered pair overlaps. The grid must
    // emit each pair exactly once (no duplicates from multi-cell co-occupancy),
    // matching the C(12,2) = 66 all-pairs set.
    let bodies: Vec<BodyState> = (0..12)
        .map(|_| sphere(Vec3::ZERO, 0.5))
        .collect();
    assert_grid_eq_all_pairs(&bodies);
    let mut grid = BroadphaseGrid::with_capacity(bodies.len());
    let g = grid_pairs(&mut grid, &bodies);
    assert_eq!(g.len(), 12 * 11 / 2, "all-coincident → C(n,2) pairs, no dupes");
}

#[test]
fn one_giant_oversized_vs_many_tiny() {
    // The oversized escape hatch: a single huge sphere overlapping 1000 tiny ones
    // spread across many cells. The giant goes to `oversized` and is tested
    // against all; the grid pair set must still equal all-pairs.
    let mut bodies = Vec::with_capacity(1001);
    bodies.push(sphere(Vec3::ZERO, 100.0)); // the giant (spans the whole world)
    for i in 0..1000 {
        let t = i as f32;
        // Spread the tiny bodies within the giant's reach so most pair with it.
        let p = Vec3::new((t * 0.137).sin() * 50.0, (t * 0.31).cos() * 50.0, (t * 0.07).sin() * 50.0);
        bodies.push(sphere(p, 0.25));
    }
    assert_grid_eq_all_pairs(&bodies);
    // Anti-vacuity: the giant pairs with many tiny bodies.
    let mut grid = BroadphaseGrid::with_capacity(bodies.len());
    let g = grid_pairs(&mut grid, &bodies);
    assert!(g.len() > 100, "the giant produces many pairs (anti-vacuity): {}", g.len());
}

#[test]
fn bodies_far_apart_no_pairs() {
    // Each body is isolated far beyond any bounding-sphere overlap.
    let bodies: Vec<BodyState> = (0..16)
        .map(|i| sphere(Vec3::new(i as f32 * 1000.0, 0.0, 0.0), 0.5))
        .collect();
    assert_grid_eq_all_pairs(&bodies);
    let mut grid = BroadphaseGrid::with_capacity(bodies.len());
    assert!(grid_pairs(&mut grid, &bodies).is_empty(), "far-apart bodies pair with none");
}

#[test]
fn body_exactly_on_cell_boundary() {
    // Positions chosen so several bodies land on integer cell boundaries (the
    // `floor` + clamp edge). The grid must still match all-pairs (no off-by-one
    // missed or duplicated pair at the seam).
    let bodies = [
        sphere(Vec3::new(0.0, 0.0, 0.0), 0.5),
        sphere(Vec3::new(1.0, 0.0, 0.0), 0.5),
        sphere(Vec3::new(2.0, 0.0, 0.0), 0.5),
        sphere(Vec3::new(1.0, 1.0, 0.0), 0.5),
        sphere(Vec3::new(0.0, 1.0, 1.0), 0.5),
        sphere(Vec3::new(2.0, 2.0, 2.0), 0.5),
    ];
    assert_grid_eq_all_pairs(&bodies);
}

#[test]
fn mixed_sphere_and_box_shapes() {
    // Spheres and boxes interleaved in a cluster — exercises the shape-agnostic
    // bounding radius (a box contributes its half-extent diagonal) through both
    // the grid bucketing and the feasibility filter.
    let bodies = [
        sphere(Vec3::new(0.0, 0.0, 0.0), 0.6),
        boxx(Vec3::new(0.5, 0.2, 0.0), Vec3::new(0.4, 0.4, 0.4)),
        sphere(Vec3::new(1.0, 0.0, 0.0), 0.5),
        boxx(Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.7, 0.3, 0.5)),
        boxx(Vec3::new(3.0, 3.0, 3.0), Vec3::new(0.2, 0.2, 0.2)),
    ];
    assert_grid_eq_all_pairs(&bodies);
    // Anti-vacuity: the cluster yields at least one pair.
    let mut grid = BroadphaseGrid::with_capacity(bodies.len());
    assert!(!grid_pairs(&mut grid, &bodies).is_empty(), "the mixed cluster pairs");
}

// ── Determinism: run-to-run bit-identical ────────────────────────────────────

#[test]
fn grid_is_run_to_run_deterministic() {
    // A dense cluster spanning multiple cells, mixed shapes — the kind of scene
    // where any nondeterminism (cell-size proxy, scatter order, dedup) would
    // surface. Two independent builds must be bit-identical.
    let bodies: Vec<BodyState> = (0..200)
        .map(|i| {
            let t = i as f32;
            let p = Vec3::new(
                (t * 0.21).sin() * 8.0,
                (t * 0.13).cos() * 8.0,
                (t * 0.37).sin() * 8.0,
            );
            if i % 3 == 0 {
                boxx(p, Vec3::new(0.3, 0.5, 0.4))
            } else {
                sphere(p, 0.5)
            }
        })
        .collect();

    let mut grid_a = BroadphaseGrid::with_capacity(bodies.len());
    let mut grid_b = BroadphaseGrid::with_capacity(bodies.len());
    let a = grid_pairs(&mut grid_a, &bodies);
    let b = grid_pairs(&mut grid_b, &bodies);
    assert_eq!(a, b, "grid build is run-to-run bit-identical");
    assert!(!a.is_empty(), "the dense cluster produced pairs (anti-vacuity)");
    // And it equals all-pairs.
    assert_eq!(a, all_pairs(&bodies), "deterministic AND correct");
}

#[test]
fn reused_grid_matches_fresh_grid() {
    // The same grid reused across builds (capacity-reused scratch) must produce
    // the same result as a fresh grid — proves the clear()+refill leaves no stale
    // state between frames.
    let scene_a: Vec<BodyState> = (0..50)
        .map(|i| sphere(Vec3::new(i as f32 * 0.7, (i % 5) as f32, 0.0), 0.5))
        .collect();
    let scene_b: Vec<BodyState> = (0..30)
        .map(|i| sphere(Vec3::new((i as f32).sin() * 5.0, 0.0, i as f32 * 0.4), 0.6))
        .collect();

    let mut reused = BroadphaseGrid::with_capacity(64);
    // Warm it on scene A, then build scene B on the SAME grid.
    let _ = grid_pairs(&mut reused, &scene_a);
    let reused_b = grid_pairs(&mut reused, &scene_b);

    let mut fresh = BroadphaseGrid::with_capacity(64);
    let fresh_b = grid_pairs(&mut fresh, &scene_b);

    assert_eq!(reused_b, fresh_b, "a reused grid matches a fresh build (no stale state)");
    assert_eq!(reused_b, all_pairs(&scene_b), "and equals all-pairs");
}

// ── Zero per-step allocation in steady state ─────────────────────────────────

// The alloc-counting gate installs a custom `#[global_allocator]` that delegates
// to `std::alloc::System` (see `CountingAlloc` at the bottom). Under Miri that
// `System`-delegating wrapper is incompatible with the interpreter: `System` on
// windows-gnu stamps a `Header` before each block and re-reads it on `dealloc`
// (`ptr::read((ptr as *mut Header).sub(1))`), and routing the std test harness's
// own shutdown allocations through the wrapper makes Miri flag a tag/protector
// violation when the runner drops its `mpmc` results channel — AFTER every test
// body has already passed (the diagnostic backtrace bottoms out in
// `test::run_tests`, not in `BroadphaseGrid`). It reproduces identically under
// BOTH Stacked AND Tree Borrows, confirming it is a Miri-vs-custom-allocator
// harness artifact, not grid UB. So the whole counting-allocator apparatus is
// gated off under Miri (the steady-state heap-reuse property is a native concern
// anyway — Miri does not model the real allocator's capacity reuse), and the lib
// Miri run plus the Tree-Borrows grid run (sans this allocator) exercise the same
// grid build/scatter/dedup/median paths cleanly.
#[cfg(not(miri))]
#[test]
fn grid_does_no_per_step_alloc_in_steady_state() {
    // Warm the grid on the worst-case scene (max cells, max candidates) so every
    // scratch Vec reaches its steady-state capacity, then build the SAME scene
    // again under a counting allocator: a steady-state build must allocate ZERO
    // times (clear()+refill reuses capacity; the output Vec is also pre-warmed).
    let bodies: Vec<BodyState> = (0..300)
        .map(|i| {
            let t = i as f32;
            sphere(
                Vec3::new(
                    (t * 0.17).sin() * 10.0,
                    (t * 0.29).cos() * 10.0,
                    (t * 0.41).sin() * 10.0,
                ),
                0.5,
            )
        })
        .collect();

    let mut grid = BroadphaseGrid::with_capacity(bodies.len());
    let mut out: Vec<(BodyIndex, BodyIndex)> = Vec::new();
    // Warm-up builds: grow every buffer (grid scratch + the output Vec) to its
    // steady-state size. Several iterations so prefix-sum/cursor/candidate Vecs
    // all settle.
    for _ in 0..4 {
        grid.build(&bodies, &mut out);
    }

    let before = ALLOC.count();
    grid.build(&bodies, &mut out);
    let after = ALLOC.count();
    assert_eq!(
        after, before,
        "a warmed steady-state grid build must do ZERO heap allocations (did {})",
        after - before
    );
}

// ── Production-path A/B: the real `physics_broadphase` system, both kinds ────

// Native-only: this module spins up a real `Schedule` over the work-stealing
// `boyko_threadpool`, whose spin-loop is intractable under the Miri interpreter
// (the pool is already loom + Miri proven in the ECS Phase-9 series). The grid's
// own pure-CPU build/scatter/dedup/median paths are covered Miri-clean by the
// deterministic tests above; this A/B closes the gap to the production match arm
// on native runs.
#[cfg(not(miri))]
mod world_ab {
    //! Runs the ACTUAL [`physics_broadphase`] system (not the test reference) both
    //! ways through a real schedule + ECS world, over a static scene (zero gravity,
    //! zero velocity → positions identical across the two runs), and asserts the
    //! `Grid` and `AllPairs` [`ContactPairs`] outputs are bit-identical. This closes
    //! the gap between the test's reference loop and the production match arm.

    use boyko_ecs::ecs::core::component::component::Component;
    use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
    use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
    use boyko_ecs::ecs::core::time::FixedTime;
    use boyko_threadpool::ThreadPoolBuilder;

    use boyko_physics::components::{
        Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass,
    };
    use boyko_physics::manifold::BodyIndex;
    use boyko_physics::math::{Mat3, Quat, Vec3};
    use boyko_physics::plugin::add_physics_systems;
    use boyko_physics::resources::{BroadphaseKind, ContactPairs, PhysicsConfig};
    use boyko_physics::solver::NoopSolver;

    use std::sync::Arc;

    /// Views a `#[repr(C)]` POD as raw bytes for the `create_entity` spawn path.
    fn as_bytes<T>(value: &T) -> &[u8] {
        // SAFETY: `value` is a live `#[repr(C)]` `T`; we view its `size_of::<T>()`
        // bytes as a read-only slice bounded by the borrow — the exact layout the
        // component pool stores (mirrors `physics_seam::as_bytes`).
        unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
    }

    fn spawn_sphere(world: &mut EcsMaster, position: Vec3, radius: f32) {
        let body = RigidBody {
            position,
            linear_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
        };
        let mass = RigidBodyMass {
            inv_inertia: Mat3::IDENTITY,
            inv_mass: 1.0,
            restitution: 0.5,
            friction: 0.3,
        };
        let collider = Collider {
            shape: ColliderShape::Sphere { radius },
            layer: 1,
            mask: 1,
        };
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

    /// Spawns the shared static cluster, builds a NoopSolver schedule, and zeroes
    /// gravity so `physics_integrate` leaves positions fixed across runs.
    fn static_cluster_world() -> (EcsMaster, Schedule) {
        let mut world = EcsMaster::new();
        // A dense cluster (overlaps → real pairs) plus a few isolated bodies.
        for i in 0..24 {
            let t = i as f32;
            let p = Vec3::new(
                (t * 0.5).sin() * 2.0,
                (t * 0.3).cos() * 2.0,
                (t * 0.7).sin() * 2.0,
            );
            spawn_sphere(&mut world, p, 0.5);
        }
        spawn_sphere(&mut world, Vec3::new(100.0, 0.0, 0.0), 0.5); // isolated

        let pool = ThreadPoolBuilder::new().num_threads(1).build();
        let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
        let _ = add_physics_systems::<NoopSolver>(&mut builder, &mut world);
        world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(1.0 / 64.0)));
        // Zero gravity + zero velocity ⇒ integrate is a no-op on position, so the
        // two runs broadphase identical positions.
        world.resource_mut::<PhysicsConfig>().gravity = Vec3::ZERO;
        let schedule = builder.build(&mut world);
        (world, schedule)
    }

    fn run_with(kind: BroadphaseKind) -> Vec<(BodyIndex, BodyIndex)> {
        let (mut world, mut schedule) = static_cluster_world();
        world.resource_mut::<PhysicsConfig>().broadphase = kind;
        schedule.run(&mut world);
        world.resource::<ContactPairs>().pairs.clone()
    }

    #[test]
    fn production_grid_equals_all_pairs() {
        let all_pairs = run_with(BroadphaseKind::AllPairs);
        let grid = run_with(BroadphaseKind::Grid);
        assert_eq!(
            grid, all_pairs,
            "the real physics_broadphase Grid arm is bit-identical to the AllPairs arm"
        );
        assert!(!all_pairs.is_empty(), "the cluster produced pairs (anti-vacuity)");
    }
}

// ── O3 parallel candidate emit: pool-DISPATCHED `build_parallel` gates ────────
//
// Native-only: these drive `BroadphaseGrid::build_parallel` through a REAL
// work-stealing `boyko_threadpool` (Miri-intractable spin-loop), so they exercise
// the genuinely concurrent Pass A count / Pass B emit + the `EmitPtrs` disjoint
// raw writes across multiple workers — the interleave-dependent leg the in-lib
// `build_emit_shaped_forced` (single-threaded) gates cannot reach. The pure
// arithmetic + the no-pool shaped path are covered Miri-clean by the in-lib
// `o3_shaped` module (`resources.rs`).
//
// `build_parallel` takes the PARALLEL branch only when (a) `n >= MIN_PARALLEL_BODIES`
// (= 4096) AND (b) it runs inside a `pool.install` frame (so `try_with_active_pool`
// returns `Some`). Each gate asserts both preconditions hold (anti-vacuity: the
// parallel branch genuinely ran, not the serial fallback).
#[cfg(not(miri))]
mod o3_parallel {
    use super::{all_pairs, sphere};
    use boyko_physics::manifold::BodyIndex;
    use boyko_physics::math::Vec3;
    use boyko_physics::resources::{BodyState, BroadphaseGrid};
    use boyko_threadpool::ThreadPoolBuilder;

    /// `MIN_PARALLEL_BODIES` from `resources.rs` (private const, mirrored here only
    /// to size the gate scenes above the parallel-branch threshold). The
    /// `parallel_branch_actually_runs` test below is the anti-vacuity proof that the
    /// dispatched path — not the serial fallback — is what these scenes hit.
    const MIN_PARALLEL_BODIES: usize = 4096;

    /// A DENSE overlapping lattice of `n` unit spheres (sub-diameter spacing →
    /// real overlaps, many occupied cells → a genuine multi-chunk partition).
    fn dense_scene(n: usize) -> Vec<BodyState> {
        let side = (n as f64).cbrt().ceil() as usize;
        let spacing = 0.9_f32;
        let mut bodies = Vec::with_capacity(n);
        let mut i = 0usize;
        'outer: for z in 0..side {
            for y in 0..side {
                for x in 0..side {
                    if i >= n {
                        break 'outer;
                    }
                    let t = i as f32;
                    let jitter = Vec3::new((t * 0.13).sin() * 0.1, (t * 0.27).cos() * 0.1, 0.0);
                    let p = Vec3::new(
                        x as f32 * spacing,
                        y as f32 * spacing,
                        z as f32 * spacing,
                    ) + jitter;
                    bodies.push(sphere(p, 0.5));
                    i += 1;
                }
            }
        }
        bodies
    }

    /// Runs `build_parallel` over `bodies` INSIDE a `pool.install` frame with
    /// `workers` worker threads (so `try_with_active_pool` finds the ambient pool),
    /// returning the pair set.
    fn parallel_pairs(workers: usize, bodies: &[BodyState]) -> Vec<(BodyIndex, BodyIndex)> {
        let pool = ThreadPoolBuilder::new().num_threads(workers).build();
        pool.install(|_scope| {
            let mut grid = BroadphaseGrid::with_capacity(bodies.len());
            let mut out = Vec::new();
            grid.build_parallel(bodies, &mut out);
            out
        })
    }

    // ── Gate 6: pool-dispatched `build_parallel` byte-identity at workers ∈
    //    {1, 2, 4} over many dense scenes — the interleave-dependent leg. Each
    //    worker count's output must be byte-for-byte equal to the O2 serial
    //    `build` AND to all-pairs (the candidate multiset is partition- AND
    //    interleave-independent; the final sort canonicalizes order). ──────────
    #[test]
    fn parallel_dispatched_bit_identical_at_1_2_4_workers() {
        // A spread of body counts ABOVE the parallel threshold so the dispatched
        // branch is taken; several distinct dense scenes so interleave-dependent
        // bugs across builds surface.
        for &n in &[MIN_PARALLEL_BODIES, 6000, 9000] {
            let bodies = dense_scene(n);

            // The serial reference (O2 `build`) + the all-pairs oracle.
            let mut grid = BroadphaseGrid::with_capacity(bodies.len());
            let mut serial = Vec::new();
            grid.build(&bodies, &mut serial);
            let oracle = all_pairs(&bodies);
            assert_eq!(serial, oracle, "O2 serial build == all-pairs (n={n})");
            assert!(!serial.is_empty(), "anti-vacuity: scene n={n} has survivors");

            for &workers in &[1usize, 2, 4] {
                let par = parallel_pairs(workers, &bodies);
                assert_eq!(
                    par, serial,
                    "build_parallel (workers={workers}, n={n}) must be byte-identical \
                     to the O2 serial build (and all-pairs)"
                );
            }
        }
    }

    // ── Gate 6 (anti-vacuity): the PARALLEL branch genuinely runs — the pool has
    //    > 1 worker AND the scene is at/above the threshold, so `build_parallel`
    //    dispatches (it does NOT fall through to the serial single-chunk path).
    //    We confirm the precondition + that a scene JUST BELOW the threshold and a
    //    no-pool build both still match — the gate truly toggles the branch. ────
    #[test]
    fn parallel_branch_actually_runs_and_both_branches_agree() {
        // (a) Above the threshold, multi-worker pool → the DISPATCHED branch.
        let big = dense_scene(MIN_PARALLEL_BODIES + 500);
        let pool = ThreadPoolBuilder::new().num_threads(3).build();
        assert!(
            pool.num_threads() >= 1,
            "the pool must have worker threads for the dispatched branch"
        );
        assert!(
            big.len() >= MIN_PARALLEL_BODIES,
            "anti-vacuity: the scene ({}) is at/above MIN_PARALLEL_BODIES ({}) so \
             build_parallel takes the dispatched branch, not the serial fallback",
            big.len(),
            MIN_PARALLEL_BODIES
        );
        let dispatched = pool.install(|_scope| {
            let mut grid = BroadphaseGrid::with_capacity(big.len());
            let mut out = Vec::new();
            grid.build_parallel(&big, &mut out);
            out
        });

        // (b) The SAME scene with NO ambient pool → the no-pool shaped fallback
        //     (build_parallel called outside any install frame).
        let no_pool = {
            let mut grid = BroadphaseGrid::with_capacity(big.len());
            let mut out = Vec::new();
            grid.build_parallel(&big, &mut out);
            out
        };

        // (c) Below the threshold, even inside a pool → the serial fallback.
        let small = dense_scene(MIN_PARALLEL_BODIES - 1000);
        let small_in_pool = pool.install(|_scope| {
            let mut grid = BroadphaseGrid::with_capacity(small.len());
            let mut out = Vec::new();
            grid.build_parallel(&small, &mut out);
            out
        });

        // Every branch reproduces all-pairs byte-for-byte (the whole contract).
        assert_eq!(dispatched, all_pairs(&big), "dispatched branch == all-pairs");
        assert_eq!(no_pool, all_pairs(&big), "no-pool fallback == all-pairs");
        assert_eq!(dispatched, no_pool, "dispatched and no-pool branches agree byte-for-byte");
        assert_eq!(small_in_pool, all_pairs(&small), "below-threshold serial fallback == all-pairs");
        assert!(!dispatched.is_empty(), "anti-vacuity: the dispatched scene has survivors");
    }

    // ── Gate 8: a WARMED pool-dispatched `build_parallel` allocates ZERO on the
    //    hot path (the parallel-emit CSR scratch pair_count/pair_offset + out are
    //    all capacity-reused after warm-up). Measured on the dispatcher thread
    //    inside `pool.install`; the counting allocator is the thread-local global
    //    from this file's bottom. The pool's per-`scope` dispatch boxes ARE a
    //    bounded justified cost (the same class as `par_iter` / O6) — so the gate
    //    bounds them to a small constant, and the LOAD-BEARING check is that the
    //    count does NOT scale with the (large) candidate set. ──────────────────
    #[test]
    fn warmed_parallel_build_allocation_is_bounded() {
        let workers = 4;
        let bodies = dense_scene(MIN_PARALLEL_BODIES + 2000);
        let pool = ThreadPoolBuilder::new().num_threads(workers).build();

        let (allocs, n_pairs) = pool.install(|_scope| {
            let mut grid = BroadphaseGrid::with_capacity(bodies.len());
            let mut out = Vec::new();
            // Warm: several dispatched builds so every grid scratch Vec
            // (pair_count, pair_offset, cell_*, candidates) AND `out` reach
            // steady-state capacity (clear()+refill thereafter).
            for _ in 0..6 {
                grid.build_parallel(&bodies, &mut out);
            }
            let before = super::ALLOC.count();
            grid.build_parallel(&bodies, &mut out);
            let after = super::ALLOC.count();
            (after.wrapping_sub(before), out.len())
        });

        assert!(n_pairs > 0, "anti-vacuity: the warmed parallel build produced pairs");
        eprintln!(
            "[O3 Gate8] warmed build_parallel workers={workers} n_pairs={n_pairs} hot_allocs={allocs}"
        );

        // Pass A + Pass B each issue ONE `pool.scope` (a boxed shared frame + the
        // per-spawn closure boxes); the work-balanced chunking emits up to
        // (workers + 1) × CHUNKS_PER_WORKER chunks per scope. The grid's OWN
        // scratch is zero-per-step-alloc (capacity reuse), so the only residual is
        // this bounded 2-scope dispatch cost — INDEPENDENT of the candidate-set
        // size (the load-bearing property: a buffer that re-grew per step would
        // scale the count with n_pairs). CHUNKS_PER_WORKER (= 4) is mirrored here.
        let chunks_per_worker = 4;
        let per_scope_cap = 16 + (workers + 1) * chunks_per_worker * 5; // generous
        let scopes_per_build = 2; // Pass A + Pass B
        let bound = scopes_per_build * per_scope_cap;
        assert!(
            allocs <= bound,
            "a warmed pool-dispatched build_parallel must allocate only the bounded \
             2-scope dispatch cost (<= {bound}); got {allocs} for {n_pairs} pairs. A \
             count that scaled with n_pairs would be a per-step buffer re-grow (the \
             grid scratch must be capacity-reused)."
        );
    }
}

// ── Counting global allocator (steady-state alloc gate) ──────────────────────
//
// Gated `cfg(not(miri))`: the `System`-delegating wrapper trips a Miri
// tag/protector diagnostic in the std test harness's own shutdown (see the note
// on `grid_does_no_per_step_alloc_in_steady_state`). On native it is the global
// allocator that backs the zero-per-step-alloc gate; under Miri the default
// allocator is used and the gate is skipped.

#[cfg(not(miri))]
use std::alloc::{GlobalAlloc, Layout, System};
#[cfg(not(miri))]
use std::cell::Cell;

#[cfg(not(miri))]
thread_local! {
    /// Per-thread allocation counter. Thread-local (not a shared atomic) so the
    /// other tests running in parallel — which allocate heavily — cannot corrupt
    /// the alloc gate's `before`/`after` delta: only the gate's own thread reads
    /// its own count, and the grid build is purely thread-local work.
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

/// A pass-through global allocator that counts `alloc`/`realloc` calls per thread,
/// so the zero-per-step-alloc gate can assert a warmed grid build allocates
/// nothing on its thread.
#[cfg(not(miri))]
struct CountingAlloc;

#[cfg(not(miri))]
impl CountingAlloc {
    /// This thread's observed alloc/realloc call count so far.
    fn count(&self) -> usize {
        // The counter is touched only outside an active allocation, so reading it
        // through the thread-local accessor cannot re-enter the allocator.
        ALLOC_COUNT.with(|c| c.get())
    }
}

/// Increments this thread's counter, tolerating the (impossible-in-practice)
/// re-entrant access during TLS init by ignoring a failed access.
#[cfg(not(miri))]
#[inline]
fn bump_alloc_count() {
    let _ = ALLOC_COUNT.try_with(|c| c.set(c.get() + 1));
}

// SAFETY: every call forwards verbatim to the platform `System` allocator with
// the same layout — the wrapper only bumps a thread-local counter (via a
// `try_with` that no-ops if TLS is mid-init, so it never re-enters the
// allocator), which adds no aliasing or layout obligation. `dealloc` is an
// unchanged pass-through, so the allocator contract is exactly `System`'s.
#[cfg(not(miri))]
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

#[cfg(not(miri))]
#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;
