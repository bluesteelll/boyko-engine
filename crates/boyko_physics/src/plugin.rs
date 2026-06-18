//! Physics-pipeline wiring — the `PhysicsPlugin`-shaped free function (plan D3 /
//! MINOR-1).
//!
//! There is no `Plugin` trait in the engine (the demo wires systems via a free
//! fn taking `&mut ScheduleBuilder`), so [`add_physics_systems`] is the faithful
//! idiom: it inserts the physics resources on the world and registers the six
//! pipeline stages on the builder in deterministic `.after(...)` order, returning
//! the stage handles so the caller can identify the physics block.
//!
//! Per MINOR-1 it does NOT call `builder.build(world)` — that consumes the
//! builder and is the caller's job.

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;

use crate::resources::{
    BroadphaseGrid, ContactPairs, IntegrationMode, Manifolds, PhysicsConfig, SolverScratch,
};
use crate::sdf_query::SdfField;
use crate::solver::RigidSolver;
use crate::systems::{
    physics_apply, physics_broadphase, physics_gather, physics_integrate, physics_narrowphase,
    physics_narrowphase_sdf, physics_solve_step,
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
/// [`add_physics_systems`] via `.after(..)`; an external caller wishing to order
/// its OWN systems relative to a physics stage needs a real `SystemKey`, which the
/// engine's privacy currently keeps internal (a pre-existing engine limitation,
/// not introduced here — a future `pub use` of `SystemKey` would let this struct
/// carry the keys directly).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicsStageKeys {
    /// Descriptor index of the [`physics_integrate`] stage — the **block head**.
    /// A caller ordering its own systems before the whole physics block keys off
    /// this (the integrate stage is the first to run and carries no `.after`).
    pub integrate: usize,
    /// Descriptor index of the [`physics_gather`] stage.
    pub gather: usize,
    /// Descriptor index of the [`physics_broadphase`] stage.
    pub broadphase: usize,
    /// Descriptor index of the [`physics_narrowphase`] stage.
    pub narrowphase: usize,
    /// Descriptor index of the [`physics_narrowphase_sdf`] SDF-collision stage, or
    /// `None` for the body-only [`add_physics_systems`] path (P2 W5).
    ///
    /// Present only when the pipeline was wired by
    /// [`add_physics_sdf`](crate::plugin::add_physics_sdf); it runs AFTER
    /// `narrowphase` and BEFORE `solve` so the solver sees both body-body and
    /// body-vs-SDF contacts.
    pub narrowphase_sdf: Option<usize>,
    /// Descriptor index of the [`physics_solve_step`] stage.
    pub solve: usize,
    /// Descriptor index of the [`physics_apply`] stage.
    pub apply: usize,
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
/// `S::default()` (the `ResMut<S>` the generic step system dispatches on, D2),
/// and the [`IntegrationMode`] derived from `S::default().owns_integration()`
/// (C2 — gates [`physics_integrate`] off for
/// an owning TGS solver so it does not double-integrate).
///
/// Stages registered in deterministic order via `.after(...)`:
/// `integrate → gather → broadphase → narrowphase → solve_step::<S> → apply`
/// (D3). `integrate` carries no `.after` (it is the block head); each later stage
/// `.after`s its predecessor — so the whole block runs in a fixed intra-order
/// regardless of registration interleaving with the caller's own systems.
///
/// Per MINOR-1 this does NOT call `builder.build(world)` — the caller owns the
/// build (`runner.rs:325` precedent).
pub fn add_physics_systems<S: RigidSolver + Default>(
    builder: &mut ScheduleBuilder,
    world: &mut EcsMaster,
) -> PhysicsStageKeys {
    // `with_sdf = false`: body-only pipeline (no `SdfField`, no SDF stage).
    add_physics_pipeline::<S>(builder, world, false)
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
pub fn add_physics_sdf<S: RigidSolver + Default>(
    builder: &mut ScheduleBuilder,
    world: &mut EcsMaster,
) -> PhysicsStageKeys {
    add_physics_pipeline::<S>(builder, world, true)
}

/// Shared wiring for [`add_physics_systems`] (`with_sdf = false`) and
/// [`add_physics_sdf`] (`with_sdf = true`): inserts the resources and registers the
/// pipeline, optionally splicing the SDF-collision stage between narrowphase and
/// solve.
fn add_physics_pipeline<S: RigidSolver + Default>(
    builder: &mut ScheduleBuilder,
    world: &mut EcsMaster,
    with_sdf: bool,
) -> PhysicsStageKeys {
    // Reused, capacity-preserving step buffers (principle 5 — no per-step alloc).
    world.insert_resource(PhysicsConfig::default());
    world.insert_resource(ContactPairs::with_capacity(INITIAL_BODY_CAPACITY));
    world.insert_resource(Manifolds::with_capacity(INITIAL_BODY_CAPACITY));
    world.insert_resource(SolverScratch::with_capacity(INITIAL_BODY_CAPACITY));
    // O2: the grid broadphase scratch (capacity-reused). Inserted unconditionally
    // so `physics_broadphase`'s `ResMut<BroadphaseGrid>` param always resolves; it
    // stays untouched while `PhysicsConfig::broadphase` is the default `AllPairs`.
    world.insert_resource(BroadphaseGrid::with_capacity(INITIAL_BODY_CAPACITY));
    if with_sdf {
        // The CPU-authoritative SDF scene (empty by default; the caller fills it
        // with the same edit list the GPU renders).
        world.insert_resource(SdfField::default());
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
    world.insert_resource(S::default());

    // Block head: integrate runs first, unordered relative to the caller's
    // pre-physics systems. `.key().0` is the public descriptor index of the
    // engine's `SystemKey` (see `PhysicsStageKeys`).
    let integrate = builder.add_system(physics_integrate).key();
    let gather = builder.add_system(physics_gather).after(integrate).key();
    let broadphase = builder.add_system(physics_broadphase).after(gather).key();
    let narrowphase = builder
        .add_system(physics_narrowphase)
        .after(broadphase)
        .key();

    // W5: the body-vs-SDF stage runs AFTER body-body narrowphase (both append to
    // `Manifolds`) and is forced BEFORE the solve via an explicit ordering edge — a
    // `ResMut`/`Res` conflict on `Manifolds` alone would serialize them but not pin
    // the order, so the edge is load-bearing. Registered only when `with_sdf` (the
    // 0%-gate: a body-only schedule never registers this stage). The `SystemConfig`
    // borrows `builder` mutably, so extract the `SystemKey` immediately (drop the
    // handle) before re-borrowing `builder` for the next stage.
    let narrowphase_sdf_key = if with_sdf {
        Some(
            builder
                .add_system(physics_narrowphase_sdf)
                .after(narrowphase)
                .key(),
        )
    } else {
        None
    };

    let solve = if let Some(sdf) = narrowphase_sdf_key {
        // Pin the SDF stage before the solve (it must finish appending its
        // manifolds before the solver reads the buffer).
        builder
            .add_system(physics_solve_step::<S>)
            .after(narrowphase)
            .after(sdf)
            .key()
    } else {
        builder
            .add_system(physics_solve_step::<S>)
            .after(narrowphase)
            .key()
    };
    let apply = builder.add_system(physics_apply).after(solve).key();

    PhysicsStageKeys {
        integrate: integrate.0,
        gather: gather.0,
        broadphase: broadphase.0,
        narrowphase: narrowphase.0,
        narrowphase_sdf: narrowphase_sdf_key.map(|k| k.0),
        solve: solve.0,
        apply: apply.0,
    }
}
