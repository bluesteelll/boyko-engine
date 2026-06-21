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
//! [`RigidSolver::owns_integration`]
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

use crate::components::{BodyType, Collider, ColliderShape, RigidBody, RigidBodyMass, Sensor};
use crate::manifold::{BodyIndex, ContactPoint, Manifold, SDF_SENTINEL};
use crate::math::Vec3;
use crate::narrowphase::box_box::box_box_contact;
use crate::narrowphase::feature_vertex_face;
use crate::narrowphase::sphere_box::sphere_box_contact;
use crate::resources::{
    BodyState, BroadphaseGrid, BroadphaseKind, ConstraintGraph, ContactPairs, IntegrationMode,
    IslandSleep, Manifolds, PhysicsConfig, SolverScratch,
};
use crate::sdf_query::{SdfField, sample_sdf};
use crate::solver::colored::ColoredSoftStepSolver;
use crate::solver::contact::is_dynamic_row;
use crate::solver::RigidSolver;

/// Minimum SDF gradient length for a usable contact normal (P2 W5, O3).
///
/// The analytic field gradient collapses toward zero on a CSG seam (where two
/// surfaces meet and the smooth-min/-max blend cancels) — a contact there has no
/// usable normal direction. The SDF narrowphase SKIPS a sample whose central-
/// difference gradient is shorter than this (the leaf normalizes such a gradient to
/// [`Vec3::ZERO`](crate::math::Vec3::ZERO)), so a degenerate-normal contact is never
/// emitted. Far above FP noise, far below a real surface gradient (≈ 1).
const SDF_NORMAL_EPS: f32 = 1.0e-4;

/// Integrates the DYNAMIC bodies' hot state for one step (plan D3 stage 1),
/// UNLESS the solver owns integration (C2).
///
/// A sound `par_iter_mut` over disjoint rows: each body reads/writes only its
/// own [`RigidBody`]. For a DYNAMIC body (`body_type == Dynamic && inv_mass !=
/// 0`) it applies gravity to linear velocity, advances position by the fixed
/// `dt`, then advances orientation by integrating the quaternion against the
/// angular velocity. Static / Kinematic bodies are SKIPPED (their pose is
/// gameplay-owned — see the body-type gate below).
///
/// # Body-type gate (single-source-of-truth with scene sync, std-lib S5)
///
/// This stage keys per-body integration on `BodyType` exactly as the
/// solver-owned integrator does (`body_type == Dynamic && inv_mass != 0`, the
/// mass test routed through [`is_dynamic_row`](crate::solver::contact) so the two
/// sites cannot drift). Integrating a Static / Kinematic body here would advance
/// it under gravity, which — once
/// [`sync_transform_to_body`](crate::scene_sync::sync_transform_to_body) makes a
/// Static / Kinematic `RigidBody.position` load-bearing (it copies the authored
/// `Transform` IN before this stage) — would drift the gameplay-authored pose
/// within one fixed window, then be snapshotted by the gather. Gating to Dynamic
/// keeps the Foundation-mode integrate's per-body set identical to the
/// solver-owned mode's, so the scene-sync pose contract holds for ANY solver
/// (owning or not — including a Foundation-mode `NoopSolver`).
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
    mut query: Query<(&mut RigidBody, &RigidBodyMass)>,
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
    query
        .par_iter_mut()
        .for_each(move |(body, mass): (&mut RigidBody, &RigidBodyMass)| {
            // S5 / C2 parity: integrate ONLY the dynamic bodies, exactly the set
            // the solver-owned integrator advances. A Static / Kinematic body's
            // pose is gameplay-authored (copied IN by `sync_transform_to_body`)
            // and must not drift under gravity. `is_dynamic_row` is the SAME
            // inv-mass predicate the coloring / solve write guards route through,
            // so the integrate set cannot drift from the solve set.
            if mass.body_type != BodyType::Dynamic || !is_dynamic_row(mass.inv_mass) {
                return;
            }
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
    query: Query<(&RigidBody, &RigidBodyMass, &Collider, Option<&Sensor>)>,
    mut scratch: ResMut<SolverScratch>,
    mut cfg: ResMut<PhysicsConfig>,
    fixed_time: Res<FixedTime>,
) {
    // OQ-1: stamp the fixed step delta once, before snapshotting, so the solver
    // reads a fresh `dt` each gather (deterministic — `FixedTime` is the
    // schedule's fixed clock).
    cfg.dt = fixed_time.delta_secs();

    let scratch = &mut *scratch;
    scratch.vn_initial.clear();
    // Refill the gather column through its single-threaded build view: clear (no
    // free — the committed pages stay resident) then push one BodyState per row.
    // Read-only `iter()` walks the rows in archetype-row order — the same order
    // `physics_apply`'s mutable walk re-visits, so row `i` is the same body in
    // both passes (the IM-1 gather/apply addressing invariant).
    //
    // S5: `Option<&Sensor>` is a NON-filtering query datum — it yields `Some` for
    // a sensor body and `None` otherwise, so the gathered row count and order are
    // identical to a world with no `Sensor` id (the 0%-gate; `is_sensor` is just
    // `false` everywhere there). The bit rides into `BodyState` for the
    // narrowphase to read.
    let mut bodies = scratch.bodies_build();
    bodies.clear();
    for (body, mass, collider, sensor) in query.iter() {
        bodies.push(BodyState::from_columns(body, mass, collider, sensor.is_some()));
    }
    let n = bodies.len();
    scratch.touched.reset(n);
}

/// Fills [`ContactPairs`] with candidate `(BodyIndex, BodyIndex)` pairs in
/// deterministic `(min, max)` order (plan D3 stage 2 / D4 / OQ1; O2 grid path).
///
/// A pair is a candidate when the bodies' bounding spheres overlap
/// (`delta.length_squared() <= (rA + rB)²`). Emitting `(min, max)` keeps the
/// order content-defined and reproducible (float add is non-associative →
/// contact iteration order must be deterministic, D4).
///
/// Two interchangeable paths, selected by [`PhysicsConfig::broadphase`] (a single
/// runtime branch — the one-branch floor):
///
/// - [`BroadphaseKind::AllPairs`] (DEFAULT): the shipped O(n²) double loop,
///   byte-identical to before O2 (the campaign 0%-gate).
/// - [`BroadphaseKind::Grid`] (opt-in, O2): a uniform-grid CSR counting-sort
///   ([`BroadphaseGrid::build`]) that emits candidates then applies the SAME
///   sphere-bound predicate and sorts by `(min, max)`. Its pair set is
///   bit-identical to all-pairs.
//
// `clippy::needless_pass_by_value`: see `physics_gather`.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_broadphase(
    scratch: Res<SolverScratch>,
    cfg: Res<PhysicsConfig>,
    mut grid: ResMut<BroadphaseGrid>,
    mut pairs: ResMut<ContactPairs>,
) {
    let bodies = scratch.bodies();
    let pairs = &mut pairs.pairs;

    match cfg.broadphase {
        // The shipped all-pairs loop, kept VERBATIM so the default path's asm is
        // byte-identical to before O2 (the 0%-gate). DO NOT refactor this arm.
        BroadphaseKind::AllPairs => {
            pairs.clear();
            let n = bodies.len();
            for i in 0..n {
                for j in (i + 1)..n {
                    // Broad-phase overlap test on the bounding radii.
                    let bound = body_bounding_radius(&bodies[i]) + body_bounding_radius(&bodies[j]);
                    let delta = bodies[j].position - bodies[i].position;
                    if delta.length_squared() <= bound * bound {
                        // `i < j` already, so `(i, j)` is `(min, max)`.
                        pairs.push((BodyIndex(i as u32), BodyIndex(j as u32)));
                    }
                }
            }
        }
        // O2/O3: the uniform-grid broadphase. `build` (and the O3
        // `build_parallel`) clear and refill `pairs` with the feasibility-filtered,
        // (min, max)-sorted candidate set — bit-identical to the all-pairs arm
        // above. `parallel_broadphase` selects the O3 parallel candidate-emit path
        // (the CSR build + oversized emit stay serial and byte-identical to O2; only
        // the per-cell emit is fanned across the ambient pool), whose pair MULTISET
        // matches the serial `build` for any worker count.
        BroadphaseKind::Grid => {
            if cfg.parallel_broadphase {
                grid.build_parallel(bodies, pairs);
            } else {
                grid.build(bodies, pairs);
            }
        }
    }

    debug_assert!(
        pairs.windows(2).all(|w| w[0] <= w[1]),
        "invariant: broadphase pairs must be emitted in sorted (min, max) order"
    );
}

/// Produces a [`Manifold`] for each overlapping pair into [`Manifolds`]
/// (plan D3 stage 3; P2 W4 convex dispatch).
///
/// Dispatches each candidate pair by the two bodies' [`ColliderShape`]s
/// (`BodyState.shape`) to the matching contact generator:
///
/// - **sphere-sphere**: inline single-point center-to-center contact (the W2
///   path).
/// - **sphere-box** / **box-sphere**: [`sphere_box_contact`] — a single
///   closest-point contact (the box is an OBB: position + body rotation +
///   half-extents).
/// - **box-box**: [`box_box_contact`] — 15-axis SAT + reference-face clip + a
///   deterministic ≤4-point reduction, biased by the per-pair reference-axis
///   hysteresis in [`Manifolds::box_axis_cache`] for stable feature ids on a
///   resting stack (P2 W3/W4).
///
/// Every emitted manifold is keyed by the dense `(a, b)` rows in `(min, max)`
/// order (IM-1 / D4) with its `normal` pointing A→B, regardless of which body was
/// the sphere or the SAT reference — so the solver's sign handling is uniform.
/// The buffer is cleared and refilled each step; the hysteresis cache persists in
/// place across frames (capacity reused).
//
// `clippy::needless_pass_by_value`: see `physics_gather`.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_narrowphase(
    scratch: Res<SolverScratch>,
    pairs: Res<ContactPairs>,
    mut manifolds: ResMut<Manifolds>,
) {
    let bodies = scratch.bodies();
    let manifolds = &mut *manifolds;
    manifolds.manifolds.clear();
    // S5: the sensor-overlap signal is rebuilt every step alongside the solver
    // buffer (capacity reused). Empty in any world with no `Sensor` id.
    manifolds.sensor_overlaps.clear();
    // Ensure the per-pair hysteresis cache can hold this frame's pairs; it is NOT
    // cleared (a single in-place table — this frame reads last frame's axes).
    manifolds.box_axis_cache.begin_frame(pairs.pairs.len());
    let out = &mut manifolds.manifolds;
    let sensor_out = &mut manifolds.sensor_overlaps;
    let axis_cache = &mut manifolds.box_axis_cache;

    for &(a, b) in &pairs.pairs {
        let ia = a.0 as usize;
        let ib = b.0 as usize;
        let ba = &bodies[ia];
        let bb = &bodies[ib];
        // S5: a pair where EITHER body is a sensor is an OVERLAP, not a contact —
        // the manifold is generated identically (same geometry) but diverted to
        // `sensor_overlaps` below so the solver never resolves it. Computed once
        // per pair; `false` for every pair in a sensor-free world (the 0%-gate:
        // the routing branch then always takes the `out` arm, byte-identical to
        // the pre-S5 push).
        let is_overlap = ba.is_sensor || bb.is_sensor;

        let manifold = match (ba.shape, bb.shape) {
            (ColliderShape::Sphere { radius: ra }, ColliderShape::Sphere { radius: rb }) => {
                sphere_sphere_manifold(a, b, ba, bb, ra, rb)
            }
            (ColliderShape::Sphere { radius }, ColliderShape::Box { half_extents }) => {
                // A is the sphere, B is the box: the generator already emits
                // normal A→B with body_a = sphere, body_b = box.
                sphere_box_contact(
                    a, b, ba.position, radius, bb.position, bb.rotation, half_extents,
                )
            }
            (ColliderShape::Box { half_extents }, ColliderShape::Sphere { radius }) => {
                // A is the box, B is the sphere: call the generator with the
                // sphere as A / box as B (keyed b, a), then remap to (a, b) order
                // so the dense rows match and the normal runs A(box)→B(sphere).
                sphere_box_contact(
                    b, a, bb.position, radius, ba.position, ba.rotation, half_extents,
                )
                .map(flip_manifold)
            }
            (
                ColliderShape::Box { half_extents: ha },
                ColliderShape::Box { half_extents: hb },
            ) => {
                let last_axis = axis_cache.get(a, b);
                box_box_contact(
                    a, b, ba.position, ba.rotation, ha, bb.position, bb.rotation, hb, last_axis,
                )
                .map(|c| {
                    // Persist this frame's chosen reference axis for next frame's
                    // hysteresis bias (per body pair, deterministic).
                    axis_cache.set(a, b, c.reference_axis);
                    c.manifold
                })
            }
        };

        if let Some(manifold) = manifold {
            debug_assert!(
                (manifold.count as usize) <= crate::math::MAX_CONTACT_POINTS,
                "invariant: manifold.count must not exceed MAX_CONTACT_POINTS"
            );
            if manifold.count > 0 {
                // S5: divert a sensor-pair overlap to the report buffer so the
                // solver never sees it; a normal pair takes the unchanged `out`
                // push (the 0%-gate — `is_overlap` is always `false` with no
                // `Sensor` id).
                if is_overlap {
                    sensor_out.push(manifold);
                } else {
                    out.push(manifold);
                }
            }
        }
    }
}

/// Builds the single-point sphere-sphere manifold for the dense pair `(a, b)`, or
/// `None` when the spheres do not overlap (the W2 path, kept inline).
///
/// The normal runs A→B along the center-to-center direction; the lone contact
/// point sits on A's surface. `feature_id` is `0` (a sphere has no distinguishing
/// feature — its class is disjoint from the box feature-id classes, which all set
/// the high bit).
#[inline]
fn sphere_sphere_manifold(
    a: BodyIndex,
    b: BodyIndex,
    ba: &BodyState,
    bb: &BodyState,
    ra: f32,
    rb: f32,
) -> Option<Manifold> {
    let delta = bb.position - ba.position;
    let dist = delta.length();
    let separation = dist - (ra + rb);
    if separation >= 0.0 {
        // Bounding-circle overlap without an actual shape contact.
        return None;
    }
    let normal = if dist > f32::MIN_POSITIVE {
        delta * dist.recip()
    } else {
        // Coincident centers: pick a stable arbitrary normal.
        Vec3::new(1.0, 0.0, 0.0)
    };
    let contact = ba.position + normal * ra;
    let mut manifold = Manifold::new(a, b);
    manifold.normal = normal;
    manifold.points[0] = ContactPoint {
        anchor_a: contact,
        anchor_b: contact,
        separation,
        feature_id: 0,
    };
    manifold.count = 1;
    Some(manifold)
}

/// Swaps a manifold's A/B roles: exchanges `body_a`/`body_b`, swaps each point's
/// `anchor_a`/`anchor_b`, and negates the normal so it still runs from the (new)
/// A toward the (new) B (P2 W4).
///
/// Used to remap a box-sphere pair: [`sphere_box_contact`] always keys the sphere
/// as A and the box as B, but the dense pair order is `(min, max)` by row, so when
/// the box is the lower row the generated manifold must be flipped back to `(box,
/// sphere)` = `(a, b)` order. `feature_id` / `separation` / `count` are unchanged
/// (they are role-symmetric).
#[inline]
fn flip_manifold(mut m: Manifold) -> Manifold {
    core::mem::swap(&mut m.body_a, &mut m.body_b);
    m.normal = m.normal * -1.0;
    for p in &mut m.points[..m.count as usize] {
        core::mem::swap(&mut p.anchor_a, &mut p.anchor_b);
    }
    m
}

/// Generates body-vs-SDF contacts against the analytic field and APPENDS them to
/// the same [`Manifolds`] buffer the body-body narrowphase fills (plan D3 SDF
/// stage; P2 W5 / C1).
///
/// For each DYNAMIC body (in dense row order, so the emission is deterministic) it
/// samples [`SdfField`] via [`sample_sdf`] and, where the body penetrates the
/// field, emits a [`Manifold`] keyed `body_a = `the body's dense row,
/// `body_b = `[`SDF_SENTINEL`] (the C1 sentinel — the solver treats body B as an
/// immovable wall, never indexing `bodies[u32::MAX]`).
///
/// # Normal convention
///
/// The field GRADIENT points outward (surface → body A). The manifold `normal`,
/// however, follows the solver's uniform `A → B` convention (body A = the real
/// body, body B = the immovable surface), so it is the gradient NEGATED (`A →
/// surface`). With that convention the solver's one-sided impulse — which pushes A
/// along `-normal` — pushes A AWAY from the surface (out of penetration), with NO
/// special-casing of the impulse sign (the C1 "rides the existing one-sided path"
/// requirement). `separation` stays `d − radius` / `d` (negative = penetrating),
/// independent of the normal-direction sign.
///
/// - **Sphere** `{ radius }`: samples the center. If `d − radius < 0` (the sphere
///   overlaps the field) it emits ONE contact with `separation = d − radius`,
///   `normal = −gradient` (A → surface), anchor `= center + normal·radius` (the
///   sphere's surface point nearest the field), `feature_id = 0`.
/// - **Box** `{ half_extents }`: samples the 8 world-OBB corners; every penetrating
///   corner (`d < 0`) is a candidate contact with `separation = d`,
///   `normal = −gradient`, anchor = the corner, and a stable per-corner
///   `feature_id`. The deepest ≤4 corners are kept (ties by lowest corner index)
///   so a box manifold never exceeds [`MAX_CONTACT_POINTS`](crate::math::MAX_CONTACT_POINTS).
///
/// A sample whose gradient is shorter than [`SDF_NORMAL_EPS`] (the CSG-seam
/// degeneracy, O3 — the leaf normalizes it to `Vec3::ZERO`) is SKIPPED: a
/// zero normal has no usable direction, so no contact is emitted.
///
/// Registered AFTER [`physics_narrowphase`] (body-body) and BEFORE
/// [`physics_solve_step`] (see [`add_physics_sdf`](crate::plugin::add_physics_sdf)),
/// so the solver sees both contact kinds. This stage does NOT clear `Manifolds`
/// (the body-body stage already cleared it this step); it only appends.
//
// `clippy::needless_pass_by_value`: see `physics_gather`.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_narrowphase_sdf(
    scratch: Res<SolverScratch>,
    field: Res<SdfField>,
    mut manifolds: ResMut<Manifolds>,
) {
    // Nothing to collide against an empty field (samples to +far everywhere).
    if field.is_empty() {
        return;
    }
    let bodies = scratch.bodies();
    let manifolds = &mut *manifolds;
    let out = &mut manifolds.manifolds;
    let sensor_out = &mut manifolds.sensor_overlaps;

    for (row, body) in bodies.iter().enumerate() {
        // Only DYNAMIC bodies collide against the SDF (a static/kinematic body's
        // contact with an immovable field would be two immovable sides — no
        // response; it also keeps the sentinel one-sided path exercised by a real
        // moving body, matching the body-vs-static-floor convention).
        if body.body_type != crate::components::BodyType::Dynamic || body.inv_mass == 0.0 {
            continue;
        }
        let a = BodyIndex(row as u32);
        // S5: a sensor body's SDF overlap is reported, not resolved — divert it to
        // the overlap buffer so the solver's one-sided wall push never fires on it
        // (the 0%-gate: `is_sensor` is `false` for every body in a sensor-free
        // world, so this always takes the `out` arm — byte-identical to pre-S5).
        let dst: &mut Vec<Manifold> = if body.is_sensor { sensor_out } else { out };
        match body.shape {
            ColliderShape::Sphere { radius } => {
                if let Some(m) = sphere_sdf_manifold(a, body, radius, &field) {
                    dst.push(m);
                }
            }
            ColliderShape::Box { half_extents } => {
                if let Some(m) = box_sdf_manifold(a, body, half_extents, &field) {
                    dst.push(m);
                }
            }
        }
    }
}

/// Builds the single-point sphere-vs-SDF manifold for the dense row `a`, or `None`
/// when the sphere does not penetrate the field / the contact normal is degenerate
/// (P2 W5).
///
/// Samples the field at the sphere center: the sphere penetrates when the center's
/// signed distance minus the radius is negative. The manifold normal is the field
/// gradient NEGATED (the solver's `A → surface` convention, so the one-sided push
/// ejects A from the surface), the anchor is the sphere's surface point nearest the
/// field (`center − gradient·radius == center + normal·radius`), and the lone
/// point's `feature_id` is `0`.
///
/// # Edge case (W5): center exactly at a field critical point
///
/// A sphere whose center sits exactly at a field critical point (a primitive center
/// under deep penetration, a subtract/smooth-blend interior saddle) has a
/// zero-length gradient: the C1 seam-skip fires and NO contact is emitted that
/// frame, so such a sphere is not pushed out THAT frame (consistent with the skip
/// — a zero normal has no usable direction). The common case (a sphere resting on
/// an SDF floor) never reaches a critical point. Read the skip as "no usable
/// direction this frame", not "always resolvable".
#[inline]
fn sphere_sdf_manifold(
    a: BodyIndex,
    body: &BodyState,
    radius: f32,
    field: &SdfField,
) -> Option<Manifold> {
    let center = body.position;
    let (d, gradient) = sample_sdf(field, center);
    let separation = d - radius;
    if separation >= 0.0 {
        // The sphere's surface clears the field — no contact.
        return None;
    }
    // O3: a degenerate (zero-length) gradient — the leaf normalizes a CSG-seam
    // gradient to ZERO — has no usable normal direction; skip it. The
    // `!is_finite()` arm is defense-in-depth: even if a non-finite gradient
    // reached here from any source, `NaN < eps²` is `false`, so without it the
    // skip would not fire and a NaN normal would poison the solver.
    if gradient.length_squared() < SDF_NORMAL_EPS * SDF_NORMAL_EPS || !gradient.is_finite() {
        return None;
    }
    // A → B normal (B = the surface): the gradient (surface → A) negated, so the
    // solver's `-P` on A pushes it out along the gradient (away from the surface).
    let normal = gradient * -1.0;
    // The sphere surface point nearest the field = center along the gradient by the
    // radius = `center + normal·radius` (normal == −gradient).
    let anchor = center + normal * radius;
    let mut m = Manifold::new(a, SDF_SENTINEL);
    m.normal = normal;
    m.points[0] = ContactPoint {
        anchor_a: anchor,
        // body B is the immovable SDF surface; its anchor mirrors A's (role
        // symmetric, the solver derives A's lever arm from `anchor − position`).
        anchor_b: anchor,
        separation,
        feature_id: 0,
    };
    m.count = 1;
    Some(m)
}

/// Builds the box-vs-SDF manifold for the dense row `a` by sampling the 8 world-OBB
/// corners, keeping the deepest ≤4 penetrating corners (P2 W5).
///
/// Each corner is the body position plus the rotated local half-extent sign vector;
/// a corner penetrates when its signed distance is negative. Penetrating corners
/// with a usable (non-degenerate) gradient become contacts (`separation = d`,
/// `normal = −gradient` — A → surface, matching the sphere path + the code so the
/// one-sided push ejects A, anchor = the corner, a per-corner `feature_id`). The
/// deepest ≤4 are kept — selected by an insertion that breaks ties by the lowest
/// corner index, so the reduction is deterministic — keeping the manifold within
/// [`MAX_CONTACT_POINTS`](crate::math::MAX_CONTACT_POINTS).
///
/// Returns `None` when no corner penetrates (with a usable normal).
///
/// # Accepted limitation (W5): single header normal on a non-planar SDF
///
/// The [`Manifold`](crate::resources::Manifold) carries ONE header `normal` for
/// all ≤4 points (the deepest corner's `−gradient`, the Box2D one-normal design),
/// while each corner samples its OWN per-corner gradient. On a near-planar SDF (a
/// box resting on an SDF floor/incline — the W5 gates) every corner shares one
/// direction, so this is exact. On a NON-planar SDF (a box straddling a CSG seam)
/// the shallower corners inherit the deepest corner's direction, which is only
/// approximate. Per-point SDF normals would need a different manifold shape and are
/// DEFERRED (see `docs/PHYSICS-P2-PLAN.md`, W5 / Reserved).
#[inline]
fn box_sdf_manifold(
    a: BodyIndex,
    body: &BodyState,
    half_extents: Vec3,
    field: &SdfField,
) -> Option<Manifold> {
    let max_points = crate::math::MAX_CONTACT_POINTS;
    // The kept contacts, deepest-first (most negative separation). Fixed capacity,
    // no allocation: at most `MAX_CONTACT_POINTS` are retained.
    let mut kept: [(ContactPoint, Vec3); crate::math::MAX_CONTACT_POINTS] =
        [(ContactPoint::default(), Vec3::ZERO); crate::math::MAX_CONTACT_POINTS];
    let mut kept_len = 0usize;

    // ── O9 AVX2 batched arm (a `+avx2` build, never under Miri) ──────────────────
    // Builds the 8 OBB corners' distances with ONE `sdf_edit_list_x8`, then each
    // penetrating corner's central-difference gradient with ONE 6-offset
    // `sdf_edit_list_x8` batch. The x8 kernel is bit-identical lane-for-lane to the
    // scalar `sdf_edit_list`, the gradient differences are taken in the SAME order
    // as `sdf_edit_list_normal`, and the FROZEN scalar `v_normalize` is reused — so
    // this arm is `f32::to_bits`-identical to the scalar arm below.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(miri)))]
    {
        box_sdf_manifold_avx2(
            body,
            half_extents,
            field,
            &mut kept,
            &mut kept_len,
            max_points,
        );
    }

    // ── Scalar reference arm (default / non-AVX2 / Miri build) ───────────────────
    // The VERBATIM frozen narrowphase loop — the bit-oracle the AVX2 arm mirrors.
    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2", not(miri))))]
    {
        // The 8 corners in a FIXED order (corner index = the 3-bit sign pattern), so
        // both the feature ids and the tie-breaking are deterministic.
        for corner in 0u32..8 {
            let sx = if corner & 1 != 0 { 1.0 } else { -1.0 };
            let sy = if corner & 2 != 0 { 1.0 } else { -1.0 };
            let sz = if corner & 4 != 0 { 1.0 } else { -1.0 };
            let local = half_extents.componentwise_mul(Vec3::new(sx, sy, sz));
            let world = body.position + body.rotation.rotate(local);

            let (d, gradient) = sample_sdf(field, world);
            if d >= 0.0 {
                continue;
            }
            // O3: skip a degenerate (zero-length) seam gradient — no usable normal.
            // The `!is_finite()` arm is defense-in-depth (mirrors the sphere path):
            // `NaN < eps²` is `false`, so without it a non-finite gradient would slip
            // through and emit a NaN-normal contact.
            if gradient.length_squared() < SDF_NORMAL_EPS * SDF_NORMAL_EPS || !gradient.is_finite() {
                continue;
            }
            // A → B normal (B = the surface): the gradient (surface → A) negated.
            let normal = gradient * -1.0;
            let point = ContactPoint {
                anchor_a: world,
                anchor_b: world,
                separation: d,
                // A vertex-vs-field contact — tag it as the vertex-face class keyed by
                // the corner index, so each corner warm-starts independently and a
                // corner id never aliases a body-body box face/edge id.
                feature_id: feature_vertex_face(corner),
            };
            insert_deepest(&mut kept, &mut kept_len, max_points, point, normal);
        }
    }

    if kept_len == 0 {
        return None;
    }
    // The manifold normal is the deepest corner's gradient (the dominant contact
    // direction); each point still carries its own anchor + separation. (W5 box-SDF
    // resting is a vertex-on-surface case; per-point normals are a future refinement
    // — the shared header normal matches the existing `Manifold` shape.)
    let mut m = Manifold::new(a, SDF_SENTINEL);
    m.normal = kept[0].1;
    for (i, (point, _)) in kept[..kept_len].iter().enumerate() {
        m.points[i] = *point;
    }
    m.count = kept_len as u8;
    Some(m)
}

/// O9 — the AVX2 batched body of [`box_sdf_manifold`]: fills `kept` / `kept_len`
/// with the penetrating corners' contacts, `f32::to_bits`-identical to the scalar
/// arm.
///
/// Two batched passes share the one [`sdf_edit_list_x8`](crate::sdf_simd::sdf_edit_list_x8)
/// kernel:
///
/// 1. **Distances**: the 8 OBB corners (the SAME fixed sign-pattern order +
///    `position + rotation·local` world transform as the scalar arm) are evaluated
///    in ONE 8-wide call — lane `i` == the scalar `sdf_edit_list(edits, corner_i)`.
/// 2. **Gradient** (per penetrating corner): the 6 central-difference offset points
///    (`±GRAD_H` on x, y, z) are packed into lanes 0..6 in the order
///    `sdf_edit_list_normal` reads them (`+x, -x, +y, -y, +z, -z`); lanes 6,7 are
///    seeded with the corner's own (finite) world point (R5 — keep `(b-a)/k` finite;
///    those lanes are never read). One 8-wide call yields the 6 samples; the
///    gradient is `[g(+x)-g(-x), g(+y)-g(-y), g(+z)-g(-z)]` (the EXACT difference
///    order of `sdf_edit_list_normal`), then the FROZEN scalar `v_normalize` (the
///    bit-identical zero-length guard is REUSED, not re-emulated).
///
/// Everything downstream — the `d >= 0` skip, the `length_squared < eps² ||
/// !is_finite` seam-skip, the `−gradient` normal, the [`ContactPoint`] /
/// [`feature_vertex_face`] / [`insert_deepest`] build — is byte-identical to the
/// scalar arm.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(miri)))]
fn box_sdf_manifold_avx2(
    body: &BodyState,
    half_extents: Vec3,
    field: &SdfField,
    kept: &mut [(ContactPoint, Vec3); crate::math::MAX_CONTACT_POINTS],
    kept_len: &mut usize,
    max_points: usize,
) {
    use core::arch::x86_64::{_mm256_loadu_ps, _mm256_storeu_ps};

    use boyko_sdf_math::{SDF_GRAD_H, v_normalize};

    use crate::sdf_simd::sdf_edit_list_x8;

    let edits = field.edits();

    // ── Build the 8 corners (same order + transform as the scalar arm) ───────────
    let mut world: [Vec3; 8] = [Vec3::ZERO; 8];
    let mut cx = [0.0f32; 8];
    let mut cy = [0.0f32; 8];
    let mut cz = [0.0f32; 8];
    for corner in 0usize..8 {
        let sx = if corner & 1 != 0 { 1.0 } else { -1.0 };
        let sy = if corner & 2 != 0 { 1.0 } else { -1.0 };
        let sz = if corner & 4 != 0 { 1.0 } else { -1.0 };
        let local = half_extents.componentwise_mul(Vec3::new(sx, sy, sz));
        let w = body.position + body.rotation.rotate(local);
        world[corner] = w;
        cx[corner] = w.x;
        cy[corner] = w.y;
        cz[corner] = w.z;
    }

    // ── Pass 1: the 8 corner distances in ONE 8-wide evaluation ──────────────────
    let mut dist = [0.0f32; 8];
    // SAFETY: the AVX2 `cfg` + `target_feature` gate guarantees AVX2 is present on
    //   the executing CPU (a non-AVX2 host cannot link this arm). `sdf_edit_list_x8`
    //   is `#[target_feature(enable = "avx2")]`, so it must be called from an `unsafe`
    //   block on stable. Each load/store touches exactly 8 contiguous `f32` of an
    //   in-bounds `[f32; 8]` stack buffer (unaligned variants accept any alignment).
    unsafe {
        let px = _mm256_loadu_ps(cx.as_ptr());
        let py = _mm256_loadu_ps(cy.as_ptr());
        let pz = _mm256_loadu_ps(cz.as_ptr());
        let d = sdf_edit_list_x8(edits, px, py, pz);
        _mm256_storeu_ps(dist.as_mut_ptr(), d);
    }

    // ── Pass 2: per penetrating corner, the central-difference gradient ──────────
    let h = SDF_GRAD_H;
    for corner in 0usize..8 {
        let d = dist[corner];
        if d >= 0.0 {
            continue;
        }
        let w = world[corner];
        // Six offset points in `sdf_edit_list_normal`'s read order, lanes 0..6:
        //   0:+x 1:-x  2:+y 3:-y  4:+z 5:-z. Lanes 6,7 = the corner itself (finite
        //   seed, R5 — never read).
        let gx = [w.x + h, w.x - h, w.x, w.x, w.x, w.x, w.x, w.x];
        let gy = [w.y, w.y, w.y + h, w.y - h, w.y, w.y, w.y, w.y];
        let gz = [w.z, w.z, w.z, w.z, w.z + h, w.z - h, w.z, w.z];
        let mut g = [0.0f32; 8];
        // SAFETY: same AVX2-availability + `target_feature` invariant as pass 1;
        //   each load/store is 8 contiguous `f32` of an in-bounds `[f32; 8]` stack
        //   buffer (unaligned variants).
        unsafe {
            let px = _mm256_loadu_ps(gx.as_ptr());
            let py = _mm256_loadu_ps(gy.as_ptr());
            let pz = _mm256_loadu_ps(gz.as_ptr());
            let gd = sdf_edit_list_x8(edits, px, py, pz);
            _mm256_storeu_ps(g.as_mut_ptr(), gd);
        }
        // Central differences in the EXACT order of `sdf_edit_list_normal`
        // (lib.rs:384-388), then the FROZEN scalar `v_normalize` (zero-length guard
        // reused byte-identically).
        let n = v_normalize([g[0] - g[1], g[2] - g[3], g[4] - g[5]]);
        let gradient = Vec3::new(n[0], n[1], n[2]);
        // O3 seam-skip — byte-identical to the scalar arm.
        if gradient.length_squared() < SDF_NORMAL_EPS * SDF_NORMAL_EPS || !gradient.is_finite() {
            continue;
        }
        let normal = gradient * -1.0;
        let point = ContactPoint {
            anchor_a: w,
            anchor_b: w,
            separation: d,
            feature_id: feature_vertex_face(corner as u32),
        };
        insert_deepest(kept, kept_len, max_points, point, normal);
    }
}

/// Inserts `point` (with its gradient `normal`) into the deepest-first `kept`
/// buffer, keeping at most `cap` entries ordered by most-negative `separation`,
/// ties broken by the lower corner `feature_id` (P2 W5 — deterministic reduction).
///
/// A new point displaces the shallowest kept entry only when it is strictly deeper;
/// an equally-deep point with a lower `feature_id` wins the tie (the corners are
/// inserted in index order, so the first-inserted — lowest index — already holds
/// the slot, making the reduction order-stable without an explicit tie compare on
/// the displaced side).
#[inline]
fn insert_deepest(
    kept: &mut [(ContactPoint, Vec3)],
    kept_len: &mut usize,
    cap: usize,
    point: ContactPoint,
    normal: Vec3,
) {
    // Find the sorted insertion position (deepest = most-negative separation first).
    // Corners arrive in ascending index order, so a `<` (strict) compare keeps the
    // earlier (lower-index) corner ahead on a tie — the deterministic tie-break.
    let mut pos = *kept_len;
    while pos > 0 && point.separation < kept[pos - 1].0.separation {
        pos -= 1;
    }
    if pos >= cap {
        // Shallower than every kept entry and the buffer is full — drop it.
        return;
    }
    let end = (*kept_len).min(cap - 1);
    // Shift the entries at [pos, end) right by one to open the slot at `pos`.
    let mut i = end;
    while i > pos {
        kept[i] = kept[i - 1];
        i -= 1;
    }
    kept[pos] = (point, normal);
    if *kept_len < cap {
        *kept_len += 1;
    }
}

/// Builds the [`ConstraintGraph`] from this step's manifolds — constraint islands
/// + greedy graph coloring (plan O4, Decision 2 / Decision 7).
///
/// Registered ONLY by [`add_physics_colored`](crate::plugin::add_physics_colored)
/// (gated on [`PhysicsConfig::colored`]), AFTER narrowphase and BEFORE the solve,
/// so the partition reflects the same manifold set the solver consumes. **O4
/// produces the partition only — it does NOT change the solve**: the shipped
/// [`SoftStepSolver`](crate::solver::SoftStepSolver) still solves in manifold
/// order, so the simulation output is byte-identical whether this stage runs or
/// not (the 0%-gate; a future O5 stage consumes the graph).
///
/// A body row is DYNAMIC iff its gathered `inv_mass != 0.0` (a static / kinematic
/// body has `inv_mass == 0`); the [`SDF_SENTINEL`](crate::manifold::SDF_SENTINEL)
/// `body_b` (`u32::MAX`) is out of range, so the bounds-checked predicate returns
/// `false` for it — the sentinel is treated as ground (Box2D's rule), never an
/// island node. The build is a pure deterministic function of the manifolds (in
/// manifold order) and the dynamic-body set, reusing the graph's buffers (no
/// per-step alloc in steady state).
//
// `clippy::needless_pass_by_value`: `Res<_>` is a by-value `SystemParam`; the body
// reads it via a `&*` reborrow.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_build_graph(
    scratch: Res<SolverScratch>,
    manifolds: Res<Manifolds>,
    mut graph: ResMut<ConstraintGraph>,
) {
    let bodies = scratch.bodies();
    let n_dynamic = bodies.len();
    // A row is dynamic iff it has a non-zero inverse mass (static/kinematic = 0).
    // The sentinel `u32::MAX` (and any out-of-range row) is non-dynamic — ground.
    //
    // MT soundness: this MUST be the SAME predicate the colored solve's `*_movable`
    // write guard uses — the coloring grants exclusive per-color ownership only to
    // the rows it marks dynamic here, so the solve may write ONLY those rows. Both
    // sites route through `is_dynamic_row` so they cannot drift (see its docs).
    let is_dynamic = |row: u32| {
        let i = row as usize;
        i < bodies.len() && is_dynamic_row(bodies[i].inv_mass)
    };
    graph.build(&manifolds.manifolds, n_dynamic, is_dynamic);
}

/// Runs the colored TGS-Soft solver for one step over the prebuilt
/// [`ConstraintGraph`] (Phase O5, Decision 7) — the SINGLE-THREADED colored
/// solve that REPLACES the default [`physics_solve_step`] under
/// [`add_physics_colored`](crate::plugin::add_physics_colored).
///
/// Calls [`ColoredSoftStepSolver::solve_colored`](crate::solver::ColoredSoftStepSolver::solve_colored)
/// directly (not through [`RigidSolver::solve`], whose signature carries no
/// graph): the solver builds its SoA `ContactColumns` in color order, runs the
/// substep loop solving colors `0..n_colors` sequentially (a Gauss-Seidel sweep
/// across colors), then stores the converged impulses in canonical order
/// (IM-2b). Registered ONLY on the colored path, where it stands in for
/// `physics_solve_step` — the default solver's stage is NOT registered, so the
/// two never both run. A non-colored world never reaches this stage (the
/// 0%-gate; the shipped [`SoftStepSolver`](crate::solver::SoftStepSolver) is
/// byte-untouched).
///
/// The colored solve reorders the contact sweep vs the reference manifold-order
/// sweep → DIFFERENT (but valid) converged values, validated against tolerance
/// gates (the Phase O5 value change). It is run-to-run bit-identical and never
/// moves a static body.
///
/// # O8 sleeping (plan O8 / Decision 5)
///
/// When [`PhysicsConfig::sleeping`] is on, this stage drives
/// [`ColoredSoftStepSolver::solve_colored_sleeping`](crate::solver::ColoredSoftStepSolver::solve_colored_sleeping),
/// threading the [`IslandSleep`] resource so slept islands skip ONLY their SOLVE +
/// INTEGRATE — `physics_gather` still walks every row (IM-1 intact). When off, it
/// drives the byte-identical
/// [`solve_colored`](crate::solver::ColoredSoftStepSolver::solve_colored) (the
/// `IslandSleep` resource is read but untouched — the 0%-gate).
//
// `clippy::needless_pass_by_value`: `ResMut<_>` / `Res<_>` are by-value
// `SystemParam`s used through reborrows — the same false-positive as the other
// physics stages.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_solve_colored(
    mut solver: ResMut<ColoredSoftStepSolver>,
    cfg: Res<PhysicsConfig>,
    manifolds: Res<Manifolds>,
    graph: Res<ConstraintGraph>,
    mut scratch: ResMut<SolverScratch>,
    mut sleep: ResMut<IslandSleep>,
) {
    if cfg.sleeping {
        solver.solve_colored_sleeping(&cfg, &manifolds.manifolds, &graph, &mut scratch, &mut sleep);
    } else {
        // Sleeping off: byte-identical to the O6/O7 colored path; `IslandSleep` is
        // resolved (so the param exists) but never read or written.
        let _ = &mut sleep;
        solver.solve_colored(&cfg, &manifolds.manifolds, &graph, &mut scratch);
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
    let bodies = scratch.bodies();
    let mut row = 0usize;
    for mut body in query.iter_mut() {
        // Guard the index (never break) so `row` counts EVERY live row to the
        // end: this lets the `debug_assert!` below catch a desync in BOTH
        // directions — a despawn (live rows < snapshot) ends with `row < len`,
        // a spawn (live rows > snapshot) ends with `row > len`. Breaking on
        // overflow would mask the spawn case (`row` would stop exactly at `len`).
        if row < bodies.len() && scratch.touched.get(row) {
            let state = &bodies[row];
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
        row == bodies.len(),
        "invariant: no structural change between gather and apply (live row count {} != snapshot len {})",
        row,
        bodies.len()
    );
}

/// Broad-phase bounding radius of a body, computed from its real shape (P2 W2/W4).
///
/// The smallest sphere enclosing the collider: a [`Sphere`](ColliderShape::Sphere)
/// contributes its radius directly; a [`Box`](ColliderShape::Box) contributes the
/// length of its half-extents diagonal (`half_extents.length()`) — the OBB's
/// circumradius, orientation-invariant — so a box body is a correct broadphase
/// CANDIDATE regardless of its rotation. The broadphase proxy is intentionally
/// shape-agnostic here; the precise per-pair narrowphase lives in
/// [`physics_narrowphase`]'s shape dispatch (P2 W4).
#[inline]
pub fn body_bounding_radius(body: &BodyState) -> f32 {
    match body.shape {
        ColliderShape::Sphere { radius } => radius,
        ColliderShape::Box { half_extents } => half_extents.length(),
    }
}

#[cfg(test)]
mod o9_manifold_tests {
    //! O9 full-`Manifold` differential gate for the box-vs-SDF narrowphase.
    //!
    //! [`box_sdf_manifold`] is private + cfg-gated: a single build compiles ONLY one
    //! arm (the AVX2 arm under `+avx2`, the verbatim scalar arm otherwise). This
    //! module compares [`box_sdf_manifold`] (the COMPILED arm) against an INDEPENDENT
    //! scalar reference ([`scalar_box_sdf_manifold`]) that folds the field through the
    //! FROZEN scalar oracle [`sample_sdf`] / [`boyko_sdf_math::sdf_edit_list`] and
    //! replays the SAME post-processing (corner build, `d >= 0` skip, the seam-skip,
    //! `−gradient` normal, `feature_vertex_face`, [`insert_deepest`], deepest-first
    //! order).
    //!
    //! - In a `+avx2` build this is a TRUE scalar-oracle-vs-AVX2-arm differential:
    //!   `box_sdf_manifold` runs [`box_sdf_manifold_avx2`], the reference runs the
    //!   scalar leaf. Full-`Manifold` `to_bits` equality here PROVES the AVX2 arm is
    //!   byte-identical to the scalar fold.
    //! - In a default build both sides fold the same scalar leaf, so it degenerates
    //!   to a self-consistency check of the reference (still useful: it pins the
    //!   reference against the shipped scalar arm).
    //!
    //! The generator includes scenes where SOME corners penetrate and some do not
    //! (the review's noted refinement — the AVX2 arm skips gradient work for
    //! non-penetrating corners; the emitted manifold must still match).

    use boyko_sdf_math::{SdfEdit, sdf_kind, sdf_op, v_normalize, SDF_GRAD_H};

    use super::*;
    use crate::math::Quat;

    /// Deterministic splitmix64 (matches the kernel-test RNG).
    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Self {
            Self(seed)
        }
        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next_u64() % n
        }
        fn f32_in(&mut self, range: f32) -> f32 {
            let u = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
            (u * 2.0 - 1.0) * range
        }
    }

    /// The INDEPENDENT scalar reference: byte-for-byte the scalar arm of
    /// [`box_sdf_manifold`], folding the field through the FROZEN scalar leaf
    /// (`sample_sdf` → `sdf_edit_list` / `sdf_edit_list_normal`). This is the oracle
    /// the AVX2 arm must match in a `+avx2` build.
    ///
    /// It re-derives the gradient via the leaf's exact central-difference order and
    /// the same `v_normalize` the AVX2 arm reuses, so the only difference vs the
    /// shipped scalar arm is that this copy is local to the test (cannot drift in a
    /// `+avx2` build where the shipped scalar arm is cfg'd out).
    fn scalar_box_sdf_manifold(
        a: BodyIndex,
        body: &BodyState,
        half_extents: Vec3,
        field: &SdfField,
    ) -> Option<Manifold> {
        let max_points = crate::math::MAX_CONTACT_POINTS;
        let mut kept: [(ContactPoint, Vec3); crate::math::MAX_CONTACT_POINTS] =
            [(ContactPoint::default(), Vec3::ZERO); crate::math::MAX_CONTACT_POINTS];
        let mut kept_len = 0usize;
        let edits = field.edits();
        let h = SDF_GRAD_H;

        for corner in 0u32..8 {
            let sx = if corner & 1 != 0 { 1.0 } else { -1.0 };
            let sy = if corner & 2 != 0 { 1.0 } else { -1.0 };
            let sz = if corner & 4 != 0 { 1.0 } else { -1.0 };
            let local = half_extents.componentwise_mul(Vec3::new(sx, sy, sz));
            let world = body.position + body.rotation.rotate(local);
            let w = [world.x, world.y, world.z];

            let d = boyko_sdf_math::sdf_edit_list(edits, w);
            if d >= 0.0 {
                continue;
            }
            // Central difference in the leaf's EXACT read order, then the frozen
            // `v_normalize` — identical to the AVX2 arm's gradient.
            let n = v_normalize([
                boyko_sdf_math::sdf_edit_list(edits, [w[0] + h, w[1], w[2]])
                    - boyko_sdf_math::sdf_edit_list(edits, [w[0] - h, w[1], w[2]]),
                boyko_sdf_math::sdf_edit_list(edits, [w[0], w[1] + h, w[2]])
                    - boyko_sdf_math::sdf_edit_list(edits, [w[0], w[1] - h, w[2]]),
                boyko_sdf_math::sdf_edit_list(edits, [w[0], w[1], w[2] + h])
                    - boyko_sdf_math::sdf_edit_list(edits, [w[0], w[1], w[2] - h]),
            ]);
            let gradient = Vec3::new(n[0], n[1], n[2]);
            if gradient.length_squared() < SDF_NORMAL_EPS * SDF_NORMAL_EPS
                || !gradient.is_finite()
            {
                continue;
            }
            let normal = gradient * -1.0;
            let point = ContactPoint {
                anchor_a: world,
                anchor_b: world,
                separation: d,
                feature_id: feature_vertex_face(corner),
            };
            insert_deepest(&mut kept, &mut kept_len, max_points, point, normal);
        }

        if kept_len == 0 {
            return None;
        }
        let mut m = Manifold::new(a, SDF_SENTINEL);
        m.normal = kept[0].1;
        for (i, (point, _)) in kept[..kept_len].iter().enumerate() {
            m.points[i] = *point;
        }
        m.count = kept_len as u8;
        Some(m)
    }

    /// Full `to_bits` equality of two manifolds: count, body keys, header normal,
    /// AND every live point (anchors / separation / feature id) in deepest-first
    /// order. Reports the first divergence with the scene.
    fn assert_manifold_bit_eq(
        got: &Option<Manifold>,
        want: &Option<Manifold>,
        scene: &str,
    ) {
        match (got, want) {
            (None, None) => {}
            (Some(g), Some(w)) => {
                assert_eq!(g.count, w.count, "manifold count differs ({scene})");
                assert_eq!(g.body_a, w.body_a, "body_a differs ({scene})");
                assert_eq!(g.body_b, w.body_b, "body_b differs ({scene})");
                assert_eq!(
                    [g.normal.x.to_bits(), g.normal.y.to_bits(), g.normal.z.to_bits()],
                    [w.normal.x.to_bits(), w.normal.y.to_bits(), w.normal.z.to_bits()],
                    "header normal bits differ ({scene})"
                );
                for i in 0..g.count as usize {
                    let gp = &g.points[i];
                    let wp = &w.points[i];
                    assert_eq!(
                        [
                            gp.anchor_a.x.to_bits(), gp.anchor_a.y.to_bits(), gp.anchor_a.z.to_bits(),
                            gp.anchor_b.x.to_bits(), gp.anchor_b.y.to_bits(), gp.anchor_b.z.to_bits(),
                            gp.separation.to_bits(), gp.feature_id,
                        ],
                        [
                            wp.anchor_a.x.to_bits(), wp.anchor_a.y.to_bits(), wp.anchor_a.z.to_bits(),
                            wp.anchor_b.x.to_bits(), wp.anchor_b.y.to_bits(), wp.anchor_b.z.to_bits(),
                            wp.separation.to_bits(), wp.feature_id,
                        ],
                        "contact point {i} bits differ ({scene})"
                    );
                }
            }
            _ => panic!(
                "manifold presence differs ({scene}): got.is_some()={} want.is_some()={}",
                got.is_some(),
                want.is_some()
            ),
        }
    }

    /// Builds a `BodyState` box with the given pose (the only fields the box-SDF
    /// narrowphase reads are `position`, `rotation`, `shape`).
    fn box_state(position: Vec3, rotation: Quat, half: Vec3) -> BodyState {
        BodyState {
            inv_inertia: crate::math::Mat3::ZERO,
            inv_inertia_local: crate::math::Mat3::ZERO,
            position,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            rotation,
            inv_mass: 1.0,
            restitution: 0.0,
            friction: 0.5,
            body_type: crate::components::BodyType::Dynamic,
            is_sensor: false,
            shape: ColliderShape::Box { half_extents: half },
        }
    }

    fn rand_quat(rng: &mut Rng) -> Quat {
        Quat::new(
            rng.f32_in(1.0),
            rng.f32_in(1.0),
            rng.f32_in(1.0),
            rng.f32_in(1.0),
        )
        .normalize()
    }

    fn rand_edit(rng: &mut Rng) -> SdfEdit {
        let center = [rng.f32_in(2.0), rng.f32_in(2.0), rng.f32_in(2.0)];
        let kind = if rng.below(2) == 0 { sdf_kind::SPHERE } else { sdf_kind::BOX };
        let op = match rng.below(4) {
            0 => sdf_op::UNION,
            1 => sdf_op::SUBTRACT,
            2 => sdf_op::INTERSECT,
            _ => sdf_op::UNION,
        };
        let smoothness = match rng.below(3) {
            0 => 0.0,
            1 => 0.2,
            _ => 1.5,
        };
        if kind == sdf_kind::BOX {
            SdfEdit::box_shape(
                center,
                [0.3 + rng.f32_in(1.0).abs(), 0.3 + rng.f32_in(1.0).abs(), 0.3 + rng.f32_in(1.0).abs()],
                op,
                smoothness,
            )
        } else {
            SdfEdit::sphere(center, 0.3 + rng.f32_in(1.5).abs(), op, smoothness)
        }
    }

    /// THE full-`Manifold` differential gate. Random (box pose, half-extents, edit
    /// list) over many scenes — including all-clear, all-penetrate, and the
    /// MIXED (some corners penetrate, some do not) cases — asserts full `to_bits`
    /// `Manifold` equality between [`box_sdf_manifold`] (the compiled arm) and the
    /// scalar oracle reference.
    #[test]
    fn box_sdf_manifold_matches_scalar_oracle() {
        let mut rng = Rng::new(0x0900_3a17_face_0009);
        let a = BodyIndex(0);
        let mut scenes = 0usize;
        let mut produced_some = 0usize;
        let mut produced_mixed = 0usize;

        for _ in 0..2000 {
            let count = 1 + (rng.below(6)) as usize; // 1..=6 edits
            let edits: Vec<SdfEdit> = (0..count).map(|_| rand_edit(&mut rng)).collect();
            let field = SdfField::from_edits(&edits);

            // Place the box near the field origin so corners straddle surfaces (forces
            // the MIXED some-penetrate-some-clear case), with random pose + extents.
            let pos = Vec3::new(rng.f32_in(1.5), rng.f32_in(1.5), rng.f32_in(1.5));
            let rot = rand_quat(&mut rng);
            let half = Vec3::new(
                0.2 + rng.f32_in(1.2).abs(),
                0.2 + rng.f32_in(1.2).abs(),
                0.2 + rng.f32_in(1.2).abs(),
            );
            let body = box_state(pos, rot, half);

            let got = box_sdf_manifold(a, &body, half, &field);
            let want = scalar_box_sdf_manifold(a, &body, half, &field);

            let scene = format!("pos={pos:?} half={half:?} edits={edits:?}");
            assert_manifold_bit_eq(&got, &want, &scene);

            scenes += 1;
            if let Some(m) = got {
                produced_some += 1;
                // A mixed scene = at least one contact kept AND fewer than 8 corners
                // contributed (some were skipped) — count corners that penetrate.
                let penetrating = (0u32..8)
                    .filter(|&corner| {
                        let sx = if corner & 1 != 0 { 1.0 } else { -1.0 };
                        let sy = if corner & 2 != 0 { 1.0 } else { -1.0 };
                        let sz = if corner & 4 != 0 { 1.0 } else { -1.0 };
                        let local = half.componentwise_mul(Vec3::new(sx, sy, sz));
                        let w = body.position + body.rotation.rotate(local);
                        sample_sdf(&field, w).0 < 0.0
                    })
                    .count();
                if penetrating > 0 && penetrating < 8 {
                    produced_mixed += 1;
                }
                let _ = m;
            }
        }
        // Anti-vacuity: the gate must actually EXERCISE the contact-producing path
        // and the mixed (some-penetrate) path, not just the all-clear `None` case.
        assert!(scenes >= 1000, "differential must run >= 1000 scenes (ran {scenes})");
        assert!(
            produced_some >= 50,
            "anti-vacuity: too few contact-producing scenes ({produced_some}); the generator \
             never penetrates the field"
        );
        assert!(
            produced_mixed >= 20,
            "anti-vacuity: too few MIXED some-penetrate scenes ({produced_mixed}); the \
             non-penetrating-corner skip path is undertested"
        );
    }

    /// An empty field produces NO manifold from either arm (the narrowphase emits
    /// nothing against a `+far`-everywhere field).
    #[test]
    fn box_sdf_manifold_empty_field_is_none() {
        let field = SdfField::default();
        let body = box_state(Vec3::ZERO, Quat::IDENTITY, Vec3::new(1.0, 1.0, 1.0));
        let got = box_sdf_manifold(BodyIndex(0), &body, Vec3::new(1.0, 1.0, 1.0), &field);
        let want = scalar_box_sdf_manifold(BodyIndex(0), &body, Vec3::new(1.0, 1.0, 1.0), &field);
        assert_manifold_bit_eq(&got, &want, "empty field");
        assert!(got.is_none(), "empty field must produce no manifold");
    }

    /// A box fully submerged in a large SDF box (ALL 8 corners penetrate): the
    /// deepest ≤4 are kept, deepest-first, byte-identical between the arms.
    #[test]
    fn box_sdf_manifold_all_corners_penetrate_matches() {
        // A huge SDF box centered at origin; a small body box at origin is fully
        // inside, so every corner has d < 0.
        let field = SdfField::from_edits(&[SdfEdit::box_shape(
            [0.0, 0.0, 0.0],
            [10.0, 10.0, 10.0],
            sdf_op::UNION,
            0.0,
        )]);
        let half = Vec3::new(1.0, 1.0, 1.0);
        let body = box_state(Vec3::new(0.1, -0.2, 0.3), Quat::IDENTITY, half);
        let got = box_sdf_manifold(BodyIndex(0), &body, half, &field);
        let want = scalar_box_sdf_manifold(BodyIndex(0), &body, half, &field);
        assert_manifold_bit_eq(&got, &want, "all-corners-penetrate");
        // It must produce a manifold capped at MAX_CONTACT_POINTS.
        let m = got.expect("a fully-submerged box must produce a manifold");
        assert!(
            m.count as usize <= crate::math::MAX_CONTACT_POINTS,
            "manifold must be capped at MAX_CONTACT_POINTS"
        );
    }
}

