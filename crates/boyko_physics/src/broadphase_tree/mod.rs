//! The tree broadphase ([`BroadphaseKind::Tree`]): AllPairs' exact pair set from a packed 8-wide
//! BVH over the moving rows and a persistent static set, serial, with no heap allocation per
//! step (`docs/physics/perf-campaign/levers/broadphase/04-DESIGN-REV2.md`, rulings W1 and W2 in
//! `levers/00-RULINGS.md`).
//!
//! # What it computes
//!
//! Exactly the pair set of the shipped all-pairs loop, in `(min, max)` order: the predicate is the
//! same expression on the same bits ([`sphere_bound_feasible`], evaluated 8-wide in
//! `kernel.rs`), the cull is conservative, every pair is emitted by exactly one owner, and the
//! assembly places pairs by integer counts alone. So `ContactPairs` is a function of the exact
//! set — not of the tree's shape, the worker count, the admission history, or the kernel arm —
//! and the pose bytes downstream are AllPairs' pose bytes.
//!
//! # Row kinds (D8)
//!
//! Every row is classed from its bits `(x, y, z, r)`, `r = body_bounding_radius`:
//! * **Excluded** — any NaN, or a non-finite position with `|r| ≤ 2^60`. Against any non-Wide
//!   row the predicate is false (a finite bound against an infinite or NaN distance), so the
//!   row pairs with Wide rows only.
//! * **Wide** — `r` non-finite, `|r| > 2^60`, or `‖p‖∞ + |r| > 2^60`. An exact O(N) loop over
//!   every other row, Excluded ones included, skipping the Wide rows below it (they own the
//!   pair).
//! * **Normal** — everything else. Every intermediate of the kernel stays finite.
//!
//! # Sets (D3)
//!
//! A Normal row is in one of **Q** (queried; the active tree, rebuilt every step), **S** (the
//! static set: a tree and a sorted list `SS` of its internal pairs, both persistent) or **Z**
//! (the sleeper set: a tree and a sorted list `SL` of the pairs with one Z endpoint and the other
//! in S ∪ Z, both persistent). One verify pass per step locates every row's previous record — in
//! place on `Identity`, through `prev_row` on `Rows`, in place after a `Reset` — and decides:
//! * *carried*: a member whose bits, kind and class are unchanged keeps its leaf;
//! * *evicted*: a member that moved, reshaped, changed class or kind, or was located twice
//!   (the consumed mark makes the locator injective) — its leaf is killed and its pairs filtered;
//! * *vanished*: a member no row located (despawned, or a tail row past `N`) — found by the
//!   count, killed by a leaf scan;
//! * *pending*: a non-member that fits a set — a static that is still (bits equal to its
//!   record), or a row the sleep hint offers to Z — accrues that set's rent.
//!
//! Correctness rests on the bit compare, the injective locator and the filtered lists alone
//! (Lemma D3.1); the class check is membership hygiene. A row move is a **translation** of the
//! list entries and leaf rows through `prev_row`'s inverse, with the patch rule of ruling W1:
//! an entry stays in place only if both its endpoints belong to the maximum-weight monotone
//! subsequence of the carried rows (the "jumpers" are the rest) and it is greater than the last
//! kept entry; the diverted entries are sorted and merged back. A new member waits as pending
//! and is admitted by the rent rule (D3.5, a deterministic ski-rental bound): the set's tree is
//! rebuilt over members and pending rows, only the pending rows are queried — against the set's
//! own tree and the other set's — and their additions are merged into the lists. S is admitted
//! before Z, so a pair of a new static and a new sleeper is found once, by Z's admission.
//! Compaction rebuilds a tree when its dead lanes reach `max(live, 64)`.
//!
//! # The sleeper set (L10 C3c: design C5 as amended by L10's T1–T4 and T6)
//!
//! Z serves L10's frozen-pair skip (`sleep_sets.rs`): a held island's pairs are neither queried
//! nor emitted into the stream. The verify reads a [`SleepHint`]:
//! * **T1** — a row is offered to Z iff its held record survives the step's prologue (L10's
//!   `SLEEPER` flag: `PRE_HELD` less the prologue's restores). The hint is rebuilt from the
//!   current step, so no stale mask exists and `IslandSleep::mask_rows` is never built.
//! * **T2** — a row may be a member of S or Z only if it rests and is no sensor (L10's `RESTING`,
//!   not `SENSOR`). So every `SL` pair has two resting, non-sensor endpoints, at least one held
//!   (**Invariant V**, debug-asserted by the colored broadphase): a static whose rotation or shape
//!   changed with `(x, y, z, r)` bit-equal leaves S, and its pairs with Z are queried again. L10
//!   hands the tree its hint only on a step with at least one sleeper; on any other step the
//!   verify reads [`NoHint`], so Z dissolves and S keeps its C1 class (a flush step, where no
//!   row rests, does not dissolve S).
//! * **T3** — `SL` IS [`ContactPairs::withheld`]: strictly sorted, never merged into the stream.
//!   The logical pair view is the stream ⊎ `withheld` ([`ContactPairs::pairs`]), whose length —
//!   `P_logical` — sizes the narrowphase's hysteresis table and feeds `phys_bp_pairs`.
//! * **T4** — [`release`](BroadphaseTree::release): the rows whose records L10's epilogue
//!   restores after the verify (its D2 scan and D3) leave Z in the same step: their leaves are
//!   killed and their `withheld` pairs merged into the stream, which the narrowphase then
//!   collides (or skips, for a still-held partner).
//! * **T6** — `withheld` is non-empty only after a tree-path step: a kind switch or a brute step
//!   dissolves Z and empties it ([`clear_sleepers`](BroadphaseTree::clear_sleepers)); a `Reset`
//!   and a step without the sleep-skip read [`NoHint`], which evicts every Z member.
//!
//! Without the sleep-skip Z is empty, and every step is the static-set step byte for byte: the
//! same stream, the same records, the same leaf-list counts.
//!
//! # A step (N > `brute_max_rows`)
//!
//! | zone | what |
//! |---|---|
//! | `phys_bp_verify` | the locator, the verify pass, maintenance (leaf scan, list pass, compaction, admission) — unconditionally, once per step; the cursor stamp follows the assembly, outside the zones |
//! | `phys_bp_build` | the active tree over Q |
//! | `phys_bp_query` | each Q row, in Morton order, against the active tree (`row > query`), the static tree and the sleeper tree, by the selected [`QueryKernel`](crate::broadphase_tree::QueryKernel); then each Wide row's loop. Every row's partners form one segment `[< row \| > row]` in one stream |
//! | `phys_bp_assemble` | rev counts → bucket starts → a scatter over rows ascending → `resize(P)` → a per-row merge of its forward run, its `SS` run and its bucket (`SL` is not merged: T3) |
//!
//! The counters `phys_bp_queried` (`|Q| + |Wide|`), `phys_bp_members` (`|S| + |Z|`) and
//! `phys_bp_rebuilds` (the step's admissions and compactions) are emitted once per such step.
//! With `N ≤ brute_max_rows` the step is [`all_pairs_into`], the state is untouched and the
//! cursor is not stamped, so the next tree step is a `Reset`.
//!
//! # Query kernels (C3b)
//!
//! Two kernels answer the Q rows, selected by [`BroadphaseTree::set_query_kernel`], and they
//! write the same bytes: the stream and every Q row's `(seg, nrev, nfwd)`.
//!
//! * [`QueryKernel::RowWalk`](crate::broadphase_tree::QueryKernel::RowWalk) (C1's kernel): per Q row, one depth-first walk of each tree, the
//!   active one filtered to `row > query` after the exact test.
//! * [`QueryKernel::LeafList`](crate::broadphase_tree::QueryKernel::LeafList) (the default; design C3b, F1 — a packet traversal over the eight
//!   Morton-adjacent rows of a leaf): per active leaf node `L`, one walk of each tree with `L`'s
//!   box collects the candidate leaves (`PackedBvh8::collect_leaves`); each row of `L` then
//!   tests them eight at a time against its own query box — the static ones first, the active
//!   ones with the max-row cut — and runs the exact test on the kept ones, the active ones with
//!   `row > query` in the mask (`kernel.rs`, "The leaf-list tests"). The pair set is the walk's
//!   by construction: `L`'s box contains each of its rows' query boxes, so every leaf a row's
//!   walk would test is collected; the prefilter is the walk's cull on the same bits; a leaf
//!   whose largest row is at or below the row holds nothing the row emits. The per-row sort
//!   then gives each segment the walk's bytes at the walk's offset, because segments are
//!   written in the same slot order. A segment of up to 16 entries is sorted by a branch-free
//!   network (`kernel::sort_network`, C3b F2), a longer one as the walk sorts it. A collection
//!   over `kernel::LEAF_LIST_CAP` candidates on either tree answers that leaf's rows with the
//!   per-row walk (the fallback).
//!
//! [`TreeDiag`] counts the active leaf nodes each path answered (`leaf_list_leaves`,
//! `fallback_leaves`, `row_walk_leaves`), so a receipt names the kernel that ran.
//!
//! # Storage
//!
//! Every durable buffer is a [`ScratchColumn`] on the `BROADPHASE_TREE` cohort of
//! `scratch_ids.rs` (ids 416..406 on this tree, 399..389 at the design; that file's asserts, not
//! this figure, are the placement's proof). `SL` is the cohort's column too, owned by
//! [`ContactPairs`] as its `withheld` list (T3). Function-local scratch is the traversal stack,
//! the radix histogram and the leaf-list query's three candidate lists (`kernel::CandList`,
//! about 4.4 KB each on the stack, never initialised as a whole). No `Vec`, no pool, no atomics.
//!
//! The non-default `bp-query-counts` feature (C3b, the query-cost investigation) adds the
//! `counts` module and, per tree, one probe of relaxed atomics holding the last query's counts.
//! It and the crate's own test build also count the last leaf-list pass (`LeafListCounts`).
//! Without either, none of it exists: the default build's pass is the uncounted kernel.

use core::mem::MaybeUninit;

use boyko_diag::zone;
use boyko_ecs::ecs::core::component::scratch::{ScratchBuildView, ScratchColumn};
use boyko_macros::Resource;

use crate::manifold::BodyIndex;
use crate::math::Vec3;
use crate::profiling::{
    PHYS_BP_ASSEMBLE, PHYS_BP_BUILD, PHYS_BP_MEMBERS, PHYS_BP_QUERIED, PHYS_BP_QUERY,
    PHYS_BP_REBUILDS, PHYS_BP_VERIFY, counter,
};
use crate::resources::{BodyState, ContactPairs};
use crate::row_identity::{NO_ROW, RemapCursor, RowIdentity, RowRemap};
use crate::scratch_ids::{
    TREE_ACTIVE, TREE_AUX, TREE_ITEMS, TREE_REC0, TREE_REC1, TREE_SLEEPERS, TREE_SORT_A,
    TREE_SORT_B, TREE_SS, TREE_STATICS, register_tree_column_layouts, scratch_reserve_rows,
    tree_column_id,
};
use crate::systems::body_bounding_radius;

use self::bvh::{
    Item, LANES, LEAF_R, LEAF_ROW, LEAF_X, LEAF_Y, LEAF_Z, NO_LANE_ROW, Node8, PackedBvh8,
};
use self::kernel::{
    CandList, LEAF_LIST_CAP, NETWORK_SORT_MAX, QueryBox, leaf_mask, leaf_mask_above, sort_network,
};

pub(crate) mod bvh;
#[cfg(feature = "bp-query-counts")]
pub mod counts;
pub(crate) mod kernel;
#[cfg(test)]
mod tests;

/// Row counts at or below which the Tree runs the brute all-pairs loop instead of the tree.
///
/// PROVISIONAL: the design takes this from the G4 bench (commit C3), which has not run. The
/// value is the arithmetic crossover — at 64 rows the all-pairs loop is 2 016 predicate
/// evaluations, about the cost of building and querying a one-node tree — and the G4 output
/// replaces it.
pub const TREE_BRUTE_MAX_ROWS: u32 = 64;

/// In a previous-row map handed to [`BroadphaseTree::step_translated`]: the row had no previous
/// row (a spawn). The gather's own sentinel.
pub const NO_PREV_ROW: u32 = NO_ROW;

/// The rent rule's build-to-query ratio, as `(numerator, denominator)`: a set admits its pending
/// rows when `rent ≥ pending + (num / den) · (members + pending)`. Starts at `1 / 4`; G4
/// measures it (D3.5).
pub(crate) const ADMIT_BUILD_RATIO: (u64, u64) = (1, 4);

/// Dead lanes at which a set's tree is compacted: `max(live, COMPACT_MIN_DEAD)`.
const COMPACT_MIN_DEAD: u32 = 64;

/// Segment length up to which the per-row sort is an insertion sort.
const INSERTION_SORT_MAX: usize = 32;

/// Entries the leaf-list query grows the stream by, at least, when a leaf's worst case would
/// pass its initialised length.
const STREAM_GROW: usize = 4096;

/// Displacement runs of the carried rows the patch rule tells apart; above it every carried row
/// is a jumper and the list is sorted whole.
const MAX_RUNS: usize = 256;

/// `2^60`: the row-kind limit of D8.
const KIND_LIMIT: f32 = (1u64 << 60) as f32;

/// Row kind: Normal.
const KIND_NORMAL: u32 = 0;
/// Row kind: Wide.
const KIND_WIDE: u32 = 1;
/// Row kind: Excluded.
const KIND_EXCLUDED: u32 = 2;

/// Set: none (a Q, Wide or Excluded row).
const SET_NONE: u32 = 0;
/// Set: the static set S.
const SET_S: u32 = 1;
/// Set: the sleeper set Z (L10 C3c).
const SET_Z: u32 = 2;

/// Index of S in the per-set arrays (members, rent, the step's pending count).
const IX_S: usize = 0;
/// Index of Z in the per-set arrays.
const IX_Z: usize = 1;

const TAG_SLOT_MASK: u32 = 0x00ff_ffff;
const TAG_SET_SHIFT: u32 = 24;
const TAG_KIND_SHIFT: u32 = 26;
const TAG_CONSUMED: u32 = 1 << 29;
/// Pending for S this step.
const TAG_PENDING: u32 = 1 << 30;
/// Pending for Z this step (the hint offered a row that is no member).
const TAG_PENDING_Z: u32 = 1 << 31;

/// Marks a translated old row as a jumper in the inverse map (rows stay below `2^24`).
const JUMPER: u32 = 1 << 31;

/// One row's record: the bits the verify compares next step, its set and leaf slot, and this
/// step's segment. 32 B, two per cache line.
#[repr(C, align(32))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct RowRec {
    x: f32,
    y: f32,
    z: f32,
    r: f32,
    /// `slot:24 | set:2 | kind:2 | _:1 | consumed:1 | pending:1 | pending_z:1`.
    tag: u32,
    /// Q and Wide rows: the segment's start in the stream, this step.
    seg: u32,
    /// Entries below the row at the segment's front.
    nrev: u32,
    /// Entries above the row after them.
    nfwd: u32,
}

const _: () = assert!(size_of::<RowRec>() == 32 && align_of::<RowRec>() == 32);

impl RowRec {
    /// The record of a row that had none: NaN bits (equal to no Normal row's), no set, Excluded.
    const NONE: Self = Self {
        x: f32::NAN,
        y: f32::NAN,
        z: f32::NAN,
        r: f32::NAN,
        tag: KIND_EXCLUDED << TAG_KIND_SHIFT,
        seg: 0,
        nrev: 0,
        nfwd: 0,
    };

    #[inline]
    fn set(&self) -> u32 {
        (self.tag >> TAG_SET_SHIFT) & 0b11
    }

    #[inline]
    fn kind(&self) -> u32 {
        (self.tag >> TAG_KIND_SHIFT) & 0b11
    }

    #[inline]
    fn slot(&self) -> u32 {
        self.tag & TAG_SLOT_MASK
    }

    #[inline]
    fn is_pending(&self) -> bool {
        self.tag & TAG_PENDING != 0
    }

    #[inline]
    fn is_pending_z(&self) -> bool {
        self.tag & TAG_PENDING_Z != 0
    }

    /// Makes the record no member of any set (its leaf was killed elsewhere).
    #[inline]
    fn leave(&mut self) {
        self.tag &= !(TAG_SLOT_MASK | (0b11 << TAG_SET_SHIFT));
    }

    #[inline]
    fn bits_equal(&self, x: f32, y: f32, z: f32, r: f32) -> bool {
        self.x.to_bits() == x.to_bits()
            && self.y.to_bits() == y.to_bits()
            && self.z.to_bits() == z.to_bits()
            && self.r.to_bits() == r.to_bits()
    }

    /// Makes the record a member of `set` in leaf `slot`, clearing the pending mark.
    #[inline]
    fn join(&mut self, set: u32, slot: u32) {
        debug_assert!(slot <= TAG_SLOT_MASK, "invariant: leaf slots stay below 2^24");
        self.tag = (self.tag & !(TAG_SLOT_MASK | (0b11 << TAG_SET_SHIFT) | TAG_PENDING))
            | slot
            | (set << TAG_SET_SHIFT);
    }
}

/// The structural counters of a [`BroadphaseTree`], all cumulative except `members`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TreeDiag {
    /// Admissions and compactions of the static set.
    pub static_rebuilds: u64,
    /// Admissions and compactions of the sleeper set (L10 C3c; `0` without the sleep-skip).
    pub sleeper_rebuilds: u64,
    /// Members evicted (moved, reshaped, re-classed, re-kinded, located twice) or vanished.
    pub evictions: u64,
    /// `Rows` steps on which at least one member was translated.
    pub translations: u64,
    /// List entries the patch rule diverted and merged back.
    pub patches: u64,
    /// Row-steps the sleep hint offered to the sleeper set a row that was no member — pending for
    /// Z that step (L10 C3c; `0` without the sleep-skip).
    pub hint_candidates: u64,
    /// Row-steps classed Wide.
    pub wide_rows: u64,
    /// Row-steps classed Excluded.
    pub excluded_rows: u64,
    /// `Reset` classifications of a cursor that had been stamped.
    pub locator_resets: u64,
    /// The current `|S| + |Z|`.
    pub members: u64,
    /// Active leaf nodes (up to eight Q rows each) the leaf-list query answered
    /// ([`QueryKernel::LeafList`]).
    pub leaf_list_leaves: u64,
    /// Active leaf nodes the leaf-list query handed to the per-row walk because a collection
    /// held more than its cap of candidate leaves.
    pub fallback_leaves: u64,
    /// Active leaf nodes the per-row walk answered with [`QueryKernel::RowWalk`] selected.
    pub row_walk_leaves: u64,
}

/// What the last leaf-list pass did (C3b's G-LL3 counts): compiled only in the crate's test
/// build and under the non-default `bp-query-counts` feature.
#[cfg(any(test, feature = "bp-query-counts"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LeafListCounts {
    /// Active leaf nodes the leaf list answered (a fallback leaf is not counted here).
    pub leaves: u64,
    /// Q rows the leaf list answered: the live lanes of those leaf nodes.
    pub rows: u64,
    /// 8-wide box tests of the collection walks over the active tree.
    pub collect_box_active: u64,
    /// 8-wide box tests of the collection walks over the static tree.
    pub collect_box_static: u64,
    /// Candidate leaves the active collections held, summed over the leaves answered.
    pub cands_active: u64,
    /// Candidate leaves the static collections held, summed over the leaves answered; a one-leaf
    /// static tree counts its leaf once per active leaf.
    pub cands_static: u64,
    /// Prefilter tests — chunks of eight candidates — the rows ran, over both lists (the one
    /// leaf of a one-leaf static tree is tested without one).
    pub prefilter_chunks: u64,
    /// Exact tests on active candidates the prefilter kept.
    pub kept_active: u64,
    /// Exact tests on static leaves: those the prefilter kept, or a one-leaf static tree's leaf.
    pub kept_static: u64,
    /// Partners the rows emitted into their segments.
    pub emitted: u64,
    /// Insertion-sort shifts the segments need in their emission order: their inversions.
    pub sort_shifts: u64,
}

/// The kernel that answers the Q rows' queries (module docs, "Query kernels"). Both write the
/// same stream and records; the choice moves no pair and no pose byte.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QueryKernel {
    /// Per Q row, one depth-first walk of each tree (C1's kernel): the reference, and the
    /// leaf-list query's fallback.
    RowWalk,
    /// Per active leaf node, one walk of each tree collects the candidate leaves; each row then
    /// prefilters them eight at a time and runs the exact test on the kept ones (C3b, F1).
    #[default]
    LeafList,
}

/// What a tree-path step's verify reads of L10's sleep-skip (module docs, "The sleeper set"): the
/// rows the sleeper set may hold and the rows either persistent set may hold at all.
///
/// The hint chooses membership, never the pair set (Lemma D3.1): whatever it answers, the bit
/// compare, the injective locator and the filtered lists keep the output exact. So the tests
/// drive the verify with a hint of their own, and the production step has two: [`NoHint`] and
/// L10's `HeldHint` (`sleep_sets.rs`), which reads the step's row classification.
pub(crate) trait SleepHint {
    /// Whether row `r` (a current row) is offered to Z this step: under L10's `Sets`, a member of
    /// a held record the prologue did not restore (T1).
    fn frozen(&self, r: usize) -> bool;
    /// Whether row `r` may be a member of S or Z this step: under L10's `Sets` with a sleeper, a
    /// row that rests and is no sensor (T2); any row otherwise.
    fn anchor_ok(&self, r: usize) -> bool;
}

/// The hint of a step without the sleep-skip, and of every `Reset` (design D3.4): nothing is
/// offered to Z, so every Z member is evicted and `SL` empties; S keeps its C1 class (a still
/// static).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NoHint;

impl SleepHint for NoHint {
    #[inline]
    fn frozen(&self, _r: usize) -> bool {
        false
    }

    #[inline]
    fn anchor_ok(&self, _r: usize) -> bool {
        true
    }
}

/// The two classes a Normal row can fit, from the hint: S (a static the hint lets rest) and Z (a
/// non-static the hint offers). Both `false` for a Wide or Excluded row.
#[inline]
fn fits<H: SleepHint>(hint: &H, r: usize, kind: u32, is_static: bool) -> (bool, bool) {
    if kind != KIND_NORMAL {
        return (false, false);
    }
    let anchor = hint.anchor_ok(r);
    (is_static && anchor, !is_static && anchor && hint.frozen(r))
}

/// The tree broadphase's state: the active, the static and the sleeper tree, two record buffers,
/// the static pair list, the scratch stream and the row cursor. The sleeper pair list `SL` is
/// [`ContactPairs::withheld`] (T3). One per world; see the module docs.
#[derive(Resource)]
pub struct BroadphaseTree {
    /// The tree over Q, rebuilt every step.
    active: PackedBvh8,
    /// The tree over S.
    statics: PackedBvh8,
    /// The tree over Z (L10 C3c).
    sleepers: PackedBvh8,
    /// Radix ping-pong; cold: the diverted entries and the admission's additions.
    sort_a: ScratchColumn<u64>,
    /// Radix ping-pong.
    sort_b: ScratchColumn<u64>,
    /// Q rows in row order; cold: a rebuild's input.
    items: ScratchColumn<Item>,
    /// The two record buffers: `rec[cur]` holds the last step's records at a step's start.
    rec: [ScratchColumn<RowRec>; 2],
    /// Which record buffer is current.
    cur: u8,
    /// S–S pairs, strictly sorted, keyed `(min << 32) | max`.
    ss: ScratchColumn<u64>,
    /// `Rows`: the inverse map | query: the segment stream | assembly: + bucket cursors + rev
    /// entries.
    aux: ScratchColumn<u32>,
    /// This consumer's place in the gather sequence.
    cursor: RemapCursor,
    /// Each set's rent (`[IX_S]`, `[IX_Z]`).
    rent: [u64; 2],
    /// Rows at or below which the brute loop runs.
    brute_max_rows: u32,
    /// The Q rows' query kernel.
    kernel: QueryKernel,
    /// The leaf-list collection cap, lowered by the fallback gate (test builds only; every
    /// other build uses `kernel::LEAF_LIST_CAP`).
    #[cfg(test)]
    leaf_list_cap: usize,
    /// The last leaf-list pass's counts (test and `bp-query-counts` builds only).
    #[cfg(any(test, feature = "bp-query-counts"))]
    ll_counts: LeafListCounts,
    /// `|S|` and `|Z|`.
    members: [u32; 2],
    /// This step's pending rows per set (the verify writes, the maintenance reads).
    pending_step: [u32; 2],
    /// The verify saw a vanished member, or the rows changed with members: scan the leaves.
    needs_scan: bool,
    /// The verify evicted or lost an S member: filter `SS`.
    filter_ss: bool,
    /// The verify evicted or lost a member of either set: filter `SL`.
    filter_sl: bool,
    /// A `Rows` step with at least one member: translate leaves and lists.
    rows_changed: bool,
    /// The counters.
    diag: TreeDiag,
}

impl Default for BroadphaseTree {
    /// Hand-written: the columns need their reserved ids, which no derive supplies.
    #[inline]
    fn default() -> Self {
        Self::with_capacity(0)
    }
}

impl BroadphaseTree {
    /// An empty tree broadphase whose columns are sized for at least `rows` bodies (the kernel's
    /// standard reserve applies above that, so growth is in place and the base never moves).
    pub fn with_capacity(rows: usize) -> Self {
        register_tree_column_layouts();
        let node_rows = rows.max(scratch_reserve_rows(size_of::<Node8>()));
        let key_rows = rows.max(scratch_reserve_rows(size_of::<u64>()));
        let item_rows = rows.max(scratch_reserve_rows(size_of::<Item>()));
        let rec_rows = rows.max(scratch_reserve_rows(size_of::<RowRec>()));
        // The stream is at most P entries, the rev entries at most P and the bucket cursors N.
        // `scratch_reserve_rows(4)` is the kernel's row ceiling (`POOL_MAX_ROWS`), so the
        // column admits up to ~8M pairs per step — ten times the 100k-body figure of the design.
        let aux_rows = rows.max(scratch_reserve_rows(size_of::<u32>()));
        Self {
            active: PackedBvh8::new(tree_column_id(TREE_ACTIVE), node_rows),
            statics: PackedBvh8::new(tree_column_id(TREE_STATICS), node_rows),
            sleepers: PackedBvh8::new(tree_column_id(TREE_SLEEPERS), node_rows),
            sort_a: ScratchColumn::new(tree_column_id(TREE_SORT_A), key_rows),
            sort_b: ScratchColumn::new(tree_column_id(TREE_SORT_B), key_rows),
            items: ScratchColumn::new(tree_column_id(TREE_ITEMS), item_rows),
            rec: [
                ScratchColumn::new(tree_column_id(TREE_REC0), rec_rows),
                ScratchColumn::new(tree_column_id(TREE_REC1), rec_rows),
            ],
            cur: 0,
            ss: ScratchColumn::new(tree_column_id(TREE_SS), key_rows),
            aux: ScratchColumn::new(tree_column_id(TREE_AUX), aux_rows),
            cursor: RemapCursor::default(),
            rent: [0; 2],
            brute_max_rows: TREE_BRUTE_MAX_ROWS,
            kernel: QueryKernel::default(),
            #[cfg(test)]
            leaf_list_cap: LEAF_LIST_CAP,
            #[cfg(any(test, feature = "bp-query-counts"))]
            ll_counts: LeafListCounts::default(),
            members: [0; 2],
            pending_step: [0; 2],
            needs_scan: false,
            filter_ss: false,
            filter_sl: false,
            rows_changed: false,
            diag: TreeDiag::default(),
        }
    }

    /// The structural counters.
    #[inline]
    pub fn diag(&self) -> TreeDiag {
        TreeDiag {
            locator_resets: self.cursor.resets(),
            members: u64::from(self.members[IX_S] + self.members[IX_Z]),
            ..self.diag
        }
    }

    /// `|Z|`: the rows the sleeper set holds after the last step (L10 C3c; `0` without the
    /// sleep-skip).
    #[inline]
    pub fn sleeper_members(&self) -> u64 {
        u64::from(self.members[IX_Z])
    }

    /// Sets the row count at or below which the step runs the brute all-pairs loop. `0` forces
    /// the tree path on every step (the gates, the profiling harness and the benches).
    #[inline]
    pub fn set_brute_max_rows(&mut self, rows: u32) {
        self.brute_max_rows = rows;
    }

    /// The row count at or below which the step runs the brute all-pairs loop.
    #[inline]
    pub fn brute_max_rows(&self) -> u32 {
        self.brute_max_rows
    }

    /// Selects the kernel that answers the Q rows (module docs, "Query kernels"). The pair set
    /// and its order do not depend on it; the G4 bench's `tree_rowwalk` arm uses it for a
    /// same-binary A/B.
    #[inline]
    pub fn set_query_kernel(&mut self, kernel: QueryKernel) {
        self.kernel = kernel;
    }

    /// The selected query kernel.
    #[inline]
    pub fn query_kernel(&self) -> QueryKernel {
        self.kernel
    }

    /// The leaf-list collection cap: `kernel::LEAF_LIST_CAP`, or the fallback gate's lowered
    /// value.
    ///
    /// The test build narrows the production value with a `#[cfg(test)]` statement rather than
    /// splitting the body into a `#[cfg(test)]` / `#[cfg(not(test))]` pair: the workspace's
    /// source censuses blank every item whose predicate names `test`, so a `not(test)` item — the
    /// production branch — would be read as test-only
    /// (`production_reachability_census::the_cfg_test_rule_has_no_not_test_counterexample_in_the_tree`).
    /// Outside `cfg(test)` the body is the constant alone.
    #[inline]
    fn leaf_list_cap(&self) -> usize {
        let cap = LEAF_LIST_CAP;
        #[cfg(test)]
        let cap = cap.min(self.leaf_list_cap);
        cap
    }

    /// Lowers the leaf-list collection cap, so a small scene exercises the fallback.
    #[cfg(test)]
    fn set_leaf_list_cap(&mut self, cap: usize) {
        assert!(cap <= LEAF_LIST_CAP, "the cap fits the candidate buffers");
        self.leaf_list_cap = cap;
    }

    /// One step under direct drive, for a harness outside the crate that holds no gather (the
    /// G4 bench): the identity locator on every step, no cursor. Row `r`'s record is the record
    /// row `r` wrote on the previous step, so a row that keeps its bits keeps its membership and
    /// a row that changes them is evicted; a row past the previous step's count is new. Runs the
    /// brute loop at or below `brute_max_rows`, as [`physics_broadphase`] does.
    ///
    /// Not to be mixed with the gather-driven step on one instance: the cursor is never stamped
    /// here, so the next gather-driven step would classify as `Rows` against a stale gather.
    ///
    /// [`physics_broadphase`]: crate::systems::physics_broadphase
    pub fn step_direct(&mut self, bodies: &[BodyState], out: &mut ContactPairs) {
        let n = bodies.len();
        debug_assert!(n < 1 << 24, "invariant: rows stay below 2^24");
        if n <= self.brute_max_rows as usize {
            self.clear_sleepers(out);
            all_pairs_into(bodies, out);
            return;
        }
        self.run(bodies, RowRemap::Identity, out, &NoHint);
    }

    /// One `Rows` step under direct drive with an explicit previous-row map, for a harness
    /// outside the crate (the G4 maintenance arms): `prev_row[r]` is the row body `r` had on
    /// the previous step, or [`NO_PREV_ROW`] for a row that had none. A value past the previous
    /// step's count locates nothing. The map must be injective on its located values — a second
    /// row naming the same previous row is not carried (the consumed mark), so it becomes Q.
    /// Runs the brute loop at or below `brute_max_rows`, and never stamps the cursor (see
    /// [`step_direct`](Self::step_direct)).
    ///
    /// # Panics
    ///
    /// If `prev_row.len() != bodies.len()`.
    pub fn step_translated(
        &mut self,
        bodies: &[BodyState],
        prev_row: &[u32],
        out: &mut ContactPairs,
    ) {
        let n = bodies.len();
        assert_eq!(prev_row.len(), n, "invariant: prev_row is indexed by the current rows");
        debug_assert!(n < 1 << 24, "invariant: rows stay below 2^24");
        if n <= self.brute_max_rows as usize {
            self.clear_sleepers(out);
            all_pairs_into(bodies, out);
            return;
        }
        self.run(bodies, RowRemap::Rows(prev_row), out, &NoHint);
    }

    /// One step without the sleep-skip: [`step_hinted`](Self::step_hinted) with [`NoHint`].
    pub(crate) fn step(
        &mut self,
        bodies: &[BodyState],
        rows: &RowIdentity,
        out: &mut ContactPairs,
    ) {
        self.step_hinted(bodies, rows, out, &NoHint);
    }

    /// One step: fills `out` with the exact pair set of `bodies` as the stream ⊎
    /// [`ContactPairs::withheld`], both in `(min, max)` order.
    ///
    /// `rows` is the gather's row identity, which locates the records of the previous step
    /// when the rows changed; a never-gathered identity (direct drive) classifies as
    /// `Identity` on every step. `hint` is the step's sleep hint; a `Reset` reads [`NoHint`]
    /// whatever it is (design D3.4). The brute path dissolves the sleeper set (T6).
    pub(crate) fn step_hinted<H: SleepHint>(
        &mut self,
        bodies: &[BodyState],
        rows: &RowIdentity,
        out: &mut ContactPairs,
        hint: &H,
    ) {
        let n = bodies.len();
        debug_assert!(n < 1 << 24, "invariant: rows stay below 2^24");
        if n <= self.brute_max_rows as usize {
            self.clear_sleepers(out);
            all_pairs_into(bodies, out);
            return;
        }
        let remap = self.cursor.remap(rows);
        self.run(bodies, remap, out, hint);
        // Protocol P: the state is keyed by this gather's rows from here on. Once per tree-path
        // step, after the maintenance, whatever the locator was (ruling W2(a)).
        self.cursor.stamp(rows);
    }

    /// The tree path with an explicit locator and no stamp: [`step_hinted`](Self::step_hinted)
    /// minus the cursor, so a test can drive a hand-made `Rows` map.
    fn run<H: SleepHint>(
        &mut self,
        bodies: &[BodyState],
        remap: RowRemap<'_>,
        out: &mut ContactPairs,
        hint: &H,
    ) {
        let n = bodies.len();
        let rebuilds_before = self.diag.static_rebuilds + self.diag.sleeper_rebuilds;

        {
            let _zone = zone!(PHYS_BP_VERIFY);
            match remap {
                RowRemap::Identity => self.verify_identity(bodies, hint),
                // The hint is off on a `Reset` (design D3.4): the records are located in place,
                // so Z dissolves in this pass and its pairs are queried again.
                RowRemap::Reset => self.verify_identity(bodies, &NoHint),
                RowRemap::Rows(prev_row) => self.verify_rows(bodies, prev_row, hint),
            }
            self.maintain(n, out);
        }
        {
            let _zone = zone!(PHYS_BP_BUILD);
            self.build_active(n);
        }
        let queried = {
            let _zone = zone!(PHYS_BP_QUERY);
            self.query_all(n)
        };
        {
            let _zone = zone!(PHYS_BP_ASSEMBLE);
            self.assemble(n, out);
        }

        counter!(PHYS_BP_QUERIED, queried);
        counter!(PHYS_BP_MEMBERS, u64::from(self.members[IX_S] + self.members[IX_Z]));
        counter!(
            PHYS_BP_REBUILDS,
            self.diag.static_rebuilds + self.diag.sleeper_rebuilds - rebuilds_before
        );
    }

    // ── The verify ───────────────────────────────────────────────────────────

    /// The verify with the identity locator (`Identity`, `Reset`, direct drive): row `r`'s
    /// record is `rec[cur][r]`, updated in place. Evicted lanes are killed here; vanished ones
    /// (rows past `N`) by the scan in [`maintain`](Self::maintain).
    fn verify_identity<H: SleepHint>(&mut self, bodies: &[BodyState], hint: &H) {
        let n = bodies.len();
        let Self { rec, cur, statics, sleepers, diag, .. } = self;
        let mut counts = VerifyCounts::default();
        let mut view = rec[usize::from(*cur)].build_view();
        let old_len = view.len();
        view.resize(n, RowRec::NONE);
        let recs = view.as_mut_slice();
        for (r, body) in bodies.iter().enumerate() {
            let (x, y, z) = (body.position.x, body.position.y, body.position.z);
            let rr = body_bounding_radius(body);
            let kind = classify(x, y, z, rr);
            let is_static = body.inv_mass == 0.0 && !body.kinematic;
            let old = recs[r];
            let located = r < old_len;
            let still = located && old.bits_equal(x, y, z, rr);
            let (s_fit, z_fit) = fits(hint, r, kind, is_static);
            let mut tag = kind << TAG_KIND_SHIFT;
            match old.set() {
                SET_S => {
                    if s_fit && still {
                        counts.carried[IX_S] += 1;
                        tag |= (SET_S << TAG_SET_SHIFT) | old.slot();
                    } else {
                        counts.evicted[IX_S] += 1;
                        statics.kill(old.slot());
                    }
                }
                SET_Z => {
                    if z_fit && still {
                        counts.carried[IX_Z] += 1;
                        tag |= (SET_Z << TAG_SET_SHIFT) | old.slot();
                    } else {
                        counts.evicted[IX_Z] += 1;
                        sleepers.kill(old.slot());
                    }
                }
                _ => tag |= counts.pending_tag(s_fit && still, z_fit),
            }
            counts.note_kind(kind);
            recs[r] = RowRec { x, y, z, r: rr, tag, seg: 0, nrev: 0, nfwd: 0 };
        }
        drop(view);
        counts.record(diag);
        self.finish_verify(counts, false);
    }

    /// The verify with the `prev_row` locator (`Rows`): row `r`'s record is
    /// `rec[1 − cur][prev_row[r]]`, written to `rec[cur]` after the swap. A located record is
    /// marked consumed, so a second locate of it fails. The inverse map `inv[m] = r` of the
    /// carried members of both sets is built in `aux` for the leaf scan and the list translation.
    fn verify_rows<H: SleepHint>(&mut self, bodies: &[BodyState], prev_row: &[u32], hint: &H) {
        let n = bodies.len();
        debug_assert_eq!(prev_row.len(), n, "invariant: prev_row is the current gather's");
        let Self { rec, cur, aux, diag, .. } = self;
        let [a, b] = rec;
        let (old, new) = if *cur == 0 { (a, b) } else { (b, a) };
        *cur ^= 1;
        let mut old_view = old.build_view();
        let old_recs = old_view.as_mut_slice();
        let old_len = old_recs.len();
        let mut new_view = new.build_view();
        new_view.clear();
        new_view.resize(n, RowRec::NONE);
        let recs = new_view.as_mut_slice();
        let mut inv_view = aux.build_view();
        inv_view.clear();
        inv_view.resize(old_len, NO_ROW);
        let inv = inv_view.as_mut_slice();

        let mut counts = VerifyCounts::default();
        for (r, body) in bodies.iter().enumerate() {
            let (x, y, z) = (body.position.x, body.position.y, body.position.z);
            let rr = body_bounding_radius(body);
            let kind = classify(x, y, z, rr);
            let is_static = body.inv_mass == 0.0 && !body.kinematic;
            let m = prev_row[r];
            let located =
                m != NO_ROW && (m as usize) < old_len && old_recs[m as usize].tag & TAG_CONSUMED == 0;
            let old = if located {
                old_recs[m as usize].tag |= TAG_CONSUMED;
                old_recs[m as usize]
            } else {
                RowRec::NONE
            };
            let still = located && old.bits_equal(x, y, z, rr);
            let (s_fit, z_fit) = fits(hint, r, kind, is_static);
            let mut tag = kind << TAG_KIND_SHIFT;
            // An evicted member's lane is killed by the translation (no row carries it).
            match old.set() {
                SET_S => {
                    if s_fit && still {
                        counts.carried[IX_S] += 1;
                        tag |= (SET_S << TAG_SET_SHIFT) | old.slot();
                        inv[m as usize] = r as u32;
                    } else {
                        counts.evicted[IX_S] += 1;
                    }
                }
                SET_Z => {
                    if z_fit && still {
                        counts.carried[IX_Z] += 1;
                        tag |= (SET_Z << TAG_SET_SHIFT) | old.slot();
                        inv[m as usize] = r as u32;
                    } else {
                        counts.evicted[IX_Z] += 1;
                    }
                }
                _ => tag |= counts.pending_tag(s_fit && still, z_fit),
            }
            counts.note_kind(kind);
            recs[r] = RowRec { x, y, z, r: rr, tag, seg: 0, nrev: 0, nfwd: 0 };
        }
        drop(inv_view);
        drop(new_view);
        drop(old_view);
        counts.record(diag);
        self.finish_verify(counts, true);
    }

    /// Closes a verify: the vanished counts, the eviction and candidate counters and the
    /// structural flags the maintenance reads.
    fn finish_verify(&mut self, counts: VerifyCounts, rows_changed: bool) {
        let had_members = self.members[IX_S] + self.members[IX_Z] > 0;
        let mut changed = [false; 2];
        let mut vanished_any = false;
        for x in [IX_S, IX_Z] {
            let vanished = self.members[x] - counts.carried[x] - counts.evicted[x];
            self.diag.evictions += u64::from(counts.evicted[x] + vanished);
            changed[x] = counts.evicted[x] + vanished > 0;
            vanished_any |= vanished > 0;
        }
        self.diag.hint_candidates += u64::from(counts.pending[IX_Z]);
        self.pending_step = counts.pending;
        self.needs_scan = vanished_any || (rows_changed && had_members);
        self.filter_ss = changed[IX_S];
        self.filter_sl = changed[IX_S] || changed[IX_Z];
        self.rows_changed = rows_changed && had_members;
        self.members = counts.carried;
    }

    // ── Maintenance ──────────────────────────────────────────────────────────

    /// Everything a step does to the persistent sets after the verify: the leaf scan
    /// (translation and kills), the list passes (translation with the patch rule, or a filter),
    /// admission (S, then Z) and compaction. `out` holds `SL` (its `withheld` list, T3). Out of
    /// line so the verify stays compact.
    #[cold]
    #[inline(never)]
    fn maintain(&mut self, n: usize, out: &mut ContactPairs) {
        if self.rows_changed {
            self.diag.translations += 1;
            self.translate(out);
        } else {
            if self.needs_scan {
                self.scan_leaves(n);
            }
            if self.filter_ss {
                self.filter_list(n);
            }
            if self.filter_sl {
                self.filter_withheld(n, out);
            }
        }
        self.needs_scan = false;
        self.filter_ss = false;
        self.filter_sl = false;
        self.rows_changed = false;

        // S before Z (design D3.5): a pair of a new static and a new sleeper is then found once,
        // by Z's admission, whose pending rows query the static tree S's admission rebuilt.
        if self.admission_due(IX_S) {
            self.admit(n, out);
            self.rent[IX_S] = 0;
        }
        if self.admission_due(IX_Z) {
            self.admit_sleepers(n, out);
            self.rent[IX_Z] = 0;
        }
        if self.statics.dead() >= self.statics.live().max(COMPACT_MIN_DEAD) {
            self.compact(IX_S);
        }
        if self.sleepers.dead() >= self.sleepers.live().max(COMPACT_MIN_DEAD) {
            self.compact(IX_Z);
        }
        self.debug_check_members(n, out);
    }

    /// The rent rule for set `x` (D3.5): accrues this step's pending rows and says whether the
    /// set admits them now.
    #[inline]
    fn admission_due(&mut self, x: usize) -> bool {
        let pending = u64::from(self.pending_step[x]);
        if pending == 0 {
            self.rent[x] = 0;
            return false;
        }
        self.rent[x] += pending;
        let (num, den) = ADMIT_BUILD_RATIO;
        self.rent[x] * den >= pending * den + num * (u64::from(self.members[x]) + pending)
    }

    /// Kills the lanes of either tree whose row is past `N` (a shrink under the identity
    /// locator).
    fn scan_leaves(&mut self, n: usize) {
        for tree in [&mut self.statics, &mut self.sleepers] {
            for slot in 0..tree.leaves() {
                let row = tree.leaf_row(slot);
                if row != NO_LANE_ROW && row as usize >= n {
                    tree.kill(slot);
                }
            }
        }
    }

    /// Drops every `SS` entry with an endpoint that is no longer a member, keeping the order.
    fn filter_list(&mut self, n: usize) {
        let recs = self.rec[usize::from(self.cur)].as_read_slice();
        let mut view = self.ss.build_view();
        let list = view.as_mut_slice();
        let mut w = 0usize;
        for i in 0..list.len() {
            let key = list[i];
            let (a, b) = ((key >> 32) as usize, (key & 0xffff_ffff) as usize);
            if a < n && b < n && recs[a].set() == SET_S && recs[b].set() == SET_S {
                list[w] = key;
                w += 1;
            }
        }
        view.truncate(w);
    }

    /// Drops every `SL` entry that is no longer a pair of S ∪ Z with a Z endpoint, keeping the
    /// order.
    fn filter_withheld(&mut self, n: usize, out: &mut ContactPairs) {
        let recs = self.rec[usize::from(self.cur)].as_read_slice();
        let mut view = out.withheld_build();
        let list = view.as_mut_slice();
        let mut w = 0usize;
        for i in 0..list.len() {
            let (a, b) = (list[i].0.0 as usize, list[i].1.0 as usize);
            if a < n && b < n && sl_pair(recs[a].set(), recs[b].set()) {
                list[w] = list[i];
                w += 1;
            }
        }
        view.truncate(w);
    }

    /// A `Rows` step's translation: the leaf rows of both trees and the entries of both lists
    /// through the inverse map, dropping the members no row carried, with the patch rule of
    /// ruling W1 over the carried rows of both sets.
    fn translate(&mut self, out: &mut ContactPairs) {
        let Self { statics, sleepers, ss, aux, sort_a, diag, .. } = self;
        let mut inv_view = aux.build_view();
        let inv = inv_view.as_mut_slice();
        let old_len = inv.len();

        // Leaves: a carried member's lane takes its new row; every other live lane is killed.
        for tree in [&mut *statics, &mut *sleepers] {
            for slot in 0..tree.leaves() {
                let row = tree.leaf_row(slot);
                if row == NO_LANE_ROW {
                    continue;
                }
                let new_row = if (row as usize) < old_len { inv[row as usize] } else { NO_ROW };
                if new_row == NO_ROW {
                    tree.kill(slot);
                } else {
                    tree.set_leaf_row(slot, new_row);
                }
            }
        }

        // The jumpers: carried rows outside the maximum-weight monotone run subsequence.
        mark_jumpers(inv);

        // The lists: translate, drop, keep or divert.
        diag.patches += translate_list(&mut ss.build_view(), inv, sort_a);
        diag.patches += translate_list(&mut out.withheld_build(), inv, sort_a);
    }

    /// Admits the pending rows into S: the tree is rebuilt over members and pending rows, each
    /// pending row is queried against it (a partner is an old member, or a pending row above
    /// it) and against the sleeper tree, and the sorted additions are merged into `SS` and into
    /// `SL` (`out`'s `withheld` list).
    fn admit(&mut self, n: usize, out: &mut ContactPairs) {
        let Self { statics, sleepers, items, sort_a, sort_b, rec, cur, ss, diag, members, .. } = self;
        let mut recs_view = rec[usize::from(*cur)].build_view();
        let recs = recs_view.as_mut_slice();
        debug_assert_eq!(recs.len(), n);

        let mut items_view = items.build_view();
        items_view.clear();
        for slot in 0..statics.leaves() {
            let leaf = statics.leaf(slot);
            if leaf.row != NO_LANE_ROW {
                items_view.push(Item::new(leaf.x, leaf.y, leaf.z, leaf.r, leaf.row));
            }
        }
        for (r, rec) in recs.iter().enumerate() {
            if rec.is_pending() {
                debug_assert_eq!(rec.kind(), KIND_NORMAL, "only a Normal row is pending");
                items_view.push(Item::new(rec.x, rec.y, rec.z, rec.r, r as u32));
            }
        }
        statics.build(items_view.as_slice(), sort_a, sort_b);
        for slot in 0..statics.leaves() {
            let row = statics.leaf_row(slot) as usize;
            let pending = recs[row].is_pending();
            recs[row].join(SET_S, slot);
            // The pending mark is needed by the queries below; `join` cleared it.
            if pending {
                recs[row].tag |= TAG_PENDING;
            }
        }

        // `SS` additions in `sort_a`, `SL` additions (a new static with a sleeper) in `sort_b`:
        // both are free once the build has returned.
        let mut additions = sort_a.build_view();
        additions.clear();
        let mut with_sleepers = sort_b.build_view();
        with_sleepers.clear();
        for (r, rec) in recs.iter().enumerate() {
            if !rec.is_pending() {
                continue;
            }
            let r = r as u32;
            statics.query(rec.x, rec.y, rec.z, rec.r, |t| {
                if t != r && (!recs[t as usize].is_pending() || t > r) {
                    additions.push(pair_key(r.min(t), r.max(t)));
                }
            });
            sleepers.query(rec.x, rec.y, rec.z, rec.r, |t| {
                with_sleepers.push(pair_key(r.min(t), r.max(t)));
            });
        }
        for rec in recs.iter_mut() {
            rec.tag &= !TAG_PENDING;
        }
        let a = additions.len();
        if a > 0 {
            let additions = additions.as_mut_slice();
            additions.sort_unstable();
            let mut list = ss.build_view();
            merge_into_sorted(&mut list, additions, false);
            debug_assert!(strictly_sorted(list.as_slice()), "SS is strictly sorted after an admission");
        }
        if !with_sleepers.is_empty() {
            let with_sleepers = with_sleepers.as_mut_slice();
            with_sleepers.sort_unstable();
            let mut list = out.withheld_build();
            merge_into_sorted(&mut list, with_sleepers, false);
            debug_assert!(strictly_sorted(list.as_slice()), "SL is strictly sorted after an admission");
        }
        members[IX_S] = statics.leaves();
        diag.static_rebuilds += 1;
    }

    /// Admits the pending rows into Z (L10 C3c): the sleeper tree is rebuilt over members and
    /// pending rows, each pending row is queried against it (a partner is an old member, or a
    /// pending row above it) and against the static tree, and the sorted additions are merged
    /// into `SL` (`out`'s `withheld` list).
    fn admit_sleepers(&mut self, n: usize, out: &mut ContactPairs) {
        let Self { statics, sleepers, items, sort_a, sort_b, rec, cur, diag, members, .. } = self;
        let mut recs_view = rec[usize::from(*cur)].build_view();
        let recs = recs_view.as_mut_slice();
        debug_assert_eq!(recs.len(), n);

        let mut items_view = items.build_view();
        items_view.clear();
        for slot in 0..sleepers.leaves() {
            let leaf = sleepers.leaf(slot);
            if leaf.row != NO_LANE_ROW {
                items_view.push(Item::new(leaf.x, leaf.y, leaf.z, leaf.r, leaf.row));
            }
        }
        for (r, rec) in recs.iter().enumerate() {
            if rec.is_pending_z() {
                debug_assert_eq!(rec.kind(), KIND_NORMAL, "only a Normal row is pending");
                items_view.push(Item::new(rec.x, rec.y, rec.z, rec.r, r as u32));
            }
        }
        sleepers.build(items_view.as_slice(), sort_a, sort_b);
        // `join` keeps the Z pending mark, which the queries below read.
        for slot in 0..sleepers.leaves() {
            let row = sleepers.leaf_row(slot) as usize;
            recs[row].join(SET_Z, slot);
        }

        let mut additions = sort_a.build_view();
        additions.clear();
        for (r, rec) in recs.iter().enumerate() {
            if !rec.is_pending_z() {
                continue;
            }
            let r = r as u32;
            sleepers.query(rec.x, rec.y, rec.z, rec.r, |t| {
                if t != r && (!recs[t as usize].is_pending_z() || t > r) {
                    additions.push(pair_key(r.min(t), r.max(t)));
                }
            });
            statics.query(rec.x, rec.y, rec.z, rec.r, |t| {
                additions.push(pair_key(r.min(t), r.max(t)));
            });
        }
        for rec in recs.iter_mut() {
            rec.tag &= !TAG_PENDING_Z;
        }
        if !additions.is_empty() {
            let additions = additions.as_mut_slice();
            additions.sort_unstable();
            let mut list = out.withheld_build();
            merge_into_sorted(&mut list, additions, false);
            debug_assert!(strictly_sorted(list.as_slice()), "SL is strictly sorted after an admission");
        }
        members[IX_Z] = sleepers.leaves();
        diag.sleeper_rebuilds += 1;
    }

    /// Rebuilds set `x`'s tree over its live lanes only. Its lists are already filtered.
    fn compact(&mut self, x: usize) {
        let Self { statics, sleepers, items, sort_a, sort_b, rec, cur, diag, .. } = self;
        let (tree, set) = if x == IX_S { (statics, SET_S) } else { (sleepers, SET_Z) };
        let mut items_view = items.build_view();
        items_view.clear();
        for slot in 0..tree.leaves() {
            let leaf = tree.leaf(slot);
            if leaf.row != NO_LANE_ROW {
                items_view.push(Item::new(leaf.x, leaf.y, leaf.z, leaf.r, leaf.row));
            }
        }
        tree.build(items_view.as_slice(), sort_a, sort_b);
        let mut recs_view = rec[usize::from(*cur)].build_view();
        let recs = recs_view.as_mut_slice();
        for slot in 0..tree.leaves() {
            let row = tree.leaf_row(slot) as usize;
            recs[row].join(set, slot);
        }
        if x == IX_S {
            diag.static_rebuilds += 1;
        } else {
            diag.sleeper_rebuilds += 1;
        }
    }

    // ── The seam with L10 (T4, T6) ───────────────────────────────────────────

    /// T4: the sleeper-set members for which `released(row)` holds leave Z in this step — L10's
    /// epilogue restored their records after the verify (its D2 scan, D3). Their lanes are
    /// killed, and every `withheld` pair that is no longer a pair of S ∪ Z with a Z endpoint is
    /// sorted-merged into the stream, so the narrowphase sees the logical set's partition as
    /// `Off` computes it. `O(|Z| + |withheld| + |stream|)`, and only on a step that releases a
    /// member. Returns the rows released.
    pub(crate) fn release(&mut self, out: &mut ContactPairs, released: impl Fn(u32) -> bool) -> u32 {
        if self.members[IX_Z] == 0 {
            return 0;
        }
        self.release_members(out, released)
    }

    #[cold]
    #[inline(never)]
    fn release_members(&mut self, out: &mut ContactPairs, released: impl Fn(u32) -> bool) -> u32 {
        let Self { sleepers, rec, cur, sort_a, members, .. } = self;
        let mut recs_view = rec[usize::from(*cur)].build_view();
        let recs = recs_view.as_mut_slice();
        let mut count = 0u32;
        for slot in 0..sleepers.leaves() {
            let row = sleepers.leaf_row(slot);
            if row != NO_LANE_ROW && released(row) {
                sleepers.kill(slot);
                recs[row as usize].leave();
                count += 1;
            }
        }
        if count == 0 {
            return 0;
        }
        members[IX_Z] -= count;
        let (mut stream, mut withheld) = out.split_build();
        let mut moved = sort_a.build_view();
        moved.clear();
        {
            let list = withheld.as_mut_slice();
            let mut w = 0usize;
            for i in 0..list.len() {
                let (a, b) = list[i];
                if sl_pair(recs[a.0 as usize].set(), recs[b.0 as usize].set()) {
                    list[w] = list[i];
                    w += 1;
                } else {
                    moved.push(ListKey::key(list[i]));
                }
            }
            withheld.truncate(w);
        }
        // A subsequence of a sorted list, disjoint from the stream: a plain merge.
        merge_into_sorted(&mut stream, moved.as_slice(), false);
        debug_assert!(
            stream.as_slice().windows(2).all(|p| p[0] < p[1]),
            "the stream is strictly sorted after a release"
        );
        count
    }

    /// T6: dissolves the sleeper set and empties `withheld` (`out`'s), for a step on which the
    /// tree path does not run — a kind switch or the brute path emits every pair itself. The
    /// static set is untouched. O(1) when Z and `withheld` are already empty.
    #[inline]
    pub(crate) fn clear_sleepers(&mut self, out: &mut ContactPairs) {
        if self.sleepers.leaves() == 0 && out.withheld().is_empty() {
            return;
        }
        self.dissolve_sleepers(out);
    }

    #[cold]
    #[inline(never)]
    fn dissolve_sleepers(&mut self, out: &mut ContactPairs) {
        let Self { sleepers, rec, cur, sort_a, sort_b, members, rent, .. } = self;
        {
            let mut recs_view = rec[usize::from(*cur)].build_view();
            let recs = recs_view.as_mut_slice();
            for slot in 0..sleepers.leaves() {
                let row = sleepers.leaf_row(slot);
                if row != NO_LANE_ROW {
                    recs[row as usize].leave();
                }
            }
        }
        sleepers.build(&[], sort_a, sort_b);
        members[IX_Z] = 0;
        rent[IX_Z] = 0;
        out.withheld_build().clear();
    }

    /// Debug-only: every member's leaf points back at its row, no Wide or Excluded row is in a
    /// tree, each tree's live lane count is its member count, `SS` and `SL` are strictly sorted,
    /// and every `SL` entry is a pair of S ∪ Z with a Z endpoint.
    #[inline]
    fn debug_check_members(&self, n: usize, out: &ContactPairs) {
        if cfg!(debug_assertions) {
            let recs = self.rec[usize::from(self.cur)].as_read_slice();
            let mut live = [0u32; 2];
            for (r, rec) in recs.iter().enumerate().take(n) {
                let (x, tree) = match rec.set() {
                    SET_S => (IX_S, &self.statics),
                    SET_Z => (IX_Z, &self.sleepers),
                    _ => continue,
                };
                live[x] += 1;
                assert_eq!(rec.kind(), KIND_NORMAL, "row {r}: a member is Normal");
                assert_eq!(tree.leaf_row(rec.slot()), r as u32, "row {r}: its leaf slot points back at it");
            }
            assert_eq!(live, self.members, "each member count is its carried count");
            assert_eq!(live[IX_S], self.statics.live(), "live static lanes equal members");
            assert_eq!(live[IX_Z], self.sleepers.live(), "live sleeper lanes equal members");
            assert!(strictly_sorted(self.ss.as_read_slice()), "SS is strictly sorted");
            let sl = out.withheld();
            assert!(sl.windows(2).all(|p| p[0] < p[1]), "SL is strictly sorted");
            assert!(
                sl.iter().all(|&(a, b)| {
                    let (a, b) = (a.0 as usize, b.0 as usize);
                    a < n && b < n && sl_pair(recs[a].set(), recs[b].set())
                }),
                "every SL entry is a pair of S ∪ Z with a Z endpoint"
            );
        }
    }

    // ── Build and query ──────────────────────────────────────────────────────

    /// Builds the active tree over the Q rows, in row order.
    fn build_active(&mut self, n: usize) {
        let Self { active, items, sort_a, sort_b, rec, cur, .. } = self;
        let recs = rec[usize::from(*cur)].as_read_slice();
        let mut items_view = items.build_view();
        items_view.clear();
        for (r, rec) in recs.iter().enumerate().take(n) {
            if rec.kind() == KIND_NORMAL && rec.set() == SET_NONE {
                items_view.push(Item::new(rec.x, rec.y, rec.z, rec.r, r as u32));
            }
        }
        active.build(items_view.as_slice(), sort_a, sort_b);
    }

    /// Queries every Q row (Morton order) with the selected kernel and loops every Wide row
    /// (row order), appending each row's segment to the stream. Returns `|Q| + |Wide|`.
    fn query_all(&mut self, n: usize) -> u64 {
        let cap = self.leaf_list_cap();
        let Self {
            active,
            statics,
            sleepers,
            rec,
            cur,
            aux,
            kernel,
            diag,
            #[cfg(any(test, feature = "bp-query-counts"))]
            ll_counts,
            ..
        } = self;
        let mut recs_view = rec[usize::from(*cur)].build_view();
        let recs = recs_view.as_mut_slice();
        let mut stream = aux.build_view();
        let trees = Trees { active, statics, sleepers };

        match *kernel {
            QueryKernel::LeafList => leaf_list_pass(
                trees,
                recs,
                &mut stream,
                cap,
                diag,
                #[cfg(any(test, feature = "bp-query-counts"))]
                ll_counts,
            ),
            QueryKernel::RowWalk => {
                stream.clear();
                for slot in 0..active.leaves() {
                    row_walk_slot(trees, recs, &mut stream, slot);
                }
                diag.row_walk_leaves += u64::from(active.leaves().div_ceil(LANES as u32));
            }
        }

        let mut wide = 0u64;
        for w in 0..n {
            if recs[w].kind() != KIND_WIDE {
                continue;
            }
            wide += 1;
            let me = recs[w];
            let seg = stream.len();
            let mut nrev = 0u32;
            for (t, other) in recs.iter().enumerate().take(n) {
                if t == w || (other.kind() == KIND_WIDE && t < w) {
                    continue;
                }
                if sphere_bound_feasible(
                    Vec3::new(me.x, me.y, me.z),
                    me.r,
                    Vec3::new(other.x, other.y, other.z),
                    other.r,
                ) {
                    stream.push(t as u32);
                    nrev += u32::from(t < w);
                }
            }
            let rec = &mut recs[w];
            rec.seg = seg as u32;
            rec.nrev = nrev;
            rec.nfwd = (stream.len() - seg) as u32 - nrev;
        }
        u64::from(active.leaves()) + wide
    }

    // ── Assembly ─────────────────────────────────────────────────────────────

    /// Places every pair: the rev entries are bucketed by their smaller row (a count, an
    /// exclusive prefix and a scatter over rows ascending, so each bucket arrives sorted), then
    /// each row's forward run, `SS` run and bucket are merged in order.
    fn assemble(&mut self, n: usize, out: &mut ContactPairs) {
        let Self { rec, cur, aux, ss, .. } = self;
        let recs = rec[usize::from(*cur)].as_read_slice();
        let ss = ss.as_read_slice();
        let mut stream_view = aux.build_view();
        let stream_len = stream_view.len();

        // Bucket counts, then exclusive prefix: `aux[stream_len + t]` is bucket `t`'s cursor.
        stream_view.resize(stream_len + n, 0);
        let mut fwd_total = 0usize;
        {
            let a = stream_view.as_mut_slice();
            let (stream, counts) = a.split_at_mut(stream_len);
            for rec in recs.iter().take(n) {
                fwd_total += rec.nfwd as usize;
                let seg = rec.seg as usize;
                for &t in &stream[seg..seg + rec.nrev as usize] {
                    counts[t as usize] += 1;
                }
            }
            let mut sum = 0u32;
            for c in counts.iter_mut() {
                let v = *c;
                *c = sum;
                sum += v;
            }
        }
        let rev_total: usize = recs.iter().take(n).map(|r| r.nrev as usize).sum();
        stream_view.resize(stream_len + n + rev_total, 0);
        {
            let a = stream_view.as_mut_slice();
            let (stream, rest) = a.split_at_mut(stream_len);
            let (cursor, rev) = rest.split_at_mut(n);
            for (r, rec) in recs.iter().enumerate().take(n) {
                let seg = rec.seg as usize;
                for &t in &stream[seg..seg + rec.nrev as usize] {
                    let c = &mut cursor[t as usize];
                    rev[*c as usize] = r as u32;
                    *c += 1;
                }
            }
        }

        let p = fwd_total + rev_total + ss.len();
        let mut pairs = out.pairs_build();
        pairs.clear();
        pairs.resize(p, (BodyIndex(0), BodyIndex(0)));
        let pairs = pairs.as_mut_slice();
        let a = stream_view.as_slice();
        let (stream, rest) = a.split_at(stream_len);
        let (cursor, rev) = rest.split_at(n);
        let mut w = 0usize;
        let mut c = 0usize;
        let mut bucket_lo = 0usize;
        for (row, rec) in recs.iter().enumerate().take(n) {
            let fwd_start = rec.seg as usize + rec.nrev as usize;
            let fwd = &stream[fwd_start..fwd_start + rec.nfwd as usize];
            let bucket_hi = cursor[row] as usize;
            let bucket = &rev[bucket_lo..bucket_hi];
            bucket_lo = bucket_hi;
            let ss_start = c;
            while c < ss.len() && (ss[c] >> 32) as usize == row {
                c += 1;
            }
            let run = &ss[ss_start..c];
            w = merge_row(row as u32, fwd, run, bucket, pairs, w);
        }
        debug_assert_eq!(w, p, "every pair was placed exactly once");
        debug_assert_eq!(c, ss.len(), "every SS entry was placed");
    }
}

/// Per-verify tallies, per set where a set is named (`[IX_S]`, `[IX_Z]`).
#[derive(Default)]
struct VerifyCounts {
    carried: [u32; 2],
    evicted: [u32; 2],
    pending: [u32; 2],
    wide: u64,
    excluded: u64,
}

impl VerifyCounts {
    /// The pending mark of a non-member that is pending for S (`s`) or for Z (`z`) — never both:
    /// S holds statics, Z non-statics — counted.
    #[inline]
    fn pending_tag(&mut self, s: bool, z: bool) -> u32 {
        debug_assert!(!(s && z), "invariant: S and Z classes are disjoint");
        self.pending[IX_S] += u32::from(s);
        self.pending[IX_Z] += u32::from(z);
        if s {
            TAG_PENDING
        } else if z {
            TAG_PENDING_Z
        } else {
            0
        }
    }

    #[inline]
    fn note_kind(&mut self, kind: u32) {
        self.wide += u64::from(kind == KIND_WIDE);
        self.excluded += u64::from(kind == KIND_EXCLUDED);
    }

    #[inline]
    fn record(&self, diag: &mut TreeDiag) {
        diag.wide_rows += self.wide;
        diag.excluded_rows += self.excluded;
    }
}

/// The row kind of D8 from the row's bits.
#[inline]
fn classify(x: f32, y: f32, z: f32, r: f32) -> u32 {
    if x.is_nan() || y.is_nan() || z.is_nan() || r.is_nan() {
        return KIND_EXCLUDED;
    }
    let ar = r.abs();
    let extent = x.abs().max(y.abs()).max(z.abs());
    if !extent.is_finite() {
        return if ar <= KIND_LIMIT { KIND_EXCLUDED } else { KIND_WIDE };
    }
    if ar > KIND_LIMIT || extent + ar > KIND_LIMIT {
        KIND_WIDE
    } else {
        KIND_NORMAL
    }
}

/// The list key of the pair `(min, max)`.
#[inline]
const fn pair_key(min: u32, max: u32) -> u64 {
    ((min as u64) << 32) | max as u64
}

#[inline]
fn strictly_sorted<K: Ord>(keys: &[K]) -> bool {
    keys.windows(2).all(|w| w[0] < w[1])
}

/// Whether a pair whose endpoints' records are in sets `sa` and `sb` belongs to `SL`: both
/// members of S ∪ Z, at least one of Z.
#[inline]
fn sl_pair(sa: u32, sb: u32) -> bool {
    (sa == SET_Z || sb == SET_Z) && sa != SET_NONE && sb != SET_NONE
}

/// An entry of a persistent pair list: `SS` keeps `(min << 32) | max` keys, `SL` (the withheld
/// pairs, T3) the `(min, max)` tuples the logical pair view hands out. The two orders are the
/// same, so every list routine is one generic body over both.
trait ListKey: Copy + Ord + 'static {
    /// The entry of key `(min << 32) | max`.
    fn from_key(key: u64) -> Self;
    /// The entry's key `(min << 32) | max`.
    fn key(self) -> u64;
}

impl ListKey for u64 {
    #[inline]
    fn from_key(key: u64) -> Self {
        key
    }

    #[inline]
    fn key(self) -> u64 {
        self
    }
}

impl ListKey for (BodyIndex, BodyIndex) {
    #[inline]
    fn from_key(key: u64) -> Self {
        (BodyIndex((key >> 32) as u32), BodyIndex(key as u32))
    }

    #[inline]
    fn key(self) -> u64 {
        pair_key(self.0.0, self.1.0)
    }
}

/// A `Rows` step's translation of one persistent list through the inverse map `inv` (the
/// jumpers marked): an entry with an endpoint no row carried is dropped; a non-jumper entry
/// greater than the last kept one stays in place; every other entry is diverted to `sort_a`,
/// sorted and merged back (the patch rule of ruling W1). Returns the diverted count.
fn translate_list<K: ListKey>(
    list_view: &mut ScratchBuildView<'_, K>,
    inv: &[u32],
    sort_a: &mut ScratchColumn<u64>,
) -> u64 {
    let old_len = inv.len();
    let list = list_view.as_mut_slice();
    let mut diverted = sort_a.build_view();
    diverted.clear();
    let mut w = 0usize;
    let mut last_kept = 0u64;
    for i in 0..list.len() {
        let key = list[i].key();
        let (a, b) = ((key >> 32) as usize, (key & 0xffff_ffff) as usize);
        let na = if a < old_len { inv[a] } else { NO_ROW };
        let nb = if b < old_len { inv[b] } else { NO_ROW };
        if na == NO_ROW || nb == NO_ROW {
            continue;
        }
        let jumper = (na | nb) & JUMPER != 0;
        let (na, nb) = (na & !JUMPER, nb & !JUMPER);
        let new_key = pair_key(na.min(nb), na.max(nb));
        if !jumper && new_key > last_kept {
            list[w] = K::from_key(new_key);
            w += 1;
            last_kept = new_key;
        } else {
            debug_assert!(jumper, "a non-jumper entry is in order by construction");
            diverted.push(new_key);
        }
    }
    list_view.truncate(w);
    let a = diverted.len();
    if a == 0 {
        return 0;
    }
    let diverted = diverted.as_mut_slice();
    diverted.sort_unstable();
    merge_into_sorted(list_view, diverted, a > w / 4);
    debug_assert!(strictly_sorted(list_view.as_slice()), "a list is strictly sorted after a patch");
    a as u64
}

/// The three trees a Q row queries.
#[derive(Clone, Copy)]
struct Trees<'a> {
    active: &'a PackedBvh8,
    statics: &'a PackedBvh8,
    sleepers: &'a PackedBvh8,
}

/// The per-row walk of the Q row in active leaf slot `slot` (C1's query kernel): one walk of
/// each tree, the active one filtered to `row > query`, appended to the stream as the row's
/// sorted segment, and the row's record. An empty sleeper tree ends its walk at once.
#[inline]
fn row_walk_slot(
    trees: Trees<'_>,
    recs: &mut [RowRec],
    stream: &mut ScratchBuildView<'_, u32>,
    slot: u32,
) {
    let Trees { active, statics, sleepers } = trees;
    let leaf = active.leaf(slot);
    let row = leaf.row;
    debug_assert_ne!(row, NO_LANE_ROW, "the active tree has no dead lane");
    let seg = stream.len();
    active.query(leaf.x, leaf.y, leaf.z, leaf.r, |t| {
        if t > row {
            stream.push(t);
        }
    });
    statics.query(leaf.x, leaf.y, leaf.z, leaf.r, |t| {
        stream.push(t);
    });
    sleepers.query(leaf.x, leaf.y, leaf.z, leaf.r, |t| {
        stream.push(t);
    });
    let segment = &mut stream.as_mut_slice()[seg..];
    sort_segment(segment);
    let nrev = segment.partition_point(|&t| t < row);
    debug_assert!(segment.get(nrev).is_none_or(|&t| t > row), "a row is not its own partner");
    let rec = &mut recs[row as usize];
    rec.seg = seg as u32;
    rec.nrev = nrev as u32;
    rec.nfwd = (segment.len() - nrev) as u32;
}

/// The leaf-list query of every Q row (module docs, "Query kernels"; design C3b, F1). Per
/// active leaf node `L`, in slot order: one collection walk of each tree with `L`'s box; then
/// per live lane of `L`, ascending, the row's tests — the static candidates (or the static
/// tree's one leaf node, which the per-row walk also tests without a cull), the sleeper
/// candidates the same way, then the active candidates with the max-row cut, the exact test on
/// each kept leaf with `row > query` in the active mask — and the segment's sort and record,
/// exactly as the per-row walk writes them.
///
/// An empty sleeper tree is neither collected nor counted, so without the sleep-skip the pass —
/// its stream, its records and its [`LeafListCounts`] — is C3b's. The sleeper list's own figures
/// are not counted; its partners enter `emitted` and `sort_shifts` with the segment.
///
/// The stream is written by index into its initialised length, which a step leaves at least
/// at the previous step's length: before a leaf whose worst case (`live · 8` entries per
/// candidate) could pass it, the stream is grown out of line, the only call on the path. At the
/// end it is cut to the written length, and the Wide loop appends after it.
///
/// `#[inline(never)]`: one call per step, and a symbol of its own keeps its registers apart from
/// the rest of the step and gives the codegen receipt (G-LL7) something to read.
#[inline(never)]
fn leaf_list_pass(
    trees: Trees<'_>,
    recs: &mut [RowRec],
    stream: &mut ScratchBuildView<'_, u32>,
    cap: usize,
    diag: &mut TreeDiag,
    #[cfg(any(test, feature = "bp-query-counts"))] counts: &mut LeafListCounts,
) {
    let Trees { active, statics, sleepers } = trees;
    debug_assert!(active.maxrow_valid(), "invariant: the active tree is not killed or re-rowed after its build");
    debug_assert_eq!(active.dead(), 0, "the active tree has no dead lane");
    #[cfg(any(test, feature = "bp-query-counts"))]
    {
        *counts = LeafListCounts::default();
    }
    // The sleeper collection's box tests: walked like the static collection, not reported.
    #[cfg(any(test, feature = "bp-query-counts"))]
    let mut z_box_tests = 0u64;
    let mut act_slot = MaybeUninit::uninit();
    let mut sta_slot = MaybeUninit::uninit();
    let mut slp_slot = MaybeUninit::uninit();
    let act = CandList::init_in(&mut act_slot);
    let sta = CandList::init_in(&mut sta_slot);
    let slp = CandList::init_in(&mut slp_slot);
    let a_leaves = active.leaf_nodes();
    let s_leaves = statics.leaf_nodes();
    let z_leaves = sleepers.leaf_nodes();
    let s_single = statics.levels() == 1;
    let z_on = sleepers.levels() > 0;
    let z_single = sleepers.levels() == 1;
    let slots = active.leaves() as usize;
    let mut w = 0usize;
    let mut len = stream.len();
    let mut served = 0u64;
    for (l, node) in a_leaves.iter().enumerate() {
        let live = (slots - l * LANES).min(LANES);
        let box_l = active.leaf_box(l);
        let collected = active.collect_leaves::<true>(
            &box_l,
            act,
            cap,
            #[cfg(any(test, feature = "bp-query-counts"))]
            &mut counts.collect_box_active,
        ) && (s_single
            || statics.collect_leaves::<false>(
                &box_l,
                sta,
                cap,
                #[cfg(any(test, feature = "bp-query-counts"))]
                &mut counts.collect_box_static,
            ))
            && (!z_on
                || z_single
                || sleepers.collect_leaves::<false>(
                    &box_l,
                    slp,
                    cap,
                    #[cfg(any(test, feature = "bp-query-counts"))]
                    &mut z_box_tests,
                ));
        if !collected {
            w = fallback_leaf(trees, recs, stream, w, l, live);
            len = w;
            diag.fallback_leaves += 1;
            continue;
        }
        let z_count = if !z_on {
            0
        } else if z_single {
            1
        } else {
            slp.len()
        };
        let need = live * LANES * (act.len() + if s_single { 1 } else { sta.len() } + z_count);
        if w + need > len {
            len = grow_stream(stream, w + need.max(STREAM_GROW));
        }
        #[cfg(any(test, feature = "bp-query-counts"))]
        {
            counts.leaves += 1;
            counts.rows += live as u64;
            counts.cands_active += act.len() as u64;
            counts.cands_static += if s_single { 1 } else { sta.len() as u64 };
        }
        let out = stream.as_mut_slice();
        for k in 0..live {
            let (x, y, z, r) = (node.p[LEAF_X][k], node.p[LEAF_Y][k], node.p[LEAF_Z][k], node.p[LEAF_R][k]);
            let row = node.p[LEAF_ROW][k].to_bits();
            let q = QueryBox::of(x, y, z, r);
            let seg = w;
            #[cfg(any(test, feature = "bp-query-counts"))]
            {
                counts.prefilter_chunks += (act.chunks() + if s_single { 0 } else { sta.chunks() }) as u64;
            }
            if s_single {
                let s = &s_leaves[0];
                w = emit_lanes(out, w, s, leaf_mask(s, x, y, z, r));
                #[cfg(any(test, feature = "bp-query-counts"))]
                {
                    counts.kept_static += 1;
                }
            } else {
                sta.for_each_kept::<false>(&q, row, |leaf| {
                    let s = &s_leaves[leaf as usize];
                    w = emit_lanes(out, w, s, leaf_mask(s, x, y, z, r));
                    #[cfg(any(test, feature = "bp-query-counts"))]
                    {
                        counts.kept_static += 1;
                    }
                });
            }
            if z_single {
                let s = &z_leaves[0];
                w = emit_lanes(out, w, s, leaf_mask(s, x, y, z, r));
            } else if z_on {
                slp.for_each_kept::<false>(&q, row, |leaf| {
                    let s = &z_leaves[leaf as usize];
                    w = emit_lanes(out, w, s, leaf_mask(s, x, y, z, r));
                });
            }
            act.for_each_kept::<true>(&q, row, |leaf| {
                #[cfg(any(test, feature = "bp-query-counts"))]
                {
                    counts.kept_active += 1;
                }
                debug_assert!((leaf as usize) < a_leaves.len(), "a candidate is a leaf node of the tree");
                // SAFETY: `act` was filled by `active.collect_leaves` over this tree, unchanged
                // since, and holds leaf node indices only: `8·i + k` for a lane `k` of level-1
                // node `i` whose box met `box_l`, and `build` writes a finite box into lane `k`
                // only when `8·i + k < count[0]` (an empty lane's `+inf / −inf` box meets no
                // finite box, and `box_l` is a Normal row's, finite), or `0` in a one-level tree.
                // A pad lane (leaf `0`) is never kept: its empty box meets no query box. So
                // `leaf < count[0] = a_leaves.len()`.
                let a = unsafe { a_leaves.get_unchecked(leaf as usize) };
                w = emit_lanes(out, w, a, leaf_mask_above(a, x, y, z, r, row));
            });
            let segment = &mut out[seg..w];
            #[cfg(any(test, feature = "bp-query-counts"))]
            {
                counts.emitted += segment.len() as u64;
                counts.sort_shifts += inversions(segment);
            }
            sort_leaf_list_segment(segment);
            let nrev = segment.partition_point(|&t| t < row);
            debug_assert!(segment.get(nrev).is_none_or(|&t| t > row), "a row is not its own partner");
            let rec = &mut recs[row as usize];
            rec.seg = seg as u32;
            rec.nrev = nrev as u32;
            rec.nfwd = (segment.len() - nrev) as u32;
        }
        served += 1;
    }
    stream.truncate(w);
    diag.leaf_list_leaves += served;
}

/// The inversions of `segment`: the shifts its insertion sort performs.
#[cfg(any(test, feature = "bp-query-counts"))]
fn inversions(segment: &[u32]) -> u64 {
    let mut count = 0u64;
    for (i, &a) in segment.iter().enumerate() {
        count += segment[i + 1..].iter().filter(|&&b| b < a).count() as u64;
    }
    count
}

/// Writes the row of every lane of the leaf `node` set in `mask` at `out[w..]`, ascending;
/// returns the new write index.
#[inline]
fn emit_lanes(out: &mut [u32], mut w: usize, node: &Node8, mut mask: u32) -> usize {
    while mask != 0 {
        let k = mask.trailing_zeros() as usize;
        mask &= mask - 1;
        out[w] = node.p[LEAF_ROW][k].to_bits();
        w += 1;
    }
    w
}

/// The leaf-list query's fallback: the `live` rows of active leaf node `l` through the per-row
/// walk, the stream cut to `w` first. Returns the new write index, which is also the stream's
/// length.
#[cold]
#[inline(never)]
fn fallback_leaf(
    trees: Trees<'_>,
    recs: &mut [RowRec],
    stream: &mut ScratchBuildView<'_, u32>,
    w: usize,
    l: usize,
    live: usize,
) -> usize {
    stream.truncate(w);
    for k in 0..live {
        row_walk_slot(trees, recs, stream, (l * LANES + k) as u32);
    }
    stream.len()
}

/// Grows the stream to `len` entries, past its length (the new tail zero-filled); returns
/// `len`.
#[cold]
#[inline(never)]
fn grow_stream(stream: &mut ScratchBuildView<'_, u32>, len: usize) -> usize {
    debug_assert!(len > stream.len(), "a grow grows");
    stream.resize(len, 0);
    stream.len()
}

/// Sorts one row's segment: insertion sort up to [`INSERTION_SORT_MAX`] entries, the unstable
/// sort above (out of line: it is rare, and a call kept out of the query loops keeps them
/// compact).
#[inline]
fn sort_segment(segment: &mut [u32]) {
    if segment.len() <= INSERTION_SORT_MAX {
        for i in 1..segment.len() {
            let v = segment[i];
            let mut j = i;
            while j > 0 && segment[j - 1] > v {
                segment[j] = segment[j - 1];
                j -= 1;
            }
            segment[j] = v;
        }
    } else {
        sort_long_segment(segment);
    }
}

/// Sorts one leaf-list segment: the network up to `kernel::NETWORK_SORT_MAX` entries (C3b, F2),
/// [`sort_segment`] above. The per-row walk keeps [`sort_segment`] for every length, so it
/// stays the kernel C1 shipped.
#[inline]
fn sort_leaf_list_segment(segment: &mut [u32]) {
    if segment.len() <= NETWORK_SORT_MAX {
        sort_network(segment);
    } else {
        sort_segment(segment);
    }
}

/// The unstable sort of a segment longer than [`INSERTION_SORT_MAX`].
#[cold]
#[inline(never)]
fn sort_long_segment(segment: &mut [u32]) {
    segment.sort_unstable();
}

/// Merges the sorted `added` keys into the sorted `list` (disjoint keys), in place from the end.
/// With `sort_whole` the list is appended to and sorted instead (the patch rule's fallback when
/// the diverted run is large).
fn merge_into_sorted<K: ListKey>(list: &mut ScratchBuildView<'_, K>, added: &[u64], sort_whole: bool) {
    let k = list.len();
    let a = added.len();
    list.resize(k + a, K::from_key(0));
    let out = list.as_mut_slice();
    if sort_whole {
        for (slot, &key) in out[k..].iter_mut().zip(added) {
            *slot = K::from_key(key);
        }
        out.sort_unstable();
        return;
    }
    let (mut i, mut j) = (k, a);
    let mut w = k + a;
    while j > 0 {
        w -= 1;
        if i > 0 && out[i - 1].key() > added[j - 1] {
            i -= 1;
            out[w] = out[i];
        } else {
            j -= 1;
            out[w] = K::from_key(added[j]);
        }
    }
}

/// Marks the jumpers of the inverse map `inv` (old row → new row of every carried member, or
/// [`NO_ROW`]) with [`JUMPER`]: the carried rows outside the maximum-weight subsequence of
/// displacement runs whose new rows increase.
///
/// Consecutive carried rows with the same displacement `new − old` form a run, which is
/// monotone by itself; runs are chosen by a weighted longest-increasing-subsequence over their
/// `(first, last)` new rows, `O(R²)` for `R` runs. A single moved row is its own run of weight
/// one and loses to the runs around it; a block that moved as one loses to the rest. Past
/// [`MAX_RUNS`] every carried row is a jumper and the whole list is sorted.
fn mark_jumpers(inv: &mut [u32]) {
    // (first new row, last new row, first old row, weight)
    let mut runs = [(0u32, 0u32, 0u32, 0u32); MAX_RUNS];
    let mut count = 0usize;
    let mut last_d = 0i64;
    for (m, &v) in inv.iter().enumerate() {
        if v == NO_ROW {
            continue;
        }
        let d = i64::from(v) - m as i64;
        if count > 0 && d == last_d {
            runs[count - 1].1 = v;
            runs[count - 1].3 += 1;
        } else {
            if count == MAX_RUNS {
                for v in inv.iter_mut() {
                    if *v != NO_ROW {
                        *v |= JUMPER;
                    }
                }
                return;
            }
            runs[count] = (v, v, m as u32, 1);
            count += 1;
            last_d = d;
        }
    }
    if count <= 1 {
        return;
    }
    // best[j]: the heaviest increasing chain ending at run j; prev[j]: its predecessor.
    let mut best = [0u64; MAX_RUNS];
    let mut prev = [usize::MAX; MAX_RUNS];
    let mut top = 0usize;
    for j in 0..count {
        best[j] = u64::from(runs[j].3);
        for i in 0..j {
            if runs[i].1 < runs[j].0 && best[i] + u64::from(runs[j].3) > best[j] {
                best[j] = best[i] + u64::from(runs[j].3);
                prev[j] = i;
            }
        }
        if best[j] > best[top] {
            top = j;
        }
    }
    // Mark every run as a jumper, then unmark the chain.
    let mut chosen = [false; MAX_RUNS];
    let mut j = top;
    loop {
        chosen[j] = true;
        if prev[j] == usize::MAX {
            break;
        }
        j = prev[j];
    }
    let mut run = 0usize;
    let mut left = 0u32;
    for (m, v) in inv.iter_mut().enumerate() {
        if *v == NO_ROW {
            continue;
        }
        if left == 0 {
            debug_assert_eq!(runs[run].2 as usize, m, "runs tile the carried rows in order");
            left = runs[run].3;
            run += 1;
        }
        left -= 1;
        if !chosen[run - 1] {
            *v |= JUMPER;
        }
    }
}

/// Merges row `row`'s forward run, `SS` run (keys with `min == row`) and bucket into `out` at
/// `w`, all three sorted and disjoint; returns the new write index.
#[inline]
fn merge_row(
    row: u32,
    fwd: &[u32],
    ss_run: &[u64],
    bucket: &[u32],
    out: &mut [(BodyIndex, BodyIndex)],
    mut w: usize,
) -> usize {
    let (mut i, mut j, mut k) = (0usize, 0usize, 0usize);
    let a = BodyIndex(row);
    loop {
        let f = fwd.get(i).copied().unwrap_or(u32::MAX);
        let s = ss_run.get(j).map_or(u32::MAX, |&key| key as u32);
        let b = bucket.get(k).copied().unwrap_or(u32::MAX);
        let m = f.min(s).min(b);
        if m == u32::MAX {
            break;
        }
        if m == f {
            i += 1;
        } else if m == s {
            j += 1;
        } else {
            k += 1;
        }
        debug_assert!(m > row, "a pair's larger row is above its smaller row");
        out[w] = (a, BodyIndex(m));
        w += 1;
    }
    w
}

/// The one sphere-bound predicate of the broadphase: `|pb − pa|² ≤ (ra + rb)²`, the expression
/// of the shipped all-pairs loop. The Grid delegates to it; the Tree's kernel evaluates the same
/// expression 8-wide.
#[inline]
pub fn sphere_bound_feasible(pa: Vec3, ra: f32, pb: Vec3, rb: f32) -> bool {
    let bound = ra + rb;
    let delta = pb - pa;
    delta.length_squared() <= bound * bound
}

/// The brute all-pairs loop into `out`: the Tree's path below `brute_max_rows`, and the oracle
/// the gates compare every step against. The same expression, loop shape and emit order as the
/// `AllPairs` arm of `physics_broadphase`.
pub fn all_pairs_into(bodies: &[BodyState], out: &mut ContactPairs) {
    let mut pairs = out.pairs_build();
    pairs.clear();
    let n = bodies.len();
    for i in 0..n {
        for j in (i + 1)..n {
            let bound = body_bounding_radius(&bodies[i]) + body_bounding_radius(&bodies[j]);
            let delta = bodies[j].position - bodies[i].position;
            if delta.length_squared() <= bound * bound {
                pairs.push((BodyIndex(i as u32), BodyIndex(j as u32)));
            }
        }
    }
}
