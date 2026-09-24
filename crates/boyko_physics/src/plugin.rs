//! Physics-pipeline wiring — the free functions (plan D3 / MINOR-1) and the
//! `App`-facing [`PhysicsPlugin`].
//!
//! Two entry shapes over ONE implementation:
//!
//! * [`add_physics_systems`] & friends — the BUILDER form: the caller owns a
//!   `ScheduleBuilder` and an `EcsMaster` and hands both over. This is what every
//!   test / bench / hand-driven world uses. Per MINOR-1 it does NOT call
//!   `builder.build(world)` — that consumes the builder and is the caller's job.
//! * [`PhysicsPlugin`] — the APP form: `app.add_plugin(PhysicsPlugin::new())`.
//!
//! The `App` never lends the world and the fixed-schedule builder at the same
//! time (`App::add_systems_cfg_in` passes the builder to a closure while the
//! `App` — and therefore the world — is already borrowed), so the builder form is
//! UNCALLABLE from a plugin. That is why the implementation is split into
//! [`insert_physics_resources`] (world half) and [`register_physics_pipeline`]
//! (schedule half): each half needs only one of the two borrows, and both entry
//! shapes drive the same pair with the same [`PipelineOpts`].
//!
//! # The solve stage follows the solver type (2026-09-18)
//!
//! Every `add_physics_*::<S>` entry — and [`PhysicsPlugin<S>`] — selects the solve
//! stage from `S` alone, once, at wire-up (cold code). `S = `[`ColoredSoftStepSolver`]
//! — the default world's solver, named
//! [`DefaultRigidSolver`](crate::solver::DefaultRigidSolver) — wires the constraint
//! graph ([`physics_build_graph`]), the colored solve ([`physics_solve_colored`]) and
//! the per-island sleep state ([`IslandSleep`]); any other `S` wires the generic
//! [`physics_solve_step::<S>`](physics_solve_step), unchanged. So the reference
//! [`SoftStepSolver`](crate::solver::SoftStepSolver) is still selected by naming it,
//! and the foundation
//! [`NoopSolver`](crate::solver::NoopSolver) can never be paired with the colored
//! stage (the colored solver owns integration; the no-op solver does not).

use core::marker::PhantomData;
use std::any::TypeId;

use boyko_ecs::ecs::core::app::CoreSchedule;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{ScheduleBuilder, SystemConfig};
use boyko_ecs::{App, Plugin};
use boyko_scene::FixedSet;

use crate::broadphase_policy::{PhysicsStats, select_broadphase};
use crate::broadphase_tree::BroadphaseTree;
use crate::resources::{
    BroadphaseGrid, BroadphaseKind, ConstraintGraph, ContactPairs, IntegrationMode, IslandSleep,
    Manifolds, PhysicsConfig, SolverScratch,
};
use crate::scene_sync::{
    debug_assert_dynamic_bodies_are_roots, sync_body_to_transform, sync_transform_to_body,
};
use crate::sdf_query::SdfField;
use crate::sleep_sets::SleepSets;
use crate::soft::{
    SoftColorScratch, SoftRigidReaction, physics_soft_rigid_apply, physics_soft_step,
    physics_soft_step_colored, physics_soft_step_coupled,
};
use crate::solver::colored::ColoredSoftStepSolver;
use crate::solver::{DefaultRigidSolver, RigidSolver};
use crate::systems::{
    physics_apply, physics_broadphase, physics_broadphase_colored, physics_build_graph,
    physics_gather, physics_integrate, physics_narrowphase, physics_narrowphase_colored,
    physics_narrowphase_sdf, physics_solve_colored, physics_solve_step,
};

/// The pre-build stage handles of the physics pipeline (plan MINOR-1 / OQ3).
///
/// Each field is the stage system's **descriptor index** — the `usize` inside the
/// engine's `SystemKey` (`SystemConfig::key().0`), captured as the pipeline is
/// registered. Returned by [`add_physics_systems`] so the caller can identify /
/// inspect the physics block (e.g. assert the registration order, or correlate a
/// schedule diagnostic to a stage).
///
/// # Why the index, not the `SystemKey`
///
/// The engine's `SystemKey` newtype lives in a `pub(crate)` module and is not
/// re-exported, so it cannot be named by path from this crate — and the physics
/// crate makes ZERO core edits. `SystemKey`'s inner `usize` IS public
/// (`SystemKey(pub usize)`), so the stable, nameable handle this crate can expose
/// is that index. The physics block's intra-order is fully wired internally by
/// [`add_physics_systems`] via `.after(..)`. An external caller orders its OWN
/// systems against the gather by name, through [`PhysicsGatherSet`]; against any
/// other stage it needs a real `SystemKey`, which the engine's privacy currently
/// keeps internal (a pre-existing engine limitation, not introduced here — a
/// future `pub use` of `SystemKey` would let this struct carry the keys directly).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicsStageKeys {
    /// Descriptor index of the [`physics_integrate`] stage — the **block head**.
    /// A caller ordering its own systems before the whole physics block keys off
    /// this (the integrate stage is the first to run and carries no `.after`).
    pub integrate: usize,
    /// Descriptor index of the [`physics_gather`] stage.
    pub gather: usize,
    /// Descriptor index of the cold
    /// [`select_broadphase`](crate::broadphase_policy::select_broadphase) density
    /// policy (P3) — runs `.after(gather)` and `.before(broadphase)` so its fresh
    /// [`BroadphaseKind`] decision feeds this frame's build. Registered on EVERY
    /// path (the `Res<SolverScratch>` / `ResMut<PhysicsConfig>` / `ResMut<PhysicsStats>`
    /// params always resolve); in the default `Manual` mode it only counts bodies
    /// and never overrides the kind (the 0%-gate).
    pub select_broadphase: usize,
    /// Descriptor index of the [`physics_broadphase`] stage, or of
    /// [`physics_broadphase_colored`] on the colored pipeline (L10).
    pub broadphase: usize,
    /// Descriptor index of the [`physics_narrowphase`] stage, or of
    /// [`physics_narrowphase_colored`] on the colored pipeline (L10).
    pub narrowphase: usize,
    /// Descriptor index of the [`physics_narrowphase_sdf`] SDF-collision stage, or
    /// `None` for the body-only [`add_physics_systems`] path (P2 W5).
    ///
    /// Present only when the pipeline was wired by
    /// [`add_physics_sdf`](crate::plugin::add_physics_sdf); it runs AFTER
    /// `narrowphase` and BEFORE `solve` so the solver sees both body-body and
    /// body-vs-SDF contacts.
    pub narrowphase_sdf: Option<usize>,
    /// Descriptor index of the [`physics_build_graph`] constraint-graph stage, or
    /// `None` for the non-colored paths (plan O4 / Decision 7).
    ///
    /// Present when the pipeline was wired with `S = `[`ColoredSoftStepSolver`]
    /// (the default, through ANY entry — the colored solve consumes the graph) or
    /// by [`add_physics_colored`](crate::plugin::add_physics_colored) with any `S`.
    /// It runs AFTER the narrowphase stage(s) and BEFORE `solve`. With another `S`
    /// on the [`add_physics_colored`] path the graph is built but NOT consumed (the
    /// O4 partition-only shape; that solve stays byte-identical).
    pub build_graph: Option<usize>,
    /// Descriptor index of the solve stage: [`physics_solve_colored`] when the
    /// pipeline was wired with `S = `[`ColoredSoftStepSolver`], else
    /// [`physics_solve_step::<S>`](physics_solve_step).
    pub solve: usize,
    /// Descriptor index of the [`physics_soft_step`](crate::soft::physics_soft_step)
    /// SP1 XPBD soft-body pass, or `None` for the non-soft paths (plan O11 SP1).
    ///
    /// Present only when the pipeline was wired by
    /// [`add_physics_soft`](crate::plugin::add_physics_soft); it runs AFTER `solve`
    /// and BEFORE `apply` as a separate position pass on the
    /// [`SoftBody`](crate::soft::SoftBody) columns.
    pub soft_step: Option<usize>,
    /// Descriptor index of the [`physics_apply`] stage.
    pub apply: usize,
    /// The scene-sync stage descriptor indices (std-lib S5), or `None` for a
    /// pipeline wired WITHOUT pose sync ([`add_physics_systems`] and friends).
    ///
    /// Present only when the pipeline was wired by
    /// [`add_physics_systems_with_scene_sync`]: it registers
    /// [`sync_transform_to_body`](crate::scene_sync::sync_transform_to_body)
    /// `.before(integrate)` and
    /// [`sync_body_to_transform`](crate::scene_sync::sync_body_to_transform) +
    /// [`debug_assert_dynamic_bodies_are_roots`](crate::scene_sync::debug_assert_dynamic_bodies_are_roots)
    /// `.after(apply)`, all inside the fixed schedule.
    pub scene_sync: Option<SceneSyncKeys>,
}

/// Descriptor indices of the std-lib S5 scene-sync stages, captured when the
/// pipeline is wired with pose sync ([`add_physics_systems_with_scene_sync`]).
///
/// Like [`PhysicsStageKeys`], each field is the stage system's public
/// `SystemKey` inner index (`SystemConfig::key().0`). The cross-edges to the
/// physics block (`sync_transform_to_body.before(integrate)`,
/// `sync_body_to_transform.after(apply)`) are wired internally where the
/// physics stages' real `SystemKey`s are in scope (the kernel keeps `SystemKey`
/// crate-private, so an external caller cannot wire those edges itself — the
/// reason the sync is wired by the pipeline rather than a standalone function).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneSyncKeys {
    /// Descriptor index of
    /// [`sync_transform_to_body`](crate::scene_sync::sync_transform_to_body) —
    /// runs `.before(integrate)` (Static / Kinematic `Transform` → `RigidBody`).
    pub transform_to_body: usize,
    /// Descriptor index of
    /// [`sync_body_to_transform`](crate::scene_sync::sync_body_to_transform) —
    /// runs `.after(apply)` (Dynamic root `RigidBody` → `Transform`).
    pub body_to_transform: usize,
    /// Descriptor index of
    /// [`debug_assert_dynamic_bodies_are_roots`](crate::scene_sync::debug_assert_dynamic_bodies_are_roots)
    /// — the cold parented-dynamic tripwire, also `.after(apply)`.
    pub parented_dynamic_guard: usize,
}

/// Initial reserve for the reused step buffers.
///
/// The buffers grow on demand (and keep their capacity across steps), so this is
/// only the first-frame reserve to avoid early reallocation churn — not a cap.
const INITIAL_BODY_CAPACITY: usize = 1024;

/// Inserts the physics resources on `world` and registers the physics pipeline
/// on `builder`, returning the stage handles (plan D3 / MINOR-1).
///
/// Resources inserted: [`PhysicsConfig`], [`ContactPairs`], [`Manifolds`],
/// [`SolverScratch`] (all reused, capacity-preserving), the chosen solver
/// `S::default()` (the `ResMut<S>` the solve stage reads, D2),
/// and the [`IntegrationMode`] derived from `S::default().owns_integration()`
/// (C2 — gates [`physics_integrate`] off for
/// an owning TGS solver so it does not double-integrate).
///
/// Stages registered in deterministic order via `.after(...)`:
/// `integrate → gather → broadphase → narrowphase → solve → apply`
/// (D3). `integrate` carries no `.after` (it is the block head); each later stage
/// `.after`s its predecessor — so the whole block runs in a fixed intra-order
/// regardless of registration interleaving with the caller's own systems.
///
/// # What `S` selects
///
/// - `S = `[`DefaultRigidSolver`](crate::solver::DefaultRigidSolver) (=
///   [`ColoredSoftStepSolver`]) — **the default world** (owner decision,
///   2026-09-18). The [`ConstraintGraph`] + [`IslandSleep`] resources are inserted,
///   [`PhysicsConfig::colored`] is set, and the solve is
///   `build_graph →` [`physics_solve_colored`]: the colored TGS-Soft solve with the
///   O7 AVX2 cohort kernel ([`PhysicsConfig::simd_solve`] defaults to `true`, and
///   that kernel is bit-identical to the scalar colored oracle). This is the same
///   wiring as [`add_physics_colored_solve`].
/// - `S = `[`SoftStepSolver`](crate::solver::SoftStepSolver) — the REFERENCE
///   oracle: the solve is [`physics_solve_step::<SoftStepSolver>`](physics_solve_step)
///   in manifold order. Its converged values differ from the colored solve's
///   (equally valid, compared by tolerance), so a world or a replay pinned to the
///   reference must name it.
/// - Any other `S` (the foundation [`NoopSolver`](crate::solver::NoopSolver), an
///   external backend) — the solve is [`physics_solve_step::<S>`](physics_solve_step).
///
/// Per MINOR-1 this does NOT call `builder.build(world)` — the caller owns the
/// build (`runner.rs:325` precedent).
pub fn add_physics_systems<S: RigidSolver + Default>(
    builder: &mut ScheduleBuilder,
    world: &mut EcsMaster,
) -> PhysicsStageKeys {
    // `with_sdf = false`, `colored = false`, `soft = false`, `scene_sync = false`:
    // body-only pipeline (no `SdfField`, no SDF stage, no soft pass, no pose sync).
    // The constraint-graph stage is still wired when `S` is the colored solver
    // (`add_physics_pipeline` derives it from the type).
    add_physics_pipeline::<S>(builder, world, false, false, false, false, false, false)
}

/// Registers the physics pipeline WITH the std-lib S5 `Transform` ⇄ `RigidBody`
/// pose sync wrapped around it (the canonical single-source-of-truth pose
/// bridge).
///
/// Identical to [`add_physics_systems`] but additionally registers, in the SAME
/// (fixed) schedule:
///
/// - [`sync_transform_to_body`](crate::scene_sync::sync_transform_to_body)
///   `.before(integrate)` — the block head. Static / Kinematic bodies copy their
///   gameplay-authored `Transform` INTO `RigidBody` before the gather snapshots
///   it.
/// - [`sync_body_to_transform`](crate::scene_sync::sync_body_to_transform)
///   `.after(apply)` — Dynamic ROOT bodies copy the integrated `RigidBody` pose
///   back OUT to `Transform` once the solve has written it.
/// - [`debug_assert_dynamic_bodies_are_roots`](crate::scene_sync::debug_assert_dynamic_bodies_are_roots)
///   `.after(apply)` — the cold parented-dynamic tripwire (a no-op in release).
///
/// # Why the sync is wired HERE (not a standalone function)
///
/// The cross-edges `sync_transform_to_body.before(integrate)` and
/// `sync_body_to_transform.after(apply)` need the physics stages' real
/// `SystemKey`s. The kernel keeps `SystemKey` in a `pub(crate)` module (this
/// crate makes ZERO core edits — see [`PhysicsStageKeys`]), so an external caller
/// CANNOT construct one to wire those edges. Registering the sync inside the
/// pipeline — where `integrate` / `apply` are real keys in scope — is the only
/// way to pin the ordering without a kernel edit.
///
/// # Schedule-placement contract (the no-desync proof)
///
/// The fixed schedule advances fully each frame as
/// `sync_transform_to_body → integrate → gather → … → solve → apply →
/// sync_body_to_transform`, then the per-frame `propagate_transforms`
/// (`boyko_scene`, the SOLE `GlobalTransform` writer) composes the result. Every
/// pose datum has exactly one writer per window: `RigidBody` is written by
/// `sync_transform_to_body` (Static / Kinematic) or the solver (Dynamic);
/// `Transform` is written by `sync_body_to_transform` (Dynamic roots) or gameplay
/// (Static / Kinematic); `GlobalTransform` by `propagate_transforms`.
///
/// # Bit-determinism
///
/// The sync systems wrap AROUND the solve and never touch it — the copies are
/// plain field assignments (exact, no FMA, no re-normalize). The physics solve is
/// byte-identical whether or not the sync is wired (the determinism suite, which
/// uses [`add_physics_systems`], is unaffected).
///
/// The solve stage follows `S` exactly as in [`add_physics_systems`]:
/// `S = `[`DefaultRigidSolver`](crate::solver::DefaultRigidSolver) wires the colored
/// solve, any other `S` the generic step.
pub fn add_physics_systems_with_scene_sync<S: RigidSolver + Default>(
    builder: &mut ScheduleBuilder,
    world: &mut EcsMaster,
) -> PhysicsStageKeys {
    add_physics_pipeline::<S>(builder, world, false, false, false, false, false, true)
}

/// Inserts the physics resources INCLUDING the [`ConstraintGraph`] and registers
/// the physics pipeline WITH the [`physics_build_graph`] stage (plan O4 /
/// Decision 7) — the islands + greedy-coloring partition.
///
/// Identical to [`add_physics_systems`] but additionally:
/// - sets [`PhysicsConfig::colored`] = `true`;
/// - registers [`physics_build_graph`](crate::systems::physics_build_graph) AFTER
///   `narrowphase` and BEFORE `solve_step`, building the islands + coloring from
///   this step's manifolds.
///
/// **With any `S` other than [`ColoredSoftStepSolver`], O4 produces the partition
/// only — it does NOT change the solve.** The reference
/// [`SoftStepSolver`](crate::solver::SoftStepSolver) still solves in manifold order
/// over the unchanged manifold buffer, so the simulation output is byte-identical
/// to `add_physics_systems::<SoftStepSolver>` (the O4 0%-gate). With
/// `S = `[`ColoredSoftStepSolver`] this is exactly
/// `add_physics_systems::<ColoredSoftStepSolver>`: the graph is consumed by the
/// colored solve (see [`add_physics_systems`]). The returned
/// [`PhysicsStageKeys::build_graph`] carries the stage's descriptor index.
pub fn add_physics_colored<S: RigidSolver + Default>(
    builder: &mut ScheduleBuilder,
    world: &mut EcsMaster,
) -> PhysicsStageKeys {
    add_physics_pipeline::<S>(builder, world, false, true, false, false, false, false)
}

/// Inserts the physics resources and registers the COLORED-SOLVE pipeline (Phase
/// O5, Decision 7) — `physics_build_graph` (O4) followed by the
/// [`physics_solve_colored`](crate::systems::physics_solve_colored) stage, which
/// stands in for the generic [`physics_solve_step`](crate::systems::physics_solve_step).
///
/// Since 2026-09-18 this is the DEFAULT world's wiring: it forwards to
/// `add_physics_systems::<ColoredSoftStepSolver>` (the solver
/// [`DefaultRigidSolver`](crate::solver::DefaultRigidSolver) names) and is kept so
/// existing callers do not move.
///
/// The solve runs in graph-COLOR order over the solver's cohort-shaped
/// `CohortColumns` (L11 C2; a Gauss-Seidel sweep across colors), with the
/// converged impulses stored by manifold index (`solver::warm_records`, L11 C1).
/// The reference
/// [`SoftStepSolver`](crate::solver::SoftStepSolver) is byte-untouched and its
/// solve stage is NOT registered on this path — the two solvers never both run
/// (Decision 7).
///
/// # The value change (Phase O5)
///
/// The colored sweep order differs from the reference manifold-order sweep, so
/// the converged float values DIFFER (but are equally valid) — validated against
/// tolerance acceptance gates, not a bit-baseline against `SoftStepSolver`. The
/// colored solve is run-to-run bit-identical and never moves a static body; the
/// O7 AVX2 cohort kernel it runs by default ([`PhysicsConfig::simd_solve`]) is
/// bit-identical to its scalar colored oracle. The returned [`PhysicsStageKeys`]
/// carries both the `build_graph` and `solve` stage indices.
pub fn add_physics_colored_solve(
    builder: &mut ScheduleBuilder,
    world: &mut EcsMaster,
) -> PhysicsStageKeys {
    add_physics_systems::<ColoredSoftStepSolver>(builder, world)
}

/// Inserts the physics resources INCLUDING an (empty) [`SdfField`] and registers
/// the physics pipeline WITH the body-vs-SDF narrowphase stage (plan W5).
///
/// Identical to [`add_physics_systems`] but additionally:
/// - inserts a default (empty) [`SdfField`] resource the caller fills with the
///   CPU-authoritative SDF edit list (the same scene the GPU renders);
/// - registers [`physics_narrowphase_sdf`](crate::systems::physics_narrowphase_sdf)
///   AFTER `narrowphase` and BEFORE `solve_step`, so the solver sees both body-body
///   and body-vs-SDF contacts (the latter keyed by the C1
///   [`SDF_SENTINEL`](crate::manifold::SDF_SENTINEL)).
///
/// Opt-in: a body-only scene uses [`add_physics_systems`] and is byte-for-byte
/// unaffected (the SDF stage is never registered — the 0%-gate). The returned
/// [`PhysicsStageKeys::narrowphase_sdf`] carries the SDF stage's descriptor index.
/// The solve stage follows `S` as in [`add_physics_systems`]; with
/// `S = `[`DefaultRigidSolver`](crate::solver::DefaultRigidSolver) the graph build
/// runs after the SDF stage, so the colored solve sees the SDF contacts too.
///
/// ⚠ SDF + [`PhysicsConfig::sleeping`] on the colored solve is reachable through
/// this entry and UNMEASURED: no gate covers a parked pile resting on the field
/// (see `IslandSleep::begin_step`, "Not covered"). Sleeping defaults off.
///
/// The box path folds the field with
/// [`PhysicsConfig::sdf_narrowphase`](crate::resources::PhysicsConfig::sdf_narrowphase),
/// which defaults to the scalar oracle. The O9 AVX2 fold is a deliberate opt-in
/// because it is not bit-identical (see
/// [`SdfNarrowphaseKernel::Avx2`](crate::resources::SdfNarrowphaseKernel::Avx2)).
pub fn add_physics_sdf<S: RigidSolver + Default>(
    builder: &mut ScheduleBuilder,
    world: &mut EcsMaster,
) -> PhysicsStageKeys {
    add_physics_pipeline::<S>(builder, world, true, false, false, false, false, false)
}

/// Inserts the physics resources and registers the physics pipeline WITH the SP1
/// XPBD soft-body position pass (plan O11 SP1).
///
/// Identical to [`add_physics_systems`] but additionally:
/// - sets [`PhysicsConfig::soft_body`](crate::resources::PhysicsConfig) = `true`;
/// - inserts a default (empty) [`SdfField`] resource so the soft pass's
///   `Res<SdfField>` resolves (the caller fills it with the same edit list the GPU
///   renders — soft particles collide one-sided against it, sharing the rigid SDF
///   evaluator);
/// - registers [`physics_soft_step`](crate::soft::physics_soft_step) AFTER `solve`
///   and BEFORE `apply`, so it runs as a SEPARATE position pass on the
///   [`SoftBody`](crate::soft::SoftBody) columns once the rigid solve has finished.
///
/// The soft step is a STRICTLY DISJOINT integrator: it never WRITES the rigid
/// [`SolverScratch`], never sets a touched bit, and never enters `physics_apply`,
/// so the rigid simulation is byte-identical whether the soft pass runs or not.
/// Opt-in: a world that uses [`add_physics_systems`] is byte-for-byte unaffected
/// (the soft stage is never registered — the campaign 0%-gate). The returned
/// [`PhysicsStageKeys::soft_step`] carries the soft stage's descriptor index.
///
/// # Soft↔rigid coupling (`coupling`, SP2 D6/D7)
///
/// When `coupling == false` the WHOLE schedule shape is byte-identical to SP1: the
/// uncoupled [`physics_soft_step`](crate::soft::physics_soft_step) is registered
/// (no extra params, no extra resources) and no apply-side reaction stage exists.
///
/// When `coupling == true` the coupled
/// [`physics_soft_step_coupled`](crate::soft::physics_soft_step_coupled) is
/// registered in its place (it additionally READS the rigid frame-N snapshot +
/// broadphase grid and accumulates the rigid reaction), a
/// [`SoftRigidReaction`](crate::soft::SoftRigidReaction) resource is inserted, and
/// [`physics_soft_rigid_apply`](crate::soft::physics_soft_rigid_apply) is registered
/// `.after(apply)` to land the reaction on the
/// [`RigidBody`](crate::components::RigidBody) component (like an external force).
/// The coupling still never mutates the rigid scratch (IM-1 safety); the actual
/// per-particle coupling work is additionally gated by
/// [`PhysicsConfig::soft_rigid_coupling`](crate::resources::PhysicsConfig) (set
/// `true` here when `coupling == true`).
///
/// The rigid solve stage follows `S` as in [`add_physics_systems`]; the soft
/// stages do not depend on it.
///
/// ⚠ Coupling + [`PhysicsConfig::sleeping`] on the colored solve is reachable and
/// has a recorded gap: a soft→rigid reaction does not wake a sleeping body. No
/// gate covers the combination. Sleeping defaults off.
pub fn add_physics_soft<S: RigidSolver + Default>(
    builder: &mut ScheduleBuilder,
    world: &mut EcsMaster,
    coupling: bool,
) -> PhysicsStageKeys {
    add_physics_pipeline::<S>(builder, world, false, false, true, coupling, false, false)
}

/// Inserts the physics resources INCLUDING the SP4 [`SoftColorScratch`] and registers
/// the physics pipeline with the COLORED-PARALLEL soft step
/// ([`physics_soft_step_colored`](crate::soft::physics_soft_step_colored)) in place of
/// the uncoupled [`physics_soft_step`](crate::soft::physics_soft_step) (the two never
/// both run), in the SAME `.after(solve)` `.before(apply)` slot — mirroring how
/// [`add_physics_colored_solve`] stands in for the default solve (plan O11 SP4).
///
/// Identical to [`add_physics_soft`] (non-coupling) but additionally:
/// - inserts a [`SoftColorScratch`] resource (the per-constraint-type coloring
///   scratch);
/// - registers the colored soft step, which parallelizes the distance + volume (+
///   optional self-collision) projection sweeps across the engine threadpool via
///   per-type graph colorings.
///
/// # The SP4 0%-gate (opt-in twice)
///
/// The colored step is a strict SIBLING: when
/// [`PhysicsConfig::soft_body_colored`](crate::resources::PhysicsConfig) is the
/// default `false` it runs the SERIAL `step_body` per body — byte-identical to
/// [`physics_soft_step`]. The colored projection turns on only when the caller sets
/// `soft_body_colored = true` (and, for the highest-risk self-collision surface,
/// `soft_self_collision_colored = true`). The colored result is run-to-run
/// bit-deterministic and `{1, N}`-worker bit-identical, but its value DIFFERS from the
/// serial sweep (the colored visit order differs from the SP1 authoring order). This
/// is the NON-COUPLING path (the soft↔rigid coupling boundary stays serial, IM-1).
///
/// Opt-in: a world that does not call this is byte-for-byte unaffected (the colored
/// soft stage + scratch are never registered — the campaign 0%-gate). The returned
/// [`PhysicsStageKeys::soft_step`] carries the colored soft stage's descriptor index.
/// The rigid solve stage follows `S` as in [`add_physics_systems`].
pub fn add_physics_soft_colored<S: RigidSolver + Default>(
    builder: &mut ScheduleBuilder,
    world: &mut EcsMaster,
) -> PhysicsStageKeys {
    add_physics_pipeline::<S>(builder, world, false, false, true, false, true, false)
}

/// The six pipeline-SHAPE flags, as one `Copy` record.
///
/// The wiring has two halves that need DIFFERENT exclusive borrows —
/// [`insert_physics_resources`] takes `&mut EcsMaster`, [`register_physics_pipeline`]
/// takes `&mut ScheduleBuilder` — and an `App` hands out only one of them at a time
/// (`App::add_systems_cfg_in` yields the builder while it owns the world), which is
/// exactly why the pre-split single function could not be called from a `Plugin`.
/// Both halves must be handed the SAME shape or the registered stages and the
/// inserted resources disagree; passing one record instead of positional `bool`s
/// makes that mismatch unwritable. The public free functions keep their positional
/// signatures unchanged.
///
/// There is no `colored_solve` flag: the solve stage is selected by the solver TYPE
/// (see [`PipelineOpts::resolve`]), so a flag could only disagree with it.
#[derive(Clone, Copy)]
struct PipelineOpts {
    /// Splice the body-vs-SDF narrowphase stage + insert [`SdfField`] (W5).
    with_sdf: bool,
    /// Build the O4 constraint graph (islands + coloring). Implied when `S` is the
    /// colored solver, which consumes the graph.
    colored: bool,
    /// Run the SP1 XPBD soft-body position pass.
    soft: bool,
    /// SP2 soft<->rigid coupling (requires `soft`).
    coupling: bool,
    /// SP4 colored soft step (requires `soft`, excludes `coupling`).
    soft_colored: bool,
    /// Wrap the std-lib S5 `Transform` <-> `RigidBody` pose sync around the block.
    scene_sync: bool,
}

impl PipelineOpts {
    /// Resolves the shape against the solver type, once, at wire-up (cold), and
    /// returns it with the derived `colored_solve` flag.
    ///
    /// The colored solve stage reads `ResMut<ColoredSoftStepSolver>` by name, so tying
    /// it to `S` makes the solver resource inserted by [`insert_physics_resources`]
    /// (`S::default()`) exactly the one it reads, and the `IntegrationMode` stamped
    /// from `S` exactly the one it needs (it owns integration). The colored solve
    /// consumes the graph, hence the implication `colored |= colored_solve`.
    ///
    /// Called by BOTH halves, so a [`PhysicsPlugin`] build (which calls them
    /// separately) resolves and validates exactly like a free-function call.
    fn resolve<S: RigidSolver + Default>(self) -> (Self, bool) {
        let colored_solve = TypeId::of::<S>() == TypeId::of::<ColoredSoftStepSolver>();
        let resolved = Self { colored: self.colored || colored_solve, ..self };
        resolved.debug_validate();
        (resolved, colored_solve)
    }

    /// The caller-upheld invariants the type system cannot express.
    fn debug_validate(self) {
        let Self { soft, coupling, soft_colored, .. } = self;
        debug_assert!(
            !coupling || soft,
            "invariant: soft↔rigid coupling requires the soft pass (soft == true)"
        );
        debug_assert!(
            !soft_colored || soft,
            "invariant: the colored soft step requires the soft pass (soft == true)"
        );
        debug_assert!(
            !(soft_colored && coupling),
            "invariant: the colored soft step is the non-coupling path (SP4 IM-1 boundary)"
        );
    }
}

/// Shared wiring for every `add_physics_*` entry: inserts the resources and
/// registers the pipeline, optionally splicing the SDF-collision stage and/or the
/// constraint-graph stage between narrowphase and solve, and selecting the
/// generic or the colored solve stage.
///
/// The solve stage is selected by the solver TYPE (`S == ColoredSoftStepSolver`),
/// not by a flag, so the colored stage only ever runs with the colored solver
/// resource it reads and with the integration ownership that solver declares. The
/// graph is implied by it (`colored |= colored_solve`); `colored` alone with
/// another `S` is the O4 graph-only shape of [`add_physics_colored`].
#[allow(clippy::too_many_arguments)]
fn add_physics_pipeline<S: RigidSolver + Default>(
    builder: &mut ScheduleBuilder,
    world: &mut EcsMaster,
    with_sdf: bool,
    colored: bool,
    soft: bool,
    coupling: bool,
    soft_colored: bool,
    scene_sync: bool,
) -> PhysicsStageKeys {
    let opts = PipelineOpts {
        with_sdf,
        colored,
        soft,
        coupling,
        soft_colored,
        scene_sync,
    };
    insert_physics_resources::<S>(world, opts);
    // `None` — the free-function path registers the stages in NO set, exactly as
    // before the split. Joining `FixedSet::Gameplay` is the `App`/[`PhysicsPlugin`]
    // path's addition only, so every existing caller's schedule shape (and the
    // determinism goldens keyed to it) is byte-identical.
    register_physics_pipeline::<S>(builder, opts, None)
}

/// The WORLD half of the wiring: inserts every resource the chosen pipeline shape
/// needs, and checks the position-pairing stages' row selections (defect A5).
///
/// Split out of `add_physics_pipeline` so a `Plugin` can insert the resources while
/// it holds `&mut EcsMaster` and register the stages later, when the `App` hands it
/// the `&mut ScheduleBuilder`. Order is irrelevant between the two halves —
/// registration never reads a resource, it only names systems — so the split is
/// behavior-preserving by construction.
fn insert_physics_resources<S: RigidSolver + Default>(world: &mut EcsMaster, opts: PipelineOpts) {
    let (opts, _) = opts.resolve::<S>();
    let PipelineOpts { with_sdf, colored, soft, coupling, soft_colored, .. } = opts;
    // Reused, capacity-preserving step buffers (principle 5 — no per-step alloc).
    // The `colored` flag rides the config so `physics_build_graph` (registered only
    // when `colored`) and any future graph consumer share one switch.
    world.insert_resource(PhysicsConfig {
        colored,
        soft_body: soft,
        // SP2 D6/D7: the runtime coupling gate rides the config so the coupled step
        // (registered only when `coupling`) reads one switch.
        soft_rigid_coupling: coupling,
        // SP2 M1: the coupled soft step's `deepest_contact` walks the
        // `BroadphaseGrid`'s CSR cell slices + oversized list, which are populated
        // ONLY when `physics_broadphase` takes the `BroadphaseKind::Grid` arm
        // (`grid.build`). The default `AllPairs` arm never touches the grid, so
        // coupling would read empty slices ⇒ zero contacts ⇒ a silent no-op. The
        // grid is a HARD PREREQUISITE for coupling, so force it on the coupling path
        // (it is O2's proven path, bit-identical to all-pairs post-filter — safe to
        // mandate). `broadphase` runs `.after(gather)` and the coupled step
        // `.after(solve)` (itself after broadphase), so the grid is built before the
        // coupled step reads it. Off the coupling path the default `AllPairs` is
        // preserved (the 0%-gate).
        broadphase: if coupling {
            BroadphaseKind::Grid
        } else {
            PhysicsConfig::default().broadphase
        },
        ..PhysicsConfig::default()
    });
    world.insert_resource(ContactPairs::with_capacity(INITIAL_BODY_CAPACITY));
    world.insert_resource(Manifolds::with_capacity(INITIAL_BODY_CAPACITY));
    world.insert_resource(SolverScratch::with_capacity(INITIAL_BODY_CAPACITY));
    // O2: the grid broadphase scratch (capacity-reused). Inserted unconditionally
    // so `physics_broadphase`'s `ResMut<BroadphaseGrid>` param always resolves; it
    // stays untouched while `PhysicsConfig::broadphase` is the default `AllPairs`.
    world.insert_resource(BroadphaseGrid::with_capacity(INITIAL_BODY_CAPACITY));
    // The tree broadphase's state (capacity-reused). Inserted unconditionally so
    // `physics_broadphase`'s `ResMut<BroadphaseTree>` param always resolves; it
    // stays untouched while `PhysicsConfig::broadphase` is not `Tree`.
    world.insert_resource(BroadphaseTree::with_capacity(INITIAL_BODY_CAPACITY));
    // P3: the cold broadphase-policy cost-model carrier (the `select_broadphase`
    // density selector's situation key + hysteresis band). Inserted unconditionally
    // so the policy's `ResMut<PhysicsStats>` param always resolves; it cold-starts
    // with the band OFF (AllPairs), matching `PhysicsConfig::broadphase`'s default,
    // and stays inert in the default `Manual` select mode (the 0%-gate). The Grid
    // CSR buffers above are preallocated to `INITIAL_BODY_CAPACITY`, so an Auto
    // AllPairs→Grid flip is a FILL, not a frame-path `Vec::new`/grow (Principle 5).
    world.insert_resource(PhysicsStats::default());
    if colored {
        // O4: the islands + coloring scratch (capacity-reused). Inserted only on
        // the colored path so `physics_build_graph`'s `ResMut<ConstraintGraph>`
        // param resolves; the non-colored paths never register the stage, so the
        // resource is unnecessary there (the 0%-gate).
        world.insert_resource(ConstraintGraph::with_capacity(INITIAL_BODY_CAPACITY));
        // O8: the per-island sleeping state (capacity-reused). Inserted on the colored
        // path so `physics_solve_colored`'s `ResMut<IslandSleep>` param resolves; it
        // stays untouched while `PhysicsConfig::sleeping` is the default `false` (the
        // 0%-gate). Pre-sized to the body capacity (one island per body is the worst
        // case — every body its own singleton island).
        world.insert_resource(IslandSleep::with_capacity(
            INITIAL_BODY_CAPACITY,
            INITIAL_BODY_CAPACITY,
        ));
        // L10: the sleep-skip state, beside `IslandSleep`, so the colored broadphase and
        // narrowphase (registered only on this path) resolve `ResMut<SleepSets>`. It records
        // the step mode and holds nothing while `PhysicsConfig::sleeping` is off.
        world.insert_resource(SleepSets::with_capacity(INITIAL_BODY_CAPACITY));
    }
    if with_sdf || soft {
        // The CPU-authoritative SDF scene (empty by default; the caller fills it
        // with the same edit list the GPU renders). Inserted for the SDF
        // narrowphase path (`with_sdf`) AND the soft pass (`soft`), whose
        // `physics_soft_step` reads `Res<SdfField>` for one-sided particle
        // collision. An empty field collides nothing, so a soft-only world that
        // never fills it is unaffected.
        world.insert_resource(SdfField::default());
    }
    if coupling {
        // SP2 D6/D7: the per-body soft→rigid reaction accumulator (capacity-reused,
        // cleared per frame — zero per-step alloc). Inserted ONLY on the coupling
        // path so the coupled step's `ResMut<SoftRigidReaction>` and
        // `physics_soft_rigid_apply`'s param resolve; the non-coupling paths never
        // register either stage, so the resource is unnecessary there (the 0%-gate
        // on the schedule SHAPE).
        world.insert_resource(SoftRigidReaction::with_capacity(INITIAL_BODY_CAPACITY));
    }
    if soft_colored {
        // SP4: the colored-solve coloring scratch (capacity-reused; steady-state
        // zero-alloc). Inserted ONLY on the colored soft path so
        // `physics_soft_step_colored`'s `ResMut<SoftColorScratch>` param resolves; it
        // stays untouched while `PhysicsConfig::soft_body_colored` is the default
        // `false` (the SP4 0%-gate: the colored step then runs the serial `step_body`).
        world.insert_resource(SoftColorScratch::default());
    }

    // C2: stamp the integration mode from the chosen solver BEFORE inserting the
    // solver (a fresh `S::default()` is cheap and the only `&self` source for
    // `owns_integration` at wire-up time). `physics_integrate` reads this mode and
    // gates itself off for an owning TGS solver.
    let integration_mode = if S::default().owns_integration() {
        IntegrationMode::SolverOwned
    } else {
        IntegrationMode::Foundation
    };
    world.insert_resource(integration_mode);
    // The O5 solve stage is NOT generic: `physics_solve_colored` reads
    // `ResMut<ColoredSoftStepSolver>` by name. It is registered only when `S` IS
    // `ColoredSoftStepSolver` (`PipelineOpts::resolve`), so this `S::default()` is
    // exactly the resource it reads, on every entry shape — a `PhysicsPlugin<S>` cannot
    // register a colored stage whose parameter does not resolve.
    //
    // That failure was MEASURED on the render line while the solve was still a flag,
    // and it is worse than it sounds: the param resolve panics on a SCHEDULER WORKER,
    // and a worker panic in the windowed host does not take the process down — the
    // window simply stops responding, with no CPU burn and the panic text buried in
    // stderr. "Hangs on boot" is a terrible name for "one resource was missing".
    world.insert_resource(S::default());

    // Defect A5: the gather, apply and soft apply pair their rows by position, so their
    // selections must agree. Once per wire-up; a hard assert, because a disagreement is
    // silent state corruption in a release build. The signature pins in `body_set` list
    // every stage that pairs rows by position. It reads only the world, so it lives in
    // this half and runs for both entry shapes.
    crate::body_set::assert_body_set_agrees(world);
}

/// Joins one stage to `set`, or leaves it unset.
///
/// [`SystemConfig`] is a consuming builder, so an OPTIONAL `.in_set(..)` cannot be
/// written inline in a chain; this is that `Option` adapter. `None` reproduces the
/// pre-split registration byte-for-byte.
#[inline]
fn joined<'a>(cfg: SystemConfig<'a>, set: Option<FixedSet>) -> SystemConfig<'a> {
    match set {
        Some(s) => cfg.in_set(s),
        None => cfg,
    }
}

/// The SCHEDULE half of the wiring: registers the pipeline stages on `builder` in
/// the deterministic `.after(..)` order and returns their handles.
///
/// `set` joins EVERY stage to one [`FixedSet`] (the [`PhysicsPlugin`] passes
/// `FixedSet::Gameplay`). That membership is load-bearing rather than cosmetic:
/// `EnginePlugins` orders `FixedSet::Snapshot.after(FixedSet::Gameplay)`, and the
/// engine's interpolation pack (`pack_gpu_transforms`) lives in `Snapshot` — so
/// without the join the pack could read a body's `Transform` from BEFORE this
/// substep's solve, which is the one-substep lag the D4 seam exists to prevent.
/// `None` keeps the stages unset (the free-function path).
fn register_physics_pipeline<S: RigidSolver + Default>(
    builder: &mut ScheduleBuilder,
    opts: PipelineOpts,
    set: Option<FixedSet>,
) -> PhysicsStageKeys {
    let (opts, colored_solve) = opts.resolve::<S>();
    let PipelineOpts {
        with_sdf,
        colored,
        soft,
        coupling,
        soft_colored,
        scene_sync,
    } = opts;
    // S5 head: when pose sync is wired, `sync_transform_to_body` is the TRUE
    // block head — Static / Kinematic bodies copy `Transform` INTO `RigidBody`
    // before the gather snapshots it. Registered first so its real `SystemKey` is
    // in scope to pin `integrate.after(..)` below (the kernel keeps `SystemKey`
    // crate-private, so this ordering can only be wired here — see
    // `add_physics_systems_with_scene_sync`). `iter_mut` is the sync's only access,
    // so it conflicts on `RigidBody` with `physics_integrate` / `physics_gather`
    // and is serialized before them anyway; the explicit edge makes the order
    // deterministic, not merely conflict-derived.
    let transform_to_body_key = if scene_sync {
        Some(joined(builder.add_system(sync_transform_to_body), set).key())
    } else {
        None
    };

    // Block head (physics): `physics_integrate` runs first within the physics
    // block. With pose sync it runs `.after(sync_transform_to_body)` so the gather
    // sees the synced Static / Kinematic poses; without it, it carries no `.after`
    // and is unordered relative to the caller's pre-physics systems. `.key().0` is
    // the public descriptor index of the engine's `SystemKey` (see
    // `PhysicsStageKeys`).
    let integrate = if let Some(head) = transform_to_body_key {
        joined(builder.add_system(physics_integrate), set).after(head).key()
    } else {
        joined(builder.add_system(physics_integrate), set).key()
    };
    // The gather joins `PhysicsGatherSet` so an external caller can order against it by
    // name, and (on the `App` path) `set` like every other stage.
    let gather = joined(builder.add_system(physics_gather), set)
        .after(integrate)
        .in_set(PhysicsGatherSet)
        .key();
    // P3: the cold density policy runs `.after(gather)` (the body count is fresh —
    // the gather has just refilled `SolverScratch`) and `.before(broadphase)` (this
    // frame's `BroadphaseKind` decision feeds the build). `physics_broadphase` is
    // pinned `.after(select)` below so the ordering is deterministic, not merely
    // conflict-derived (the policy's `ResMut<PhysicsConfig>` vs the broadphase's
    // `Res<PhysicsConfig>` would serialize them anyway, but the edge is explicit).
    let select = joined(builder.add_system(select_broadphase), set).after(gather).key();
    // L10 (design 04 D15): the colored pipeline — the one that inserts `IslandSleep` and
    // `SleepSets` — runs the colored broadphase and narrowphase, which carry the sleep-skip;
    // every other pipeline keeps the reference stages.
    let broadphase = if colored {
        joined(builder.add_system(physics_broadphase_colored), set).after(select).key()
    } else {
        joined(builder.add_system(physics_broadphase), set).after(select).key()
    };
    let narrowphase = if colored {
        joined(builder.add_system(physics_narrowphase_colored), set)
            .after(broadphase)
            .key()
    } else {
        joined(builder.add_system(physics_narrowphase), set)
            .after(broadphase)
            .key()
    };

    // W5: the body-vs-SDF stage runs AFTER body-body narrowphase (both append to
    // `Manifolds`) and is forced BEFORE the solve via an explicit ordering edge — a
    // `ResMut`/`Res` conflict on `Manifolds` alone would serialize them but not pin
    // the order, so the edge is load-bearing. Registered only when `with_sdf` (the
    // 0%-gate: a body-only schedule never registers this stage). The `SystemConfig`
    // borrows `builder` mutably, so extract the `SystemKey` immediately (drop the
    // handle) before re-borrowing `builder` for the next stage.
    let narrowphase_sdf_key = if with_sdf {
        Some(
            joined(builder.add_system(physics_narrowphase_sdf), set)
                .after(narrowphase)
                .key(),
        )
    } else {
        None
    };

    // O4: the constraint-graph stage runs AFTER all manifold producers (body-body
    // narrowphase + the optional SDF stage) and BEFORE the solve, so it partitions
    // the exact manifold set the solver consumes. `physics_build_graph` reads
    // `Res<Manifolds>` (shared) while the SDF stage holds `ResMut<Manifolds>`, so an
    // explicit edge after the SDF stage is load-bearing (the shared read alone would
    // not pin the order). Registered only when `colored` (the 0%-gate: a non-colored
    // schedule never registers this stage, and the solve is byte-identical because
    // O4 does not consume the graph). Extract the `SystemKey` immediately (drop the
    // handle) before re-borrowing `builder`.
    let build_graph_key = if colored {
        let cfg = joined(builder.add_system(physics_build_graph), set).after(narrowphase);
        let cfg = if let Some(sdf) = narrowphase_sdf_key {
            cfg.after(sdf)
        } else {
            cfg
        };
        Some(cfg.key())
    } else {
        None
    };

    // O5: the colored-solve path (`S == ColoredSoftStepSolver`, the default world)
    // registers `physics_solve_colored` (which CONSUMES the constraint graph) in
    // place of the generic `physics_solve_step::<S>` — the two solvers never both
    // run (Decision 7). Any other `S` keeps the generic solve stage, byte-untouched.
    // The `physics_solve_colored` stage's `Res<ConstraintGraph>` makes the
    // `.after(build_graph)` edge load-bearing (not merely documentary as on the O4
    // graph-only path).
    let mut solve_cfg = if colored_solve {
        joined(builder.add_system(physics_solve_colored), set).after(narrowphase)
    } else {
        joined(builder.add_system(physics_solve_step::<S>), set).after(narrowphase)
    };
    if let Some(sdf) = narrowphase_sdf_key {
        // Pin the SDF stage before the solve (it must finish appending its
        // manifolds before the solver reads the buffer).
        solve_cfg = solve_cfg.after(sdf);
    }
    if let Some(graph) = build_graph_key {
        // Pin the graph build before the solve. On the O5 colored-solve path this is
        // load-bearing (the solve reads the graph); on the O4 graph-only path it is
        // order-neutral for the result (the solve does not read the graph) but the
        // edge documents the dependency.
        solve_cfg = solve_cfg.after(graph);
    }
    let solve = solve_cfg.key();
    let apply = joined(builder.add_system(physics_apply), set).after(solve).key();

    // SP1: the soft-body XPBD pass runs as a SEPARATE position pass AFTER the rigid
    // solve and BEFORE apply. It is a strictly disjoint integrator (it touches only
    // `SoftBody` columns, never the rigid `SolverScratch`), so it could in principle
    // run unordered relative to the rigid block — but pinning it `.after(solve)`
    // `.before(apply)` keeps the whole physics step in one deterministic window and
    // matches the plan's placement. Registered only when `soft` (the 0%-gate: a
    // non-soft schedule never registers this stage). The `apply` key already exists
    // (registered above), so the `.before(apply)` edge resolves.
    //
    // SP2 D6/D7: on the coupling path the coupled step
    // (`physics_soft_step_coupled`) is registered IN PLACE of the uncoupled step —
    // the two never both run. When `coupling == false` the schedule shape is
    // byte-identical to SP1 (the uncoupled step, no apply-side reaction stage), so
    // the schedule-shape 0%-gate holds.
    //
    // SP4: on the colored soft path the COLORED step (`physics_soft_step_colored`) is
    // registered IN PLACE of the uncoupled step (the non-coupling path; the two never
    // both run). When `soft_body_colored` is the default `false` the colored step runs
    // the SERIAL `step_body` (the SP4 0%-gate), so the schedule output is unchanged.
    let soft_step_key = if soft {
        let cfg = if coupling {
            joined(builder.add_system(physics_soft_step_coupled), set)
                .after(solve)
                .before(apply)
        } else if soft_colored {
            joined(builder.add_system(physics_soft_step_colored), set)
                .after(solve)
                .before(apply)
        } else {
            joined(builder.add_system(physics_soft_step), set)
                .after(solve)
                .before(apply)
        };
        Some(cfg.key())
    } else {
        None
    };

    // SP2 D7 apply path: the reaction lands on the `RigidBody` component AFTER
    // `physics_apply` (like an external force), so it is registered `.after(apply)`
    // ONLY on the coupling path. The `apply` key already exists (registered above),
    // so the `.after(apply)` edge resolves. Off the coupling path this stage does
    // not exist — the schedule-shape 0%-gate.
    if coupling {
        joined(builder.add_system(physics_soft_rigid_apply), set).after(apply);
    }

    // S5 tail: Dynamic ROOT bodies copy the integrated `RigidBody` pose back OUT
    // to `Transform`, `.after(apply)` so the solve has finished writing it. The
    // parented-dynamic tripwire also runs here (a no-op in release). `apply` is a
    // real `SystemKey` in scope, so these edges are wired here (the only place
    // they can be — see `add_physics_systems_with_scene_sync`). The whole tail is
    // registered only when `scene_sync` (the 0%-gate: a non-sync schedule never
    // registers these stages, and the physics block is byte-identical).
    let scene_sync_keys = if scene_sync {
        let body_to_transform = joined(builder.add_system(sync_body_to_transform), set)
            .after(apply)
            .key();
        let parented_dynamic_guard =
            joined(builder.add_system(debug_assert_dynamic_bodies_are_roots), set)
                .after(apply)
                .key();
        Some(SceneSyncKeys {
            // `transform_to_body_key` is `Some` whenever `scene_sync` (registered
            // as the block head above), so the `expect` cannot fire by construction.
            transform_to_body: transform_to_body_key
                .expect("invariant: scene_sync registers the transform_to_body head")
                .0,
            body_to_transform: body_to_transform.0,
            parented_dynamic_guard: parented_dynamic_guard.0,
        })
    } else {
        None
    };

    PhysicsStageKeys {
        integrate: integrate.0,
        gather: gather.0,
        select_broadphase: select.0,
        broadphase: broadphase.0,
        narrowphase: narrowphase.0,
        narrowphase_sdf: narrowphase_sdf_key.map(|k| k.0),
        build_graph: build_graph_key.map(|k| k.0),
        solve: solve.0,
        soft_step: soft_step_key.map(|k| k.0),
        apply: apply.0,
        scene_sync: scene_sync_keys,
    }
}

/// The system set holding the [`physics_gather`] stage of every pipeline this module
/// wires, so a caller outside this crate can order its own systems against the gather
/// by name.
///
/// # Why a set, not a key
///
/// [`PhysicsStageKeys::gather`] is the bare descriptor index inside the gather's
/// `SystemKey`. The engine keeps `SystemKey` in a `pub(crate)` module, so outside
/// `boyko_ecs` that index cannot be turned back into a key, and `.before(key)` against
/// the gather cannot be written. A set is named by its type instead: the same seam
/// `boyko_render` uses to order across plugin boundaries (for example `CsmFitSet`).
///
/// # Use
///
/// `builder.add_system(spawner).before_set(PhysicsGatherSet)` runs `spawner` before the
/// gather. The executor applies a system's `Commands` before it dispatches that system's
/// successors, so a body spawned there is in the same run's gather. Its `RigidBody` is
/// flagged added one gather late; the row key carries the entity's generation, so it is
/// still a new row to every row-keyed consumer on that gather (`row_identity.rs`).
///
/// Membership adds no ordering edge. This crate configures no run condition on the set;
/// a condition configured on it would skip the gather while the later stages still run.
#[derive(boyko_macros::SystemSet, Clone, Copy, PartialEq, Eq, Debug)]
pub struct PhysicsGatherSet;

/// The `App`-facing physics plugin: `app.add_plugin(PhysicsPlugin::new())`.
///
/// Inserts the physics resources and registers the pipeline into
/// [`CoreSchedule::Fixed`], with every stage joined to [`FixedSet::Gameplay`] —
/// the set whose own doc names "user/physics Fixed gameplay". That membership is
/// what puts the whole physics block BEFORE `FixedSet::Snapshot`, where the
/// engine's interpolation pack reads each body's post-solve `Transform`.
///
/// # Defaults, and why they differ from [`add_physics_systems`]
///
/// [`Self::new`] turns the std-lib S5 pose sync ON (the
/// [`add_physics_systems_with_scene_sync`] shape). A drawn scene reads
/// `Transform`, so a body whose solved pose never reaches it renders frozen at
/// its spawn point; the builder form keeps its sync-free default because its
/// callers are determinism harnesses that read `RigidBody` directly. Turn it off
/// with [`Self::without_scene_sync`].
///
/// The solver defaults to [`DefaultRigidSolver`] — the default world's colored
/// solve with the O7 AVX2 cohort kernel (owner decision, 2026-09-18) — so the
/// constraint graph is built and consumed, exactly as
/// `add_physics_systems::<DefaultRigidSolver>` wires it. Everything else is off, as
/// the free functions default: no SDF stage, no soft pass (the 0%-gates). The
/// reference solver is selected by naming it:
/// `PhysicsPlugin::<SoftStepSolver>::with_solver()`.
///
/// ```no_run
/// # use boyko_ecs::App;
/// # use boyko_physics::PhysicsPlugin;
/// let mut app = App::new();
/// app.add_plugin(PhysicsPlugin::new());
/// ```
pub struct PhysicsPlugin<S: RigidSolver + Default = DefaultRigidSolver> {
    /// The pipeline shape this plugin wires (see [`PipelineOpts`]).
    opts: PipelineOpts,
    /// The solver TYPE only — never a value, so the plugin inherits none of the
    /// solver's auto traits (`fn() -> S` is the variance- and auto-trait-neutral
    /// marker).
    solver: PhantomData<fn() -> S>,
}

impl PhysicsPlugin<DefaultRigidSolver> {
    /// The default pipeline on [`DefaultRigidSolver`] (the colored solve): pose
    /// sync ON, every opt-in stage off.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::with_solver()
    }
}

impl Default for PhysicsPlugin<DefaultRigidSolver> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<S: RigidSolver + Default> PhysicsPlugin<S> {
    /// [`PhysicsPlugin::new`] on an explicit solver type, e.g.
    /// `PhysicsPlugin::<SoftStepSolver>::with_solver()` for the reference solve or
    /// `PhysicsPlugin::<NoopSolver>::with_solver()`. The solve stage follows `S`
    /// exactly as in [`add_physics_systems`].
    #[inline]
    #[must_use]
    pub fn with_solver() -> Self {
        Self {
            opts: PipelineOpts {
                with_sdf: false,
                colored: false,
                soft: false,
                coupling: false,
                soft_colored: false,
                scene_sync: true,
            },
            solver: PhantomData,
        }
    }

    /// Splices the body-vs-SDF narrowphase stage and inserts the (empty)
    /// [`SdfField`] the caller fills — the [`add_physics_sdf`] shape (W5).
    #[inline]
    #[must_use]
    pub fn with_sdf(mut self) -> Self {
        self.opts.with_sdf = true;
        self
    }

    /// Builds the O4 constraint graph (islands + coloring) — the
    /// [`add_physics_colored`] shape. With any `S` other than [`ColoredSoftStepSolver`]
    /// the graph is NOT consumed and the solve stays byte-identical; with the colored
    /// solver (the default) the graph is already implied, so this is a no-op there.
    #[inline]
    #[must_use]
    pub fn colored(mut self) -> Self {
        self.opts.colored = true;
        self
    }

    /// Runs the O5 colored solve — the [`add_physics_colored_solve`] shape — by
    /// switching the solver TYPE to [`ColoredSoftStepSolver`], keeping every other
    /// option.
    ///
    /// The solve stage follows the solver type, not a flag (see the module docs), and
    /// the stage reads `ResMut<ColoredSoftStepSolver>` by name, so re-typing the plugin
    /// is what guarantees that resource is the one inserted — see the note in
    /// [`insert_physics_resources`] for what the missing-resource failure looks like
    /// from the outside (a frozen window, not a crash). On the default
    /// [`DefaultRigidSolver`] this changes nothing.
    #[inline]
    #[must_use]
    pub fn colored_solve(self) -> PhysicsPlugin<ColoredSoftStepSolver> {
        PhysicsPlugin { opts: self.opts, solver: PhantomData }
    }

    /// Adds the SP1 XPBD soft-body position pass, optionally with the SP2
    /// soft-rigid `coupling` — the [`add_physics_soft`] shape.
    #[inline]
    #[must_use]
    pub fn soft(mut self, coupling: bool) -> Self {
        self.opts.soft = true;
        self.opts.coupling = coupling;
        self
    }

    /// Registers the SP4 COLORED soft step in place of the uncoupled one — the
    /// [`add_physics_soft_colored`] shape (the non-coupling path).
    #[inline]
    #[must_use]
    pub fn soft_colored(mut self) -> Self {
        self.opts.soft = true;
        self.opts.coupling = false;
        self.opts.soft_colored = true;
        self
    }

    /// Drops the std-lib S5 pose sync (see the type doc for why it is ON by
    /// default here). The body then owns its pose in `RigidBody` alone — nothing
    /// writes `Transform`, so a renderer keeps drawing it at its spawn pose.
    #[inline]
    #[must_use]
    pub fn without_scene_sync(mut self) -> Self {
        self.opts.scene_sync = false;
        self
    }
}

impl<S: RigidSolver + Default> Plugin for PhysicsPlugin<S> {
    fn build(&self, app: &mut App) {
        let opts = self.opts;
        // The world half FIRST, while nothing else borrows the `App`; the schedule
        // half runs inside the closure, where the builder is the only borrow on
        // offer. Order between the halves is irrelevant (registration reads no
        // resource) — this one is simply the order the borrow checker allows.
        insert_physics_resources::<S>(app.world_mut(), opts);
        app.add_systems_cfg_in(CoreSchedule::Fixed, |b| {
            register_physics_pipeline::<S>(b, opts, Some(FixedSet::Gameplay));
        });
    }

    fn name(&self) -> &'static str {
        "boyko_physics::PhysicsPlugin"
    }
}
