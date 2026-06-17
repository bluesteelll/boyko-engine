//! The swappable solver seam — the [`RigidSolver`] trait + the default
//! [`NoopSolver`] (plan D2).
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

use boyko_ecs::ecs::core::resources::resource::Resource;
use boyko_macros::Resource as ResourceDerive;

use crate::manifold::Manifold;
use crate::resources::{PhysicsConfig, SolverScratch};

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
    /// `config` carries the global tunables (e.g. `substeps`, reserved for
    /// Phase 10); `manifolds` is the deterministic, dense contact buffer.
    fn solve(
        &mut self,
        config: &PhysicsConfig,
        manifolds: &[Manifold],
        scratch: &mut SolverScratch,
    );

    /// Returns `true` when this solver does no work — lets the step system
    /// early-out before touching the scratch/manifolds (the 0%-gate for the
    /// foundation's default [`NoopSolver`]). Defaults to `false`.
    #[inline]
    fn is_noop(&self) -> bool {
        false
    }
}

/// The default no-op solver — proves the seam compiles + integrates without
/// shipping any real solve (plan D2).
///
/// [`is_noop`](RigidSolver::is_noop) returns `true`, so
/// [`physics_solve_step`](crate::systems::physics_solve_step) early-outs and the
/// pipeline degenerates to integrate-only. The real Soft-Step solver lands in
/// Phase 10 as a second `RigidSolver` impl.
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
}
