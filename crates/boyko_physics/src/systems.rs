//! The fixed physics pipeline as ordinary systems (plan D3 / IM-1 / IM-2).
//!
//! The stages run in this deterministic order (pinned by `.after(...)` in
//! [`add_physics_systems`](crate::plugin::add_physics_systems)):
//!
//! 1. [`physics_integrate`] — `par_iter_mut`: gravity + `pos += vel·dt` +
//!    `rot = rot.integrate(angvel, dt)` (first-order quaternion advance). The
//!    only real-work stage in the foundation; a sound parallel pass over
//!    disjoint rows (each body writes only its own row).
//! 2. [`physics_gather`] — snapshots the rows
//!    [`BodySetFilter`](crate::body_set::BodySetFilter) selects, reading `RigidBody`,
//!    `RigidBodyMass` and `Collider`, IN ROW ORDER into the dense
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
//! 5. The solve, chosen by the solver type at wire-up:
//!    [`physics_build_graph`] → [`physics_solve_colored`] for the default world's
//!    [`ColoredSoftStepSolver`] (the O7 AVX2 cohort kernel on by default), or
//!    [`physics_solve_step`] — `if solver.is_noop() { return }` else
//!    `S::solve(..)` (the swappable seam, D2) — for any other solver, including the
//!    reference [`SoftStepSolver`](crate::solver::SoftStepSolver).
//! 6. [`physics_apply`] — writes the solved snapshot back through
//!    `Mut<RigidBody>` for touched rows, selected with [`BodyQuery`], the same rows the
//!    gather snapshots, under the "no structural change between gather and apply"
//!    invariant (IM-1).
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
//! 2. The solver integrates **SIMULATED dynamic bodies only**
//!    (`simulated && is_dynamic_row(inv_mass)`) inside its substep loop —
//!    mandatory: it applies a
//!    per-substep gravity bias, so a static floor would drift if it were
//!    integrated.
//! 3. **DO NOT un-gate** [`physics_integrate`] for an owning solver: running both
//!    would DOUBLE-INTEGRATE (the pipeline AND the solver each advance position +
//!    orientation in the same step), corrupting the simulation.

use boyko_ecs::ecs::core::iters::query::data_is_enabled::IsEnabled;
use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::system::{Entities, Res, ResMut};
use boyko_ecs::ecs::core::time::FixedTime;

use crate::body_set::{BodyApplyData, BodyGatherData, BodyQuery};
use crate::broadphase_tree::BroadphaseTree;
use crate::components::{ColliderShape, RigidBody, RigidBodyMass, Simulated};
use crate::manifold::{BodyIndex, ContactPoint, Manifold, SDF_SENTINEL};
use crate::math::Vec3;
use crate::narrowphase::axis_cache::SAT_AXIS_COUNT;
use crate::narrowphase::box_box::{BoxBoxContact, BoxBoxOutcome, Obb, box_box_classify_carried};
use crate::narrowphase::carry::{CarryIn, PairTag};
use crate::narrowphase::dispatch::try_parallel_sets;
use crate::narrowphase::feature_vertex_face;
use crate::narrowphase::reuse::{
    PairGeom, Prev, Refreshed, ReuseRecord, ReuseStep, RowFrame, build, criterion,
    fill_row_frames, is_fast, refresh,
};
use crate::narrowphase::sphere_box::sphere_box_contact;
use crate::profiling::{
    PHYS_BP_PAIRS, PHYS_NP_CHUNKS, PHYS_NP_FULL, PHYS_NP_MANIFOLDS, PHYS_NP_PAIRS, PHYS_NP_POINTS,
    PHYS_NP_REUSED, PHYS_NP_SEP_HITS, counter,
};
use crate::resources::{
    BodyState, BroadphaseGrid, BroadphaseKind, ConstraintGraph, ContactPairs, IntegrationMode,
    IslandSleep, Manifolds, PhysicsConfig, SdfNarrowphaseKernel, SleepSkip, SolverScratch,
};
use crate::row_identity::{RowIdentity, RowKey};
use crate::sdf_query::{SdfField, sample_sdf};
use crate::sleep_sets::{Route, RowCls, SleepSets, np_route};
use crate::solver::colored::ColoredSoftStepSolver;
use crate::solver::contact::{effective_inv_mass, is_dynamic_row};
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
/// own [`RigidBody`]. For a SIMULATED dynamic body
/// (`simulated && is_dynamic_row(inv_mass)`) it applies gravity to linear
/// velocity, advances position by the fixed `dt`, then advances orientation by
/// integrating the quaternion against the angular velocity. Parked / static /
/// kinematic bodies are SKIPPED (their pose is gameplay-owned — see the gate
/// below).
///
/// # Simulated gate (single-source-of-truth with scene sync, std-lib S5)
///
/// This stage keys per-body integration on the
/// [`Simulated`](crate::components::Simulated) bit (read non-filteringly via
/// [`IsEnabled<Simulated>`](boyko_ecs::ecs::core::iters::query::IsEnabled))
/// exactly as the solver-owned integrator does
/// (`simulated && is_dynamic_row(inv_mass)`, the mass test routed through
/// [`is_dynamic_row`](crate::solver::contact) so the two sites cannot drift).
/// Integrating a parked / static / kinematic body here would advance
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
    mut query: Query<(&mut RigidBody, &RigidBodyMass, IsEnabled<Simulated>)>,
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
    query.par_iter_mut().for_each(
        move |(body, mass, simulated): (&mut RigidBody, &RigidBodyMass, bool)| {
            // Decision 3/4 / C2 parity: integrate ONLY a simulated dynamic body,
            // exactly the set the solver-owned integrator advances. A parked
            // (`Simulated` OFF) or `inv_mass == 0` body's pose is frozen-in-place
            // and must not drift under gravity. `is_dynamic_row` is the SAME
            // inv-mass predicate the coloring / solve write guards route through,
            // so the integrate set cannot drift from the solve set; the
            // `Simulated` bit is AND-ed in (replacing the old `body_type ==
            // Dynamic` discrimination).
            if !simulated || !is_dynamic_row(mass.inv_mass) {
                return;
            }
            debug_assert!(
                is_dynamic_row(mass.inv_mass),
                "invariant: a Simulated body integrated here must have inv_mass != 0"
            );
            body.linear_velocity = body.linear_velocity + gravity * dt;
            body.position = body.position + body.linear_velocity * dt;
            body.rotation = body.rotation.integrate(body.angular_velocity, dt);
        },
    );
}

/// Snapshots every body into the dense, row-indexed solver scratch and stamps
/// the step `dt` (plan IM-1 / OQ-1, the gather boundary).
///
/// Walks the body set, the rows [`BodySetFilter`](crate::body_set::BodySetFilter)
/// selects, in archetype-row order, projecting the hot [`RigidBody`] + cold
/// [`RigidBodyMass`] + [`Collider`](crate::components::Collider) columns into
/// [`BodyState`] rows (deriving each body's local + world inverse inertia from its
/// shape, P2 W1). [`physics_apply`] and
/// [`physics_soft_rigid_apply`](crate::soft::physics_soft_rigid_apply) take a
/// [`BodyQuery`] with the same filter, so they re-walk the same rows in the same order
/// to write back. The row set is the filter's, not a side effect of this stage's
/// reads: dropping a read here moves no row, and a new required read that the filter
/// does not name fails the wire-up check in [`crate::body_set`].
/// The dense row index IS the [`BodyIndex`]. Resets the touched mask to the body
/// count. The snapshot columns are cleared and refilled, capacity reused (no
/// per-step alloc).
///
/// Before the per-body loop it stamps [`PhysicsConfig::dt`] from the fixed
/// clock ([`FixedTime::delta_secs`]) ONCE per gather (OQ-1): no separate `dt`
/// system, and the TGS solver later reads `h = dt / substeps`. The stamp is
/// gather-time so a hand-set `cfg.dt` is overwritten.
///
/// It also records each row's [`RowKey`] — the entity's slot index AND generation, the
/// latter read through [`Entities`] — and the rows whose `RigidBody` was added since the
/// last gather into [`SolverScratch`]'s row identity map, so the row-keyed consumers can
/// carry their state when rows move (defect A, interim) and a body on a recycled slot never
/// inherits the dead body's state (hazard H-03). `Query::iter_entities` walks exactly the
/// `iter()` order but yields the slot only. A row → entity projection for the gameplay
/// [`Contact`](crate::components::Contact) producer is still not carried.
//
// `clippy::needless_pass_by_value`: `ResMut<_>` / `Res<_>` are by-value
// `SystemParam`s mutated/read through reborrows — the same false-positive as the
// demo's `ResMut` systems.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_gather(
    query: BodyQuery<BodyGatherData>,
    entities: Entities,
    mut scratch: ResMut<SolverScratch>,
    mut cfg: ResMut<PhysicsConfig>,
    fixed_time: Res<FixedTime>,
) {
    // OQ-1: stamp the fixed step delta once, before snapshotting, so the solver
    // reads a fresh `dt` each gather (deterministic — `FixedTime` is the
    // schedule's fixed clock).
    cfg.dt = fixed_time.delta_secs();

    let scratch = &mut *scratch;
    scratch.vn_initial.build_view().clear();
    // Refill the gather column through its single-threaded build view: clear (no
    // free — the committed pages stay resident) then push one BodyState per row.
    // The walk is a `BodyQuery`, whose filter `physics_apply` shares, so both walks
    // visit the body set in the same archetype-row order and row `i` is the same body
    // in both passes (the IM-1 gather/apply addressing invariant).
    //
    // S5: `Option<&Sensor>` is a NON-filtering query datum — it yields `Some` for
    // a sensor body and `None` otherwise, so the gathered row count and order are
    // identical to a world with no `Sensor` id (the 0%-gate; `is_sensor` is just
    // `false` everywhere there). The bit rides into `BodyState` for the
    // narrowphase to read.
    //
    // Decision 3: `IsEnabled<Simulated>` / `IsEnabled<Kinematic>` are likewise
    // NON-filtering, order-preserving per-row data — they yield a `bool` for
    // EVERY matched row without dropping/reordering, so the gather order is
    // byte-identical to today (Encoding A; the same theorem as `Option<&Sensor>`).
    // The bits ride into `BodyState`, where the integrate/solve gates AND
    // `simulated` with the unchanged `is_dynamic_row` oracle.
    // The refill view is SCOPED: it publishes its frontier on `Drop`, so the borrow
    // of `scratch.bodies` has to end before `scratch.touched` is reached.
    //
    // Defect A (interim): the same walk records each row's `RowKey` (slot + generation,
    // through `Entities`: one 16 B fast-store slot load per row, whose address depends on
    // the id alone, so it issues beside the column reads) and the rows whose `RigidBody`
    // was added (`Ref` is a plain read and `Entities` declares no access, so the access
    // set is unchanged), so `finish_gather` can tell where every body sat one gather ago.
    // `iter_entities` yields exactly the `iter()` sequence.
    scratch.rows.begin_gather();
    // L10 D3 (design 04): under an active sleep-skip mode the previous step's post-solve
    // snapshot is kept as the broadphase's resting baseline — the snapshot and the baseline
    // swap, O(1), stamped with this gather — before the refill below clears the snapshot. Never
    // under the default (sleeping off) nor on a world without the colored pipeline.
    if cfg.colored && cfg.sleeping && cfg.sleep_skip != SleepSkip::Off {
        scratch.keep_baseline();
    }
    let n = {
        let SolverScratch { bodies, rows, .. } = &mut *scratch;
        let mut bodies = bodies.build_view();
        bodies.clear();
        let (mut ids, mut added) = rows.gather_views();
        for (entity, (body, mass, collider, sensor, simulated, kinematic)) in query.iter_entities()
        {
            if body.is_added() {
                added.push(bodies.len() as u32);
            }
            // The gather runs strictly after the apply window that registered every row's
            // entity, so a row the body query yields is live.
            let live = entities
                .get(entity)
                .expect("invariant: a row the body query yields is a live entity");
            ids.push(RowKey::of(live));
            bodies.push(BodyState::from_columns(
                &body,
                mass,
                collider,
                sensor.is_some(),
                simulated,
                kinematic,
            ));
        }
        debug_assert_eq!(ids.len(), bodies.len(), "invariant: one entity id per gathered row");
        bodies.len()
    };
    scratch.rows.finish_gather();
    debug_assert_eq!(n, query.iter().count(), "Encoding A: gather must not drop a row");
    scratch.touched.reset(n);
}

/// Fills [`ContactPairs`] with candidate `(BodyIndex, BodyIndex)` pairs in
/// deterministic `(min, max)` order (plan D3 stage 2 / D4 / OQ1; O2 grid path;
/// the tree broadphase).
///
/// A pair is a candidate when the bodies' bounding spheres overlap
/// (`delta.length_squared() <= (rA + rB)²`). Emitting `(min, max)` keeps the
/// order content-defined and reproducible (float add is non-associative →
/// contact iteration order must be deterministic, D4).
///
/// Three interchangeable paths, selected by [`PhysicsConfig::broadphase`] (a
/// single runtime branch — the one-branch floor):
///
/// - [`BroadphaseKind::AllPairs`] (DEFAULT): the shipped O(n²) double loop,
///   byte-identical to before O2 (the campaign 0%-gate).
/// - [`BroadphaseKind::Grid`] (opt-in, O2): a uniform-grid CSR counting-sort
///   ([`BroadphaseGrid::build`]) that emits candidates then applies the SAME
///   sphere-bound predicate and sorts by `(min, max)`. Its pair set is
///   bit-identical to all-pairs.
/// - [`BroadphaseKind::Tree`] (opt-in): the packed-BVH broadphase with a
///   persistent static set ([`BroadphaseTree::step`]), serial, heap-free per step.
///   Its pair set is all-pairs' exact set by construction (the same predicate on
///   the same bits, one owner per pair, an integer-count assembly), so it is
///   bit-identical too; at or below [`BroadphaseTree::brute_max_rows`] rows it
///   runs [`all_pairs_into`](crate::broadphase_tree::all_pairs_into) instead.
///   It reads the gather's row identity to carry its static set through row
///   changes, and opens the four `phys_bp_*` profiling spans on every tree-path
///   step.
///
/// Before the arm runs, the previous step's list is kept as `pairs_prev` and the new list is
/// stamped with this gather's sequence ([`ContactPairs::rotate`], L9 D9): the narrowphase's pair
/// carry joins this step's pairs against it, reading the jumper bitset the rotation rebuilds on a
/// step whose rows moved (L10 C0). All three arms fill the list the rotation hands them.
///
/// This is the reference pipeline's broadphase. A world wired with the colored pipeline runs
/// [`physics_broadphase_colored`] instead, which adds L10's sleep-skip around the same arms.
//
// `clippy::needless_pass_by_value`: see `physics_gather`.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_broadphase(
    scratch: Res<SolverScratch>,
    cfg: Res<PhysicsConfig>,
    mut grid: ResMut<BroadphaseGrid>,
    mut tree: ResMut<BroadphaseTree>,
    mut pairs: ResMut<ContactPairs>,
) {
    let pairs = &mut *pairs;
    // L9 D9: before the kind match, so every arm fills the swapped-in list. On a step whose
    // rows moved it also rebuilds the carry's jumper bitset (L10 C0, design 06 Δ9).
    pairs.rotate(&scratch.rows);
    broadphase_arms(scratch.bodies(), &scratch.rows, &cfg, &mut grid, &mut tree, pairs);
}

/// The colored pipeline's broadphase (L10 design 04 D15): [`physics_broadphase`]'s rotation and
/// kind arms, with L10's sleep-skip prologue between them — registered only where the colored
/// pipeline inserts [`IslandSleep`] and [`SleepSets`], while the reference pipeline keeps
/// [`physics_broadphase`].
///
/// The prologue records the step's sleep-skip mode — `Off` when sleeping is off, else
/// [`PhysicsConfig::sleep_skip`] — which every later stage reads instead of the configuration
/// (design 04 D9). Nothing is held before L10 C3b, so the pair list is the reference
/// broadphase's, bit for bit.
//
// `clippy::needless_pass_by_value`: see `physics_gather`.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_broadphase_colored(
    scratch: Res<SolverScratch>,
    cfg: Res<PhysicsConfig>,
    mut grid: ResMut<BroadphaseGrid>,
    mut tree: ResMut<BroadphaseTree>,
    mut pairs: ResMut<ContactPairs>,
    mut sets: ResMut<SleepSets>,
) {
    let pairs = &mut *pairs;
    // L9 D9 and L10 C0, as above: the prologue reads the jumper bitset the rotation builds.
    pairs.rotate(&scratch.rows);
    // L10 A1 (design 04; C2a: the step mode only).
    sets.open_step(&cfg, &scratch.rows);
    broadphase_arms(scratch.bodies(), &scratch.rows, &cfg, &mut grid, &mut tree, pairs);
}

/// The broadphase's kind arms over the rotated list (the body of [`physics_broadphase`] after the
/// rotation), shared by both pipelines' broadphase systems: the list's fill, its order assert and
/// its counter.
#[inline]
fn broadphase_arms(
    bodies: &[BodyState],
    rows: &RowIdentity,
    cfg: &PhysicsConfig,
    grid: &mut BroadphaseGrid,
    tree: &mut BroadphaseTree,
    pairs: &mut ContactPairs,
) {
    match cfg.broadphase {
        // The shipped all-pairs loop, kept VERBATIM so the default path's asm is
        // byte-identical to before O2 (the 0%-gate). DO NOT refactor this arm.
        BroadphaseKind::AllPairs => {
            // The ONLY change to this arm is the receiver: `pairs` is the column's
            // refill view instead of a `&mut Vec`. The bound test, the emit order
            // and the loop shape below are untouched.
            let mut pairs = pairs.pairs.build_view();
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
        // The tree broadphase: the exact set, serial, on the calling thread. The row
        // identity is the gather's (`scratch.rows`), which the tree's verify uses to
        // carry its static set through a row change.
        BroadphaseKind::Tree => tree.step(bodies, rows, pairs),
    }

    // Strict: the pairs are unique as well as sorted. The narrowphase's hysteresis
    // table is keyed by pair, and the parallel narrowphase's hint argument (Lemma 1 in
    // `narrowphase/axis_cache.rs`) needs every key to appear once per frame.
    debug_assert!(
        pairs.pairs_stream().windows(2).all(|w| w[0] < w[1]),
        "invariant: broadphase pairs must be emitted unique and in sorted (min, max) order"
    );
    // The LOGICAL pair count (L10 design 04 runner O3): the stream until L10 C3c withholds.
    counter!(PHYS_BP_PAIRS, pairs.pairs().len() as u64);
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
/// - **box-box**: [`box_box_contact`](crate::narrowphase::box_box::box_box_contact)'s
///   kernel — 15-axis SAT + reference-face clip + a deterministic ≤4-point reduction,
///   biased by the per-pair reference-axis hysteresis in [`Manifolds::box_axis_cache`] for
///   stable feature ids on a resting stack (P2 W3/W4) — on boxes built from the step's
///   per-row orientation frames (L9 D2, filled at the entry of either path below).
///
/// Every emitted manifold is keyed by the dense `(a, b)` rows in `(min, max)`
/// order (IM-1 / D4) with its `normal` pointing A→B, regardless of which body was
/// the sphere or the SAT reference — so the solver's sign handling is uniform.
/// The buffer is cleared and refilled each step; the hysteresis cache persists in
/// place across frames (capacity reused).
///
/// # The pair carry (L9 D9, `narrowphase/carry.rs`)
///
/// Every pair writes a tag, and the next step joins its pairs to those tags through the row
/// identity: a box pair the SAT separated carries its separating axis, which the next step
/// evaluates first, skipping the SAT while it still separates (L9a (ii), exact). The carry is
/// classified before either path runs and stamped after, with the pair list its tags index.
///
/// # Contact reuse (L9b, `narrowphase/reuse.rs`)
///
/// With [`PhysicsConfig::contact_reuse`] on, a slow touching box pair writes a reuse record beside
/// its tag, and the next step refreshes that record from the current poses instead of running the
/// SAT and the clip while the relative motion stays within the reuse distance. Off by default:
/// then every pair takes the path above, bit for bit.
///
/// # The serial loop and the parallel chunks (L5)
///
/// With [`PhysicsConfig::parallel_narrowphase`] on (the default since L5 C4), the step
/// first offers its pairs to the parallel narrowphase (`narrowphase/dispatch.rs`):
/// contiguous chunks collided across the ambient pool's workers, joined back in pair
/// order, with the box-box axis writes replayed serially in pair order. The dispatch
/// declines — and this system runs
/// [`narrowphase_serial`], today's loop — when the flag is off, when no pool of at least
/// two workers is attached, or when the pairs make fewer than two chunks. Both paths
/// collide a pair through the one [`collide_pair`], and the parallel path's manifold
/// stream, sensor stream, pair tags and hysteresis table state equal the serial path's (the
/// dispatch module's Lemma 3, the axis cache's Lemmas 1 and 2, and the carry's lemma L9-J).
//
// `clippy::needless_pass_by_value`: see `physics_gather`.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_narrowphase(
    scratch: Res<SolverScratch>,
    pairs: Res<ContactPairs>,
    cfg: Res<PhysicsConfig>,
    mut manifolds: ResMut<Manifolds>,
) {
    let _ = narrowphase_step::<false>(&scratch, &pairs, &cfg, &mut manifolds, &[]);
}

/// The colored pipeline's narrowphase (L10 design 04 D15, 08 D1′/D-H): [`physics_narrowphase`]
/// with its arm chosen by the step's [`SleepSets`] — the `Sets` arm when the broadphase said so,
/// which skips the pairs of held islands, else the `Off` arm, which collides every pair as
/// [`physics_narrowphase`] does. Registered only where the colored pipeline inserts
/// [`SleepSets`].
///
/// Nothing is held before L10 C3b, whose broadphase first writes the per-row classification
/// the `Sets` arm reads, so the broadphase never selects it yet and every step takes the `Off`
/// arm, bit for bit the reference narrowphase.
//
// `clippy::needless_pass_by_value`: see `physics_gather`.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_narrowphase_colored(
    scratch: Res<SolverScratch>,
    pairs: Res<ContactPairs>,
    cfg: Res<PhysicsConfig>,
    mut manifolds: ResMut<Manifolds>,
    mut sets: ResMut<SleepSets>,
) {
    if sets.np_sets() {
        let skips = narrowphase_step::<true>(&scratch, &pairs, &cfg, &mut manifolds, sets.row_cls());
        sets.note_held_skips(skips);
    } else {
        let _ = narrowphase_step::<false>(&scratch, &pairs, &cfg, &mut manifolds, &[]);
    }
}

/// One step of the narrowphase on L10's arm `SETS` (the body of [`physics_narrowphase`]):
/// the hysteresis table's frame, the pair carry's classification, the parallel chunks or the
/// serial loop, the carry's stamp and the step's counters. Returns the number of pairs the
/// `Sets` arm skipped for a held endpoint (`0` on the `Off` arm). `cls` is the step's per-row
/// classification, read on the `Sets` arm only.
#[inline]
fn narrowphase_step<const SETS: bool>(
    scratch: &SolverScratch,
    contact_pairs: &ContactPairs,
    cfg: &PhysicsConfig,
    manifolds: &mut Manifolds,
    cls: &[RowCls],
) -> u32 {
    let bodies = scratch.bodies();
    // The stream: the pairs this step collides, which its tags index (L10 design 06 D-D D2).
    let pairs = contact_pairs.pairs_stream();
    // Ensure the per-pair hysteresis cache can hold this frame's pairs; it is NOT
    // cleared (a single in-place table — this frame reads last frame's axes). When the
    // rows changed since the cache was last keyed, every box pair's previous axis is
    // pre-read through the row identity map first, before any write of this step
    // (defect A, interim). The table is sized by the LOGICAL pair count (L10 design 04 A3),
    // which is the stream's until L10 C3c withholds pairs.
    let keys = manifolds.box_axis_cache.begin_frame_synced(
        pairs,
        contact_pairs.pairs().len(),
        bodies,
        &scratch.rows,
    );
    let prefetched = keys.prefetched;
    debug_assert_eq!(
        keys.any(),
        manifolds.box_axis_cache.keys_changed(),
        "invariant: the key change's causes union to the table's keys_changed"
    );
    // L9b: the step's reuse parameters, and whether a hit must re-key its hysteresis entry
    // (ruling W1), which `begin_frame_synced` has just settled.
    let reuse = ReuseStep::new(
        cfg.contact_reuse,
        cfg.contact_reuse_distance,
        cfg.dt,
        manifolds.box_axis_cache.keys_changed(),
    );
    // L9 D9: how this step's pairs join the previous step's tags — classified here, before
    // either path opens the carry, and stamped below, after the pair loop.
    let carry = manifolds.pair_carry.source(contact_pairs, &scratch.rows).with_reuse(reuse);
    // L5: the flag is a request; the dispatch returns 0 whenever it runs no chunk, and
    // then the serial loop below produces the step's streams.
    let (chunks, parallel_skips) = if cfg.parallel_narrowphase {
        try_parallel_sets::<SETS>(manifolds, bodies, pairs, prefetched, carry, cls)
    } else {
        (0, 0)
    };
    let held_skips = if chunks == 0 {
        narrowphase_serial_sets::<SETS>(manifolds, bodies, pairs, prefetched, carry, cls)
    } else {
        parallel_skips
    };
    manifolds.pair_carry.stamp(contact_pairs, &scratch.rows);

    // Profiling: the step's narrowphase work, counted after the loop from what it emitted,
    // so the loop itself carries no instrument. The point sum walks the solver's manifolds
    // only while the profiler is armed.
    counter!(PHYS_NP_PAIRS, pairs.len() as u64);
    counter!(PHYS_NP_MANIFOLDS, manifolds.solver_manifolds().len() as u64);
    counter!(
        PHYS_NP_POINTS,
        manifolds.solver_manifolds().iter().map(|m| u64::from(m.count)).sum::<u64>()
    );
    counter!(PHYS_NP_CHUNKS, chunks as u64);
    // L9: the step's pair classes, from the tags in one walk, only while the profiler is armed.
    // `full + reused + sep_hits + non-box + held-skipped = pairs` (the closure a reader checks).
    if boyko_diag::zone_enabled!(PHYS_NP_FULL) {
        let classes = manifolds.pair_classes();
        counter!(PHYS_NP_REUSED, classes.reused);
        counter!(PHYS_NP_SEP_HITS, classes.sep_hits);
        counter!(PHYS_NP_FULL, classes.full);
    }
    held_skips
}

/// [`narrowphase_serial_with`] with no pair carry and contact reuse off: every pair misses its
/// join (and still writes its tag). For the tests' direct callers, which hold no `ContactPairs`;
/// the next system step's carry is then a Reset.
#[cfg(test)]
pub(crate) fn narrowphase_serial(
    manifolds: &mut Manifolds,
    bodies: &[BodyState],
    pairs: &[(BodyIndex, BodyIndex)],
    prefetched: bool,
) {
    narrowphase_serial_with(manifolds, bodies, pairs, prefetched, CarryIn::NONE);
}

/// [`narrowphase_serial_sets`] on the `Off` arm: for the tests' direct callers.
#[cfg(test)]
pub(crate) fn narrowphase_serial_with(
    manifolds: &mut Manifolds,
    bodies: &[BodyState],
    pairs: &[(BodyIndex, BodyIndex)],
    prefetched: bool,
    carry: CarryIn<'_>,
) {
    let _ = narrowphase_serial_sets::<false>(manifolds, bodies, pairs, prefetched, carry, &[]);
}

/// The serial narrowphase loop: every candidate pair in `(min, max)` order, the
/// manifold pushed into the solver buffer or the sensor-overlap buffer, the pair's tag
/// and reuse record written (L9 D9), and a box-box pair's chosen axis written into the
/// hysteresis table in the same iteration. The per-row orientation frames are filled and
/// the pair carry opened first (L9 D2, D9).
///
/// The path a step takes whenever the parallel narrowphase does not dispatch, and the
/// oracle that path's gates compare against. `prefetched` is what
/// `BoxAxisCache::begin_frame_synced` returned for this frame; `carry` is what
/// `PairCarry::source` returned for it.
///
/// `SETS` is L10's arm (design 08 D1′), with the parallel chunks' route predicate: on the
/// `Sets` arm a non-sensor pair with a held endpoint is not collided — it writes the held-skip
/// tag, no axis (the serial form of the chunks' `AXIS_NONE` commit) and no manifold — and the
/// loop returns how many it skipped. On the `Off` arm (`cls` unread) every pair is collided and
/// it returns `0`.
pub(crate) fn narrowphase_serial_sets<const SETS: bool>(
    manifolds: &mut Manifolds,
    bodies: &[BodyState],
    pairs: &[(BodyIndex, BodyIndex)],
    prefetched: bool,
    carry: CarryIn<'_>,
    cls: &[RowCls],
) -> u32 {
    // Disjoint field borrows of one `Manifolds`: two refill views, the hysteresis cache,
    // the frame column and the pair carry. The views are taken once for the whole pair
    // loop, not per push.
    let Manifolds {
        manifolds: solver_out,
        sensor_overlaps,
        box_axis_cache: axis_cache,
        row_frames,
        pair_carry,
        ..
    } = manifolds;
    let mut out = solver_out.build_view();
    out.clear();
    // S5: the sensor-overlap signal is rebuilt every step alongside the solver
    // buffer (capacity reused). Empty in any world with no `Sensor` id.
    let mut sensor_out = sensor_overlaps.build_view();
    sensor_out.clear();
    // L9 D2: every box row's frame, once, before the first pair (or `None`: the pairs build
    // their frames per pair, the same bits).
    let frames = fill_row_frames(row_frames, bodies, pairs.len(), carry.reuse().on);
    // L9 D9: the previous step's tags and records, joined pair by pair with one monotone cursor.
    let (join, tag_column, record_column) = pair_carry.open(carry, pairs.len());
    let reuse = join.reuse();
    let mut join = join.cursor();
    let mut tag_view = tag_column.build_view();
    let tags = tag_view.as_mut_slice();
    let mut record_view = record_column.build_view();
    let records = record_view.as_mut_slice();
    let mut held_skips = 0u32;

    for (k, &(a, b)) in pairs.iter().enumerate() {
        let ba = &bodies[a.0 as usize];
        let bb = &bodies[b.0 as usize];
        // S5: a pair where EITHER body is a sensor is an OVERLAP, not a contact —
        // the manifold is generated identically (same geometry) but diverted to
        // `sensor_overlaps` below so the solver never resolves it. Computed once
        // per pair; `false` for every pair in a sensor-free world (the 0%-gate:
        // the routing branch then always takes the `out` arm, byte-identical to
        // the pre-S5 push).
        let is_overlap = ba.is_sensor || bb.is_sensor;

        let PairOut { manifold, axis, tag, record } = match np_route::<SETS>(cls, a, b) {
            // L10 (design 08 D1′): a held endpoint — the held-skip tag, no `set` (the mirror
            // keys the held pair, design 06 D-C) and no manifold.
            Route::Skip => {
                held_skips += 1;
                PairOut::held_skip()
            }
            Route::Stream => collide_pair(
                a,
                b,
                ba,
                bb,
                frames,
                reuse,
                || join.prev(a, b),
                || axis_cache.read_hint(prefetched, k, a, b),
            ),
            Route::Restore => {
                unreachable!("invariant: no pair routes to the kept source before L10 C3b builds it")
            }
        };
        if let Some(axis) = axis {
            // Persist this frame's chosen reference axis for next frame's
            // hysteresis bias (per body pair, deterministic).
            axis_cache.set(a, b, axis);
        }
        // Past the tag column's reserve a pair goes untagged, and the carry's stamp stays
        // invalid (`PairCarry::stamp`), so no later step reads a tag this step did not write.
        if let Some(slot) = tags.get_mut(k) {
            *slot = tag;
        }
        // A record exists only on a step that reuses, which `open` grew the column for.
        if let Some(record) = record {
            records[k] = record;
        }

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
    held_skips
}

/// One candidate pair's collision (L9 D9): what both narrowphase paths store for it.
pub(crate) struct PairOut {
    /// The manifold, or `None` when the shapes do not touch (or every point of a reused face
    /// lifted off, D6).
    pub(crate) manifold: Option<Manifold>,
    /// The SAT axis the hysteresis table stores for the pair this step, or `None` when it stores
    /// nothing.
    pub(crate) axis: Option<usize>,
    /// The pair's tag.
    pub(crate) tag: PairTag,
    /// The reuse record the pair writes at its slot, iff its tag carries `REC` (L9b).
    pub(crate) record: Option<ReuseRecord>,
}

impl PairOut {
    /// The output of a pair whose generator ran on today's path: no record.
    #[inline]
    fn today(manifold: Option<Manifold>, axis: Option<usize>, tag: PairTag) -> Self {
        Self { manifold, axis, tag, record: None }
    }

    /// The output of a non-box pair: its manifold and a tag that records only whether it was
    /// emitted.
    #[inline]
    fn non_box(manifold: Option<Manifold>) -> Self {
        let tag = PairTag::non_box(pushes(manifold.as_ref()));
        Self::today(manifold, None, tag)
    }

    /// The output of a pair L10's sleep-skip did not collide because an endpoint is held
    /// (design 08 D1′): no manifold, no axis, the held-skip tag, no record.
    #[inline]
    pub(crate) fn held_skip() -> Self {
        Self::today(None, None, PairTag::HELD_SKIP)
    }
}

/// Collides one candidate pair `(a, b)` by the two bodies' shapes (L9 D9): the manifold (or
/// `None` when the shapes do not touch), the SAT axis the hysteresis table stores, the pair's tag
/// and its reuse record.
///
/// A pure function of the two bodies, their frames, the step's reuse parameters, what the pair
/// left in the previous step and the hint, which is what lets the parallel narrowphase run it per
/// pair on any thread. `prev` (the tag and the record the pair's two bodies wrote in the previous
/// step, through the join) and `hint` are called only for a box-box pair — the only generator that
/// reads either — so they cost nothing on the other shape pairs, and `hint` is not called when the
/// carried separating axis still separates the pair (L9a (ii)) or the pair reuses its record
/// (L9b, D10). Both narrowphase paths call this one function.
///
/// `frames` is the step's per-row orientation frame column (L9 D2), or `None` on a step
/// whose fill declined; a box pair then builds its two frames with the same
/// [`RowFrame::axes_of`], so the result does not depend on which (`narrowphase/reuse.rs`).
///
/// - **sphere-sphere**: inline single-point center-to-center contact (the W2 path).
/// - **sphere-box**: [`sphere_box_contact`], which emits normal A→B with
///   body_a = sphere, body_b = box.
/// - **box-sphere**: the same generator with the sphere as A and the box as B (keyed
///   `b, a`), remapped to `(a, b)` order so the dense rows match and the normal runs
///   A(box)→B(sphere).
/// - **box-box**: [`collide_box_pair`] — the carried separating axis, the reuse record
///   (L9b), then the classifier biased by the hint.
#[inline]
#[allow(clippy::too_many_arguments)]
pub(crate) fn collide_pair<'r>(
    a: BodyIndex,
    b: BodyIndex,
    ba: &BodyState,
    bb: &BodyState,
    frames: Option<&[RowFrame]>,
    reuse: ReuseStep,
    prev: impl FnOnce() -> Prev<'r>,
    hint: impl FnOnce() -> Option<usize>,
) -> PairOut {
    match (ba.shape, bb.shape) {
        (ColliderShape::Sphere { radius: ra }, ColliderShape::Sphere { radius: rb }) => {
            PairOut::non_box(sphere_sphere_manifold(a, b, ba, bb, ra, rb))
        }
        (ColliderShape::Sphere { radius }, ColliderShape::Box { half_extents }) => PairOut::non_box(
            sphere_box_contact(a, b, ba.position, radius, bb.position, bb.rotation, half_extents),
        ),
        (ColliderShape::Box { half_extents }, ColliderShape::Sphere { radius }) => PairOut::non_box(
            sphere_box_contact(b, a, bb.position, radius, ba.position, ba.rotation, half_extents)
                .map(flip_manifold),
        ),
        (ColliderShape::Box { half_extents: ha }, ColliderShape::Box { half_extents: hb }) => {
            let (oa, ob) = match frames {
                Some(frames) => (
                    Obb::from_frame(ba.position, &frames[a.0 as usize], ha),
                    Obb::from_frame(bb.position, &frames[b.0 as usize], hb),
                ),
                None => (
                    Obb::new(ba.position, ba.rotation, ha),
                    Obb::new(bb.position, bb.rotation, hb),
                ),
            };
            let radii = || match frames {
                Some(frames) => (frames[a.0 as usize].radius, frames[b.0 as usize].radius),
                None => (ha.length(), hb.length()),
            };
            let out = collide_box_pair(a, b, ba, bb, &oa, &ob, radii, reuse, prev(), hint);
            // L9's axis commit is `PairTag::rekeys` (L10 C0, design 06 D-C): a pair writes its
            // hysteresis axis only with a tag that re-keys and carries that axis, and on a step
            // whose key set changed it writes one iff its tag re-keys.
            debug_assert!(
                out.axis.is_none_or(|axis| out.tag.rekeys() && out.tag.axis() == Some(axis as u8))
                    && (!reuse.rekey || out.axis.is_some() == out.tag.rekeys()),
                "invariant: a box pair's axis write is PairTag::rekeys: axis {:?}, tag {:#06x}, \
                 rekey step {}",
                out.axis,
                out.tag.bits(),
                reuse.rekey
            );
            out
        }
    }
}

/// The box-box arm of [`collide_pair`] on the boxes `(oa, ob)`:
///
/// 1. **The carried separating axis (L9a (ii)).** A `SEP` tag's axis is evaluated first; while it
///    separates, the pair is separated and nothing else runs.
/// 2. **The reuse record (L9b).** A `REC` tag's record, flipped into the current roles, is kept
///    iff the pair is slow ([`slow_geom`]) and passes [`criterion`]: its [refresh](refresh) is the
///    output, the record is copied to this step's slot, and the hysteresis table is written only on
///    a step whose key set changed (ruling W1).
/// 3. **The full collision** — the classifier biased by the hint. A slow pair builds a record from
///    a contact and emits that record's refresh (D7); any other pair emits the contact.
///
/// A `REC` tag never carries `SEP`, so at most one of 1 and 2 applies.
#[inline]
#[allow(clippy::too_many_arguments)]
fn collide_box_pair(
    a: BodyIndex,
    b: BodyIndex,
    ba: &BodyState,
    bb: &BodyState,
    oa: &Obb,
    ob: &Obb,
    radii: impl Fn() -> (f32, f32),
    reuse: ReuseStep,
    prev: Prev<'_>,
    hint: impl FnOnce() -> Option<usize>,
) -> PairOut {
    // The pair's reuse geometry, computed at most once: `None` until needed, then whether the pair
    // is slow (`Some(Some(g))`) or takes today's path (`Some(None)`).
    let mut geom = None;
    if let Some(stored) = prev.record {
        let slow = slow_geom(ba, bb, oa, ob, &radii, reuse);
        geom = Some(slow);
        if let Some(g) = slow {
            let record = if prev.flipped { stored.flipped() } else { *stored };
            if let Some(out) = reuse_record(&record, a, b, ba, bb, oa, ob, &g, prev.tag, reuse) {
                return out;
            }
        }
    }
    match box_box_classify_carried(oa, ob, a, b, prev.tag.sep_axis(), hint) {
        BoxBoxOutcome::Contact(c) => {
            let slow = geom.unwrap_or_else(|| slow_geom(ba, bb, oa, ob, &radii, reuse));
            match slow {
                Some(g) => record_contact(c, a, b, ba, bb, oa, ob, &g, reuse),
                None => {
                    let tag = PairTag::box_contact(c.reference_axis, c.manifold.count > 0);
                    PairOut::today(Some(c.manifold), Some(c.reference_axis), tag)
                }
            }
        }
        BoxBoxOutcome::Separated(axis) => {
            debug_assert!(
                axis < SAT_AXIS_COUNT,
                "invariant: a separating SAT axis is canonical 0..15"
            );
            PairOut::today(None, None, PairTag::box_separated(axis, false))
        }
        BoxBoxOutcome::StillSeparated(axis) => {
            PairOut::today(None, None, PairTag::box_separated(axis, true))
        }
        BoxBoxOutcome::NoContact => PairOut::today(None, None, PairTag::BOX_NO_CONTACT),
    }
}

/// The reuse geometry of a box pair that takes the reuse path this step (L9b D3, D8), or `None`
/// when it takes today's: reuse off, a sensor on either side, τ_eff not positive (a degenerate
/// box, or τ = 0), or a fast pair.
#[inline]
fn slow_geom(
    ba: &BodyState,
    bb: &BodyState,
    oa: &Obb,
    ob: &Obb,
    radii: &impl Fn() -> (f32, f32),
    reuse: ReuseStep,
) -> Option<PairGeom> {
    if !reuse.on || ba.is_sensor || bb.is_sensor {
        return None;
    }
    let (ra, rb) = radii();
    let g = PairGeom::new(oa, ob, ra, rb, reuse.tau);
    (g.tau_eff > 0.0 && !is_fast(ba, bb, &g, reuse.dt2)).then_some(g)
}

/// A slow pair's record `record` (in the current roles) on this step's boxes: the hit's output,
/// or `None` for a miss (the shapes changed, the criterion failed, the edge degenerated). `tag` is
/// the previous step's tag in the current roles, whose axis is the record's.
#[inline]
#[allow(clippy::too_many_arguments)]
fn reuse_record(
    record: &ReuseRecord,
    a: BodyIndex,
    b: BodyIndex,
    ba: &BodyState,
    bb: &BodyState,
    oa: &Obb,
    ob: &Obb,
    g: &PairGeom,
    tag: PairTag,
    reuse: ReuseStep,
) -> Option<PairOut> {
    debug_assert!(tag.axis().is_some(), "invariant: a REC tag carries its record's SAT axis");
    let axis = usize::from(tag.axis()?);
    if !criterion(record, oa, ob, ba.rotation, bb.rotation, g) {
        return None;
    }
    match refresh(record, oa, ob, a, b) {
        Refreshed::Contact(m) => {
            let pushed = m.count > 0;
            Some(PairOut {
                manifold: pushed.then_some(m),
                // D10 and ruling W1: a hit writes no axis, except on a step whose key set changed,
                // where it re-keys its entry with the axis its record was built on.
                axis: reuse.rekey.then_some(axis),
                tag: PairTag::box_recorded(axis, pushed, true),
                record: Some(record.with_parity(reuse.parity)),
            })
        }
        Refreshed::Separated(sep) => {
            Some(PairOut::today(None, None, PairTag::box_separated_by_record(sep)))
        }
        Refreshed::Degenerate => None,
    }
}

/// A slow pair's full collision `c`: the record built from it, and that record's refresh as the
/// output (D7), so the output is a pure function of the record and the poses.
#[inline]
#[allow(clippy::too_many_arguments)]
fn record_contact(
    c: BoxBoxContact,
    a: BodyIndex,
    b: BodyIndex,
    ba: &BodyState,
    bb: &BodyState,
    oa: &Obb,
    ob: &Obb,
    g: &PairGeom,
    reuse: ReuseStep,
) -> PairOut {
    let axis = c.reference_axis;
    let record = build(&c, oa, ob, ba.rotation, bb.rotation, g, reuse.parity);
    match refresh(&record, oa, ob, a, b) {
        Refreshed::Contact(m) => {
            let pushed = m.count > 0;
            PairOut {
                manifold: pushed.then_some(m),
                axis: Some(axis),
                tag: PairTag::box_recorded(axis, pushed, false),
                record: Some(record),
            }
        }
        // An edge record re-evaluates the very axis the full collision chose, on the same
        // boxes: it overlaps and exists. Kept total rather than trusted.
        Refreshed::Separated(_) | Refreshed::Degenerate => {
            debug_assert!(false, "invariant: a record refreshes to a contact on the poses it was built on");
            let tag = PairTag::box_contact(axis, c.manifold.count > 0);
            PairOut::today(Some(c.manifold), Some(axis), tag)
        }
    }
}

/// Whether a generator's output is emitted: some manifold with at least one point (the loops'
/// routing condition), which is the tag's `PUSHED` bit.
#[inline]
fn pushes(m: Option<&Manifold>) -> bool {
    m.is_some_and(|m| m.count > 0)
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
///   Which fold builds those corners is
///   [`PhysicsConfig::sdf_narrowphase`](crate::resources::PhysicsConfig::sdf_narrowphase),
///   read ONCE per step here — default
///   [`Scalar`](crate::resources::SdfNarrowphaseKernel::Scalar), the oracle the GPU
///   goldens are blessed against.
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
    cfg: Res<PhysicsConfig>,
    mut manifolds: ResMut<Manifolds>,
) {
    // Nothing to collide against an empty field (samples to +far everywhere).
    if field.is_empty() {
        return;
    }
    // O9: which box kernel folds the field. Hoisted out of the body loop — one
    // resource read per step, not per body.
    let kernel = cfg.sdf_narrowphase;
    let bodies = scratch.bodies();
    let manifolds = &mut *manifolds;
    let mut out = manifolds.manifolds.build_view();
    let mut sensor_out = manifolds.sensor_overlaps.build_view();

    for (row, body) in bodies.iter().enumerate() {
        // Only a SIMULATED dynamic body collides against the SDF (a parked /
        // static / kinematic body's contact with an immovable field would be two
        // immovable sides — no response; it also keeps the sentinel one-sided path
        // exercised by a real moving body, matching the body-vs-static-floor
        // convention). The `simulated` bit AND-ed with `is_dynamic_row` reproduces
        // the old `body_type == Dynamic && inv_mass != 0` gate (Decision 3).
        if !body.simulated || !is_dynamic_row(body.inv_mass) {
            continue;
        }
        let a = BodyIndex(row as u32);
        // S5: a sensor body's SDF overlap is reported, not resolved — divert it to
        // the overlap buffer so the solver's one-sided wall push never fires on it
        // (the 0%-gate: `is_sensor` is `false` for every body in a sensor-free
        // world, so this always takes the `out` arm — byte-identical to pre-S5).
        let dst = if body.is_sensor { &mut sensor_out } else { &mut out };
        match body.shape {
            ColliderShape::Sphere { radius } => {
                if let Some(m) = sphere_sdf_manifold(a, body, radius, &field) {
                    dst.push(m);
                }
            }
            ColliderShape::Box { half_extents } => {
                if let Some(m) = box_sdf_manifold(a, body, half_extents, &field, kernel) {
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
///
/// # Kernel selection (O9) — why the default is SCALAR
///
/// `kernel` picks which fold builds the corner distances and gradients, and the
/// DEFAULT [`SdfNarrowphaseKernel::Scalar`] is load-bearing, not inertia: the
/// [`Avx2`](SdfNarrowphaseKernel::Avx2) fold diverges from the scalar oracle on the
/// SIGN OF ZERO (`+0` where the oracle gives `-0`) at a `±0` tie, the fix is
/// owner-deferred (2026-09-02), and the scalar fold is the CPU oracle the committed
/// GPU goldens are blessed against. **Do not turn this back into a `cfg`.** It WAS
/// one — the arm was chosen by `cfg(target_feature = "avx2")` alone until
/// 2026-09-03, so setting the workspace ISA baseline to `x86-64-v3` moved every
/// build onto the divergent arm with nothing at any layer able to say otherwise.
/// The branch costs one predictable compare per box body per step, taken OUTSIDE
/// the 8-corner loop; that is the price of a numeric choice being a decision
/// somebody makes rather than a side effect of a build flag.
#[inline]
fn box_sdf_manifold(
    a: BodyIndex,
    body: &BodyState,
    half_extents: Vec3,
    field: &SdfField,
    kernel: SdfNarrowphaseKernel,
) -> Option<Manifold> {
    let max_points = crate::math::MAX_CONTACT_POINTS;
    // The kept contacts, deepest-first (most negative separation). Fixed capacity,
    // no allocation: at most `MAX_CONTACT_POINTS` are retained.
    let mut kept: [(ContactPoint, Vec3); crate::math::MAX_CONTACT_POINTS] =
        [(ContactPoint::default(), Vec3::ZERO); crate::math::MAX_CONTACT_POINTS];
    let mut kept_len = 0usize;

    match kernel {
        SdfNarrowphaseKernel::Scalar => {
            box_sdf_manifold_scalar(body, half_extents, field, &mut kept, &mut kept_len, max_points);
        }
        SdfNarrowphaseKernel::Avx2 => {
            // The kernel is x86-64 + AVX2 only and is never taken under Miri (no
            // intrinsic support), so the variant degrades to the scalar fold rather
            // than failing to build — selecting it is legal on every target.
            #[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(miri)))]
            box_sdf_manifold_avx2(body, half_extents, field, &mut kept, &mut kept_len, max_points);
            #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2", not(miri))))]
            box_sdf_manifold_scalar(body, half_extents, field, &mut kept, &mut kept_len, max_points);
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

/// The VERBATIM frozen scalar body of [`box_sdf_manifold`] — the bit-oracle every
/// other arm is measured against, and the arm a default build runs.
///
/// Samples the 8 OBB corners one at a time through the frozen scalar leaf
/// (`sample_sdf` → `sdf_edit_list` / `sdf_edit_list_normal`) and fills
/// `kept` / `kept_len` with the penetrating corners' contacts, deepest-first.
///
/// Compiled on EVERY target: it is both the default arm and the fallback the
/// [`Avx2`](SdfNarrowphaseKernel::Avx2) variant degrades to off x86-64 / under Miri.
/// It folds through `sdf_edit_list`, which IS the sole CPU↔GPU oracle (the W4
/// invariant: the x8 kernel is a CPU-only accelerator and is never a golden input),
/// so a reordered or "simplified" operation here changes the number a committed
/// golden is compared against. It changes only with the goldens.
fn box_sdf_manifold_scalar(
    body: &BodyState,
    half_extents: Vec3,
    field: &SdfField,
    kept: &mut [(ContactPoint, Vec3); crate::math::MAX_CONTACT_POINTS],
    kept_len: &mut usize,
    max_points: usize,
) {
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
        insert_deepest(kept, kept_len, max_points, point, normal);
    }
}

/// O9 — the AVX2 batched body of [`box_sdf_manifold`]: fills `kept` / `kept_len`
/// with the penetrating corners' contacts. Reached ONLY by an explicit
/// [`SdfNarrowphaseKernel::Avx2`], never by a build flag.
///
/// ⚠ **The intended property is `f32::to_bits` identity with
/// [`box_sdf_manifold_scalar`], and it does NOT hold.** Everything this function
/// itself does is byte-identical to the scalar arm (see the per-pass notes below);
/// the divergence is one level down, in
/// [`sdf_edit_list_x8`](crate::sdf_simd::sdf_edit_list_x8), which returns `+0` where
/// the scalar `sdf_edit_list` returns `-0` at a `±0` tie —
/// `MAXPS`/`MINPS` take the second operand on such a tie where `f32::max`/`min` take
/// the first, and `clamp01_x8`'s operand swap does not cover every tie site in the
/// fold. Its standing gate is
/// `sdf_simd::o9_kernel_tests::x8_bits_eq_scalar_bits_widened_proptest`, which is
/// `#[ignore]`d and RED; the fix is owner-deferred (2026-09-02). The witness is
/// adversarial and the manifold-level differential
/// (`box_sdf_manifold_matches_scalar_oracle`) passes, but `+0 == -0` is exactly what
/// a value comparison cannot see, which is why this arm is opt-in.
///
/// Two batched passes share the one [`sdf_edit_list_x8`](crate::sdf_simd::sdf_edit_list_x8)
/// kernel:
///
/// 1. **Distances**: the 8 OBB corners (the SAME fixed sign-pattern order +
///    `position + rotation·local` world transform as the scalar arm) are evaluated
///    in ONE 8-wide call — lane `i` is the batched counterpart of the scalar
///    `sdf_edit_list(edits, corner_i)`, and matches it bit-for-bit everywhere except
///    the `±0` tie named above.
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
/// Registered for every world whose solver is [`ColoredSoftStepSolver`] (the
/// default — the colored solve consumes the partition) and by
/// [`add_physics_colored`](crate::plugin::add_physics_colored) with any solver,
/// AFTER narrowphase and BEFORE the solve, so the partition reflects the same
/// manifold set the solver consumes. With the reference
/// [`SoftStepSolver`](crate::solver::SoftStepSolver) on the `add_physics_colored`
/// path the partition is NOT consumed — that solver still solves in manifold order,
/// so its output is byte-identical whether this stage runs or not (the O4 0%-gate).
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
    // sites route through `is_dynamic_row` so they cannot drift (see its docs), over the
    // same `effective_inv_mass` of the same row (L10 D8, ruling W1; no row is held before
    // L10 C3b).
    let is_dynamic = |row: u32| {
        let i = row as usize;
        i < bodies.len() && is_dynamic_row(effective_inv_mass(bodies[i].inv_mass, false))
    };
    graph.build(manifolds.solver_manifolds(), n_dynamic, is_dynamic);
}

/// Runs the colored TGS-Soft solver for one step over the prebuilt
/// [`ConstraintGraph`] (Phase O5, Decision 7) — the default world's solve since
/// 2026-09-18, registered in place of the generic [`physics_solve_step`] whenever
/// the pipeline is wired with `S = `[`ColoredSoftStepSolver`]
/// ([`DefaultRigidSolver`](crate::solver::DefaultRigidSolver)).
///
/// Calls [`ColoredSoftStepSolver::solve_colored`](crate::solver::ColoredSoftStepSolver::solve_colored)
/// directly (not through [`RigidSolver::solve`], whose signature carries no
/// graph): the solver builds its cohort tables (`CohortColumns`) in color order, runs the
/// substep loop solving colors `0..n_colors` sequentially (a Gauss-Seidel sweep
/// across colors), then stores the converged impulses by manifold index
/// (`solver::warm_records`, L11 C1). Registered ONLY on the colored path, where it
/// stands in for `physics_solve_step` — the generic step stage is NOT registered,
/// so the two never both run. A world wired with the reference
/// [`SoftStepSolver`](crate::solver::SoftStepSolver) never reaches this stage (that
/// solver is byte-untouched). Per color the sweep runs the O7 AVX2 cohort kernel
/// when [`PhysicsConfig::simd_solve`] is on (the default) and the scalar oracle
/// otherwise; the two produce the same bits.
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
        solver.solve_colored_sleeping(
            &cfg,
            manifolds.solver_manifolds(),
            &graph,
            &mut scratch,
            &mut sleep,
        );
    } else {
        // Sleeping off: byte-identical to the O6/O7 colored path; `IslandSleep` is
        // resolved (so the param exists) but never read or written.
        let _ = &mut sleep;
        solver.solve_colored(&cfg, manifolds.solver_manifolds(), &graph, &mut scratch);
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
    solver.solve(&cfg, manifolds.solver_manifolds(), &mut scratch);
}

/// Writes the solved snapshot back into the [`RigidBody`] column for touched
/// rows (plan D3 stage 5 / IM-1).
///
/// Walks the body set with a [`BodyQuery`], the query type [`physics_gather`] also
/// takes: the two stages' archetype selections are equal (checked once at wire-up, see
/// [`crate::body_set`]) and both sweep the matched archetypes in ascending id order, so
/// a manual row counter makes walk position `i` the snapshot row `i` = [`BodyIndex`].
/// For each touched row the whole [`RigidBody`] is written back through the
/// [`Mut`](boyko_ecs::ecs::core::iters::query::Mut) guard, so
/// `Changed<RigidBody>` fires for moving bodies (MINOR-2: a documented
/// whole-body choice; a later refinement may split position/velocity).
///
/// Correct UNDER the "no structural change between gather and apply" invariant:
/// no physics stage spawns, despawns or migrates an entity, so row `i` is the same
/// body in both passes. Ordering alone does not uphold it for other systems: the
/// executor applies each finished system's `Commands` in the apply window before it
/// dispatches further systems, so a `Commands` system with no ordering against the
/// physics block can land a structural change between the gather and this stage.
/// Order such a system before the gather
/// (`builder.add_system(spawner).before_set(PhysicsGatherSet)`, see
/// [`PhysicsGatherSet`](crate::plugin::PhysicsGatherSet)); this crate offers no set
/// for ordering after this stage. A violation is caught in debug builds by the
/// `debug_assert!` below, which today blocks the process from a worker thread
/// instead of failing it (lane A6); a release build can write solved state into the
/// wrong entities.
//
// `clippy::needless_pass_by_value`: `Res<_>` is a by-value `SystemParam` read
// via a `&*` reborrow — the same false-positive as the demo's `apply_ball_motion`.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_apply(mut query: BodyQuery<BodyApplyData>, scratch: Res<SolverScratch>) {
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
    //! [`box_sdf_manifold`] is private and both arms now compile in every build; the
    //! arm is chosen by the [`SdfNarrowphaseKernel`] argument. These cases pass
    //! [`SdfNarrowphaseKernel::Avx2`] — the arm that needs watching — and compare the
    //! result against an INDEPENDENT scalar reference ([`scalar_box_sdf_manifold`])
    //! that folds the field through the FROZEN scalar oracle [`sample_sdf`] /
    //! [`boyko_sdf_math::sdf_edit_list`] and replays the SAME post-processing (corner
    //! build, `d >= 0` skip, the seam-skip, `−gradient` normal,
    //! `feature_vertex_face`, [`insert_deepest`], deepest-first order).
    //!
    //! - In a `+avx2` build this is a TRUE scalar-oracle-vs-AVX2-arm differential:
    //!   the `Avx2` variant runs [`box_sdf_manifold_avx2`], the reference runs the
    //!   scalar leaf.
    //! - Off x86-64 / under Miri the `Avx2` variant degrades to the scalar fold, so
    //!   both sides fold the same leaf and it becomes a self-consistency check of the
    //!   reference (still useful: it pins the reference against the shipped scalar
    //!   arm).
    //!
    //! ⚠ **A pass here is NOT bit-identity of the two folds.** These are manifold
    //! shapes over ordinary scenes; the known `±0` divergence between the x8 and
    //! scalar leaves lives in an adversarial tie palette and is invisible to a
    //! `Manifold` comparison in either direction (`+0 == -0` compares equal, and
    //! `to_bits` equality here only says no case in THIS generator hit the tie). The
    //! gate that does see it is
    //! `sdf_simd::o9_kernel_tests::x8_bits_eq_scalar_bits_widened_proptest`, and it is
    //! RED. Read a green here as "the wrapper is faithful", never as "the arms agree".
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
            simulated: true,
            kinematic: false,
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

            let got = box_sdf_manifold(a, &body, half, &field, SdfNarrowphaseKernel::Avx2);
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
        let got =
            box_sdf_manifold(BodyIndex(0), &body, Vec3::new(1.0, 1.0, 1.0), &field, SdfNarrowphaseKernel::Avx2);
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
        let got = box_sdf_manifold(BodyIndex(0), &body, half, &field, SdfNarrowphaseKernel::Avx2);
        let want = scalar_box_sdf_manifold(BodyIndex(0), &body, half, &field);
        assert_manifold_bit_eq(&got, &want, "all-corners-penetrate");
        // It must produce a manifold capped at MAX_CONTACT_POINTS.
        let m = got.expect("a fully-submerged box must produce a manifold");
        assert!(
            m.count as usize <= crate::math::MAX_CONTACT_POINTS,
            "manifold must be capped at MAX_CONTACT_POINTS"
        );
    }

    /// No-FMA / no-approx grep gate: `systems.rs` must contain ZERO fused
    /// (`fmadd` / `fmsub` / `fnmadd` / `fnmsub` / `fmaddsub` / `fmsubadd`), ZERO
    /// approximate (`rsqrt` / `rcp`) and ZERO `mul_add` / `algebraic_` CALL-SITES.
    ///
    /// The sibling of `sdf_simd::…::sdf_simd_has_no_fma_or_approx_callsites` and
    /// `solver::simd::…::solver_simd_has_no_fma_or_approx_callsites`, with the same
    /// needle list, the same comment skip and the same non-vacuity witness — a
    /// deliberate copy, because the four must not diverge.
    ///
    /// **Why it did not exist until 2026-09-03:** this file's O9 kernel
    /// ([`box_sdf_manifold_avx2`]) sits behind `cfg(target_feature = "avx2")`, and
    /// nothing enabled AVX2 in this workspace until the `x86-64-v3` baseline landed
    /// on 2026-09-02. A vectorised kernel that was never COMPILED was also never
    /// CENSUSED, so the day it started building it became the one AVX2 file in the
    /// crate with nothing keeping it clean. It is clean today; that is the property
    /// this test freezes, not a claim about the past.
    ///
    /// The stake here is the same one the other two carry: this kernel feeds
    /// `sdf_edit_list_x8`, whose fold is the CPU oracle the committed GPU goldens are
    /// compared against. A fused op rounds ONCE where the scalar leaf rounds TWICE,
    /// and `rsqrt`/`rcp` are ~12-bit approximations that differ between Intel and
    /// AMD — either would move a golden without moving a line of shader code.
    ///
    /// Doc-comment prose naming the banned ops (this comment does) is allowed — only
    /// NON-comment lines are scanned.
    #[test]
    fn systems_has_no_fma_or_approx_callsites() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("systems.rs");
        let contents = std::fs::read_to_string(&path).expect("systems.rs must be readable");

        // Match a CALL-SITE: each stem completed to a real `_ps(` invocation, over
        // both vector widths. The needles are ASSEMBLED from fragments at runtime so
        // no full call token appears as a string literal in THIS source — the census
        // scans its own file, so a literal would flag the definition line.
        let suffix = "_ps(";
        let widths = ["_mm256_", "_mm_"];
        let stems = ["fmadd", "fmsub", "fnmadd", "fnmsub", "fmaddsub", "fmsubadd", "rsqrt", "rcp"];
        let mut banned: Vec<String> = Vec::with_capacity(widths.len() * stems.len() + 2);
        for w in widths {
            for s in stems {
                banned.push(format!("{w}{s}{suffix}"));
            }
        }
        // The safe-Rust route to the same single rounding — reachable without ever
        // typing an intrinsic, which an intrinsic-only ban would never see.
        banned.push(format!("{}{}", "mul_add", "("));
        // `algebraic_mul` / `_add` / `_sub` / `_div` / `_rem` (stable 1.98): the
        // sanctioned per-operation fast-math API, which permits exactly the two
        // freedoms — contraction and reassociation — this crate's determinism rests
        // on refusing. The stem alone is banned so a UFCS spelling cannot defeat it.
        banned.push(format!("{}{}", "algebraic", "_"));

        let mut hits = Vec::new();
        for (i, line) in contents.lines().enumerate() {
            let trimmed = line.trim_start();
            // Skip doc / line comments — prose may name the banned ops to document
            // the prohibition. `//!` starts with `//`, so one check covers both.
            if trimmed.starts_with("//") {
                continue;
            }
            for b in &banned {
                if line.contains(b.as_str()) {
                    hits.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
                }
            }
        }
        assert!(
            hits.is_empty(),
            "no-FMA/no-approx invariant violated: systems.rs has banned op call-sites (the O9 \
             box-vs-SDF kernel feeds the CPU fold the GPU goldens are blessed against):\n{}",
            hits.join("\n"),
        );

        // Non-vacuity: a census that scans the wrong text passes for the wrong
        // reason. The witness is ASSEMBLED like the needles rather than written as a
        // literal — a literal witness would be found in the census's own assertion
        // line and the check would pass even over a file with no intrinsics left in
        // it. `loadu` rather than `mul`: this file's kernel packs and stores lanes
        // and delegates the arithmetic to `sdf_edit_list_x8`, so `_mm256_mul_ps(`
        // never appears here and asserting it would fail on correct code.
        let witness = format!("{}{}{}", "_mm256_", "loadu", suffix);
        assert!(
            contents.contains(&witness),
            "census scanned {} but found no `{witness}` call-site — the file moved or was \
             rewritten, so an empty hit list proves nothing",
            path.display(),
        );
    }
}


#[cfg(test)]
mod pair_tag_rekeys_tests {
    //! L10 C0 (design 06 D-C; plan E3): `PairTag::rekeys` is L9's axis commit, extracted. Over
    //! every outcome a box pair can reach — a full contact, a record built, a record hit, a
    //! separation, a carried separation, a box pair with no contact, and the non-box pairs —
    //! a pair writes its hysteresis axis only with a tag that re-keys and carries that axis,
    //! and on a step whose key set changed it writes one iff its tag re-keys. Mutation
    //! `rekeys = has(REC)` turns this red on the first full contact.

    use super::*;
    use crate::components::{Collider, ColliderShape, RigidBody, RigidBodyMass};
    use crate::math::{Mat3, Quat, Vec3};

    /// xorshift64*: a seeded, dependency-free generator.
    struct Rng(u64);

    impl Rng {
        fn unit(&mut self) -> f32 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
        }

        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            lo + (hi - lo) * self.unit()
        }
    }

    fn body(position: Vec3, rotation: Quat, shape: ColliderShape, v: Vec3, inv_mass: f32) -> BodyState {
        let body = RigidBody { position, linear_velocity: v, rotation, angular_velocity: Vec3::ZERO };
        let mass = RigidBodyMass { inv_inertia: Mat3::IDENTITY, inv_mass, restitution: 0.0, friction: 0.5 };
        let collider = Collider { shape, layer: 1, mask: 1 };
        BodyState::from_columns(&body, &mass, &collider, false, true, false)
    }

    /// A static slab, boxes of random orientation packed above it (most pairs touch, the
    /// far ones separate), every third one fast, and two spheres among them.
    fn scene(seed: u64) -> Vec<BodyState> {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let slab = ColliderShape::Box { half_extents: Vec3::new(4.0, 0.5, 4.0) };
        let mut bodies = vec![body(Vec3::new(0.0, -0.5, 0.0), Quat::IDENTITY, slab, Vec3::ZERO, 0.0)];
        for i in 0..10 {
            let q = Quat::new(rng.range(-0.3, 0.3), rng.range(-1.0, 1.0), rng.range(-0.3, 0.3), 1.0)
                .normalize();
            let p = Vec3::new(rng.range(-1.5, 1.5), rng.range(0.3, 1.6), rng.range(-1.5, 1.5));
            let h = Vec3::new(rng.range(0.3, 0.6), rng.range(0.3, 0.6), rng.range(0.3, 0.6));
            let v = if i % 3 == 0 { Vec3::new(0.0, -2.0, 0.0) } else { Vec3::ZERO };
            bodies.push(body(p, q, ColliderShape::Box { half_extents: h }, v, 1.0));
        }
        for _ in 0..2 {
            let p = Vec3::new(rng.range(-1.0, 1.0), rng.range(0.3, 1.2), rng.range(-1.0, 1.0));
            bodies.push(body(p, Quat::IDENTITY, ColliderShape::Sphere { radius: 0.4 }, Vec3::ZERO, 1.0));
        }
        bodies
    }

    /// The outcome classes the scenes reached.
    #[derive(Debug, Default)]
    struct Seen {
        full: u64,
        built: u64,
        hits: u64,
        rekeyed_hits: u64,
        separated: u64,
        sep_hits: u64,
        non_box: u64,
    }

    /// The commit rule on one output: `rekey` is whether the step's key set changed.
    fn check(out: &PairOut, rekey: bool, seen: &mut Seen) {
        let tag = out.tag;
        if let Some(axis) = out.axis {
            assert!(
                tag.rekeys() && tag.axis() == Some(axis as u8),
                "a pair wrote axis {axis} with tag {:#06x}: its tag must re-key and carry it",
                tag.bits()
            );
        }
        if rekey {
            assert_eq!(
                out.axis.is_some(),
                tag.rekeys(),
                "on a key-change step a pair writes an axis iff its tag re-keys: tag {:#06x}",
                tag.bits()
            );
        }
        if !tag.has(PairTag::BOX) {
            seen.non_box += 1;
        } else if tag.has(PairTag::REC | PairTag::HIT) {
            seen.hits += 1;
            seen.rekeyed_hits += u64::from(out.axis.is_some());
        } else if tag.has(PairTag::REC) {
            seen.built += 1;
        } else if tag.has(PairTag::SEPHIT) {
            seen.sep_hits += 1;
        } else if tag.has(PairTag::SEP) {
            seen.separated += 1;
        } else if tag.axis().is_some() {
            seen.full += 1;
        }
    }

    #[test]
    fn rekeys_is_the_axis_commit_of_every_box_outcome() {
        let mut seen = Seen::default();
        for seed in 0..24 {
            let bodies = scene(seed);
            let n = bodies.len() as u32;
            for reuse_on in [false, true] {
                for rekey in [false, true] {
                    let step = |parity| ReuseStep { on: reuse_on, tau: 1.0e-3, dt2: 1.0 / 3600.0, rekey, parity };
                    for a in 0..n {
                        for b in a + 1..n {
                            let (ia, ib) = (BodyIndex(a), BodyIndex(b));
                            let (ba, bb) = (&bodies[a as usize], &bodies[b as usize]);
                            // The pair's first step: no join, no hint.
                            let first = collide_pair(ia, ib, ba, bb, None, step(false), || Prev::NONE, || None);
                            check(&first, rekey, &mut seen);
                            // Its next step at the same poses reads what the first left: a record
                            // hits, a separating axis still separates, a contact reads its hint.
                            let prev = Prev {
                                tag: first.tag,
                                record: first.record.as_ref().filter(|_| reuse_on),
                                flipped: false,
                            };
                            let hint = first.axis;
                            let next = collide_pair(ia, ib, ba, bb, None, step(true), || prev, || hint);
                            check(&next, rekey, &mut seen);
                        }
                    }
                }
            }
        }
        println!("rekeys coverage: {seen:?}");
        assert!(
            seen.full > 0
                && seen.built > 0
                && seen.hits > 0
                && seen.rekeyed_hits > 0
                && seen.separated > 0
                && seen.sep_hits > 0
                && seen.non_box > 0,
            "anti-vacuity: every outcome class must be reached: {seen:?}"
        );
    }
}
