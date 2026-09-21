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
//! A Normal row is in one of **Q** (queried; the active tree, rebuilt every step) or **S** (the
//! static set: a tree and a sorted list `SS` of its internal pairs, both persistent). One verify
//! pass per step locates every row's previous record — in place on `Identity`, through
//! `prev_row` on `Rows`, in place after a `Reset` — and decides:
//! * *carried*: a member whose bits, kind and class are unchanged keeps its leaf;
//! * *evicted*: a member that moved, reshaped, changed class or kind, or was located twice
//!   (the consumed mark makes the locator injective) — its leaf is killed and its pairs filtered;
//! * *vanished*: a member no row located (despawned, or a tail row past `N`) — found by the
//!   count, killed by a leaf scan;
//! * *pending*: a non-member static that is still (bits equal to its record) accrues rent.
//!
//! Correctness rests on the bit compare, the injective locator and the filtered lists alone
//! (Lemma D3.1); the class check is membership hygiene. A row move is a **translation** of the
//! list entries and leaf rows through `prev_row`'s inverse, with the patch rule of ruling W1:
//! an entry stays in place only if both its endpoints belong to the maximum-weight monotone
//! subsequence of the carried rows (the "jumpers" are the rest) and it is greater than the last
//! kept entry; the diverted entries are sorted and merged back. A new static waits as pending
//! and is admitted by the rent rule (D3.5, a deterministic ski-rental bound): the tree is rebuilt
//! over members and pending rows, only the pending rows are queried, their additions are merged
//! into `SS`. Compaction rebuilds the tree when the dead lanes reach `max(live, 64)`.
//!
//! The sleeper set Z, its list SL, the sleep hint and `IslandSleep::mask_rows` are commit C5 of
//! the design and are not here.
//!
//! # A step (N > `brute_max_rows`)
//!
//! | zone | what |
//! |---|---|
//! | `phys_bp_verify` | the locator, the verify pass, maintenance (leaf scan, list pass, compaction, admission) — unconditionally, once per step; the cursor stamp follows the assembly, outside the zones |
//! | `phys_bp_build` | the active tree over Q |
//! | `phys_bp_query` | each Q row, in Morton order, against the active tree (`row > query`) and the static tree; then each Wide row's loop. Every row's partners form one segment `[< row \| > row]` in one stream |
//! | `phys_bp_assemble` | rev counts → bucket starts → a scatter over rows ascending → `resize(P)` → a per-row merge of its forward run, its `SS` run and its bucket |
//!
//! The counters `phys_bp_queried` (`|Q| + |Wide|`), `phys_bp_members` (`|S|`) and
//! `phys_bp_rebuilds` (the step's admissions and compactions) are emitted once per such step.
//! With `N ≤ brute_max_rows` the step is [`all_pairs_into`], the state is untouched and the
//! cursor is not stamped, so the next tree step is a `Reset`.
//!
//! # Storage
//!
//! Every durable buffer is a [`ScratchColumn`] on the `BROADPHASE_TREE` cohort of
//! `scratch_ids.rs` (ids 399..389). Function-local scratch is the traversal stack and the radix
//! histogram. No `Vec`, no pool, no atomics.

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
    TREE_ACTIVE, TREE_AUX, TREE_ITEMS, TREE_REC0, TREE_REC1, TREE_SORT_A, TREE_SORT_B, TREE_SS,
    TREE_STATICS, register_tree_column_layouts, scratch_reserve_rows, tree_column_id,
};
use crate::systems::body_bounding_radius;

use self::bvh::{Item, NO_LANE_ROW, Node8, PackedBvh8};

pub(crate) mod bvh;
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

const TAG_SLOT_MASK: u32 = 0x00ff_ffff;
const TAG_SET_SHIFT: u32 = 24;
const TAG_KIND_SHIFT: u32 = 26;
const TAG_CONSUMED: u32 = 1 << 29;
const TAG_PENDING: u32 = 1 << 30;

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
    /// `slot:24 | set:2 | kind:2 | _:1 | consumed:1 | pending:1 | _:1`.
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
    /// Admissions and compactions of the sleeper set (commit C5; `0` until then).
    pub sleeper_rebuilds: u64,
    /// Members evicted (moved, reshaped, re-classed, re-kinded, located twice) or vanished.
    pub evictions: u64,
    /// `Rows` steps on which at least one member was translated.
    pub translations: u64,
    /// List entries the patch rule diverted and merged back.
    pub patches: u64,
    /// Rows the sleep hint offered as sleeper candidates (commit C5; `0` until then).
    pub hint_candidates: u64,
    /// Row-steps classed Wide.
    pub wide_rows: u64,
    /// Row-steps classed Excluded.
    pub excluded_rows: u64,
    /// `Reset` classifications of a cursor that had been stamped.
    pub locator_resets: u64,
    /// The current `|S| + |Z|`.
    pub members: u64,
}

/// The tree broadphase's state: the active and the static tree, two record buffers, the static
/// pair list, the scratch stream and the row cursor (the sleeper tree and its list are commit
/// C5's; their column ids are reserved). One per world; see the module docs.
#[derive(Resource)]
pub struct BroadphaseTree {
    /// The tree over Q, rebuilt every step.
    active: PackedBvh8,
    /// The tree over S.
    statics: PackedBvh8,
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
    /// The static set's rent.
    rent: u64,
    /// Rows at or below which the brute loop runs.
    brute_max_rows: u32,
    /// `|S|`.
    members: u32,
    /// This step's pending static rows (the verify writes, the maintenance reads).
    pending_step: u32,
    /// The verify saw a vanished member, or the rows changed with members: scan the leaves.
    needs_scan: bool,
    /// The verify evicted or lost a member: filter the list.
    needs_filter: bool,
    /// A `Rows` step with at least one member: translate leaves and list.
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
            rent: 0,
            brute_max_rows: TREE_BRUTE_MAX_ROWS,
            members: 0,
            pending_step: 0,
            needs_scan: false,
            needs_filter: false,
            rows_changed: false,
            diag: TreeDiag::default(),
        }
    }

    /// The structural counters.
    #[inline]
    pub fn diag(&self) -> TreeDiag {
        TreeDiag {
            locator_resets: self.cursor.resets(),
            members: u64::from(self.members),
            ..self.diag
        }
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
            all_pairs_into(bodies, out);
            return;
        }
        self.run(bodies, RowRemap::Identity, out);
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
            all_pairs_into(bodies, out);
            return;
        }
        self.run(bodies, RowRemap::Rows(prev_row), out);
    }

    /// One step: fills `out` with the exact pair set of `bodies`, in `(min, max)` order.
    ///
    /// `rows` is the gather's row identity, which locates the records of the previous step
    /// when the rows changed; a never-gathered identity (direct drive) classifies as
    /// `Identity` on every step.
    pub(crate) fn step(
        &mut self,
        bodies: &[BodyState],
        rows: &RowIdentity,
        out: &mut ContactPairs,
    ) {
        let n = bodies.len();
        debug_assert!(n < 1 << 24, "invariant: rows stay below 2^24");
        if n <= self.brute_max_rows as usize {
            all_pairs_into(bodies, out);
            return;
        }
        let remap = self.cursor.remap(rows);
        self.run(bodies, remap, out);
        // Protocol P: the state is keyed by this gather's rows from here on. Once per tree-path
        // step, after the maintenance, whatever the locator was (ruling W2(a)).
        self.cursor.stamp(rows);
    }

    /// The tree path with an explicit locator and no stamp: [`step`](Self::step) minus the
    /// cursor, so a test can drive a hand-made `Rows` map.
    fn run(&mut self, bodies: &[BodyState], remap: RowRemap<'_>, out: &mut ContactPairs) {
        let n = bodies.len();
        let rebuilds_before = self.diag.static_rebuilds + self.diag.sleeper_rebuilds;

        {
            let _zone = zone!(PHYS_BP_VERIFY);
            match remap {
                RowRemap::Identity | RowRemap::Reset => self.verify_identity(bodies),
                RowRemap::Rows(prev_row) => self.verify_rows(bodies, prev_row),
            }
            self.maintain(n);
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
        counter!(PHYS_BP_MEMBERS, u64::from(self.members));
        counter!(
            PHYS_BP_REBUILDS,
            self.diag.static_rebuilds + self.diag.sleeper_rebuilds - rebuilds_before
        );
    }

    // ── The verify ───────────────────────────────────────────────────────────

    /// The verify with the identity locator (`Identity`, `Reset`, direct drive): row `r`'s
    /// record is `rec[cur][r]`, updated in place. Evicted lanes are killed here; vanished ones
    /// (rows past `N`) by the scan in [`maintain`](Self::maintain).
    fn verify_identity(&mut self, bodies: &[BodyState]) {
        let n = bodies.len();
        let Self { rec, cur, statics, diag, .. } = self;
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
            let mut tag = kind << TAG_KIND_SHIFT;
            if old.set() == SET_S {
                if kind == KIND_NORMAL && still && is_static {
                    counts.carried += 1;
                    tag |= (SET_S << TAG_SET_SHIFT) | old.slot();
                } else {
                    counts.evicted += 1;
                    statics.kill(old.slot());
                }
            } else if kind == KIND_NORMAL && is_static && still {
                counts.pending += 1;
                tag |= TAG_PENDING;
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
    /// carried members is built in `aux` for the leaf scan and the list translation.
    fn verify_rows(&mut self, bodies: &[BodyState], prev_row: &[u32]) {
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
            let mut tag = kind << TAG_KIND_SHIFT;
            if old.set() == SET_S {
                if kind == KIND_NORMAL && still && is_static {
                    counts.carried += 1;
                    tag |= (SET_S << TAG_SET_SHIFT) | old.slot();
                    inv[m as usize] = r as u32;
                } else {
                    counts.evicted += 1;
                }
            } else if kind == KIND_NORMAL && is_static && still {
                counts.pending += 1;
                tag |= TAG_PENDING;
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

    /// Closes a verify: the vanished count, the eviction counter and the structural flags the
    /// maintenance reads.
    fn finish_verify(&mut self, counts: VerifyCounts, rows_changed: bool) {
        let vanished = self.members - counts.carried - counts.evicted;
        self.diag.evictions += u64::from(counts.evicted + vanished);
        self.pending_step = counts.pending;
        self.needs_scan = vanished > 0 || (rows_changed && self.members > 0);
        self.needs_filter = counts.evicted + vanished > 0;
        self.rows_changed = rows_changed && self.members > 0;
        self.members = counts.carried;
    }

    // ── Maintenance ──────────────────────────────────────────────────────────

    /// Everything a step does to the static set after the verify: the leaf scan (translation
    /// and kills), the list pass (translation with the patch rule, or a filter), compaction and
    /// admission. Runs only on steps with a change; out of line so the verify stays compact.
    #[cold]
    #[inline(never)]
    fn maintain(&mut self, n: usize) {
        if self.rows_changed {
            self.diag.translations += 1;
            self.translate();
        } else {
            if self.needs_scan {
                self.scan_leaves(n);
            }
            if self.needs_filter {
                self.filter_list(n);
            }
        }
        self.needs_scan = false;
        self.needs_filter = false;
        self.rows_changed = false;

        let pending = u64::from(self.pending_step);
        if pending == 0 {
            self.rent = 0;
        } else {
            self.rent += pending;
            let (num, den) = ADMIT_BUILD_RATIO;
            if self.rent * den >= pending * den + num * (u64::from(self.members) + pending) {
                self.admit(n);
                self.rent = 0;
            }
        }
        if self.statics.dead() >= self.statics.live().max(COMPACT_MIN_DEAD) {
            self.compact();
        }
        self.debug_check_members(n);
    }

    /// Kills the lanes whose row is past `N` (a shrink under the identity locator).
    fn scan_leaves(&mut self, n: usize) {
        for slot in 0..self.statics.leaves() {
            let row = self.statics.leaf_row(slot);
            if row != NO_LANE_ROW && row as usize >= n {
                self.statics.kill(slot);
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

    /// A `Rows` step's translation: the leaf rows and the list entries through the inverse map,
    /// dropping the members no row carried, with the patch rule of ruling W1.
    fn translate(&mut self) {
        let Self { statics, ss, aux, sort_a, diag, .. } = self;
        let mut inv_view = aux.build_view();
        let inv = inv_view.as_mut_slice();
        let old_len = inv.len();

        // Leaves: a carried member's lane takes its new row; every other live lane is killed.
        for slot in 0..statics.leaves() {
            let row = statics.leaf_row(slot);
            if row == NO_LANE_ROW {
                continue;
            }
            let new_row = if (row as usize) < old_len { inv[row as usize] } else { NO_ROW };
            if new_row == NO_ROW {
                statics.kill(slot);
            } else {
                statics.set_leaf_row(slot, new_row);
            }
        }

        // The jumpers: carried rows outside the maximum-weight monotone run subsequence.
        mark_jumpers(inv);

        // The list: translate, drop, keep or divert.
        let mut list_view = ss.build_view();
        let list = list_view.as_mut_slice();
        let mut diverted = sort_a.build_view();
        diverted.clear();
        let mut w = 0usize;
        let mut last_kept = 0u64;
        for i in 0..list.len() {
            let key = list[i];
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
                list[w] = new_key;
                w += 1;
                last_kept = new_key;
            } else {
                debug_assert!(jumper, "a non-jumper entry is in order by construction");
                diverted.push(new_key);
            }
        }
        list_view.truncate(w);
        let a = diverted.len();
        diag.patches += a as u64;
        if a == 0 {
            return;
        }
        let diverted = diverted.as_mut_slice();
        diverted.sort_unstable();
        merge_into_sorted(&mut list_view, diverted, a > w / 4);
        debug_assert!(strictly_sorted(list_view.as_slice()), "SS is strictly sorted after a patch");
    }

    /// Admits the pending rows into S: the tree is rebuilt over members and pending rows, each
    /// pending row is queried against it (a partner is an old member, or a pending row above
    /// it), and the sorted additions are merged into `SS`.
    fn admit(&mut self, n: usize) {
        let Self { statics, items, sort_a, sort_b, rec, cur, ss, diag, members, .. } = self;
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

        let mut additions = sort_a.build_view();
        additions.clear();
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
        *members = statics.leaves();
        diag.static_rebuilds += 1;
    }

    /// Rebuilds the static tree over its live lanes only. The list is already filtered.
    fn compact(&mut self) {
        let Self { statics, items, sort_a, sort_b, rec, cur, diag, .. } = self;
        let mut items_view = items.build_view();
        items_view.clear();
        for slot in 0..statics.leaves() {
            let leaf = statics.leaf(slot);
            if leaf.row != NO_LANE_ROW {
                items_view.push(Item::new(leaf.x, leaf.y, leaf.z, leaf.r, leaf.row));
            }
        }
        statics.build(items_view.as_slice(), sort_a, sort_b);
        let mut recs_view = rec[usize::from(*cur)].build_view();
        let recs = recs_view.as_mut_slice();
        for slot in 0..statics.leaves() {
            let row = statics.leaf_row(slot) as usize;
            recs[row].join(SET_S, slot);
        }
        diag.static_rebuilds += 1;
    }

    /// Debug-only: every member's leaf points back at its row, no Wide or Excluded row is in a
    /// tree, the live lane count is the member count, and `SS` is strictly sorted.
    #[inline]
    fn debug_check_members(&self, n: usize) {
        if cfg!(debug_assertions) {
            let recs = self.rec[usize::from(self.cur)].as_read_slice();
            let mut live = 0u32;
            for (r, rec) in recs.iter().enumerate().take(n) {
                if rec.set() == SET_S {
                    live += 1;
                    assert_eq!(rec.kind(), KIND_NORMAL, "row {r}: a member is Normal");
                    assert_eq!(
                        self.statics.leaf_row(rec.slot()),
                        r as u32,
                        "row {r}: its leaf slot points back at it"
                    );
                }
            }
            assert_eq!(live, self.members, "the member count is the carried count");
            assert_eq!(live, self.statics.live(), "live lanes equal members");
            assert!(strictly_sorted(self.ss.as_read_slice()), "SS is strictly sorted");
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

    /// Queries every Q row (Morton order) and loops every Wide row (row order), appending each
    /// row's segment to the stream. Returns `|Q| + |Wide|`.
    fn query_all(&mut self, n: usize) -> u64 {
        let Self { active, statics, rec, cur, aux, .. } = self;
        let mut recs_view = rec[usize::from(*cur)].build_view();
        let recs = recs_view.as_mut_slice();
        let mut stream = aux.build_view();
        stream.clear();

        for slot in 0..active.leaves() {
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
            let segment = &mut stream.as_mut_slice()[seg..];
            sort_segment(segment);
            let nrev = segment.partition_point(|&t| t < row);
            debug_assert!(segment.get(nrev).is_none_or(|&t| t > row), "a row is not its own partner");
            let rec = &mut recs[row as usize];
            rec.seg = seg as u32;
            rec.nrev = nrev as u32;
            rec.nfwd = (segment.len() - nrev) as u32;
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

/// Per-verify tallies.
#[derive(Default)]
struct VerifyCounts {
    carried: u32,
    evicted: u32,
    pending: u32,
    wide: u64,
    excluded: u64,
}

impl VerifyCounts {
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
fn strictly_sorted(keys: &[u64]) -> bool {
    keys.windows(2).all(|w| w[0] < w[1])
}

/// Sorts one row's segment: insertion sort up to [`INSERTION_SORT_MAX`] entries, the unstable
/// sort above.
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
        segment.sort_unstable();
    }
}

/// Merges the sorted `added` keys into the sorted `list` (disjoint keys), in place from the end.
/// With `sort_whole` the list is appended to and sorted instead (the patch rule's fallback when
/// the diverted run is large).
fn merge_into_sorted(list: &mut ScratchBuildView<'_, u64>, added: &[u64], sort_whole: bool) {
    let k = list.len();
    let a = added.len();
    list.resize(k + a, 0);
    let out = list.as_mut_slice();
    if sort_whole {
        out[k..].copy_from_slice(added);
        out.sort_unstable();
        return;
    }
    let (mut i, mut j) = (k, a);
    let mut w = k + a;
    while j > 0 {
        w -= 1;
        if i > 0 && out[i - 1] > added[j - 1] {
            i -= 1;
            out[w] = out[i];
        } else {
            j -= 1;
            out[w] = added[j];
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
