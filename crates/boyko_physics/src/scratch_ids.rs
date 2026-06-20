//! Synthetic [`ComponentId`]s for the rigid solver's transient scratch columns
//! (audit Stage P — the std::Vec gather-mirror remediation).
//!
//! The rigid solver's three gather/cache mirrors moved off `std::Vec` onto the
//! engine's own [`ScratchColumn<T>`](boyko_ecs::ecs::core::component::scratch::ScratchColumn),
//! killing the parallel data system that root-caused the SP4 colored-solve data
//! race. A `ScratchColumn` is backed by a [`ComponentPool`], which reads its
//! element [`Layout`] from the global `ComponentRegistry` by `ComponentId` — so
//! each scratch column needs a registered id even though its element type
//! (`BodyState` / `BodyEffective`) is NOT a `#[derive(Component)]` table column.
//!
//! # The reserved synthetic band (id choice)
//!
//! Production component ids are minted by `register_new` from a process-global
//! counter that starts at `0` and climbs UPWARD as `#[derive(Component)]` types
//! are first touched (component_registry.rs `NEXT_ID`). To avoid colliding with
//! that ascending production range, the scratch ids occupy a fixed band at the
//! TOP of `[0, MAX_COMPONENTS)` (`MAX_COMPONENTS == 512`): reaching it from the
//! production counter would require 500+ distinct component types, far beyond any
//! realistic world. If a production type ever DID climb into the band, the
//! `register_layout` collision check panics loudly (a wrong-type slot is never
//! silently aliased) — fail-fast, not silent corruption.
//!
//! The three ids are distinct so the three columns are independent pools; two of
//! them store the SAME element type (`BodyEffective`), which is fine — the
//! per-slot collision check keys on `(slot, TypeId)`, and two different slots
//! holding one type never conflict.
//!
//! # Registration discipline
//!
//! [`register_scratch_layouts`] installs all three layouts idempotently (a
//! same-type re-register is a silent no-op). It is called from each
//! `ScratchColumn` owner's constructor BEFORE `ScratchColumn::new` (which reads
//! the layout), so a freshly-defaulted solver / scratch resource always finds its
//! ids registered. Registration is process-global + write-once, so repeated calls
//! across many worlds cost one branch each after the first.

use boyko_ecs::ecs::core::component::component_registry::{MAX_COMPONENTS, register_layout};
use boyko_ecs::ecs::constants::{POOL_MAX_ROWS, POOL_MIN_ROWS, POOL_TARGET_DATA_BYTES};
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

use crate::resources::BodyState;
use crate::solver::contact::BodyEffective;

/// Reserve-row ceiling for a scratch column of element size `stride`, mirroring
/// the engine's own `ComponentPool` sizing
/// (`clamp(POOL_TARGET_DATA_BYTES / stride, POOL_MIN_ROWS, POOL_MAX_ROWS)`).
///
/// `ComponentPool::new(id, reserve_rows)` takes `reserve_rows` as a HARD ceiling
/// (it bypasses the internal clamp), and a `ScratchColumn::push` past it panics.
/// The engine's table/dense pools never reserve fewer than `POOL_MIN_ROWS`
/// (65 536 on the syscall arms) — the reservation is pure address space
/// (demand-committed, zero resident cost until used), so a generous ceiling costs
/// nothing and removes the per-step grow-cap hazard. A scratch column for the
/// solver's per-body rows is thus reserved to the SAME budget as a real column.
#[inline]
pub(crate) const fn scratch_reserve_rows(stride: usize) -> usize {
    let by_budget = POOL_TARGET_DATA_BYTES / stride;
    if by_budget < POOL_MIN_ROWS {
        POOL_MIN_ROWS
    } else if by_budget > POOL_MAX_ROWS {
        POOL_MAX_ROWS
    } else {
        by_budget
    }
}

/// Synthetic id for [`SolverScratch`](crate::resources::SolverScratch)'s
/// `bodies: ScratchColumn<BodyState>` gather target (mirror 2). Top of the band.
pub(crate) const SCRATCH_ID_BODY_STATE: usize = MAX_COMPONENTS - 1;

/// Synthetic id for
/// [`ColoredSoftStepSolver`](crate::solver::ColoredSoftStepSolver)'s
/// `bodies: ScratchColumn<BodyEffective>` colored per-body view (mirror 1, the
/// race-fix column).
pub(crate) const SCRATCH_ID_BODY_EFF_COLORED: usize = MAX_COMPONENTS - 2;

/// Synthetic id for
/// [`SoftStepSolver`](crate::solver::SoftStepSolver)'s
/// `bodies: ScratchColumn<BodyEffective>` serial per-body view (mirror 3).
pub(crate) const SCRATCH_ID_BODY_EFF_SERIAL: usize = MAX_COMPONENTS - 3;

/// Registers the [`Layout`](std::alloc::Layout) of every scratch element type
/// under its reserved synthetic id, idempotently.
///
/// Called from each scratch-column owner's constructor before
/// `ScratchColumn::new` (which reads the registered layout). The underlying
/// `register_layout` is write-once + process-global: a same-type re-register is a
/// silent no-op, so calling this from every constructor across every world costs
/// at most one `OnceLock::set` per id for the process lifetime.
///
/// # Panics
/// * a production `#[derive(Component)]` type already occupies one of the reserved
///   band ids with a DIFFERENT type (the `register_layout` collision check —
///   fail-fast, see the module docs).
#[inline]
pub(crate) fn register_scratch_layouts() {
    register_layout::<BodyState>(SCRATCH_ID_BODY_STATE);
    register_layout::<BodyEffective>(SCRATCH_ID_BODY_EFF_COLORED);
    register_layout::<BodyEffective>(SCRATCH_ID_BODY_EFF_SERIAL);
}

/// The [`ComponentId`] wrapper for [`SCRATCH_ID_BODY_STATE`].
#[inline]
pub(crate) fn body_state_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_BODY_STATE)
}

/// The [`ComponentId`] wrapper for [`SCRATCH_ID_BODY_EFF_COLORED`].
#[inline]
pub(crate) fn body_eff_colored_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_BODY_EFF_COLORED)
}

/// The [`ComponentId`] wrapper for [`SCRATCH_ID_BODY_EFF_SERIAL`].
#[inline]
pub(crate) fn body_eff_serial_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_BODY_EFF_SERIAL)
}
