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

/// Top of the rigid colored solver's contact-column band (audit Stage P — P2).
///
/// The colored solver's SoA contact working set (`ContactColumns`) moved off 31
/// parallel `std::Vec`s onto 31 kernel-native [`ScratchColumn`]s, killing the
/// whole-struct `&mut *self.cols` reborrow each parallel worker performed (the
/// rigid Tree-Borrows race). The band is a CONTIGUOUS descending run starting one
/// id BELOW the three body-mirror ids ([`SCRATCH_ID_BODY_EFF_SERIAL`] == 509), so
/// the rigid columns occupy `508 ..= 478` (31 ids) with no overlap.
///
/// [`ScratchColumn`]: boyko_ecs::ecs::core::component::scratch::ScratchColumn
pub(crate) const SCRATCH_ID_CONTACT_BAND_TOP: usize = MAX_COMPONENTS - 4;

/// Number of `ScratchColumn`s backing the colored solver's `ContactColumns`.
pub(crate) const CONTACT_COLUMN_COUNT: usize = 31;

/// Bottom of the contact-column band (inclusive): `508 - (31 - 1) == 478`.
/// Headroom below this id remains free for future physics scratch columns.
pub(crate) const SCRATCH_ID_CONTACT_BAND_BOTTOM: usize =
    SCRATCH_ID_CONTACT_BAND_TOP - (CONTACT_COLUMN_COUNT - 1);

/// The [`ComponentId`] for contact column `k` (`0`-based, in `ContactColumns`
/// field order), descending from [`SCRATCH_ID_CONTACT_BAND_TOP`].
///
/// `k == 0` -> id `508`, ascending `k` -> descending id, `k == 30` -> id `478`.
#[inline]
pub(crate) fn contact_column_id(k: usize) -> ComponentId {
    debug_assert!(k < CONTACT_COLUMN_COUNT, "contact column index out of band");
    ComponentId::new(SCRATCH_ID_CONTACT_BAND_TOP - k)
}

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
    register_contact_column_layouts();
}

/// Registers the [`Layout`](std::alloc::Layout) of every contact column's element
/// type under its band id (audit Stage P — P2), in `ContactColumns` field order.
///
/// The 31 columns and their element types (field order, descending from
/// [`SCRATCH_ID_CONTACT_BAND_TOP`]):
/// * `ra_{x,y,z}`, `rb_{x,y,z}`, `normal_{x,y,z}`, `tangent1_{x,y,z}`,
///   `tangent2_{x,y,z}` — 15 × `f32`;
/// * `separation`, `friction`, `restitution`, `normal_impulse`,
///   `tangent1_impulse`, `tangent2_impulse` — 6 × `f32`;
/// * `body_a`, `body_b` — 2 × `u32`;
/// * `b_is_sentinel` — `bool`;
/// * `warm_key` — `u64`;
/// * `vn_initial` — `f32`;
/// * `color_offsets`, `canonical`, `group_start`, `color_group_start` — 4 × `u32`;
/// * `manifold_base` — `(u32, u32)`.
///
/// Idempotent + process-global (each `register_layout` is write-once): re-entry
/// from another world / solver costs one branch per id after the first.
#[inline]
fn register_contact_column_layouts() {
    // Field order MUST match `ContactColumns` so `contact_column_id(k)` lines up
    // with the `k`-th declared column.
    let mut k = SCRATCH_ID_CONTACT_BAND_TOP;
    // ra/rb/normal/tangent1/tangent2 (15) + separation/friction/restitution +
    // the three impulses (6) = 21 f32 columns.
    for _ in 0..21 {
        register_layout::<f32>(k);
        k -= 1;
    }
    register_layout::<u32>(k); // body_a
    k -= 1;
    register_layout::<u32>(k); // body_b
    k -= 1;
    register_layout::<bool>(k); // b_is_sentinel
    k -= 1;
    register_layout::<u64>(k); // warm_key
    k -= 1;
    register_layout::<f32>(k); // vn_initial
    k -= 1;
    register_layout::<u32>(k); // color_offsets
    k -= 1;
    register_layout::<u32>(k); // canonical
    k -= 1;
    register_layout::<u32>(k); // group_start
    k -= 1;
    register_layout::<u32>(k); // color_group_start
    k -= 1;
    register_layout::<(u32, u32)>(k); // manifold_base
    debug_assert_eq!(
        k, SCRATCH_ID_CONTACT_BAND_BOTTOM,
        "contact column band must end exactly at the reserved bottom id"
    );
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
