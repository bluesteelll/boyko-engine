//! The fixed physics pipeline as ordinary systems (plan D3 / IM-1 / IM-2).
//!
//! The stages run in this deterministic order (pinned by `.after(...)` in
//! [`add_physics_systems`](crate::plugin::add_physics_systems)):
//!
//! 1. [`physics_integrate`] — `par_iter_mut`: gravity + `pos += vel·dt` +
//!    `rot = rot.integrate(angvel, dt)` (first-order quaternion advance). The
//!    only real-work stage in the foundation; a sound parallel pass over
//!    disjoint rows (each body writes only its own row).
//! 2. [`physics_gather`] — snapshots `(&RigidBody, &RigidBodyMass, &Collider)`
//!    IN ROW ORDER into the dense
//!    [`SolverScratch::bodies`](crate::resources::SolverScratch), derives each
//!    body's local + world inverse inertia, stamps the step `dt` into
//!    [`PhysicsConfig`], and resets the touched mask (the seam's gather
//!    boundary, IM-1 / OQ-1).
//! 3. [`physics_broadphase`] — fills [`ContactPairs`] with `(BodyIndex,
//!    BodyIndex)` candidate pairs in deterministic `(min, max)` order (D4); the
//!    foundation runs a real circle-circle-feasible all-pairs over the snapshot
//!    so the seam is exercised end-to-end (OQ1).
//! 4. [`physics_narrowphase`] — produces [`Manifold`]s (BodyIndex-keyed) into
//!    [`Manifolds`] for the overlapping pairs.
//! 5. [`physics_solve_step`] — `if solver.is_noop() { return }` else
//!    `S::solve(..)` (the swappable seam, D2).
//! 6. [`physics_apply`] — writes the solved snapshot back through
//!    `Mut<RigidBody>` for touched rows, under the "no structural change between
//!    gather and apply" invariant (IM-1).
//!
//! # Determinism precondition (IM-2)
//!
//! Pair/solve order keys on the dense [`BodyIndex`] = archetype row order, which
//! is deterministic across runs **only under a deterministic spawn/despawn
//! order** (the entity-id counter is a `Relaxed` atomic shared by parallel
//! `Commands` workers). The foundation's single-threaded tests satisfy this; a
//! content-defined ordering key independent of id/row is the Phase-10+ path if
//! parallel-spawn determinism is ever required.
//!
//! # Integration-ownership contract (C2 — authoritative)
//!
//! When the chosen solver returns `true` from
//! [`RigidSolver::owns_integration`](crate::solver::RigidSolver::owns_integration)
//! (the TGS [`SoftStepSolver`](crate::solver::SoftStepSolver)), the plugin
//! inserts [`IntegrationMode::SolverOwned`](crate::resources::IntegrationMode) and:
//!
//! 1. [`physics_integrate`] is gated OFF, so broad/narrowphase consume the
//!    **pre-integration (end-of-previous-frame) snapshot** — this is correct and
//!    intentional for TGS (it supersedes the foundation docstrings' "integrate
//!    then gather" ordering for the owning-solver mode; the solver re-projects and
//!    integrates internally).
//! 2. The solver integrates **DYNAMIC bodies only** (`body_type == Dynamic` /
//!    `inv_mass != 0`) inside its substep loop — mandatory: it applies a
//!    per-substep gravity bias, so a static floor would drift if it were
//!    integrated.
//! 3. **DO NOT un-gate** [`physics_integrate`] for an owning solver: running both
//!    would DOUBLE-INTEGRATE (the pipeline AND the solver each advance position +
//!    orientation in the same step), corrupting the simulation.

use boyko_ecs::ecs::core::iters::query::data::Mut;
use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_ecs::ecs::core::time::FixedTime;

use crate::components::{Collider, ColliderShape, RigidBody, RigidBodyMass};
use crate::manifold::{BodyIndex, ContactPoint, Manifold};
use crate::math::Vec3;
use crate::resources::{
    BodyState, ContactPairs, IntegrationMode, Manifolds, PhysicsConfig, SolverScratch,
};
use crate::solver::RigidSolver;

/// Integrates every body's hot state for one step (plan D3 stage 1), UNLESS the
/// solver owns integration (C2).
///
/// A sound `par_iter_mut` over disjoint rows: each body reads/writes only its
/// own [`RigidBody`]. Applies gravity to linear velocity, advances position by
/// the fixed `dt`, then advances orientation by integrating the quaternion
/// against the angular velocity.
///
/// # C2 integration-ownership gate
///
/// When [`IntegrationMode::SolverOwned`] is set (an owning TGS solver such as the
/// [`SoftStepSolver`](crate::solver::SoftStepSolver)), this stage EARLY-RETURNS:
/// the solver integrates DYNAMIC bodies inside its own substep loop, so running
/// this stage too would DOUBLE-INTEGRATE (see the C2 contract block in the module
/// docs). The stage stays monomorphic (it is NOT generic over the solver) — the
/// mode rides as a plain resource the plugin stamps from
/// `S::default().owns_integration()`.
//
// `clippy::needless_pass_by_value`: `Res<_>` is a by-value `SystemParam` by
// protocol (the param system delivers an owned guard; `&Res<_>` is not a valid
// param type). The body reads it via a `&*` reborrow, which clippy cannot
// credit, so the lint false-positives here. Same idiom as the demo's
// `integrate_balls`.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_integrate(
    mut query: Query<&mut RigidBody>,
    cfg: Res<PhysicsConfig>,
    dt: Res<FixedTime>,
    mode: Res<IntegrationMode>,
) {
    // C2 gate: an owning solver integrates inside its substep loop. DO NOT
    // un-gate this — running both the pipeline and the solver double-integrates.
    if *mode == IntegrationMode::SolverOwned {
        return;
    }
    let dt = dt.delta_secs();
    let gravity = cfg.gravity;
    query.par_iter_mut().for_each(move |body: &mut RigidBody| {
        body.linear_velocity = body.linear_velocity + gravity * dt;
        body.position = body.position + body.linear_velocity * dt;
        body.rotation = body.rotation.integrate(body.angular_velocity, dt);
    });
}

/// Snapshots every body into the dense, row-indexed solver scratch and stamps
/// the step `dt` (plan IM-1 / OQ-1, the gather boundary).
///
/// Walks the bodies in archetype-row order via `iter()` (the same order
/// [`physics_apply`] re-walks to write back), projecting the hot [`RigidBody`] +
/// cold [`RigidBodyMass`] + [`Collider`] columns into [`BodyState`] rows
/// (deriving each body's local + world inverse inertia from its shape, P2 W1).
/// The dense row index IS the [`BodyIndex`]. Resets the touched mask to the body
/// count. The snapshot `Vec`s are cleared and refilled, capacity reused (no
/// per-step alloc).
///
/// Before the per-body loop it stamps [`PhysicsConfig::dt`] from the fixed
/// clock ([`FixedTime::delta_secs`]) ONCE per gather (OQ-1): no separate `dt`
/// system, and the TGS solver later reads `h = dt / substeps`. The stamp is
/// gather-time so a hand-set `cfg.dt` is overwritten.
///
/// The row→entity projection (for the gameplay
/// [`Contact`](crate::components::Contact) producer) is NOT gathered in the
/// foundation: `Entity` is not a `QueryData` in the engine, so it is deferred to
/// the Phase-10 `Contact` producer (the only consumer) — see [`SolverScratch`].
//
// `clippy::needless_pass_by_value`: `ResMut<_>` / `Res<_>` are by-value
// `SystemParam`s mutated/read through reborrows — the same false-positive as the
// demo's `ResMut` systems.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_gather(
    query: Query<(&RigidBody, &RigidBodyMass, &Collider)>,
    mut scratch: ResMut<SolverScratch>,
    mut cfg: ResMut<PhysicsConfig>,
    fixed_time: Res<FixedTime>,
) {
    // OQ-1: stamp the fixed step delta once, before snapshotting, so the solver
    // reads a fresh `dt` each gather (deterministic — `FixedTime` is the
    // schedule's fixed clock).
    cfg.dt = fixed_time.delta_secs();

    let scratch = &mut *scratch;
    scratch.clear();
    // Read-only `iter()` walks the rows in archetype-row order — the same order
    // `physics_apply`'s mutable walk re-visits, so row `i` is the same body in
    // both passes (the IM-1 gather/apply addressing invariant).
    for (body, mass, collider) in query.iter() {
        scratch
            .bodies
            .push(BodyState::from_columns(body, mass, collider));
    }
    scratch.touched.reset(scratch.bodies.len());
}

/// Fills [`ContactPairs`] with candidate `(BodyIndex, BodyIndex)` pairs in
/// deterministic `(min, max)` order (plan D3 stage 2 / D4 / OQ1).
///
/// The foundation runs a real, broad-phase-feasible all-pairs over the gathered
/// snapshot: a pair is a candidate when the bodies' bounding circles overlap.
/// Emitting `(min, max)` keeps the order content-defined and reproducible
/// (float add is non-associative → contact iteration order must be
/// deterministic, D4). A real BVH/grid broadphase is Phase 10.
//
// `clippy::needless_pass_by_value`: see `physics_gather`.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_broadphase(scratch: Res<SolverScratch>, mut pairs: ResMut<ContactPairs>) {
    let bodies = &scratch.bodies;
    let pairs = &mut pairs.pairs;
    pairs.clear();

    let n = bodies.len();
    for i in 0..n {
        for j in (i + 1)..n {
            // Broad-phase overlap test on the bounding radii (the foundation's
            // cheap proxy; a real broadphase uses a spatial structure).
            let bound = body_bounding_radius(&bodies[i]) + body_bounding_radius(&bodies[j]);
            let delta = bodies[j].position - bodies[i].position;
            if delta.length_squared() <= bound * bound {
                // `i < j` already, so `(i, j)` is `(min, max)` — emit in the
                // deterministic order D4 requires.
                pairs.push((BodyIndex(i as u32), BodyIndex(j as u32)));
            }
        }
    }

    debug_assert!(
        pairs.windows(2).all(|w| w[0] <= w[1]),
        "invariant: broadphase pairs must be emitted in sorted (min, max) order"
    );
}

/// Produces a [`Manifold`] for each overlapping pair into [`Manifolds`]
/// (plan D3 stage 3).
///
/// A real sphere-sphere narrowphase over the candidate pairs: for an
/// overlapping pair it emits a single contact point (`count = 1`; the capacity
/// is 4 but spheres need only one) with the separation along the
/// center-to-center 3D normal (the foundation's end-to-end seam exercise — the
/// no-op solver still ignores these manifolds). A full box-box 4-point clipper
/// is Phase-10+ work. Manifolds are BodyIndex-keyed (IM-1). The buffer is
/// cleared and refilled, capacity reused.
//
// `clippy::needless_pass_by_value`: see `physics_gather`.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_narrowphase(
    scratch: Res<SolverScratch>,
    pairs: Res<ContactPairs>,
    mut manifolds: ResMut<Manifolds>,
) {
    let bodies = &scratch.bodies;
    let out = &mut manifolds.manifolds;
    out.clear();

    for &(a, b) in &pairs.pairs {
        let ia = a.0 as usize;
        let ib = b.0 as usize;
        let (ra, rb) = (collider_radius(&bodies[ia]), collider_radius(&bodies[ib]));
        let delta = bodies[ib].position - bodies[ia].position;
        let dist = delta.length();
        let separation = dist - (ra + rb);
        if separation >= 0.0 {
            // Bounding-circle overlap without an actual shape contact — skip.
            continue;
        }
        let normal = if dist > f32::MIN_POSITIVE {
            delta * dist.recip()
        } else {
            // Coincident centers: pick a stable arbitrary normal.
            Vec3::new(1.0, 0.0, 0.0)
        };
        let contact = bodies[ia].position + normal * ra;
        let mut manifold = Manifold::new(a, b);
        manifold.normal = normal;
        manifold.points[0] = ContactPoint {
            anchor_a: contact,
            anchor_b: contact,
            separation,
            feature_id: 0,
        };
        manifold.count = 1;
        debug_assert!(
            (manifold.count as usize) <= crate::math::MAX_CONTACT_POINTS,
            "invariant: manifold.count must not exceed MAX_CONTACT_POINTS"
        );
        out.push(manifold);
    }
}

/// Runs the swappable solver for one step, or early-outs for a no-op solver
/// (plan D3 stage 4 / D2).
///
/// Monomorphized over `S: RigidSolver` — `S::solve` is a direct, inlinable call
/// (zero vtable, principle 1). A no-op solver (the foundation default
/// [`NoopSolver`](crate::solver::NoopSolver)) returns before touching the
/// scratch/manifolds (the 0%-gate). A real solver mutates `scratch.bodies` in
/// place and flags `scratch.touched`.
//
// `clippy::needless_pass_by_value`: `ResMut<S>` / `Res<_>` are by-value
// `SystemParam`s; the body uses them through reborrows.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_solve_step<S: RigidSolver>(
    mut solver: ResMut<S>,
    cfg: Res<PhysicsConfig>,
    manifolds: Res<Manifolds>,
    mut scratch: ResMut<SolverScratch>,
) {
    if solver.is_noop() {
        return;
    }
    solver.solve(&cfg, &manifolds.manifolds, &mut scratch);
}

/// Writes the solved snapshot back into the [`RigidBody`] column for touched
/// rows (plan D3 stage 5 / IM-1).
///
/// Re-walks the body rows in the same order [`physics_gather`] snapshotted them
/// (`iter_mut().enumerate()` → row index `i` = [`BodyIndex`]). For each touched
/// row the whole [`RigidBody`] is written back through the [`Mut`] guard, so
/// `Changed<RigidBody>` fires for moving bodies (MINOR-2: a documented
/// whole-body choice; a later refinement may split position/velocity).
///
/// Correct UNDER the "no structural change between gather and apply" invariant:
/// the entire pipeline runs within one schedule pass and no stage spawns or
/// despawns, so row `i` is the same body in both passes. A user inserting a
/// structural command mid-pipeline is a documented misuse, caught here by the
/// `debug_assert!`.
//
// `clippy::needless_pass_by_value`: `Res<_>` is a by-value `SystemParam` read
// via a `&*` reborrow — the same false-positive as the demo's `apply_ball_motion`.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_apply(mut query: Query<Mut<RigidBody>>, scratch: Res<SolverScratch>) {
    let scratch = &*scratch;
    let mut row = 0usize;
    for mut body in query.iter_mut() {
        // Guard the index (never break) so `row` counts EVERY live row to the
        // end: this lets the `debug_assert!` below catch a desync in BOTH
        // directions — a despawn (live rows < snapshot) ends with `row < len`,
        // a spawn (live rows > snapshot) ends with `row > len`. Breaking on
        // overflow would mask the spawn case (`row` would stop exactly at `len`).
        if row < scratch.bodies.len() && scratch.touched.get(row) {
            let state = &scratch.bodies[row];
            // Deref-write through the `Mut` guard bumps the row's `changed`
            // tick exactly for solved bodies (precise change detection).
            *body = RigidBody {
                position: state.position,
                linear_velocity: state.linear_velocity,
                rotation: state.rotation,
                angular_velocity: state.angular_velocity,
            };
            // Note: the field set/order of `RigidBody` and `BodyState` differ
            // (the hot column vs the gathered SoA snapshot), so this is an
            // explicit field projection, not a `*body = *state`.
        }
        row += 1;
    }
    debug_assert!(
        row == scratch.bodies.len(),
        "invariant: no structural change between gather and apply (live row count {} != snapshot len {})",
        row,
        scratch.bodies.len()
    );
}

/// Broad-phase bounding radius of a body, computed from its real shape (P2 W2).
///
/// The smallest sphere enclosing the collider: a [`Sphere`](ColliderShape::Sphere)
/// contributes its radius directly; an [`Aabb`](ColliderShape::Aabb) contributes
/// the length of its half-extents diagonal (`half_extents.length()`), so a box
/// body is still a correct broadphase CANDIDATE even though box↔box narrowphase
/// is W4. Kept separate from [`collider_radius`] so the broadphase proxy and the
/// narrowphase shape can diverge later without churning callers.
#[inline]
fn body_bounding_radius(body: &BodyState) -> f32 {
    match body.shape {
        ColliderShape::Sphere { radius } => radius,
        ColliderShape::Aabb { half_extents } => half_extents.length(),
    }
}

/// Narrow-phase sphere radius of a body for the foundation's sphere-sphere test
/// (P2 W2).
///
/// A [`Sphere`](ColliderShape::Sphere) contributes its real radius. An
/// [`Aabb`](ColliderShape::Aabb) has no sphere-sphere contact — box↔box uses an
/// SAT clipper (W4) — so this returns a NEGATIVE sentinel: with a non-positive
/// narrowphase radius the `separation = dist − (rA + rB)` test in
/// [`physics_narrowphase`] cannot drop below zero for a box pair, so the W2
/// narrowphase skips it (the broadphase still treats the box as a candidate via
/// its real [`body_bounding_radius`]). Spheres keep generating genuine contacts.
#[inline]
fn collider_radius(body: &BodyState) -> f32 {
    match body.shape {
        ColliderShape::Sphere { radius } => radius,
        // No sphere-sphere contact for a box (W4 owns box contacts); the
        // sentinel forces `separation >= 0` so the narrowphase skips this pair.
        ColliderShape::Aabb { .. } => NO_SPHERE_CONTACT_RADIUS,
    }
}

/// Sentinel narrowphase radius for a non-sphere shape (P2 W2): a large negative
/// value so `separation = dist − (rA + rB)` cannot go below zero for any pair
/// involving a box, making the sphere-sphere narrowphase skip it. Box contacts
/// land in W4 (SAT).
const NO_SPHERE_CONTACT_RADIUS: f32 = f32::MIN;
