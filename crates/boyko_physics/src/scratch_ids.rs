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
//! that ascending production range, the scratch ids occupy a fixed region at
//! the TOP of `[0, MAX_COMPONENTS)` (`MAX_COMPONENTS == 512`), floored at
//! [`SCRATCH_REGION_MIN_ID`] — reaching that floor from the production counter
//! takes 384 distinct component types against a measured ~142 (see the constant's
//! docs for the census). If a production type ever DID climb into the region, the
//! `register_layout` collision check panics loudly (a wrong-type slot is never
//! silently aliased) — fail-fast, not silent corruption. The floor is the margin
//! that keeps that panic unreachable; the collision check is the proof.
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

use crate::broadphase_tree::RowRec;
use crate::broadphase_tree::bvh::{Item, Node8};
use crate::manifold::{BodyIndex, Manifold};
use crate::math::Vec3;
use crate::narrowphase::axis_cache::AxisEntry;
use crate::narrowphase::carry::PairTag;
use crate::narrowphase::reuse::RowFrame;
use crate::resources::BodyState;
use crate::row_identity::{RowKey, SleepLatch};
use crate::solver::contact::BodyEffective;
use crate::solver::soft_step::{ManifoldConstraint, PointConstraint};
use crate::solver::colored::{CohortCold, CohortHead, ManifoldTag, RankBlock};
use crate::solver::warm_records::{WarmRecord, WarmRun};
use crate::solver::warm_start::WarmEntry;

/// Top of the rigid colored solver's contact-column band (audit Stage P — P2).
///
/// The colored solver's contact working set (`CohortColumns`, the cohort-shaped
/// tables since L11 C2; the per-point `ContactColumns` SoA before) moved off
/// parallel `std::Vec`s onto kernel-native [`ScratchColumn`]s, killing the
/// whole-struct `&mut *self.cols` reborrow each parallel worker performed (the
/// rigid Tree-Borrows race). The band is a CONTIGUOUS descending run starting one
/// id BELOW the three body-mirror ids ([`SCRATCH_ID_BODY_EFF_SERIAL`] == 509), so
/// the rigid columns occupy `508 ..= 499` (10 ids) with no overlap; the band keeps
/// its top, and the ids the per-point columns freed are headroom below the cohorts.
///
/// [`ScratchColumn`]: boyko_ecs::ecs::core::component::scratch::ScratchColumn
pub(crate) const SCRATCH_ID_CONTACT_BAND_TOP: usize = MAX_COMPONENTS - 4;

/// Number of `ScratchColumn`s backing the colored solver's `CohortColumns`.
///
/// 31 before L11 C1 (which deleted the per-point `warm_key` and the `canonical`
/// order — the warm store is per manifold now, `solver/warm_records.rs` — and added
/// the per-manifold `plan`), 30 before L11 C2, which replaced the 25 per-point SoA
/// columns and `manifold_base` with the four cohort tables (`heads`, `blocks`,
/// `cold`, `rank_cold`), the per-manifold `tags` and the per-color cohort CSR.
pub(crate) const CONTACT_COLUMN_COUNT: usize = 10;

/// Bottom of the contact-column band (inclusive): `508 - (10 - 1) == 499`.
pub(crate) const SCRATCH_ID_CONTACT_BAND_BOTTOM: usize =
    SCRATCH_ID_CONTACT_BAND_TOP - (CONTACT_COLUMN_COUNT - 1);

/// The [`ComponentId`] for contact column `k` (`0`-based, in `CohortColumns`
/// field order), descending from [`SCRATCH_ID_CONTACT_BAND_TOP`].
///
/// `k == 0` -> id `508`, ascending `k` -> descending id, `k == 9` -> id `499`.
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

/// Highest id in the SOLVER cohort (inclusive) — the body mirrors plus the
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
// arithmetic alone. Per cohort it is comfortable: the solver sweeps 10 contact
// columns (30 before L11 C2), the graph 8, the broadphase 13, the soft scratch 19,
// each far under the 64-slot period.
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

/// Number of ids in the solver tail that are named one by one below: three above
/// the warm-store run and three under it.
const SOLVER_TAIL_NAMED_COUNT: usize = 6;

/// Number of `ScratchColumn`s backing the double-buffered warm stores: the serial
/// solver's `read` + `write` [`WarmStartTable`](crate::solver::warm_start::WarmStartTable)s,
/// then the colored solver's per-manifold records (L11 C1): two record columns, two
/// key columns and the cold sorted index. See [`warm_table_id`].
pub(crate) const WARM_TABLE_COLUMN_COUNT: usize = 7;

/// Number of ids in the solver tail.
const SOLVER_TAIL_COLUMN_COUNT: usize = SOLVER_TAIL_NAMED_COUNT + WARM_TABLE_COLUMN_COUNT;

/// Synthetic id for [`SolverScratch`](crate::resources::SolverScratch)'s
/// `vn_initial: ScratchColumn<f32>` — the per-contact-point approach velocity the
/// serial TGS solver captures before its substep loop and consumes in the
/// post-loop restitution pass.
///
/// The COLORED solver has its own `vn0` rows inside `CohortColumns`
/// (`contact_column_id(3)`, one `[f32; 8]` per cohort rank); the two are different
/// columns of different shape (per rank in the colored cohort layout vs per point
/// in the serial build) and must not share an id.
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

/// Top of the warm-start table run, one id below the last named tail column.
pub(crate) const SCRATCH_ID_WARM_TABLE_TOP: usize = SCRATCH_ID_TOUCHED_AWAKE - 1;

/// Synthetic id for the serial TGS solver's per-manifold constraint column
/// (`SoftStepSolver::manifolds`), rebuilt each solve in manifold order.
pub(crate) const SCRATCH_ID_SERIAL_MANIFOLD_CONSTRAINTS: usize =
    SCRATCH_ID_WARM_TABLE_TOP - WARM_TABLE_COLUMN_COUNT;

/// Synthetic id for the serial TGS solver's flattened per-point constraint column
/// (`SoftStepSolver::points`), indexed by `manifold.point_start + p`.
pub(crate) const SCRATCH_ID_SERIAL_POINT_CONSTRAINTS: usize =
    SCRATCH_ID_SERIAL_MANIFOLD_CONSTRAINTS - 1;

/// Synthetic id for the colored solver's O8 integrate-freeze snapshot
/// (`ColoredSoftStepSolver::frozen`) — the `(row, BodyState)` pairs of slept
/// bodies, captured before the substep loop and restored after it.
pub(crate) const SCRATCH_ID_COLORED_FROZEN_ROWS: usize =
    SCRATCH_ID_SERIAL_POINT_CONSTRAINTS - 1;

/// Bottom of the solver tail (inclusive), and so of the whole solver cohort.
const SCRATCH_ID_SOLVER_TAIL_BOTTOM: usize =
    SCRATCH_ID_CONTACT_BAND_BOTTOM - SOLVER_TAIL_COLUMN_COUNT;

// The tail ids are written as explicit offsets rather than handed out by an index
// function, because they are three DIFFERENT element types rather than a
// homogeneous run. This assert is what keeps that spelling honest: add a fourth
// id without moving the bottom and the run has a hole the width check would still
// wave through.
const _: () = assert!(
    SCRATCH_ID_WARM_TABLE_TOP - (WARM_TABLE_COLUMN_COUNT - 1)
        == SCRATCH_ID_SERIAL_MANIFOLD_CONSTRAINTS + 1,
    "the solver tail has a hole between the warm-start run and the constraint columns below it"
);

const _: () = assert!(
    SCRATCH_ID_COLORED_FROZEN_ROWS == SCRATCH_ID_SOLVER_TAIL_BOTTOM,
    "the solver tail's lowest id is not its declared bottom: the contiguous run has a hole, and SOLVER_COHORT_WIDTH stops covering every tail column"
);

/// The [`ComponentId`] for warm-store column `k`:
///
/// | `k` | column | element |
/// |---|---|---|
/// | 0 | the serial solver's read table | `WarmEntry` |
/// | 1 | its write table | `WarmEntry` |
/// | 2 | the colored solver's records, side 0 | `WarmRecord` |
/// | 3 | its records, side 1 | `WarmRecord` |
/// | 4 | its keys, side 0 | `u64` |
/// | 5 | its keys, side 1 | `u64` |
/// | 6 | its cold sorted index | `(u64, u32)` |
///
/// The read/write BINDING is only true at construction: each solver's store swaps
/// its two sides wholesale, so after the first step the side read is the one built
/// under the other id. That is harmless and deliberate — the point of separate ids
/// is that the two columns keep DIFFERENT cache-set staggers while they alternate
/// roles, not that a role owns an id.
#[inline]
pub(crate) fn warm_table_id(k: usize) -> ComponentId {
    debug_assert!(k < WARM_TABLE_COLUMN_COUNT, "warm store index out of cohort");
    ComponentId::new(SCRATCH_ID_WARM_TABLE_TOP - k)
}

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

/// Synthetic id for [`ContactPairs`](crate::resources::ContactPairs)'s pair list.
///
/// One id below the grid's own thirteen and part of the SAME cohort: the grid
/// writes this buffer at index `i` in the pass that reads its counting columns,
/// so it is one loop, not two.
pub(crate) const SCRATCH_ID_CONTACT_PAIRS: usize = SCRATCH_ID_BROADPHASE_BOTTOM - 1;

/// Synthetic id for [`ContactPairs`](crate::resources::ContactPairs)'s previous
/// step's pair list (L9 C2, the pair carry's `pairs_prev`).
///
/// One id below the pair list and part of the same cohort: the two columns swap
/// roles at the start of every broadphase, so either one is the list the grid
/// and the tree write and the narrowphase reads, and the narrowphase's join reads
/// the other in the same pass.
pub(crate) const SCRATCH_ID_CONTACT_PAIRS_PREV: usize = SCRATCH_ID_CONTACT_PAIRS - 1;

/// Bottom of the broadphase COHORT (inclusive) — two below the grid's own bottom,
/// because both pair lists belong to it.
const BROADPHASE_COHORT_BOTTOM: usize = SCRATCH_ID_CONTACT_PAIRS_PREV;

/// Width of the broadphase cohort: the grid's columns plus the two pair lists.
const BROADPHASE_COHORT_WIDTH: usize = BROADPHASE_COLUMN_COUNT + 2;

const _: () = assert!(
    BROADPHASE_COHORT_WIDTH <= POOL_STAGGER_LINES,
    "the broadphase cohort is wider than one stagger period"
);

const _: () = assert!(
    SCRATCH_ID_BROADPHASE_TOP < SCRATCH_ID_GRAPH_BOTTOM,
    "the broadphase cohort overlaps the constraint-graph cohort"
);

// —— The SOFT-COUPLING cohort (audit Stage 4) ————————————————————
//
// `SoftRigidReaction`'s two columns are accumulated into by the SAME loop, one
// contact at a time (`accumulate` writes row `idx` of both), and drained by the
// same loop in `physics_soft_rigid_apply`. One cohort of two.

/// Number of `ScratchColumn`s backing
/// [`SoftRigidReaction`](crate::soft::coupling::SoftRigidReaction).
pub(crate) const SOFT_COUPLING_COLUMN_COUNT: usize = 2;

/// Top of the soft-coupling cohort — one id below the narrowphase cohort's bottom.
pub(crate) const SCRATCH_ID_SOFT_COUPLING_TOP: usize = SCRATCH_ID_NARROWPHASE_BOTTOM - 1;

/// Bottom of the soft-coupling cohort (inclusive).
pub(crate) const SCRATCH_ID_SOFT_COUPLING_BOTTOM: usize =
    SCRATCH_ID_SOFT_COUPLING_TOP - (SOFT_COUPLING_COLUMN_COUNT - 1);

const _: () = assert!(
    SOFT_COUPLING_COLUMN_COUNT <= POOL_STAGGER_LINES,
    "the soft-coupling cohort is wider than one stagger period"
);

const _: () = assert!(
    SCRATCH_ID_SOFT_COUPLING_TOP < SCRATCH_ID_NARROWPHASE_BOTTOM,
    "the soft-coupling cohort overlaps the narrowphase cohort"
);

/// The [`ComponentId`] for soft-coupling column `k` (`0` = `dv_lin`, `1` = `dv_ang`).
#[inline]
pub(crate) fn soft_coupling_column_id(k: usize) -> ComponentId {
    debug_assert!(k < SOFT_COUPLING_COLUMN_COUNT, "soft-coupling column index out of cohort");
    ComponentId::new(SCRATCH_ID_SOFT_COUPLING_TOP - k)
}

/// Registers the element layout of both [`SoftRigidReaction`] columns, idempotently.
pub(crate) fn register_soft_coupling_column_layouts() {
    for k in 0..SOFT_COUPLING_COLUMN_COUNT {
        register_layout::<Vec3>(soft_coupling_column_id(k).get());
    }
}

// —— The SOFT-GRAPH cohort (audit Stage 4) ———————————————————————
//
// `SoftColorScratch` holds THREE `ParticleColorGraph`s (distance, volume,
// self-collision) of six columns each, plus the per-substep pair list. Each graph
// sweeps its own six together — the occupancy matrix is probed and set while
// `chosen` is pushed, then the counting sort walks `chosen` / `color_start` /
// `cursor` / `color_items` in one pass — so six is the cohort that has to be
// distinct. The three graphs are colored at different times and could share slots,
// and `pair_list` co-sweeps with the self-collision graph.
//
// All nineteen are laid out as ONE contiguous run anyway. Nineteen is far under the
// stagger period, so the over-constraint is free, and one run means one width check
// instead of four assertions about which graph may alias which.

/// Number of `ScratchColumn`s in one
/// [`ParticleColorGraph`](crate::soft::colored::ParticleColorGraph):
/// `color_occ`, `chosen`, `color_start`, `color_items`, `cursor`, `seen_scratch`.
pub(crate) const SOFT_GRAPH_COLUMNS_PER_INSTANCE: usize = 6;

/// Number of `ParticleColorGraph` instances a `SoftColorScratch` owns.
pub(crate) const SOFT_GRAPH_INSTANCES: usize = 3;

/// Total ids in the soft-graph cohort: three graphs plus the pair list.
pub(crate) const SOFT_GRAPH_COLUMN_COUNT: usize =
    SOFT_GRAPH_COLUMNS_PER_INSTANCE * SOFT_GRAPH_INSTANCES + 1;

/// Top of the soft-graph cohort — one id below the soft-coupling cohort's bottom.
pub(crate) const SCRATCH_ID_SOFT_GRAPH_TOP: usize = SCRATCH_ID_SOFT_COUPLING_BOTTOM - 1;

/// Bottom of the soft-graph cohort (inclusive) — the pair list's id.
pub(crate) const SCRATCH_ID_SOFT_GRAPH_BOTTOM: usize =
    SCRATCH_ID_SOFT_GRAPH_TOP - (SOFT_GRAPH_COLUMN_COUNT - 1);

const _: () = assert!(
    SOFT_GRAPH_COLUMN_COUNT <= POOL_STAGGER_LINES,
    "the soft-graph cohort is wider than one stagger period"
);

const _: () = assert!(
    SCRATCH_ID_SOFT_GRAPH_TOP < SCRATCH_ID_SOFT_COUPLING_BOTTOM,
    "the soft-graph cohort overlaps the soft-coupling cohort"
);

/// The [`ComponentId`] for column `k` of `ParticleColorGraph` instance `instance`
/// (`0` = distance, `1` = volume, `2` = self-collision).
///
/// ⚠ Each INSTANCE needs its own six ids. Three graphs under one set would compile,
/// run, and give all three the same `pool_base_stagger` — the silent cache-set
/// collapse a same-type re-registration cannot report.
#[inline]
pub(crate) fn soft_graph_column_id(instance: usize, k: usize) -> ComponentId {
    debug_assert!(instance < SOFT_GRAPH_INSTANCES, "soft graph instance out of cohort");
    debug_assert!(k < SOFT_GRAPH_COLUMNS_PER_INSTANCE, "soft graph column index out of cohort");
    ComponentId::new(SCRATCH_ID_SOFT_GRAPH_TOP - (instance * SOFT_GRAPH_COLUMNS_PER_INSTANCE + k))
}

/// The [`ComponentId`] for `SoftColorScratch`'s per-substep self-collision pair list
/// — the cohort's lowest id.
#[inline]
pub(crate) fn soft_pair_list_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_SOFT_GRAPH_BOTTOM)
}

/// Registers the element layout of every soft-graph column, idempotently.
///
/// Per instance: `color_occ` and `seen_scratch` are `u64` bitset words, the other
/// four are `u32`; the pair list is `(u32, u32)`.
pub(crate) fn register_soft_graph_column_layouts() {
    for instance in 0..SOFT_GRAPH_INSTANCES {
        for k in 0..SOFT_GRAPH_COLUMNS_PER_INSTANCE {
            let id = soft_graph_column_id(instance, k).get();
            match k {
                0 | 5 => register_layout::<u64>(id),
                _ => register_layout::<u32>(id),
            }
        }
    }
    register_layout::<(u32, u32)>(soft_pair_list_id().get());
}

// —— The ROW-IDENTITY cohort (defect A, interim) ————————————————————
//
// `RowIdentity` (row_identity.rs) records the gather's per-row `RowKey`s (slot index +
// generation, one `u64`) and builds the previous-row map and its stage-2 row list (T5);
// `IslandSleep` and `BoxAxisCache` each keep one carry scratch. Eight
// columns in TWO runs, plus `IslandSleep`'s island contact key in the gap between them,
// because the loops they are swept in differ:
//
// * UPPER run (3 ids, directly below the soft-graph cohort): the current and previous
//   row keys and the added rows. The gather pushes the keys and the added rows at row `i`
//   in the loop that pushes the BodyState snapshot, and the two key columns swap roles
//   every gather, so all three must clear `SCRATCH_ID_BODY_STATE`'s slot.
// * LOWER run (5 ids): `prev_row`, the stage-2 sort pool, the latch carry, the axis
//   carry and the stage-2 row list. `prev_row` is read inside both solvers' constraint
//   builds, which sweep the SOLVER cohort; the axis carry is written and read beside the
//   narrowphase's pair list, manifolds and axis slots; the stage-2 list is pushed beside
//   `prev_row` in the build's stage 2 (L9 C2, T5).
// * THE ISLAND CONTACT KEY (one id, in the gap): `IslandSleep::begin_step` sweeps it beside
//   the graph's `island_of` and `island_manifold_start`, and `permute_latch` sweeps it beside
//   `prev_row` and the latch carry. Its id is the highest id below the upper run whose slot
//   clears the constraint-graph cohort, derived independently of `prev_row`'s. An ordering
//   assert keeps it strictly between the runs, so the cohort width check also proves it
//   clear of every other row-identity column.
//
// ⚠ THE GAP BETWEEN THE RUNS IS LOAD-BEARING. The solver cohort is 44 ids wide and owns
// stagger slots 20..=63, so `prev_row` cannot sit anywhere in a contiguous run below the
// soft-graph cohort (slots 31..=37) without sharing a cache set with a contact column
// (slot 34 is `color_offsets`). The lower run therefore starts at the highest id whose
// slot clears the whole solver cohort — COMPUTED rather than hand-picked, so a cohort
// above that moves re-derives it — and the ids between the runs, except the island contact
// key's, stay free for a later cohort. The asserts below prove every co-swept family at
// compile time.

/// Whether `id` shares a cache-set stagger slot with any id of the contiguous run
/// `bottom ..= top`.
const fn shares_stagger_slot(id: usize, top: usize, bottom: usize) -> bool {
    let slot = id % POOL_STAGGER_LINES;
    let mut k = bottom;
    while k <= top {
        if k % POOL_STAGGER_LINES == slot {
            return true;
        }
        k += 1;
    }
    false
}

/// The highest id at or below `start` whose stagger slot clears the run `bottom ..= top`.
///
/// Fails const evaluation (underflow) if the run is a full stagger period wide, which the
/// cohort width asserts already rule out.
const fn highest_id_clear_of(start: usize, top: usize, bottom: usize) -> usize {
    let mut id = start;
    while shares_stagger_slot(id, top, bottom) {
        id -= 1;
    }
    id
}

/// Number of ids in the row-identity cohort's upper run.
const ROW_IDENTITY_UPPER_COUNT: usize = 3;

/// Number of ids in the row-identity cohort's lower run.
const ROW_IDENTITY_LOWER_COUNT: usize = 5;

/// Top of the row-identity cohort — one id below the soft-graph cohort's bottom.
pub(crate) const SCRATCH_ID_ROW_IDENTITY_TOP: usize = SCRATCH_ID_SOFT_GRAPH_BOTTOM - 1;

/// Synthetic id for `RowIdentity`'s current row → `RowKey` column. Top of the upper run.
pub(crate) const SCRATCH_ID_ROW_ENTITY: usize = SCRATCH_ID_ROW_IDENTITY_TOP;

/// Synthetic id for `RowIdentity`'s previous row → `RowKey` column.
pub(crate) const SCRATCH_ID_ROW_ENTITY_PREV: usize = SCRATCH_ID_ROW_IDENTITY_TOP - 1;

/// Synthetic id for `RowIdentity`'s added-rows column. Bottom of the upper run.
pub(crate) const SCRATCH_ID_ROW_ADDED: usize = SCRATCH_ID_ROW_IDENTITY_TOP - 2;

/// Synthetic id for `RowIdentity`'s `prev_row` map. Top of the lower run: the highest id
/// below the upper run whose stagger slot clears the solver cohort.
pub(crate) const SCRATCH_ID_ROW_PREV: usize =
    highest_id_clear_of(SCRATCH_ID_ROW_ADDED - 1, SOLVER_COHORT_TOP, SOLVER_COHORT_BOTTOM);

/// Synthetic id for `RowIdentity`'s stage-2 sort pool.
pub(crate) const SCRATCH_ID_ROW_REMAP_SORT: usize = SCRATCH_ID_ROW_PREV - 1;

/// Synthetic id for `IslandSleep`'s latch carry scratch.
pub(crate) const SCRATCH_ID_SLEEP_LATCH_PREV: usize = SCRATCH_ID_ROW_PREV - 2;

/// Synthetic id for `BoxAxisCache`'s per-pair carried axis.
pub(crate) const SCRATCH_ID_AXIS_REMAP: usize = SCRATCH_ID_ROW_PREV - 3;

/// Synthetic id for `RowIdentity`'s stage-2 row list (T5, L9 C2). Bottom of the lower run.
pub(crate) const SCRATCH_ID_ROW_STAGE2: usize = SCRATCH_ID_ROW_PREV - 4;

/// Bottom of the row-identity cohort (inclusive).
pub(crate) const SCRATCH_ID_ROW_IDENTITY_BOTTOM: usize = SCRATCH_ID_ROW_STAGE2;

/// Synthetic id for `IslandSleep`'s per-row island contact key (defect A4), in the gap
/// between the two runs: the highest id below the upper run whose stagger slot clears the
/// constraint-graph cohort.
///
/// Derived independently of [`SCRATCH_ID_ROW_PREV`], which is not searched again from it:
/// if a future cohort move makes the two searches collide, the ordering assert below fires
/// and the id must be re-derived.
pub(crate) const SCRATCH_ID_SLEEP_ISLAND_KEY: usize = highest_id_clear_of(
    SCRATCH_ID_ROW_ADDED - 1,
    SCRATCH_ID_GRAPH_TOP,
    SCRATCH_ID_GRAPH_BOTTOM,
);

// `begin_step`: the island contact key beside `island_of` and `island_manifold_start`.
const _: () = assert!(
    !shares_stagger_slot(
        SCRATCH_ID_SLEEP_ISLAND_KEY,
        SCRATCH_ID_GRAPH_TOP,
        SCRATCH_ID_GRAPH_BOTTOM
    ),
    "begin_step sweeps the sleep key beside island_of and island_manifold_start, and its slot is \
     one of the graph cohort's"
);

// `permute_latch`: the island contact key beside `prev_row` and the latch carry. Because
// the key lies strictly between the runs, the cohort width check below proves its slot
// distinct from both, and it cannot equal any row-identity id.
const _: () = assert!(
    SCRATCH_ID_ROW_PREV < SCRATCH_ID_SLEEP_ISLAND_KEY
        && SCRATCH_ID_SLEEP_ISLAND_KEY < SCRATCH_ID_ROW_ADDED,
    "the sleep key sits between the row-identity runs, so the cohort width proves it clear of \
     prev_row and the latch carry and it shares no id"
);

/// Number of ids the row-identity cohort spans, top and bottom inclusive, gap included.
const ROW_IDENTITY_COHORT_WIDTH: usize =
    SCRATCH_ID_ROW_IDENTITY_TOP - SCRATCH_ID_ROW_IDENTITY_BOTTOM + 1;

const _: () = assert!(
    SCRATCH_ID_ROW_IDENTITY_TOP < SCRATCH_ID_SOFT_GRAPH_BOTTOM,
    "the row-identity cohort overlaps the soft-graph cohort"
);

const _: () = assert!(
    SCRATCH_ID_ROW_ADDED == SCRATCH_ID_ROW_IDENTITY_TOP - (ROW_IDENTITY_UPPER_COUNT - 1)
        && SCRATCH_ID_ROW_STAGE2 == SCRATCH_ID_ROW_PREV - (ROW_IDENTITY_LOWER_COUNT - 1),
    "a row-identity run has a hole: its named ids no longer tile the declared counts"
);

const _: () = assert!(
    ROW_IDENTITY_COHORT_WIDTH <= POOL_STAGGER_LINES,
    "the row-identity cohort, gap included, is wider than one stagger period, so its own \
     columns are no longer provably on distinct slots"
);

// The gather loop: BodyState snapshot + row ids + added rows at one index.
const _: () = assert!(
    !shares_stagger_slot(SCRATCH_ID_BODY_STATE, SCRATCH_ID_ROW_IDENTITY_TOP, SCRATCH_ID_ROW_ADDED),
    "the gather pushes the BodyState snapshot, the row ids and the added rows together, and \
     one of the row-identity upper run's slots is BODY_STATE's"
);

// The constraint builds: `prev_row` beside every solver-cohort column.
const _: () = assert!(
    !shares_stagger_slot(SCRATCH_ID_ROW_PREV, SOLVER_COHORT_TOP, SOLVER_COHORT_BOTTOM),
    "prev_row is read inside the constraint builds, and its slot is one of the solver cohort's"
);

// The narrowphase: the axis carry beside the broadphase + narrowphase union.
const _: () = assert!(
    !shares_stagger_slot(
        SCRATCH_ID_AXIS_REMAP,
        SCRATCH_ID_BROADPHASE_TOP,
        SCRATCH_ID_NARROWPHASE_BOTTOM
    ),
    "the axis carry is swept beside the pair list and the narrowphase columns, and its slot is \
     one of the broadphase + narrowphase union's"
);

/// Registers the element layout of every row-identity column, idempotently.
///
/// Two `RowKey` columns, four `u32` columns (the added rows, `prev_row`, the stage-2 row
/// list and the island contact key), the `(RowKey, u32)` sort pool, the `SleepLatch` carry
/// and the `u8` axis carry.
pub(crate) fn register_row_identity_layouts() {
    register_layout::<RowKey>(SCRATCH_ID_ROW_ENTITY);
    register_layout::<RowKey>(SCRATCH_ID_ROW_ENTITY_PREV);
    register_layout::<u32>(SCRATCH_ID_ROW_ADDED);
    register_layout::<u32>(SCRATCH_ID_ROW_PREV);
    register_layout::<(RowKey, u32)>(SCRATCH_ID_ROW_REMAP_SORT);
    register_layout::<SleepLatch>(SCRATCH_ID_SLEEP_LATCH_PREV);
    register_layout::<u8>(SCRATCH_ID_AXIS_REMAP);
    register_layout::<u32>(SCRATCH_ID_ROW_STAGE2);
    register_layout::<u32>(SCRATCH_ID_SLEEP_ISLAND_KEY);
}

// ── The BROADPHASE-TREE cohort (the tree broadphase, `broadphase_tree/`) ─────
//
// `BroadphaseTree`'s eleven ids: the nine columns of commit C1 — two packed trees, the radix
// ping-pong pair, the item list, two record buffers, the static pair list and the scratch
// stream — and two reserved for commit C5, the sleeper tree and its pair list. The verify
// sweeps a record buffer beside `BodyState` (slot 63) and, on a `Rows` step, `prev_row` (slot
// 37); the assembly writes `ContactPairs` (slot 16, or 15 on the steps its two lists have
// swapped roles, L9 C2) beside the stream and the records; commit C5's hint reads
// `TOUCHED_AWAKE` (in the solver cohort, slots 38..=63). The cohort sits directly below the
// row-identity cohort, ids 416..=406, slots 32..=22, so every one of those co-swept families is
// clear of it — each is asserted below. (Every id here moved up by 18 when L11 C2 narrowed the
// solver cohort from 44 ids to 26, and down by one when L9 C2 added the stage-2 row list; the
// asserts, not this prose, are the proof.)

/// Number of `ScratchColumn`s backing [`BroadphaseTree`](crate::broadphase_tree::BroadphaseTree).
pub(crate) const TREE_COLUMN_COUNT: usize = 11;

/// Tree column `k`: the active tree's nodes.
pub(crate) const TREE_ACTIVE: usize = 0;
/// Tree column `k`: the static tree's nodes.
pub(crate) const TREE_STATICS: usize = 1;
/// Tree column `k`: the sleeper tree's nodes (commit C5; the id is reserved now).
pub(crate) const TREE_SLEEPERS: usize = 2;
/// Tree column `k`: radix ping-pong A, the diverted entries, the admission's additions.
pub(crate) const TREE_SORT_A: usize = 3;
/// Tree column `k`: radix ping-pong B.
pub(crate) const TREE_SORT_B: usize = 4;
/// Tree column `k`: the items of a build.
pub(crate) const TREE_ITEMS: usize = 5;
/// Tree column `k`: record buffer 0.
pub(crate) const TREE_REC0: usize = 6;
/// Tree column `k`: record buffer 1.
pub(crate) const TREE_REC1: usize = 7;
/// Tree column `k`: the static–static pair list.
pub(crate) const TREE_SS: usize = 8;
/// Tree column `k`: the sleeper pair list (commit C5; the id is reserved now).
pub(crate) const TREE_SL: usize = 9;
/// Tree column `k`: the scratch stream (inverse map, segments, buckets).
pub(crate) const TREE_AUX: usize = 10;

/// Top of the broadphase-tree cohort — one id below the row-identity cohort's bottom.
pub(crate) const SCRATCH_ID_TREE_TOP: usize = SCRATCH_ID_ROW_IDENTITY_BOTTOM - 1;

/// Bottom of the broadphase-tree cohort (inclusive).
pub(crate) const SCRATCH_ID_TREE_BOTTOM: usize = SCRATCH_ID_TREE_TOP - (TREE_COLUMN_COUNT - 1);

const _: () = assert!(
    TREE_COLUMN_COUNT <= POOL_STAGGER_LINES,
    "the broadphase-tree cohort is wider than one stagger period"
);

const _: () = assert!(
    SCRATCH_ID_TREE_TOP < SCRATCH_ID_ROW_IDENTITY_BOTTOM,
    "the broadphase-tree cohort overlaps the row-identity cohort"
);

// The verify: a record buffer beside the BodyState snapshot.
const _: () = assert!(
    !shares_stagger_slot(SCRATCH_ID_BODY_STATE, SCRATCH_ID_TREE_TOP, SCRATCH_ID_TREE_BOTTOM),
    "the tree's verify streams BodyState beside its records, and one of the cohort's slots is BODY_STATE's"
);

// The assembly: the pair list beside the stream and the records. Either pair column is the
// list on a given step: the two swap roles at every broadphase (L9 C2).
const _: () = assert!(
    !shares_stagger_slot(SCRATCH_ID_CONTACT_PAIRS, SCRATCH_ID_TREE_TOP, SCRATCH_ID_TREE_BOTTOM)
        && !shares_stagger_slot(
            SCRATCH_ID_CONTACT_PAIRS_PREV,
            SCRATCH_ID_TREE_TOP,
            SCRATCH_ID_TREE_BOTTOM
        ),
    "the tree's assembly writes ContactPairs beside its stream, and one of the cohort's slots is a pair column's"
);

// A `Rows` step: `prev_row` beside the records and the inverse map.
const _: () = assert!(
    !shares_stagger_slot(SCRATCH_ID_ROW_PREV, SCRATCH_ID_TREE_TOP, SCRATCH_ID_TREE_BOTTOM),
    "the tree's verify reads prev_row beside its records on a Rows step, and one of the cohort's slots is ROW_PREV's"
);

// The hint (C5): the awake mask beside the records.
const _: () = assert!(
    !shares_stagger_slot(SCRATCH_ID_TOUCHED_AWAKE, SCRATCH_ID_TREE_TOP, SCRATCH_ID_TREE_BOTTOM),
    "the tree's verify reads the awake mask beside its records on a hint step, and one of the cohort's slots is TOUCHED_AWAKE's"
);

/// The [`ComponentId`] for tree column `k` (one of the `TREE_*` indices), descending from
/// [`SCRATCH_ID_TREE_TOP`].
#[inline]
pub(crate) fn tree_column_id(k: usize) -> ComponentId {
    debug_assert!(k < TREE_COLUMN_COUNT, "tree column index out of cohort");
    ComponentId::new(SCRATCH_ID_TREE_TOP - k)
}

/// Registers the element layout of every [`BroadphaseTree`] column, idempotently: three
/// `Node8` columns, four `u64`, one `Item`, two `RowRec` and one `u32`. The two C5 ids are
/// registered with the type they will hold, so the cohort's shape is fixed now.
///
/// [`BroadphaseTree`]: crate::broadphase_tree::BroadphaseTree
pub(crate) fn register_tree_column_layouts() {
    for k in [TREE_ACTIVE, TREE_STATICS, TREE_SLEEPERS] {
        register_layout::<Node8>(tree_column_id(k).get());
    }
    for k in [TREE_SORT_A, TREE_SORT_B, TREE_SS, TREE_SL] {
        register_layout::<u64>(tree_column_id(k).get());
    }
    register_layout::<Item>(tree_column_id(TREE_ITEMS).get());
    register_layout::<RowRec>(tree_column_id(TREE_REC0).get());
    register_layout::<RowRec>(tree_column_id(TREE_REC1).get());
    register_layout::<u32>(tree_column_id(TREE_AUX).get());
}

/// The lowest id the physics scratch region may occupy.
///
/// The region grows DOWNWARD from the top of the id space while production
/// `#[derive(Component)]` ids climb UPWARD from `0`, and this is the line that
/// keeps the two apart. It is a MARGIN, not a proof — the proof is
/// `register_layout`'s collision check, which panics loudly the moment a
/// production type lands on a reserved slot. What the margin buys is that the
/// panic stays unreachable in practice.
///
/// # Why 128 ids, measured 2026-09-09
///
/// The previous value was 64, written when the region was 34 ids wide, and the
/// only number behind it was a guess in prose: "500+ distinct component types,
/// far beyond any realistic world". Counting `#[derive(Component)]` sites under
/// `src/` across the largest binary's dependency closure (`boyko_demo`: ecs 45 +
/// ui 36 + render 21 + physics 17 + scene 14 + demo 8 + input 1) gives **142** —
/// and that is an OVER-count, because it includes `#[cfg(test)]` types no
/// shipping process ever mints.
///
/// Finishing Stage 4 needs roughly 90 scratch ids, which does not fit under 64.
/// At 128 the scratch side keeps ~38 ids of headroom and the production side gets
/// 384 against a measured ~142 — a 2.7x margin on the side that actually grows,
/// and the census above is the thing to re-run before moving this number again.
const SCRATCH_REGION_MIN_ID: usize = MAX_COMPONENTS - 128;

// The broadphase-tree cohort is the region's lowest edge today. The floor is asserted
// against the LOWEST cohort rather than against whichever one happened to be last
// when this was written — add a cohort below and move this assert with it.
const _: () = assert!(
    SCRATCH_ID_TREE_BOTTOM >= SCRATCH_REGION_MIN_ID,
    "the physics scratch region has grown below SCRATCH_REGION_MIN_ID; production \
     ids climb from 0 and the reserved region is no longer comfortably out of \
     their reach. Re-run the census in that constant's docs before lowering it"
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
/// `(BodyIndex, BodyIndex)` pair column (`candidates`, k = 10), then the two
/// `ContactPairs` lists. The registry's collision check keys on `(slot, TypeId)`,
/// so registering each under its REAL type means a wrong-typed reuse of a slot
/// panics loudly rather than aliasing.
pub(crate) fn register_broadphase_column_layouts() {
    for k in 0..BROADPHASE_COLUMN_COUNT {
        match k {
            9 => register_layout::<f32>(broadphase_column_id(k).get()),
            10 => register_layout::<(BodyIndex, BodyIndex)>(broadphase_column_id(k).get()),
            _ => register_layout::<u32>(broadphase_column_id(k).get()),
        }
    }
    register_layout::<(BodyIndex, BodyIndex)>(SCRATCH_ID_CONTACT_PAIRS);
    register_layout::<(BodyIndex, BodyIndex)>(SCRATCH_ID_CONTACT_PAIRS_PREV);
}

/// The [`ComponentId`] for [`ContactPairs`](crate::resources::ContactPairs)'s
/// pair list.
#[inline]
pub(crate) fn contact_pairs_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_CONTACT_PAIRS)
}

/// The [`ComponentId`] for [`ContactPairs`](crate::resources::ContactPairs)'s
/// previous step's pair list (L9 C2).
#[inline]
pub(crate) fn contact_pairs_prev_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_CONTACT_PAIRS_PREV)
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
    register_row_identity_layouts();
}

/// Registers the [`Layout`](std::alloc::Layout) of every contact column's element
/// type under its band id (audit Stage P — P2), in `CohortColumns` field order.
///
/// The 10 columns and their element types (field order, descending from
/// [`SCRATCH_ID_CONTACT_BAND_TOP`]; L11 C2):
/// * `heads` — `CohortHead` (320 B, one per cohort);
/// * `blocks` — `RankBlock` (320 B, one per cohort rank);
/// * `cold` — `CohortCold` (64 B, one per cohort);
/// * `rank_cold` — `[f32; 8]` (the `vn0` row of each cohort rank);
/// * `plan` — `WarmRun` (L11 C1: each manifold's warm-source run);
/// * `tags` — `ManifoldTag` (each manifold's point count and frozen flag);
/// * `color_offsets`, `group_start`, `color_group_start`, `color_cohort_start` —
///   4 × `u32`.
///
/// Idempotent + process-global (each `register_layout` is write-once): re-entry
/// from another world / solver costs one branch per id after the first.
#[inline]
fn register_contact_column_layouts() {
    // Field order MUST match `CohortColumns` so `contact_column_id(k)` lines up
    // with the `k`-th declared column.
    let mut k = SCRATCH_ID_CONTACT_BAND_TOP;
    register_layout::<CohortHead>(k); // heads
    k -= 1;
    register_layout::<RankBlock>(k); // blocks
    k -= 1;
    register_layout::<CohortCold>(k); // cold
    k -= 1;
    register_layout::<[f32; 8]>(k); // rank_cold
    k -= 1;
    register_layout::<WarmRun>(k); // plan
    k -= 1;
    register_layout::<ManifoldTag>(k); // tags
    k -= 1;
    register_layout::<u32>(k); // color_offsets
    k -= 1;
    register_layout::<u32>(k); // group_start
    k -= 1;
    register_layout::<u32>(k); // color_group_start
    k -= 1;
    register_layout::<u32>(k); // color_cohort_start
    debug_assert_eq!(
        k, SCRATCH_ID_CONTACT_BAND_BOTTOM,
        "contact column band must end exactly at the reserved bottom id"
    );
}

/// Registers the element layout of every solver-tail column, idempotently.
///
/// One `f32` (`vn_initial`), two `BitSet256` chunk columns, the warm-store run
/// (two `WarmEntry` tables, two `WarmRecord` columns, two `u64` key columns and the
/// `(u64, u32)` cold index — the slot table at [`warm_table_id`]) and the three
/// constraint / freeze columns. Every same-typed pair is registered under DIFFERENT
/// ids on purpose — see [`SCRATCH_ID_TOUCHED_AWAKE`] for the failure a shared id
/// would produce.
fn register_solver_tail_layouts() {
    register_layout::<f32>(SCRATCH_ID_VN_INITIAL);
    register_layout::<BitSet256>(SCRATCH_ID_TOUCHED_SOLVER);
    register_layout::<BitSet256>(SCRATCH_ID_TOUCHED_AWAKE);
    register_layout::<WarmEntry>(warm_table_id(0).get());
    register_layout::<WarmEntry>(warm_table_id(1).get());
    register_layout::<WarmRecord>(warm_table_id(2).get());
    register_layout::<WarmRecord>(warm_table_id(3).get());
    register_layout::<u64>(warm_table_id(4).get());
    register_layout::<u64>(warm_table_id(5).get());
    register_layout::<(u64, u32)>(warm_table_id(6).get());
    register_layout::<ManifoldConstraint>(SCRATCH_ID_SERIAL_MANIFOLD_CONSTRAINTS);
    register_layout::<PointConstraint>(SCRATCH_ID_SERIAL_POINT_CONSTRAINTS);
    register_layout::<(u32, BodyState)>(SCRATCH_ID_COLORED_FROZEN_ROWS);
}

// —— The NARROWPHASE cohort (audit Stage 4; L5; L9) ———————————————————
//
// `Manifolds`' buffers are swept together by the narrowphase: one pass over the
// candidate pairs writes a manifold into either `manifolds` or `sensor_overlaps`
// and probes `box_axis_cache` for the box-box reference axis. The parallel
// narrowphase (L5, `narrowphase/dispatch.rs`) adds two columns to the same
// cohort: its chunk loop writes the staging column and the per-pair axis commit
// beside `pairs` and the axis slots, and its compaction reads the staging column
// beside `manifolds` and `sensor_overlaps`. Contact reuse (L9 C1,
// `narrowphase/reuse.rs`) adds the per-row orientation frames, read at a box
// pair's two rows in the same pair loop, and its pair carry (L9 C2,
// `narrowphase/carry.rs`) adds the two per-pair tag columns — this step's, written
// at pair `k`, and the previous step's, read at the joined slot beside
// `pairs_prev` — and the jumper bitset the join probes at a pair's two rows. One
// cohort, so their ids must be pairwise distinct mod `POOL_STAGGER_LINES`; they MAY
// reuse the solver / graph / broadphase slots, since those loops never run at the
// same index at the same moment.

/// Number of `ScratchColumn`s backing [`Manifolds`](crate::resources::Manifolds):
/// the solver buffer, the sensor-overlap buffer, the box-axis cache slots, the
/// parallel narrowphase's staging column, its per-pair axis commit, the per-row
/// orientation frames (L9 C1), the two per-pair tag columns and the jumper bitset
/// (L9 C2).
pub(crate) const NARROWPHASE_COLUMN_COUNT: usize = 9;

/// Top of the narrowphase cohort — one id below the broadphase cohort's bottom.
pub(crate) const SCRATCH_ID_NARROWPHASE_TOP: usize = BROADPHASE_COHORT_BOTTOM - 1;

/// Bottom of the narrowphase cohort (inclusive).
pub(crate) const SCRATCH_ID_NARROWPHASE_BOTTOM: usize =
    SCRATCH_ID_NARROWPHASE_TOP - (NARROWPHASE_COLUMN_COUNT - 1);

const _: () = assert!(
    NARROWPHASE_COLUMN_COUNT <= POOL_STAGGER_LINES,
    "the narrowphase cohort is wider than one stagger period"
);

const _: () = assert!(
    SCRATCH_ID_NARROWPHASE_TOP < BROADPHASE_COHORT_BOTTOM,
    "the narrowphase cohort overlaps the broadphase cohort"
);

// —— The one buffer that belongs to TWO cohorts —————————————————————
//
// `ContactPairs::pairs` is swept by BOTH phases: the broadphase fills it beside
// the grid's columns, and the narrowphase then reads element `i` of it in the
// same loop that writes the manifold columns. So the distinctness that has to
// hold is over the UNION of the two cohorts, not either one alone — a property
// no per-cohort width check states.
//
// It holds because the two runs are ADJACENT with no gap and their combined width
// is under one stagger period. Both halves of that are asserted here rather than
// left to be re-derived by whoever next moves a cohort boundary.
const _: () = assert!(
    SCRATCH_ID_NARROWPHASE_TOP + 1 == BROADPHASE_COHORT_BOTTOM,
    "the broadphase and narrowphase runs are no longer adjacent, so the union ContactPairs::pairs is swept in is not a contiguous run and its residues are no longer provably distinct"
);

/// Width of the broadphase + narrowphase union, the run `ContactPairs::pairs` is
/// actually swept in.
const BROADPHASE_NARROWPHASE_UNION_WIDTH: usize =
    SCRATCH_ID_BROADPHASE_TOP - SCRATCH_ID_NARROWPHASE_BOTTOM + 1;

const _: () = assert!(
    BROADPHASE_NARROWPHASE_UNION_WIDTH <= POOL_STAGGER_LINES,
    "the broadphase + narrowphase union is wider than one stagger period, and ContactPairs::pairs is swept in both halves of it"
);

/// The [`ComponentId`] for narrowphase column `k` (`0` = `manifolds`,
/// `1` = `sensor_overlaps`, `2` = the box-axis cache slots, `3` = the parallel
/// narrowphase's staging column, `4` = its per-pair axis commit, `5` = the per-row
/// orientation frames, `6` and `7` = the two per-pair tag columns, `8` = the jumper
/// bitset).
#[inline]
pub(crate) fn narrowphase_column_id(k: usize) -> ComponentId {
    debug_assert!(k < NARROWPHASE_COLUMN_COUNT, "narrowphase column index out of cohort");
    ComponentId::new(SCRATCH_ID_NARROWPHASE_TOP - k)
}

/// Registers the element layout of every [`Manifolds`] column, idempotently.
///
/// Three `Manifold` columns under DIFFERENT ids — the solver buffer, the
/// sensor-overlap buffer and the parallel narrowphase's staging column are swept
/// in the same passes, so a shared id would put element `i` of two of them in one
/// cache set — plus the `AxisEntry` slot table, the `u8` axis commit, the
/// `RowFrame` column, the two `PairTag` columns (under different ids too: they swap
/// roles every step and one is read while the other is written) and the `u64`
/// jumper bitset.
pub(crate) fn register_narrowphase_column_layouts() {
    register_layout::<Manifold>(narrowphase_column_id(0).get());
    register_layout::<Manifold>(narrowphase_column_id(1).get());
    register_layout::<AxisEntry>(narrowphase_column_id(2).get());
    register_layout::<Manifold>(narrowphase_column_id(3).get());
    register_layout::<u8>(narrowphase_column_id(4).get());
    register_layout::<RowFrame>(narrowphase_column_id(5).get());
    register_layout::<PairTag>(narrowphase_column_id(6).get());
    register_layout::<PairTag>(narrowphase_column_id(7).get());
    register_layout::<u64>(narrowphase_column_id(8).get());
    register_row_identity_layouts();
}

/// The [`ComponentId`] for `Manifolds::manifolds`.
#[inline]
pub(crate) fn manifolds_id() -> ComponentId {
    narrowphase_column_id(0)
}

/// The [`ComponentId`] for `Manifolds::sensor_overlaps`.
#[inline]
pub(crate) fn sensor_overlaps_id() -> ComponentId {
    narrowphase_column_id(1)
}

/// The [`ComponentId`] for the [`BoxAxisCache`](crate::narrowphase::axis_cache::BoxAxisCache)
/// slot table.
#[inline]
pub(crate) fn box_axis_cache_id() -> ComponentId {
    narrowphase_column_id(2)
}

/// The [`ComponentId`] for `Manifolds::np_stage`, the parallel narrowphase's staging
/// column (L5 D1).
#[inline]
pub(crate) fn np_stage_id() -> ComponentId {
    narrowphase_column_id(3)
}

/// The [`ComponentId`] for the
/// [`BoxAxisCache`](crate::narrowphase::axis_cache::BoxAxisCache)'s per-pair axis
/// commit (L5 D3).
#[inline]
pub(crate) fn axis_commit_id() -> ComponentId {
    narrowphase_column_id(4)
}

/// The [`ComponentId`] for `Manifolds::row_frames`, the per-row orientation frames
/// (L9 D2).
#[inline]
pub(crate) fn row_frames_id() -> ComponentId {
    narrowphase_column_id(5)
}

/// The [`ComponentId`] for `Manifolds::pair_tag`, one of the pair carry's two per-pair
/// tag columns (L9 D9).
#[inline]
pub(crate) fn pair_tag_id() -> ComponentId {
    narrowphase_column_id(6)
}

/// The [`ComponentId`] for `Manifolds::pair_tag_prev`, the other per-pair tag column
/// (L9 D9).
#[inline]
pub(crate) fn pair_tag_prev_id() -> ComponentId {
    narrowphase_column_id(7)
}

/// The [`ComponentId`] for `Manifolds::jumper_bits`, the pair carry's bitset of the rows
/// stage 2 resolved (L9 D9).
#[inline]
pub(crate) fn jumper_bits_id() -> ComponentId {
    narrowphase_column_id(8)
}

/// The [`ComponentId`] wrapper for [`SCRATCH_ID_SERIAL_MANIFOLD_CONSTRAINTS`].
#[inline]
pub(crate) fn serial_manifold_constraints_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_SERIAL_MANIFOLD_CONSTRAINTS)
}

/// The [`ComponentId`] wrapper for [`SCRATCH_ID_SERIAL_POINT_CONSTRAINTS`].
#[inline]
pub(crate) fn serial_point_constraints_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_SERIAL_POINT_CONSTRAINTS)
}

/// The [`ComponentId`] wrapper for [`SCRATCH_ID_COLORED_FROZEN_ROWS`].
#[inline]
pub(crate) fn colored_frozen_rows_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_COLORED_FROZEN_ROWS)
}

/// The [`ComponentId`] wrapper for [`SCRATCH_ID_ROW_ENTITY`].
#[inline]
pub(crate) fn row_entity_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_ROW_ENTITY)
}

/// The [`ComponentId`] wrapper for [`SCRATCH_ID_ROW_ENTITY_PREV`].
#[inline]
pub(crate) fn row_entity_prev_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_ROW_ENTITY_PREV)
}

/// The [`ComponentId`] wrapper for [`SCRATCH_ID_ROW_ADDED`].
#[inline]
pub(crate) fn row_added_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_ROW_ADDED)
}

/// The [`ComponentId`] wrapper for [`SCRATCH_ID_ROW_PREV`].
#[inline]
pub(crate) fn row_prev_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_ROW_PREV)
}

/// The [`ComponentId`] wrapper for [`SCRATCH_ID_ROW_REMAP_SORT`].
#[inline]
pub(crate) fn row_remap_sort_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_ROW_REMAP_SORT)
}

/// The [`ComponentId`] wrapper for [`SCRATCH_ID_SLEEP_LATCH_PREV`].
#[inline]
pub(crate) fn sleep_latch_prev_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_SLEEP_LATCH_PREV)
}

/// The [`ComponentId`] wrapper for [`SCRATCH_ID_SLEEP_ISLAND_KEY`].
#[inline]
pub(crate) fn sleep_island_key_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_SLEEP_ISLAND_KEY)
}

/// The [`ComponentId`] wrapper for [`SCRATCH_ID_AXIS_REMAP`].
#[inline]
pub(crate) fn axis_remap_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_AXIS_REMAP)
}

/// The [`ComponentId`] wrapper for [`SCRATCH_ID_ROW_STAGE2`].
#[inline]
pub(crate) fn row_stage2_id() -> ComponentId {
    ComponentId::new(SCRATCH_ID_ROW_STAGE2)
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
        ids.extend((0..WARM_TABLE_COLUMN_COUNT).map(|k| warm_table_id(k).get()));
        ids.extend([
            SCRATCH_ID_SERIAL_MANIFOLD_CONSTRAINTS,
            SCRATCH_ID_SERIAL_POINT_CONSTRAINTS,
            SCRATCH_ID_COLORED_FROZEN_ROWS,
        ]);
        ids
    }

    fn graph_cohort_ids() -> Vec<usize> {
        (0..GRAPH_COLUMN_COUNT).map(|k| graph_column_id(k).get()).collect()
    }

    fn broadphase_cohort_ids() -> Vec<usize> {
        let mut ids: Vec<usize> =
            (0..BROADPHASE_COLUMN_COUNT).map(|k| broadphase_column_id(k).get()).collect();
        ids.push(SCRATCH_ID_CONTACT_PAIRS);
        ids.push(SCRATCH_ID_CONTACT_PAIRS_PREV);
        ids
    }

    fn narrowphase_cohort_ids() -> Vec<usize> {
        (0..NARROWPHASE_COLUMN_COUNT).map(|k| narrowphase_column_id(k).get()).collect()
    }

    fn soft_coupling_cohort_ids() -> Vec<usize> {
        (0..SOFT_COUPLING_COLUMN_COUNT).map(|k| soft_coupling_column_id(k).get()).collect()
    }

    fn soft_graph_cohort_ids() -> Vec<usize> {
        let mut ids = Vec::with_capacity(SOFT_GRAPH_COLUMN_COUNT);
        for instance in 0..SOFT_GRAPH_INSTANCES {
            for k in 0..SOFT_GRAPH_COLUMNS_PER_INSTANCE {
                ids.push(soft_graph_column_id(instance, k).get());
            }
        }
        ids.push(soft_pair_list_id().get());
        ids
    }

    /// `ContactPairs::pairs` is written by the broadphase and read by the
    /// narrowphase, so the distinctness it needs spans BOTH cohorts. The const
    /// asserts prove that from adjacency plus a width; this proves it over the ids
    /// actually handed out, which is the statement that survives a column being
    /// allocated outside its run.
    #[test]
    fn the_broadphase_narrowphase_union_has_distinct_slots() {
        let mut ids = broadphase_cohort_ids();
        ids.extend(narrowphase_cohort_ids());
        assert_cohort_slots_distinct(
            "broadphase + narrowphase union",
            &ids,
            BROADPHASE_COHORT_WIDTH + NARROWPHASE_COLUMN_COUNT,
        );
    }

    #[test]
    fn scratch_band_stagger_slots_are_distinct() {
        assert_cohort_slots_distinct("solver cohort", &solver_cohort_ids(), SOLVER_COHORT_WIDTH);
        assert_cohort_slots_distinct("graph cohort", &graph_cohort_ids(), GRAPH_COLUMN_COUNT);
        assert_cohort_slots_distinct(
            "broadphase cohort",
            &broadphase_cohort_ids(),
            BROADPHASE_COHORT_WIDTH,
        );
        assert_cohort_slots_distinct(
            "narrowphase cohort",
            &narrowphase_cohort_ids(),
            NARROWPHASE_COLUMN_COUNT,
        );
        assert_cohort_slots_distinct(
            "soft-coupling cohort",
            &soft_coupling_cohort_ids(),
            SOFT_COUPLING_COLUMN_COUNT,
        );
        assert_cohort_slots_distinct(
            "soft-graph cohort",
            &soft_graph_cohort_ids(),
            SOFT_GRAPH_COLUMN_COUNT,
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
                BROADPHASE_COHORT_BOTTOM,
            ),
            (
                "narrowphase cohort",
                narrowphase_cohort_ids(),
                SCRATCH_ID_NARROWPHASE_TOP,
                SCRATCH_ID_NARROWPHASE_BOTTOM,
            ),
            (
                "soft-coupling cohort",
                soft_coupling_cohort_ids(),
                SCRATCH_ID_SOFT_COUPLING_TOP,
                SCRATCH_ID_SOFT_COUPLING_BOTTOM,
            ),
            (
                "soft-graph cohort",
                soft_graph_cohort_ids(),
                SCRATCH_ID_SOFT_GRAPH_TOP,
                SCRATCH_ID_SOFT_GRAPH_BOTTOM,
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
            ("narrowphase", narrowphase_cohort_ids()),
            ("soft-coupling", soft_coupling_cohort_ids()),
            ("soft-graph", soft_graph_cohort_ids()),
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

    // —— The row-identity cohort (defect A interim fix, Decision 4; two runs) ————————

    /// The row-identity cohort's eight ids: the upper run, then the lower run.
    fn row_identity_cohort_ids() -> Vec<usize> {
        vec![
            SCRATCH_ID_ROW_ENTITY,
            SCRATCH_ID_ROW_ENTITY_PREV,
            SCRATCH_ID_ROW_ADDED,
            SCRATCH_ID_ROW_PREV,
            SCRATCH_ID_ROW_REMAP_SORT,
            SCRATCH_ID_SLEEP_LATCH_PREV,
            SCRATCH_ID_AXIS_REMAP,
            SCRATCH_ID_ROW_STAGE2,
        ]
    }

    /// The cohort is TWO contiguous runs: three ids directly below the soft-graph cohort,
    /// then a gap, then five ids whose top is the highest id clear of the solver cohort's
    /// slots. A single contiguous run would put `prev_row` on a contact column's slot. The
    /// gap holds the sleep key, checked by `the_sleep_key_loops_have_distinct_slots`.
    #[test]
    fn row_identity_cohort_is_two_contiguous_runs() {
        let ids = row_identity_cohort_ids();
        let (upper, lower) = ids.split_at(ROW_IDENTITY_UPPER_COUNT);
        for (name, run, top, bottom, count) in [
            ("upper run", upper, SCRATCH_ID_ROW_IDENTITY_TOP, SCRATCH_ID_ROW_ADDED, ROW_IDENTITY_UPPER_COUNT),
            ("lower run", lower, SCRATCH_ID_ROW_PREV, SCRATCH_ID_ROW_IDENTITY_BOTTOM, ROW_IDENTITY_LOWER_COUNT),
        ] {
            let mut run = run.to_vec();
            run.sort_unstable();
            assert_eq!(run.len(), count, "row-identity {name}: declared count");
            assert_eq!((run[0], run[run.len() - 1]), (bottom, top), "row-identity {name}: bounds");
            for w in run.windows(2) {
                assert_eq!(w[1], w[0] + 1, "row-identity {name}: hole or duplicate between {} and {}", w[0], w[1]);
            }
        }
        assert_eq!(
            SCRATCH_ID_ROW_IDENTITY_TOP + 1,
            SCRATCH_ID_SOFT_GRAPH_BOTTOM,
            "the upper run sits directly below the soft-graph cohort"
        );
        // Constant, so checked at compile time: a test build fails if the runs ever merge.
        const {
            assert!(
                SCRATCH_ID_ROW_PREV + 1 < SCRATCH_ID_ROW_ADDED,
                "the two row-identity runs must be separated by a gap"
            );
        }
        assert!(
            ids.iter().all(|&id| id >= SCRATCH_REGION_MIN_ID),
            "every row-identity id stays above the scratch region floor {SCRATCH_REGION_MIN_ID}"
        );
    }

    /// The eight row-identity ids and the sleep key in their gap each get their own slot.
    #[test]
    fn row_identity_cohort_has_distinct_slots() {
        let mut ids = row_identity_cohort_ids();
        ids.push(SCRATCH_ID_SLEEP_ISLAND_KEY);
        assert_cohort_slots_distinct(
            "row-identity cohort + sleep key",
            &ids,
            ROW_IDENTITY_UPPER_COUNT + ROW_IDENTITY_LOWER_COUNT + 1,
        );
    }

    /// U9 (defect A4): the sleep key's two loops. `IslandSleep::begin_step` sweeps it beside
    /// the graph cohort's `island_of` and `island_manifold_start`, and `permute_latch`
    /// sweeps it beside `prev_row` and the latch carry. The key also sits strictly between
    /// the row-identity runs, checked here on runtime values (the twin of the const
    /// ordering assert).
    #[test]
    fn the_sleep_key_loops_have_distinct_slots() {
        let mut begin_step = graph_cohort_ids();
        begin_step.push(SCRATCH_ID_SLEEP_ISLAND_KEY);
        assert_cohort_slots_distinct(
            "begin_step first loop: graph cohort + sleep key",
            &begin_step,
            GRAPH_COLUMN_COUNT + 1,
        );
        assert_cohort_slots_distinct(
            "permute_latch: prev_row + latch carry + sleep key",
            &[
                SCRATCH_ID_ROW_PREV,
                SCRATCH_ID_SLEEP_LATCH_PREV,
                SCRATCH_ID_SLEEP_ISLAND_KEY,
            ],
            3,
        );
        // Runtime values (`black_box`), so this is not a restatement of the const assert.
        let lower_top = std::hint::black_box(SCRATCH_ID_ROW_PREV);
        let key = std::hint::black_box(SCRATCH_ID_SLEEP_ISLAND_KEY);
        let upper_bottom = std::hint::black_box(SCRATCH_ID_ROW_ADDED);
        assert!(
            lower_top < key && key < upper_bottom,
            "the sleep key {key} must lie strictly between the row-identity runs (lower run top \
             {lower_top}, upper run bottom {upper_bottom})"
        );
    }

    /// The gather pushes the `BodyState` snapshot, the row id and the added row at one
    /// index, and the two id columns swap roles every gather.
    #[test]
    fn the_gather_loop_columns_have_distinct_slots() {
        assert_cohort_slots_distinct(
            "gather loop (BodyState + both row-id columns + added rows)",
            &[
                SCRATCH_ID_BODY_STATE,
                SCRATCH_ID_ROW_ENTITY,
                SCRATCH_ID_ROW_ENTITY_PREV,
                SCRATCH_ID_ROW_ADDED,
            ],
            4,
        );
    }

    /// `prev_row` is read inside both solvers' constraint builds, beside every solver-cohort
    /// column.
    #[test]
    fn the_constraint_builds_with_prev_row_have_distinct_slots() {
        let mut ids = solver_cohort_ids();
        ids.push(SCRATCH_ID_ROW_PREV);
        assert_cohort_slots_distinct("solver cohort + prev_row", &ids, SOLVER_COHORT_WIDTH + 1);
    }

    /// The axis carry is written and read beside the pair list, the manifolds and the axis
    /// slots.
    #[test]
    fn the_narrowphase_union_with_the_axis_carry_has_distinct_slots() {
        let mut ids = broadphase_cohort_ids();
        ids.extend(narrowphase_cohort_ids());
        ids.push(SCRATCH_ID_AXIS_REMAP);
        assert_cohort_slots_distinct(
            "broadphase + narrowphase union + axis carry",
            &ids,
            BROADPHASE_COHORT_WIDTH + NARROWPHASE_COLUMN_COUNT + 1,
        );
    }

    #[test]
    fn row_identity_cohort_shares_no_id_with_another_cohort() {
        let mut row = row_identity_cohort_ids();
        row.push(SCRATCH_ID_SLEEP_ISLAND_KEY);
        for (name, ids) in [
            ("solver", solver_cohort_ids()),
            ("graph", graph_cohort_ids()),
            ("broadphase", broadphase_cohort_ids()),
            ("narrowphase", narrowphase_cohort_ids()),
            ("soft-coupling", soft_coupling_cohort_ids()),
            ("soft-graph", soft_graph_cohort_ids()),
        ] {
            for id in &row {
                assert!(!ids.contains(id), "row-identity id {id} also belongs to the {name} cohort");
            }
        }
    }

    /// Each row-identity id is registered with its own element layout: a wrong-typed
    /// registration would size the column for the wrong element.
    #[test]
    fn row_identity_layouts_register_each_element_size() {
        register_row_identity_layouts();
        for (id, size) in [
            (SCRATCH_ID_ROW_ENTITY, size_of::<RowKey>()),
            (SCRATCH_ID_ROW_ENTITY_PREV, size_of::<RowKey>()),
            (SCRATCH_ID_ROW_ADDED, size_of::<u32>()),
            (SCRATCH_ID_ROW_PREV, size_of::<u32>()),
            (SCRATCH_ID_ROW_REMAP_SORT, size_of::<(RowKey, u32)>()),
            // The widened (defect A4) carry element: latch + island contact key.
            (SCRATCH_ID_SLEEP_LATCH_PREV, 8),
            (SCRATCH_ID_AXIS_REMAP, size_of::<u8>()),
            (SCRATCH_ID_ROW_STAGE2, size_of::<u32>()),
            (SCRATCH_ID_SLEEP_ISLAND_KEY, 4),
        ] {
            assert_eq!(
                boyko_ecs::ecs::core::component::component_registry::get_component_size(id),
                Some(size),
                "row-identity id {id}: registered element size"
            );
        }
    }
}
