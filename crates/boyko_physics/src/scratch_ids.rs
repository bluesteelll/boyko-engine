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
use boyko_ecs::ecs::constants::{
    POOL_MAX_ROWS, POOL_MIN_ROWS, POOL_STAGGER_LINES, POOL_TARGET_DATA_BYTES,
};
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_utils::bit_mask::bit_set_256::BitSet256;

use crate::manifold::BodyIndex;
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

// ── The cache-set invariant on the whole scratch band (P2-CACHE-FIX) ─────────
//
// `ComponentPool` staggers each pool's in-reservation base by
// `pool_base_stagger(id) = (id % POOL_STAGGER_LINES) * CACHE_LINE_SIZE`, so
// element `i` of neighbouring columns lands in DIFFERENT L1/L2 sets. Two ids
// congruent mod `POOL_STAGGER_LINES` collapse back onto the same set — and the
// solver sweeps ~24 contact columns per contact, which is exactly the
// conflict-miss storm that cost the measured ~40 % rigid-solver regression when
// these columns first moved off `std::Vec` (constants.rs, `pool_base_stagger`).
//
// The band is a single CONTIGUOUS descending run, so distinctness reduces to a
// width check: a run of at most `POOL_STAGGER_LINES` consecutive integers has
// pairwise-distinct residues mod `POOL_STAGGER_LINES`. Asserting the width is
// therefore a compile-time proof of the whole property.
//
// ⚠ This is the constraint a future migration is most likely to break, because
// it breaks SILENTLY: adding columns is a correctness no-op and the cost shows
// up only as a throughput regression nobody attributes to id assignment. Widen
// the band past `POOL_STAGGER_LINES` and this assert fires at compile time;
// allocate a column OUTSIDE the run and `scratch_band_stagger_slots_are_distinct`
// fires at test time.

/// Highest id in the SOLVER cohort (inclusive) — the body mirrors plus the 31
/// contact columns, which the colored solve sweeps together at slot `i`.
const SOLVER_COHORT_TOP: usize = SCRATCH_ID_BODY_STATE;

/// Lowest id in the solver cohort (inclusive) — the bottom of the SOLVER TAIL
/// below, NOT of the contact band. Each tail column is swept inside a loop that
/// also touches a body mirror, so the tail belongs to this cohort and the run
/// has to reach down over it for the width check to prove distinctness for every
/// column the cohort actually contains.
const SOLVER_COHORT_BOTTOM: usize = SCRATCH_ID_SOLVER_TAIL_BOTTOM;

/// Number of ids the solver cohort spans, top and bottom inclusive.
const SOLVER_COHORT_WIDTH: usize = SOLVER_COHORT_TOP - SOLVER_COHORT_BOTTOM + 1;

// ⚠ THE CONSTRAINT IS PER-COHORT, NOT GLOBAL, and the difference decides whether
// the migration is possible at all.
//
// `pool_base_stagger(id) = (id % POOL_STAGGER_LINES) * CACHE_LINE_SIZE` spreads
// columns across L1/L2 sets, and two ids congruent mod 64 collapse onto one set.
// What matters is only that columns swept together AT INDEX `i` IN ONE HOT LOOP
// — a COHORT — do not collide; columns in different loops never contend for a set
// at the same moment, so distinct cohorts may reuse the same slots freely.
//
// That distinction is load-bearing. Migrating physics' remaining `std::Vec` bulk
// brings the live column count to ~94, and 94 consecutive ids CANNOT have
// pairwise-distinct residues mod 64 — a single global band would be impossible on
// arithmetic alone. Per cohort it is comfortable: the solver sweeps 31 contact
// columns, the graph 8, the broadphase 13, the soft scratch 19, each far under
// the 64-slot period.
//
// Each cohort therefore gets its OWN contiguous run, and a run of at most
// `POOL_STAGGER_LINES` consecutive integers has pairwise-distinct residues — so
// asserting each cohort's width is a compile-time proof for that cohort.
//
// ⚠ This is the constraint a future migration is most likely to break, because it
// breaks SILENTLY: adding a colliding column is a correctness no-op and shows up
// only as a throughput regression nobody attributes to id assignment. Widen a
// cohort past the period and its assert fires at compile time; allocate a column
// outside its cohort's run and `scratch_band_stagger_slots_are_distinct` fires at
// test time.

// Solver cohort contiguity: the contact band must begin exactly one id below the
// lowest body mirror, or the run has a hole and the width check no longer implies
// distinctness.
const _: () = assert!(
    SCRATCH_ID_CONTACT_BAND_TOP + 1 == SCRATCH_ID_BODY_EFF_SERIAL,
    "the solver cohort is not contiguous: the contact band must start one id \
     below SCRATCH_ID_BODY_EFF_SERIAL, or the width check below stops implying \
     pairwise-distinct cache-set staggers"
);

const _: () = assert!(
    SOLVER_COHORT_WIDTH <= POOL_STAGGER_LINES,
    "the solver cohort is wider than one stagger period: two of its columns now \
     share a cache-set slot and element i of both lands in the same L1/L2 set — \
     the P2 conflict-miss storm"
);

// ── The SOLVER TAIL of the solver cohort (audit Stage 4) ────────────────────
//
// `SolverScratch`'s two non-body buffers and `IslandSleep`'s awake mask moved off
// `std::Vec` onto kernel columns. They are NOT a cohort of their own, because
// every one of them is swept inside a loop that also touches a body mirror:
//
// * `vn_initial` is pushed by `SoftStepSolver::build_constraints` while the
//   BodyState snapshot ([`SCRATCH_ID_BODY_STATE`]) is read, and re-read by
//   `apply_restitution` against that same snapshot;
// * `touched` takes a bit inside both solvers' `write_back` loop — the loop that
//   writes the snapshot — and is read by `physics_apply` as it walks it;
// * `awake_rows` is probed by `write_back_awake` in that same loop.
//
// So the tail EXTENDS the solver cohort's contiguous run downward instead of
// starting a new one, and `SOLVER_COHORT_WIDTH` covers it. Reading them as a
// separate cohort would be the cheaper bookkeeping and the wrong claim: a
// separate run proves nothing about collisions against the body mirrors, which
// is exactly the pair that shares a loop.

/// Number of ids in the solver tail.
const SOLVER_TAIL_COLUMN_COUNT: usize = 3;

/// Synthetic id for [`SolverScratch`](crate::resources::SolverScratch)'s
/// `vn_initial: ScratchColumn<f32>` — the per-contact-point approach velocity the
/// serial TGS solver captures before its substep loop and consumes in the
/// post-loop restitution pass.
///
/// The COLORED solver has its own `vn_initial` inside `ContactColumns`
/// (`contact_column_id(25)`); the two are different columns of different length
/// (per-point in the colored SoA build vs per-point in the serial build) and must
/// not share an id.
pub(crate) const SCRATCH_ID_VN_INITIAL: usize = SCRATCH_ID_CONTACT_BAND_BOTTOM - 1;

/// Synthetic id for the chunk column behind
/// [`SolverScratch`](crate::resources::SolverScratch)'s `touched` mask — one
/// `BitSet256` per 256 body rows.
pub(crate) const SCRATCH_ID_TOUCHED_SOLVER: usize = SCRATCH_ID_CONTACT_BAND_BOTTOM - 2;

/// Synthetic id for the chunk column behind
/// [`IslandSleep`](crate::resources::IslandSleep)'s `awake_rows` mask.
///
/// ⚠ A SECOND [`TouchedMask`](crate::resources::TouchedMask) needs a SECOND id,
/// and the reason it is not optional is that reusing the first one FAILS
/// SILENTLY: `register_layout` treats a same-type re-registration as a no-op, so
/// both masks would compile, run, and land on the same `pool_base_stagger` —
/// element `i` of both in one L1/L2 set, which is the conflict-miss storm the
/// stagger exists to prevent.
pub(crate) const SCRATCH_ID_TOUCHED_AWAKE: usize = SCRATCH_ID_CONTACT_BAND_BOTTOM - 3;

/// Bottom of the solver tail (inclusive), and so of the whole solver cohort.
const SCRATCH_ID_SOLVER_TAIL_BOTTOM: usize =
    SCRATCH_ID_CONTACT_BAND_BOTTOM - SOLVER_TAIL_COLUMN_COUNT;

// The tail ids are written as explicit offsets rather than handed out by an index
// function, because they are three DIFFERENT element types rather than a
// homogeneous run. This assert is what keeps that spelling honest: add a fourth
// id without moving the bottom and the run has a hole the width check would still
// wave through.
const _: () = assert!(
    SCRATCH_ID_TOUCHED_AWAKE == SCRATCH_ID_SOLVER_TAIL_BOTTOM,
    "the solver tail's lowest named id is not its declared bottom: the contiguous      run has a hole, and SOLVER_COHORT_WIDTH stops covering every tail column"
);

// ── The CONSTRAINT-GRAPH cohort (audit Stage 4) ─────────────────────────────
//
// `ConstraintGraph`'s eight buffers are swept together by `build` — union-find
// over `uf_parent`/`uf_size`, then `island_of`, then the two CSR pairs, then the
// coloring occupancy — so they are one cohort and must not collide with each
// other. They MAY share slots with the solver cohort above: the graph is built
// before the solve and the two loops never run at the same index at the same
// time.

/// Number of `ScratchColumn`s backing [`ConstraintGraph`](crate::resources::ConstraintGraph).
pub(crate) const GRAPH_COLUMN_COUNT: usize = 8;

/// Top of the constraint-graph cohort — one id below the solver cohort's bottom.
pub(crate) const SCRATCH_ID_GRAPH_TOP: usize = SOLVER_COHORT_BOTTOM - 1;

/// Bottom of the constraint-graph cohort (inclusive).
pub(crate) const SCRATCH_ID_GRAPH_BOTTOM: usize =
    SCRATCH_ID_GRAPH_TOP - (GRAPH_COLUMN_COUNT - 1);

const _: () = assert!(
    GRAPH_COLUMN_COUNT <= POOL_STAGGER_LINES,
    "the constraint-graph cohort is wider than one stagger period"
);

// Cohorts may SHARE cache-set slots (they never sweep at the same index at the
// same time) but must never share an ID: two pools under one `ComponentId` would
// register different element types, and the registry's same-type re-register is a
// SILENT no-op, so the collision would not announce itself.
const _: () = assert!(
    SCRATCH_ID_GRAPH_TOP < SOLVER_COHORT_BOTTOM,
    "the constraint-graph cohort overlaps the solver cohort"
);

// The scratch region grows DOWNWARD from the top of the id space, toward the
// production counter climbing up from 0. This is the floor that keeps the two
// apart: cross it and a `#[derive(Component)]` type could claim a scratch id.
const _: () = assert!(
    SCRATCH_ID_GRAPH_BOTTOM >= MAX_COMPONENTS - 64,
    "the physics scratch region has grown more than 64 ids below the top of the      component-id space; production ids climb from 0 and the reserved band is no      longer comfortably out of their reach"
);

/// The [`ComponentId`] for graph column `k` (`0`-based, in field order),
/// descending from [`SCRATCH_ID_GRAPH_TOP`].
#[inline]
pub(crate) fn graph_column_id(k: usize) -> ComponentId {
    debug_assert!(k < GRAPH_COLUMN_COUNT, "graph column index out of cohort");
    ComponentId::new(SCRATCH_ID_GRAPH_TOP - k)
}

// ── The BROADPHASE cohort (audit Stage 4) ───────────────────────────────────
//
// `BroadphaseGrid`'s thirteen buffers are swept together by `build` — the fine
// CSR (`counts` -> `cell_start` -> `cell_bodies` via `cursor`), the coarse
// size-class CSR, the oversized list, the radius scratch and the pair
// bookkeeping. One cohort, so their ids must be pairwise distinct mod
// `POOL_STAGGER_LINES`; they MAY reuse the solver's and graph's slots, since
// those loops never run at the same index at the same moment.

/// Number of `ScratchColumn`s backing [`BroadphaseGrid`](crate::resources::BroadphaseGrid).
pub(crate) const BROADPHASE_COLUMN_COUNT: usize = 13;

/// Top of the broadphase cohort — one id below the graph cohort's bottom.
pub(crate) const SCRATCH_ID_BROADPHASE_TOP: usize = SCRATCH_ID_GRAPH_BOTTOM - 1;

/// Bottom of the broadphase cohort (inclusive).
pub(crate) const SCRATCH_ID_BROADPHASE_BOTTOM: usize =
    SCRATCH_ID_BROADPHASE_TOP - (BROADPHASE_COLUMN_COUNT - 1);

const _: () = assert!(
    BROADPHASE_COLUMN_COUNT <= POOL_STAGGER_LINES,
    "the broadphase cohort is wider than one stagger period"
);

const _: () = assert!(
    SCRATCH_ID_BROADPHASE_TOP < SCRATCH_ID_GRAPH_BOTTOM,
    "the broadphase cohort overlaps the constraint-graph cohort"
);

// The scratch region grows DOWNWARD from the top of the id space toward the
// production counter climbing up from 0, and the broadphase cohort is now its
// lowest edge. This is the floor that keeps the two apart — cross it and a
// `#[derive(Component)]` type could claim a scratch id. It moves down with each
// cohort added, which is exactly why it is asserted against the LOWEST one
// rather than against whichever cohort happened to be last when it was written.
const _: () = assert!(
    SCRATCH_ID_BROADPHASE_BOTTOM >= MAX_COMPONENTS - 64,
    "the physics scratch region has grown more than 64 ids below the top of the      component-id space; production ids climb from 0 and the reserved band is no      longer comfortably out of their reach"
);

/// The [`ComponentId`] for broadphase column `k` (`0`-based, in field order).
#[inline]
pub(crate) fn broadphase_column_id(k: usize) -> ComponentId {
    debug_assert!(k < BROADPHASE_COLUMN_COUNT, "broadphase column index out of cohort");
    ComponentId::new(SCRATCH_ID_BROADPHASE_TOP - k)
}

/// Registers the element layout of every [`BroadphaseGrid`] column, idempotently.
///
/// Eleven `u32` columns, one `f32` (`scratch_radii`, k = 9) and one
/// `(BodyIndex, BodyIndex)` pair column (`candidates`, k = 10). The registry's
/// collision check keys on `(slot, TypeId)`, so registering each under its REAL
/// type means a wrong-typed reuse of a slot panics loudly rather than aliasing.
pub(crate) fn register_broadphase_column_layouts() {
    for k in 0..BROADPHASE_COLUMN_COUNT {
        match k {
            9 => register_layout::<f32>(broadphase_column_id(k).get()),
            10 => register_layout::<(BodyIndex, BodyIndex)>(broadphase_column_id(k).get()),
            _ => register_layout::<u32>(broadphase_column_id(k).get()),
        }
    }
}

/// Registers the element layout of every [`ConstraintGraph`] column, idempotently.
///
/// Seven `u32` columns and one `u64` (`color_occ`'s bitset words). Same-type
/// re-registration is a silent no-op, so calling this from the constructor costs
/// one branch per id after the first.
pub(crate) fn register_graph_column_layouts() {
    for k in 0..GRAPH_COLUMN_COUNT - 1 {
        register_layout::<u32>(graph_column_id(k).get());
    }
    register_layout::<u64>(graph_column_id(GRAPH_COLUMN_COUNT - 1).get());
}

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
    register_solver_tail_layouts();
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

/// Registers the element layout of every solver-tail column, idempotently.
///
/// One `f32` (`vn_initial`) and two `BitSet256` chunk columns. The two masks are
/// registered under DIFFERENT ids on purpose — see [`SCRATCH_ID_TOUCHED_AWAKE`].
fn register_solver_tail_layouts() {
    register_layout::<f32>(SCRATCH_ID_VN_INITIAL);
    register_layout::<BitSet256>(SCRATCH_ID_TOUCHED_SOLVER);
    register_layout::<BitSet256>(SCRATCH_ID_TOUCHED_AWAKE);
}

/// The [`ComponentId`] wrapper for [`SCRATCH_ID_VN_INITIAL`].
#[inline]
pub(crate) fn vn_initial_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_VN_INITIAL)
}

/// The [`ComponentId`] wrapper for [`SCRATCH_ID_TOUCHED_SOLVER`].
#[inline]
pub(crate) fn touched_solver_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_TOUCHED_SOLVER)
}

/// The [`ComponentId`] wrapper for [`SCRATCH_ID_TOUCHED_AWAKE`].
#[inline]
pub(crate) fn touched_awake_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_TOUCHED_AWAKE)
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

#[cfg(test)]
mod tests {
    use super::*;
    use boyko_ecs::ecs::constants::pool_base_stagger;

    /// Enumerates a cohort's ids and asserts every one gets its OWN cache-set slot.
    ///
    /// The const asserts above prove the property for a cohort as a RANGE. This
    /// proves it for the ids actually handed out, which is the stronger statement
    /// and the one that survives a column allocated outside its run: a new id
    /// colliding with an existing one turns this red even though the declared width
    /// is untouched. It compares against the kernel's own `pool_base_stagger`
    /// rather than re-deriving `% 64`, so the day `POOL_STAGGER_LINES` moves the
    /// test follows it instead of testing a stale rule.
    fn assert_cohort_slots_distinct(name: &str, ids: &[usize], declared_width: usize) {
        assert_eq!(
            ids.len(),
            declared_width,
            "anti-vacuity: the enumerated {name} ids must cover the whole declared              cohort, or this checks a subset and a collision can hide outside it"
        );
        for (i, &a) in ids.iter().enumerate() {
            for &b in &ids[i + 1..] {
                assert_ne!(
                    pool_base_stagger(a),
                    pool_base_stagger(b),
                    "{name}: ids {a} and {b} share cache-set stagger slot {} of {} —                      element i of both columns lands in the same L1/L2 set, which is                      the P2 conflict-miss storm the stagger exists to prevent",
                    a % POOL_STAGGER_LINES,
                    POOL_STAGGER_LINES
                );
            }
        }
    }

    fn solver_cohort_ids() -> Vec<usize> {
        let mut ids = vec![
            SCRATCH_ID_BODY_STATE,
            SCRATCH_ID_BODY_EFF_COLORED,
            SCRATCH_ID_BODY_EFF_SERIAL,
        ];
        ids.extend((0..CONTACT_COLUMN_COUNT).map(|k| contact_column_id(k).get()));
        ids.extend([
            SCRATCH_ID_VN_INITIAL,
            SCRATCH_ID_TOUCHED_SOLVER,
            SCRATCH_ID_TOUCHED_AWAKE,
        ]);
        ids
    }

    fn graph_cohort_ids() -> Vec<usize> {
        (0..GRAPH_COLUMN_COUNT).map(|k| graph_column_id(k).get()).collect()
    }

    fn broadphase_cohort_ids() -> Vec<usize> {
        (0..BROADPHASE_COLUMN_COUNT).map(|k| broadphase_column_id(k).get()).collect()
    }

    #[test]
    fn scratch_band_stagger_slots_are_distinct() {
        assert_cohort_slots_distinct("solver cohort", &solver_cohort_ids(), SOLVER_COHORT_WIDTH);
        assert_cohort_slots_distinct("graph cohort", &graph_cohort_ids(), GRAPH_COLUMN_COUNT);
        assert_cohort_slots_distinct(
            "broadphase cohort",
            &broadphase_cohort_ids(),
            BROADPHASE_COLUMN_COUNT,
        );
    }

    /// Each cohort is a contiguous run with no hole and no duplicate — the premise
    /// the width-based const asserts rest on.
    #[test]
    fn scratch_band_is_a_contiguous_run() {
        for (name, mut ids, top, bottom) in [
            ("solver cohort", solver_cohort_ids(), SOLVER_COHORT_TOP, SOLVER_COHORT_BOTTOM),
            ("graph cohort", graph_cohort_ids(), SCRATCH_ID_GRAPH_TOP, SCRATCH_ID_GRAPH_BOTTOM),
            (
                "broadphase cohort",
                broadphase_cohort_ids(),
                SCRATCH_ID_BROADPHASE_TOP,
                SCRATCH_ID_BROADPHASE_BOTTOM,
            ),
        ] {
            ids.sort_unstable();
            assert_eq!(ids[0], bottom, "{name} starts at its declared bottom");
            assert_eq!(ids[ids.len() - 1], top, "{name} ends at its declared top");
            for w in ids.windows(2) {
                assert_eq!(
                    w[1],
                    w[0] + 1,
                    "hole or duplicate in the {name} between {} and {} — the width                      assert stops implying distinct staggers once the run is not dense",
                    w[0],
                    w[1]
                );
            }
        }
    }

    /// The cohorts must not OVERLAP: sharing a stagger slot across cohorts is fine,
    /// but sharing an ID is a registry collision on a different element type.
    #[test]
    fn cohorts_do_not_share_ids() {
        let cohorts = [
            ("solver", solver_cohort_ids()),
            ("graph", graph_cohort_ids()),
            ("broadphase", broadphase_cohort_ids()),
        ];
        for (i, (na, a)) in cohorts.iter().enumerate() {
            for (nb, b) in &cohorts[i + 1..] {
                for id in b {
                    assert!(
                        !a.contains(id),
                        "{nb} id {id} also belongs to the {na} cohort — two pools would                          register different element types under one ComponentId, and the                          registry's same-type re-register is a SILENT no-op"
                    );
                }
            }
        }
    }
}
