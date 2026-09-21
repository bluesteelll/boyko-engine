//! The swappable solver seam — the [`RigidSolver`] trait + the foundation
//! [`NoopSolver`] (plan D2), the reference [`SoftStepSolver`] (P2 W2), and the
//! colored [`ColoredSoftStepSolver`] that the default world runs
//! ([`DefaultRigidSolver`], 2026-09-18).
//!
//! # Static dispatch, deliberately NOT object-safe (D2)
//!
//! `RigidSolver: Resource` and `Resource: Sized` (`Send + Sync + Sized`), so the
//! trait is **not object-safe** — `dyn RigidSolver` does not compile. This is by
//! design: the step system [`physics_solve_step`](crate::systems::physics_solve_step)
//! is generic over `S: RigidSolver`, the solver instance rides as a `ResMut<S>`,
//! and the user picks the backend at schedule-build time. Monomorphization makes
//! `S::solve` a direct, inlinable call with zero vtable (principle 1) — the
//! solver's per-contact loop (the Phase-10 hot loop) inlines across the seam
//! rather than being firewalled behind a vtable.
//!
//! A new solver (a real Soft-Step solver, an SDF backend, or an external
//! Rapier/Jolt adapter) slots in by implementing this trait on its own
//! `Resource` type and registering it via
//! [`add_physics_systems`](crate::plugin::add_physics_systems) — no edit to this
//! crate (open for external backends, unlike an enum).
//!
//! # Integration ownership (C2)
//!
//! A solver that runs its own substep loop (the TGS [`SoftStepSolver`]) also
//! integrates the bodies inside that loop. It signals this by returning `true`
//! from [`RigidSolver::owns_integration`]; the plugin then inserts an
//! [`IntegrationMode::SolverOwned`](crate::resources::IntegrationMode) resource
//! so the pipeline's [`physics_integrate`](crate::systems::physics_integrate)
//! early-returns and does NOT double-integrate. See the C2 contract block in
//! [`crate::systems`].

pub mod colored;
pub mod contact;
pub mod simd;
pub mod soft_step;
pub(crate) mod warm_records;
pub mod warm_start;

use boyko_ecs::ecs::core::resources::resource::Resource;
use boyko_macros::Resource as ResourceDerive;

use crate::manifold::Manifold;
use crate::resources::{PhysicsConfig, SolverScratch};

pub use colored::ColoredSoftStepSolver;
pub use soft_step::SoftStepSolver;
pub use crate::row_identity::WarmSeedStats;

/// The solver the default physics world runs: the colored TGS-Soft solve
/// ([`ColoredSoftStepSolver`]) with the O7 AVX2 cohort kernel on
/// ([`PhysicsConfig::simd_solve`] defaults to `true`). Owner decision, 2026-09-18.
///
/// Rust has no default type parameters on functions, so this alias is the single
/// place the default is named: `add_physics_systems::<DefaultRigidSolver>` (or any
/// other `add_physics_*` entry) wires the constraint graph and the colored solve
/// stage, because the plugin selects the solve stage from the solver TYPE.
///
/// The previous default, [`SoftStepSolver`], stays in the tree as the reference
/// oracle and is still selected by naming it: `add_physics_*::<SoftStepSolver>`.
/// Its converged values differ from the colored solve's (a different, equally
/// valid Gauss-Seidel sweep order), so the two are compared by tolerance, never by
/// bits.
pub type DefaultRigidSolver = ColoredSoftStepSolver;

/// The swappable rigid-body solver seam (plan D2).
///
/// A solver reads the ordered `manifolds` and mutates the dense
/// [`SolverScratch::bodies`](crate::resources::SolverScratch) in place, setting
/// [`SolverScratch::touched`](crate::resources::SolverScratch) for every row it
/// writes (the apply stage writes back only touched rows). The solve is
/// single-threaded over the deterministic manifold order (plan D4) — a pair
/// `(a, b)` writes both rows, so parallel pair-solve would race.
///
/// `Resource: Sized` makes this trait non-object-safe on purpose (see the
/// module docs): use the generic `S: RigidSolver` dispatch, never `dyn`.
pub trait RigidSolver: Resource + 'static {
    /// Resolves all contacts for one step, mutating `scratch.bodies` in place
    /// and flagging `scratch.touched` for every written row.
    ///
    /// `config` carries the global tunables (`substeps`, `dt`, the soft-constraint
    /// set); `manifolds` is the deterministic, dense contact buffer.
    fn solve(
        &mut self,
        config: &PhysicsConfig,
        manifolds: &[Manifold],
        scratch: &mut SolverScratch,
    );

    /// Returns `true` when this solver does no work — lets the step system
    /// early-out before touching the scratch/manifolds (the 0%-gate for the
    /// foundation [`NoopSolver`]). Defaults to `false`.
    #[inline]
    fn is_noop(&self) -> bool {
        false
    }

    /// Returns `true` when this solver runs its own substep integration (TGS),
    /// so the pipeline's [`physics_integrate`](crate::systems::physics_integrate)
    /// must NOT also integrate (the C2 integration-ownership gate). Defaults to
    /// `false` (the foundation [`NoopSolver`] leaves integration to the
    /// pipeline).
    ///
    /// The plugin reads this on the chosen solver at wire-up time and inserts the
    /// matching [`IntegrationMode`](crate::resources::IntegrationMode) resource;
    /// `physics_integrate` then early-returns for a solver-owned step. See the C2
    /// contract block in [`crate::systems`].
    #[inline]
    fn owns_integration(&self) -> bool {
        false
    }
}

/// The foundation no-op solver — proves the seam compiles + integrates without
/// shipping any real solve (plan D2).
///
/// [`is_noop`](RigidSolver::is_noop) returns `true`, so
/// [`physics_solve_step`](crate::systems::physics_solve_step) early-outs and the
/// pipeline degenerates to integrate-only. The real Soft-Step solver is
/// [`SoftStepSolver`].
#[derive(ResourceDerive, Default)]
pub struct NoopSolver;

impl RigidSolver for NoopSolver {
    /// Does nothing — the no-op solver leaves every body state untouched.
    #[inline]
    fn solve(
        &mut self,
        _config: &PhysicsConfig,
        _manifolds: &[Manifold],
        _scratch: &mut SolverScratch,
    ) {
    }

    /// Always `true` — the step system skips the solve entirely.
    #[inline]
    fn is_noop(&self) -> bool {
        true
    }

    // `owns_integration` keeps the trait default `false`: the no-op solver does
    // NOT integrate, so the pipeline's `physics_integrate` stays live.
}
