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
    BroadphaseGrid, ConstraintGraph, ContactPairs, IntegrationMode, IslandSleep, Manifolds,
    PhysicsConfig, SolverScratch,
};
use crate::sdf_query::SdfField;
use crate::soft::physics_soft_step;
use crate::solver::colored::ColoredSoftStepSolver;
use crate::solver::RigidSolver;
use crate::systems::{
    physics_apply, physics_broadphase, physics_build_graph, physics_gather, physics_integrate,
    physics_narrowphase, physics_narrowphase_sdf, physics_solve_colored, physics_solve_step,
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
    /// Descriptor index of the [`physics_build_graph`] constraint-graph stage, or
    /// `None` for the non-colored paths (plan O4 / Decision 7).
    ///
    /// Present only when the pipeline was wired by
    /// [`add_physics_colored`](crate::plugin::add_physics_colored); it runs AFTER
    /// the narrowphase stage(s) and BEFORE `solve`, building (but in O4 NOT
    /// consuming) the islands + coloring. The solve stays byte-identical (the
    /// 0%-gate).
    pub build_graph: Option<usize>,
    /// Descriptor index of the [`physics_solve_step`] stage.
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
    // `with_sdf = false`, `colored = false`, `soft = false`: body-only pipeline (no
    // `SdfField`, no SDF stage, no constraint-graph stage, no soft pass) —
    // byte-identical to the shipped path.
    add_physics_pipeline::<S>(builder, world, false, false, false, false)
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
/// **O4 produces the partition only — it does NOT change the solve.** The shipped
/// [`SoftStepSolver`](crate::solver::SoftStepSolver) still solves in manifold order
/// over the unchanged manifold buffer, so the simulation output is byte-identical
/// to [`add_physics_systems`] (the campaign 0%-gate; a future O5 stage consumes the
/// graph). The opt-in is the entire gate: a world that never calls this never
/// builds the graph. The returned [`PhysicsStageKeys::build_graph`] carries the
/// stage's descriptor index.
pub fn add_physics_colored<S: RigidSolver + Default>(
    builder: &mut ScheduleBuilder,
    world: &mut EcsMaster,
) -> PhysicsStageKeys {
    add_physics_pipeline::<S>(builder, world, false, true, false, false)
}

/// Inserts the physics resources and registers the COLORED-SOLVE pipeline (Phase
/// O5, Decision 7) — `physics_build_graph` (O4) followed by the single-threaded
/// [`physics_solve_colored`](crate::systems::physics_solve_colored) stage, which
/// REPLACES the default [`physics_solve_step`](crate::systems::physics_solve_step).
///
/// Unlike [`add_physics_colored`] (which builds the graph but leaves the shipped
/// [`SoftStepSolver`](crate::solver::SoftStepSolver) solving in manifold order —
/// the O4 byte-identical, partition-only path), this wires the
/// [`ColoredSoftStepSolver`](crate::solver::ColoredSoftStepSolver): the solve
/// runs in graph-COLOR order over the solver's SoA `ContactColumns` (a
/// Gauss-Seidel sweep across colors), with the converged impulses stored in
/// canonical order (IM-2b). The shipped `SoftStepSolver` is byte-untouched and
/// its solve stage is NOT registered on this path — the two solvers never both
/// run (Decision 7).
///
/// # The value change (Phase O5)
///
/// The colored sweep order differs from the reference manifold-order sweep, so
/// the converged float values DIFFER (but are equally valid) — validated against
/// tolerance acceptance gates, not a bit-baseline against `SoftStepSolver`. The
/// colored solve is run-to-run bit-identical and never moves a static body. This
/// path takes NO solver type parameter: the colored solver is fixed
/// ([`ColoredSoftStepSolver`](crate::solver::ColoredSoftStepSolver)), since the
/// colored solve consumes the graph through its own entry point, not the generic
/// [`RigidSolver`] seam.
///
/// Opt-in: a world that does not call this is byte-for-byte unaffected (the
/// colored stage is never registered — the campaign 0%-gate). The returned
/// [`PhysicsStageKeys`] carries both the `build_graph` and `solve` stage indices.
pub fn add_physics_colored_solve(
    builder: &mut ScheduleBuilder,
    world: &mut EcsMaster,
) -> PhysicsStageKeys {
    add_physics_pipeline::<ColoredSoftStepSolver>(builder, world, false, true, true, false)
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
    add_physics_pipeline::<S>(builder, world, true, false, false, false)
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
/// The soft step is a STRICTLY DISJOINT integrator: it never reads or writes the
/// rigid [`SolverScratch`], never sets a touched bit, and never enters
/// `physics_apply`, so the rigid simulation is byte-identical whether the soft pass
/// runs or not. Opt-in: a world that uses [`add_physics_systems`] is byte-for-byte
/// unaffected (the soft stage is never registered — the campaign 0%-gate). The
/// returned [`PhysicsStageKeys::soft_step`] carries the soft stage's descriptor
/// index.
pub fn add_physics_soft<S: RigidSolver + Default>(
    builder: &mut ScheduleBuilder,
    world: &mut EcsMaster,
) -> PhysicsStageKeys {
    add_physics_pipeline::<S>(builder, world, false, false, false, true)
}

/// Shared wiring for [`add_physics_systems`] (`with_sdf = false`,
/// `colored = false`), [`add_physics_sdf`] (`with_sdf = true`),
/// [`add_physics_colored`] (`colored = true`, graph-only — the default solve
/// runs), and [`add_physics_colored_solve`] (`colored = true` + `colored_solve =
/// true` — the Phase-O5 colored solve REPLACES the default solve): inserts the
/// resources and registers the pipeline, optionally splicing the SDF-collision
/// stage and/or the constraint-graph stage between narrowphase and solve, and
/// selecting the default or the colored solve stage.
fn add_physics_pipeline<S: RigidSolver + Default>(
    builder: &mut ScheduleBuilder,
    world: &mut EcsMaster,
    with_sdf: bool,
    colored: bool,
    colored_solve: bool,
    soft: bool,
) -> PhysicsStageKeys {
    // The colored solve requires the constraint graph; the type system cannot
    // express it, so guard the invariant the callers uphold.
    debug_assert!(
        !colored_solve || colored,
        "invariant: the colored solve stage requires the constraint graph (colored == true)"
    );
    // Reused, capacity-preserving step buffers (principle 5 — no per-step alloc).
    // The `colored` flag rides the config so `physics_build_graph` (registered only
    // when `colored`) and any future graph consumer share one switch.
    world.insert_resource(PhysicsConfig {
        colored,
        soft_body: soft,
        ..PhysicsConfig::default()
    });
    world.insert_resource(ContactPairs::with_capacity(INITIAL_BODY_CAPACITY));
    world.insert_resource(Manifolds::with_capacity(INITIAL_BODY_CAPACITY));
    world.insert_resource(SolverScratch::with_capacity(INITIAL_BODY_CAPACITY));
    // O2: the grid broadphase scratch (capacity-reused). Inserted unconditionally
    // so `physics_broadphase`'s `ResMut<BroadphaseGrid>` param always resolves; it
    // stays untouched while `PhysicsConfig::broadphase` is the default `AllPairs`.
    world.insert_resource(BroadphaseGrid::with_capacity(INITIAL_BODY_CAPACITY));
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
        let cfg = builder.add_system(physics_build_graph).after(narrowphase);
        let cfg = if let Some(sdf) = narrowphase_sdf_key {
            cfg.after(sdf)
        } else {
            cfg
        };
        Some(cfg.key())
    } else {
        None
    };

    // O5: the colored-solve path registers `physics_solve_colored` (which CONSUMES
    // the constraint graph) in place of the default generic `physics_solve_step::<S>`
    // — the two solvers never both run (Decision 7). The default path keeps the
    // shipped solve stage, byte-untouched. The `physics_solve_colored` stage's
    // `Res<ConstraintGraph>` makes the `.after(build_graph)` edge load-bearing (not
    // merely documentary as on the O4 graph-only path).
    let mut solve_cfg = if colored_solve {
        builder.add_system(physics_solve_colored).after(narrowphase)
    } else {
        builder.add_system(physics_solve_step::<S>).after(narrowphase)
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
    let apply = builder.add_system(physics_apply).after(solve).key();

    // SP1: the soft-body XPBD pass runs as a SEPARATE position pass AFTER the rigid
    // solve and BEFORE apply. It is a strictly disjoint integrator (it touches only
    // `SoftBody` columns, never the rigid `SolverScratch`), so it could in principle
    // run unordered relative to the rigid block — but pinning it `.after(solve)`
    // `.before(apply)` keeps the whole physics step in one deterministic window and
    // matches the plan's placement. Registered only when `soft` (the 0%-gate: a
    // non-soft schedule never registers this stage). The `apply` key already exists
    // (registered above), so the `.before(apply)` edge resolves.
    let soft_step_key = if soft {
        Some(
            builder
                .add_system(physics_soft_step)
                .after(solve)
                .before(apply)
                .key(),
        )
    } else {
        None
    };

    PhysicsStageKeys {
        integrate: integrate.0,
        gather: gather.0,
        broadphase: broadphase.0,
        narrowphase: narrowphase.0,
        narrowphase_sdf: narrowphase_sdf_key.map(|k| k.0),
        build_graph: build_graph_key.map(|k| k.0),
        solve: solve.0,
        soft_step: soft_step_key.map(|k| k.0),
        apply: apply.0,
    }
}
