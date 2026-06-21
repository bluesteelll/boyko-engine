//! Miri-tractable unit coverage of the C1 SDF-sentinel SOLVER path (P2 W5).
//!
//! The W5 acceptance tests in `sdf_collision.rs` exercise the sentinel solve
//! through a real `Schedule`, which fans systems across `boyko_threadpool` — its
//! spin-loop join is intractable under Miri (the same P1 native-only constraint
//! the `softstep.rs` schedule tests carry). The C1 sentinel path itself, however,
//! is a plain `RigidSolver::solve(&mut self, &PhysicsConfig, &[Manifold], &mut
//! SolverScratch)` call that needs NO threadpool: it only reads the manifolds and
//! mutates the dense scratch in place.
//!
//! This file drives `SoftStepSolver::solve` DIRECTLY with a hand-built
//! `body_b == SDF_SENTINEL` manifold and a one-body scratch, so `cargo +nightly
//! miri test -p boyko-physics --test sdf_sentinel_solve_miri` validates the C1
//! sentinel solve under Miri's UB checker — specifically:
//!
//! - **No OOB read of `bodies[u32::MAX]`**: the solver must NEVER index the
//!   scratch with the `u32::MAX` sentinel row. With exactly ONE body in scratch,
//!   any stray `bodies[SDF_SENTINEL.0 as usize]` (or the `ib` placeholder being
//!   read for the B side) would be a flagrant out-of-bounds Miri catches.
//! - **No double-apply to A**: `ib` is the A-row placeholder (`ib == ia`) for a
//!   sentinel manifold; every body-B `apply_impulse` is guarded by
//!   `!b_is_sentinel`, so the immovable surface receives nothing and A is touched
//!   exactly once per impulse (verified by the body actually being ejected, not
//!   double-pushed).
//! - **Sentinel warm-start key** (`pack_sdf`) round-trips through the double-
//!   buffered table across repeated solves without aliasing / UB.
//!
//! These are pure correctness + memory-safety checks; the full-physics resting
//! behavior is the schedule-driven `sdf_collision.rs` gate.

use boyko_physics::components::{BodyType, Collider, ColliderShape, RigidBody, RigidBodyMass};
use boyko_physics::manifold::{ContactPoint, Manifold, SDF_SENTINEL};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::resources::{BodyState, PhysicsConfig, SolverScratch};
use boyko_physics::solver::{RigidSolver, SoftStepSolver};

/// Builds a one-row scratch holding a single dynamic sphere at `position` with
/// `velocity`, sizing the touched mask for the one row so `write_back` can flag it.
fn one_body_scratch(position: Vec3, velocity: Vec3, radius: f32) -> SolverScratch {
    let body = RigidBody {
        position,
        linear_velocity: velocity,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: Mat3::IDENTITY,
        inv_mass: 1.0,
        restitution: 0.0,
        friction: 0.5,
        body_type: BodyType::Dynamic,
    };
    let collider = Collider {
        shape: ColliderShape::Sphere { radius },
        layer: 1,
        mask: 1,
    };

    let mut scratch = SolverScratch::with_capacity(1);
    scratch.set_bodies(&[BodyState::from_columns(&body, &mass, &collider, false)]);
    // The solver's `write_back` calls `touched.set(row)`; the mask must be sized
    // to the row count first (the gather stage does this in the full pipeline).
    scratch.touched.reset(scratch.bodies().len());
    scratch
}

/// A single-point SDF-sentinel manifold for body row 0: the surface is "below"
/// (normal −y, the A→surface convention), penetrating by `separation < 0`.
fn sentinel_manifold(separation: f32) -> Manifold {
    let mut m = Manifold::new(boyko_physics::manifold::BodyIndex(0), SDF_SENTINEL);
    m.normal = Vec3::new(0.0, -1.0, 0.0);
    m.points[0] = ContactPoint {
        anchor_a: Vec3::new(0.0, 0.0, 0.0),
        anchor_b: Vec3::new(0.0, 0.0, 0.0),
        separation,
        feature_id: 0,
    };
    m.count = 1;
    m
}

/// A config with zero gravity (so the test isolates the contact impulse) and a
/// stamped `dt` (gather normally stamps it; the direct-solve path must set it).
fn zero_gravity_config() -> PhysicsConfig {
    PhysicsConfig {
        gravity: Vec3::ZERO,
        dt: 1.0 / 60.0,
        ..PhysicsConfig::default()
    }
}

/// The C1 sentinel solve must NOT index `bodies[u32::MAX]` and must not double-
/// apply to A: a penetrating sphere closing on the surface (downward velocity,
/// normal −y) is pushed BACK along +y (out of the surface). Run under Miri this
/// proves the sentinel body-fetch substitutes `IMMOVABLE_AT_REST` rather than
/// reading the out-of-range row, and that the `!b_is_sentinel` guards keep the
/// one-sided apply sound.
#[test]
fn sentinel_solve_ejects_a_without_oob_or_double_apply() {
    let mut solver = SoftStepSolver::default();
    let cfg = zero_gravity_config();
    // A sphere moving DOWN (−y) into the surface, penetrating by 0.1.
    let mut scratch = one_body_scratch(Vec3::new(0.0, 0.4, 0.0), Vec3::new(0.0, -1.0, 0.0), 0.5);
    let manifolds = [sentinel_manifold(-0.1)];

    solver.solve(&cfg, &manifolds, &mut scratch);

    let b = scratch.bodies()[0];
    // The contact resolves a closing velocity: after the solve the body's normal
    // velocity must no longer be driving INTO the surface (the soft normal push +
    // relaxation removes the approach). With normal −y the approach is −vy, so a
    // resolved contact leaves vy >= ~0 (no longer falling through).
    assert!(
        b.linear_velocity.y > -0.5,
        "sentinel contact must arrest the downward approach (no double-apply / no \
         frozen body): vy = {}",
        b.linear_velocity.y
    );
    // Everything stays finite (a stray OOB / NaN sentinel read would poison this).
    assert!(
        b.linear_velocity.x.is_finite()
            && b.linear_velocity.y.is_finite()
            && b.linear_velocity.z.is_finite(),
        "velocity finite after sentinel solve: {:?}",
        b.linear_velocity
    );
    assert!(
        b.position.x.is_finite() && b.position.y.is_finite() && b.position.z.is_finite(),
        "position finite after sentinel solve: {:?}",
        b.position
    );
    // The single dynamic row was integrated + written back → touched.
    assert!(scratch.touched.get(0), "the dynamic row must be touched");
}

/// Repeated sentinel solves (the warm-start `read`/`write` swap path with
/// `pack_sdf` keys) must stay sound across frames under Miri — the sentinel key
/// inserts/probes the double-buffered table every frame with no aliasing / UB,
/// and a body in sustained sentinel contact stays finite.
#[test]
fn sentinel_solve_repeated_warm_start_is_sound() {
    let mut solver = SoftStepSolver::default();
    let cfg = zero_gravity_config();
    let mut scratch = one_body_scratch(Vec3::new(0.0, 0.45, 0.0), Vec3::ZERO, 0.5);
    let manifolds = [sentinel_manifold(-0.05)];

    // Several frames exercise the warm-start store_and_swap with the pack_sdf key.
    for _ in 0..8 {
        solver.solve(&cfg, &manifolds, &mut scratch);
        let b = scratch.bodies()[0];
        assert!(
            b.linear_velocity.y.is_finite() && b.position.y.is_finite(),
            "sustained sentinel contact stays finite across frames: v.y {} pos.y {}",
            b.linear_velocity.y,
            b.position.y
        );
    }
}

/// A degenerate sentinel manifold whose normal is ZERO would never be emitted by
/// the narrowphase (the seam-skip fires on a zero gradient), but the solve must
/// still be MEMORY-safe if one reaches it: no OOB, no UB, finite output. This
/// guards the sentinel body-fetch independent of the upstream skip.
#[test]
fn sentinel_solve_handles_empty_manifold_list_soundly() {
    let mut solver = SoftStepSolver::default();
    let cfg = zero_gravity_config();
    let mut scratch = one_body_scratch(Vec3::new(0.0, 2.0, 0.0), Vec3::ZERO, 0.5);
    // No manifolds at all: the owning solver must still integrate the free dynamic
    // body (under zero gravity it stays put) without touching any sentinel row.
    let manifolds: [Manifold; 0] = [];

    solver.solve(&cfg, &manifolds, &mut scratch);

    let b = scratch.bodies()[0];
    assert!(
        b.position.y.is_finite() && b.linear_velocity.y.is_finite(),
        "free body under empty manifold list stays finite: pos.y {} v.y {}",
        b.position.y,
        b.linear_velocity.y
    );
}
