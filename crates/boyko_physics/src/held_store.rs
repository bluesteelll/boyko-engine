//! L10's held store (design `04-DESIGN-REV2.md` D5, `06-DESIGN-REV2.2.md` B2–B4,
//! `08-DESIGN-REV2.3.md` A3′): what a held island keeps while its pairs skip the narrowphase, its
//! rows skip the graph's colouring and the solve, and every public view still reports it.
//!
//! One **record** ([`HeldIsland`], 48 B) per held island, and per record three contiguous runs:
//!
//! * its **row table** — the members ascending, then the anchors (every non-member partner of a
//!   kept manifold or kept pair) ascending, all in the CURRENT gather's rows. A Rows step
//!   translates only these (4 B per row), which is why the kept data below is island-local;
//! * its **kept manifolds** ([`KeptManifold`]) — the island's solver manifolds as `Off` computes
//!   them, `body_a` / `body_b` holding row-table indices (the SDF sentinel kept as is), sorted by
//!   the canonical ordinal the stream is sorted by;
//! * its **kept pairs** ([`KeptPair`]) — every pair with a member endpoint whose `Off` state is a
//!   reuse record, a separating axis or a manifold (design 06 B2), with its tag and, for a `REC`
//!   pair, its reuse record in a fourth run. A pair between two held islands is kept by both
//!   (Invariant K, design 06 B3).
//!
//! Beside them, `of_row` maps a current row to its record, and `order` lists every live kept
//! manifold by ordinal, so a logical view merges the stream and the store with no per-step work.
//!
//! A restored record is tombstoned (`DEAD`) and its data stays readable until a later step's
//! compaction (design 06 B3: a move-in of the same step may read a tombstoned record's copy of a
//! cross pair), which runs at the start of a broadphase once the dead slots outnumber the live
//! ones.
//!
//! Every column is a kernel [`ScratchColumn`] owned by [`Manifolds`](crate::resources::Manifolds)
//! (principle 0); the store is written by the broadphase alone and read by the narrowphase, the
//! graph, the solve and the views. No heap allocation on any step.

use boyko_ecs::ecs::core::component::scratch::ScratchColumn;

use crate::manifold::{BodyIndex, Manifold, SDF_SENTINEL};
use crate::narrowphase::carry::PairTag;
use crate::narrowphase::reuse::ReuseRecord;
use crate::resources::HELD_BASE;
use crate::scratch_ids::{
    held_kept_id, held_kept_pair_id, held_kept_reuse_id, held_of_row_id, held_order_id,
    held_records_id, held_rows_id, register_held_store_layouts, scratch_reserve_rows,
};
use crate::solver::warm_records::ord;

/// "No record", "no row", "no slot".
pub(crate) const NONE: u32 = u32::MAX;

/// One held island (design 04 D5, 06 §2, 08 O6). 48 B, every field a `u32`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HeldIsland {
    /// The island's union-find root under `Off` (a member row, current rows), which the graph
    /// pre-roots the members under so the island keeps `Off`'s id (design 04 D7).
    pub(crate) root: u32,
    /// The first row of the record's row table.
    pub(crate) rows_start: u32,
    /// Members (dynamic rows of the island), the first `n_members` rows of the table.
    pub(crate) n_members: u32,
    /// Anchors, the rows after the members.
    pub(crate) n_anchors: u32,
    /// The first kept manifold slot.
    pub(crate) kept_start: u32,
    /// Kept manifolds: the island's manifold count under `Off`, which `island_len` adds.
    pub(crate) kept_len: u32,
    /// Kept box-box manifolds (the D3 rule restores an island that keeps one on a clear).
    pub(crate) box_kept: u32,
    /// Live points of the kept manifolds.
    pub(crate) points: u32,
    /// [`DEAD`](Self::DEAD) and [`RESTORE`](Self::RESTORE).
    pub(crate) flags: u32,
    /// The first kept pair.
    pub(crate) pairs_start: u32,
    /// Kept pairs.
    pub(crate) pairs_len: u32,
    /// The first reuse record of the record's `REC` pairs (the word design 08 O6 left as `_p`).
    pub(crate) reuse_start: u32,
}

const _: () = assert!(
    size_of::<HeldIsland>() == 48 && align_of::<HeldIsland>() == 4,
    "HeldIsland is the 48 B, align 4 record of design 06 §2"
);

impl HeldIsland {
    /// The record was restored (tombstoned); its runs stay readable until compaction.
    pub(crate) const DEAD: u32 = 1 << 0;
    /// The record is marked for restore this step (cleared when it is tombstoned).
    pub(crate) const RESTORE: u32 = 1 << 1;

    /// Whether the record is live (not tombstoned).
    #[inline]
    pub(crate) fn live(&self) -> bool {
        self.flags & Self::DEAD == 0
    }

    /// The row-table range of the record.
    #[inline]
    pub(crate) fn rows_range(&self) -> core::ops::Range<usize> {
        let lo = self.rows_start as usize;
        lo..lo + (self.n_members + self.n_anchors) as usize
    }

    /// The members' row-table range.
    #[inline]
    pub(crate) fn members_range(&self) -> core::ops::Range<usize> {
        let lo = self.rows_start as usize;
        lo..lo + self.n_members as usize
    }

    /// The anchors' row-table range.
    #[inline]
    pub(crate) fn anchors_range(&self) -> core::ops::Range<usize> {
        let lo = (self.rows_start + self.n_members) as usize;
        lo..lo + self.n_anchors as usize
    }

    /// The kept manifold slots of the record.
    #[inline]
    pub(crate) fn kept_range(&self) -> core::ops::Range<usize> {
        let lo = self.kept_start as usize;
        lo..lo + self.kept_len as usize
    }

    /// The kept pairs of the record.
    #[inline]
    pub(crate) fn pairs_range(&self) -> core::ops::Range<usize> {
        let lo = self.pairs_start as usize;
        lo..lo + self.pairs_len as usize
    }
}

/// One kept manifold (design 04 D5): the manifold with `body_a` / `body_b` holding indices into
/// its record's row table (the SDF sentinel kept as is), and its record. 156 B.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct KeptManifold {
    /// The manifold, island-local.
    pub(crate) m: Manifold,
    /// The record that keeps it.
    pub(crate) record: u32,
}

const _: () = assert!(size_of::<KeptManifold>() == 156, "a kept manifold is a manifold and a u32");

/// One kept pair (design 06 §2): its two row-table indices in its tag's body order (`la` is the
/// lower current row, body A), its kept manifold and reuse record (record-relative, or
/// [`NONE`]), and its tag. 20 B.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct KeptPair {
    /// Row-table index of the pair's lower row (its body A).
    pub(crate) la: u32,
    /// Row-table index of the pair's higher row (its body B).
    pub(crate) lb: u32,
    /// The pair's kept manifold, relative to the record's first slot, or [`NONE`].
    pub(crate) m_slot: u32,
    /// The pair's reuse record, relative to the record's first one, or [`NONE`] (no `REC`).
    pub(crate) r_slot: u32,
    /// The pair's tag in `Off`'s steady state, `(la, lb)` roles.
    pub(crate) tag: PairTag,
    /// Zero.
    pub(crate) _p: u16,
}

const _: () = assert!(
    size_of::<KeptPair>() == 20 && align_of::<KeptPair>() == 4,
    "a kept pair is the 20 B, align 4 element of design 06 §2"
);

/// One entry of the sorts L10 runs in place (design 08 A3′ OQ2's `restore_sort`, the move-in's
/// ordinal merge, the compaction's `order` rebuild): a key and a slot. 16 B.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RestoreSort {
    /// The sort key.
    pub(crate) key: u64,
    /// The slot it names.
    pub(crate) slot: u32,
    /// A second word the sort carries along (a record index), else zero.
    pub(crate) aux: u32,
}

const _: () = assert!(size_of::<RestoreSort>() == 16, "a sort entry is 16 B (design 08 A3′)");

impl RestoreSort {
    /// The entry `(key, slot)`.
    #[inline]
    pub(crate) const fn new(key: u64, slot: u32) -> Self {
        Self { key, slot, aux: 0 }
    }
}

/// The read-only surface of the store: what the views, the graph, the narrowphase's mirror and
/// the solve read. `Copy`, shared slices only.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HeldView<'a> {
    records: &'a [HeldIsland],
    rows: &'a [u32],
    kept: &'a [KeptManifold],
    of_row: &'a [u32],
    order: &'a [u32],
    pairs: &'a [KeptPair],
    reuse: &'a [ReuseRecord],
    live_records: u32,
    live_kept: u32,
    live_points: u32,
}

impl<'a> HeldView<'a> {
    /// The view of an empty store.
    pub(crate) const EMPTY: Self = Self {
        records: &[],
        rows: &[],
        kept: &[],
        of_row: &[],
        order: &[],
        pairs: &[],
        reuse: &[],
        live_records: 0,
        live_kept: 0,
        live_points: 0,
    };

    /// Whether no island is held.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.live_records == 0
    }

    /// The records, live and dead.
    #[inline]
    pub(crate) fn records(&self) -> &'a [HeldIsland] {
        self.records
    }

    /// The row table entries `range` (a record's members, anchors or whole table).
    #[inline]
    pub(crate) fn rows(&self, range: core::ops::Range<usize>) -> &'a [u32] {
        &self.rows[range]
    }

    /// The kept pairs `range` (a record's run).
    #[inline]
    pub(crate) fn pairs(&self, range: core::ops::Range<usize>) -> &'a [KeptPair] {
        &self.pairs[range]
    }

    /// Every kept pair, live and dead.
    #[inline]
    pub(crate) fn all_pairs(&self) -> &'a [KeptPair] {
        self.pairs
    }

    /// Kept pair `pair`'s reuse record, `record`'s run holding it.
    #[inline]
    pub(crate) fn reuse_of(&self, record: &HeldIsland, pair: &KeptPair) -> Option<&'a ReuseRecord> {
        (pair.r_slot != NONE).then(|| &self.reuse[(record.reuse_start + pair.r_slot) as usize])
    }

    /// The live kept slots, by ordinal.
    #[inline]
    pub(crate) fn order(&self) -> &'a [u32] {
        self.order
    }

    /// The live kept manifolds (the logical view's share).
    #[inline]
    pub(crate) fn live_kept(&self) -> u32 {
        self.live_kept
    }

    /// Their live points (`WarmSeedStats::carry_points`' share, design 04 A6).
    #[inline]
    pub(crate) fn live_points(&self) -> u32 {
        self.live_points
    }

    /// The record holding current row `row` as a member, or [`NONE`].
    #[inline]
    pub(crate) fn record_of_row(&self, row: u32) -> u32 {
        self.of_row.get(row as usize).copied().unwrap_or(NONE)
    }

    /// The row-table index of current row `row` in `record` (members first, then anchors), or
    /// `None`.
    ///
    /// A live record: two binary searches. A record touching a vanished row or a jumper is
    /// restored, so a surviving record's two runs are sorted (design 06 B3). A tombstoned record
    /// is translated on every Rows step like a live one — a move-in of the step that restored it
    /// reads its copy of a cross pair (design 06 B3) — so on that step a vanished row reads
    /// [`NONE`] and a jumper sits out of order in its runs, where a binary search can miss a row
    /// that is present (review W2 of L10 C3c); its table is scanned instead.
    #[inline]
    pub(crate) fn local_of(&self, record: &HeldIsland, row: u32) -> Option<u32> {
        if !record.live() {
            return self.local_of_dead(record, row);
        }
        let members = &self.rows[record.members_range()];
        let anchors = &self.rows[record.anchors_range()];
        debug_assert!(
            members.is_sorted() && anchors.is_sorted(),
            "invariant: a live record's member and anchor runs are sorted (design 06 B3)"
        );
        if let Ok(i) = members.binary_search(&row) {
            return Some(i as u32);
        }
        anchors.binary_search(&row).ok().map(|i| record.n_members + i as u32)
    }

    /// [`local_of`](Self::local_of) for a tombstoned record: a linear scan of its row table, in
    /// table order, so the position is the row-table index. The translation maps distinct
    /// previous rows to distinct current rows, so a present row matches at most one entry; only
    /// [`NONE`] repeats, and no caller looks it up. Off the steady path: only a move-in on the
    /// step a record restored reads a tombstoned one.
    #[cold]
    #[inline(never)]
    fn local_of_dead(&self, record: &HeldIsland, row: u32) -> Option<u32> {
        debug_assert_ne!(row, NONE, "invariant: a row-table lookup names a present row");
        self.rows[record.rows_range()].iter().position(|&r| r == row).map(|i| i as u32)
    }

    /// `record`'s kept pair with row-table indices `(la, lb)`, if it keeps one (a binary search:
    /// the run is sorted by `(la, lb)`).
    #[inline]
    pub(crate) fn find_pair(&self, record: &HeldIsland, la: u32, lb: u32) -> Option<&'a KeptPair> {
        let run = &self.pairs[record.pairs_range()];
        run.binary_search_by_key(&(la, lb), |p| (p.la, p.lb)).ok().map(|i| &run[i])
    }

    /// The tag a live record keeps for current pair `(a, b)` (`a < b`): the record of either
    /// endpoint that holds it as a member, the pair found by its two row-table indices. `None`
    /// when no live record keeps the pair.
    pub(crate) fn kept_tag_of(&self, a: u32, b: u32) -> Option<PairTag> {
        [a, b].into_iter().find_map(|x| {
            let k = self.record_of_row(x);
            let rec = self.records.get(k as usize).filter(|r| r.live())?;
            let (la, lb) = (self.local_of(rec, a)?, self.local_of(rec, b)?);
            self.find_pair(rec, la, lb).map(|p| p.tag)
        })
    }

    /// Kept slot `slot`'s manifold in current rows.
    #[inline]
    pub(crate) fn manifold(&self, slot: u32) -> Manifold {
        let km = &self.kept[slot as usize];
        let record = &self.records[km.record as usize];
        let at = |local: u32| self.rows[record.rows_start as usize + local as usize];
        let mut m = km.m;
        m.body_a = BodyIndex(at(km.m.body_a.0));
        if m.body_b != SDF_SENTINEL {
            m.body_b = BodyIndex(at(km.m.body_b.0));
        }
        m
    }

    /// Kept slot `slot`'s manifold in current rows, or `None` when a body of it vanished on this
    /// gather (its row-table entry is [`NONE`] — the SDF sentinel's bits, so the kept manifold's
    /// own `body_b` tells an SDF contact from a vanished partner). Only a record being restored
    /// can hold such a row (design 08 OQ1: its restore entry is skipped).
    #[inline]
    pub(crate) fn manifold_present(&self, slot: u32) -> Option<Manifold> {
        let km = &self.kept[slot as usize];
        let record = &self.records[km.record as usize];
        let at = |local: u32| self.rows[record.rows_start as usize + local as usize];
        let a = at(km.m.body_a.0);
        let sdf = km.m.body_b == SDF_SENTINEL;
        let b = if sdf { SDF_SENTINEL.0 } else { at(km.m.body_b.0) };
        (a != NONE && (sdf || b != NONE)).then(|| {
            let mut m = km.m;
            m.body_a = BodyIndex(a);
            m.body_b = BodyIndex(b);
            m
        })
    }

    /// Kept slot `slot`'s canonical ordinal in current rows ([`ord`]): the key its manifold holds
    /// in the order `Off` emits the stream.
    #[inline]
    pub(crate) fn ordinal(&self, slot: u32) -> u64 {
        let m = self.manifold(slot);
        ord(m.body_a.0, m.body_b.0)
    }

    /// The kept slot behind handle `handle` (`HELD_BASE + slot`), if it names a live one.
    #[inline]
    pub(crate) fn slot_of_handle(&self, handle: u32) -> Option<u32> {
        let slot = handle.checked_sub(HELD_BASE)?;
        let km = self.kept.get(slot as usize)?;
        self.records[km.record as usize].live().then_some(slot)
    }

    /// The number of live kept manifolds whose ordinal is below `key` (a binary search of
    /// `order`).
    #[inline]
    pub(crate) fn rank_below(&self, key: u64) -> usize {
        self.order.partition_point(|&s| self.ordinal(s) < key)
    }
}

/// The held store (module docs), owned by [`Manifolds`](crate::resources::Manifolds).
pub(crate) struct HeldStore {
    /// The records, live and tombstoned, in move-in order.
    records: ScratchColumn<HeldIsland>,
    /// The records' row tables, in current rows (translated on every Rows step).
    rows: ScratchColumn<u32>,
    /// The records' kept manifolds (island-local).
    kept: ScratchColumn<KeptManifold>,
    /// Current row → the record holding it as a member, or [`NONE`].
    of_row: ScratchColumn<u32>,
    /// The live kept slots sorted by ordinal, and the other side of the ping-pong a merge or a
    /// filter writes into.
    order: [ScratchColumn<u32>; 2],
    /// Which side of `order` is current.
    cur: u8,
    /// The records' kept pairs.
    kept_pair: ScratchColumn<KeptPair>,
    /// The kept `REC` pairs' reuse records.
    kept_reuse: ScratchColumn<ReuseRecord>,
    /// Live records.
    live_records: u32,
    /// Live kept manifolds.
    live_kept: u32,
    /// Their live points.
    live_points: u32,
    /// Kept manifold slots of tombstoned records not yet compacted.
    dead_kept: u32,
    /// Tombstoned records not yet compacted.
    dead_records: u32,
}

/// Where a record under construction started, to roll it back.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Mark {
    rows: usize,
    kept: usize,
    pairs: usize,
    reuse: usize,
}

/// The ordinal-only view of `records`, `rows` and `kept` (what [`HeldView::ordinal`] reads),
/// built from the three fields so a caller can hold `order` mutably beside it.
#[inline]
fn ordinal_view<'a>(
    records: &'a ScratchColumn<HeldIsland>,
    rows: &'a ScratchColumn<u32>,
    kept: &'a ScratchColumn<KeptManifold>,
) -> HeldView<'a> {
    HeldView {
        records: records.as_read_slice(),
        rows: rows.as_read_slice(),
        kept: kept.as_read_slice(),
        ..HeldView::EMPTY
    }
}

impl Default for HeldStore {
    /// Hand-written because the columns need their reserved ids.
    #[inline]
    fn default() -> Self {
        Self::with_capacity(0)
    }
}

impl HeldStore {
    /// An empty store, reserved at the kernel's column budget (address space, not commit).
    pub(crate) fn with_capacity(rows: usize) -> Self {
        register_held_store_layouts();
        let u32_rows = rows.max(scratch_reserve_rows(size_of::<u32>()));
        Self {
            records: ScratchColumn::new(
                held_records_id(),
                scratch_reserve_rows(size_of::<HeldIsland>()),
            ),
            rows: ScratchColumn::new(held_rows_id(), u32_rows),
            kept: ScratchColumn::new(held_kept_id(), scratch_reserve_rows(size_of::<KeptManifold>())),
            of_row: ScratchColumn::new(held_of_row_id(), u32_rows),
            order: [
                ScratchColumn::new(held_order_id(0), u32_rows),
                ScratchColumn::new(held_order_id(1), u32_rows),
            ],
            cur: 0,
            kept_pair: ScratchColumn::new(
                held_kept_pair_id(),
                scratch_reserve_rows(size_of::<KeptPair>()),
            ),
            kept_reuse: ScratchColumn::new(
                held_kept_reuse_id(),
                scratch_reserve_rows(size_of::<ReuseRecord>()),
            ),
            live_records: 0,
            live_kept: 0,
            live_points: 0,
            dead_kept: 0,
            dead_records: 0,
        }
    }

    /// The read-only view.
    #[inline]
    pub(crate) fn view(&self) -> HeldView<'_> {
        HeldView {
            records: self.records.as_read_slice(),
            rows: self.rows.as_read_slice(),
            kept: self.kept.as_read_slice(),
            of_row: self.of_row.as_read_slice(),
            order: self.order[usize::from(self.cur)].as_read_slice(),
            pairs: self.kept_pair.as_read_slice(),
            reuse: self.kept_reuse.as_read_slice(),
            live_records: self.live_records,
            live_kept: self.live_kept,
            live_points: self.live_points,
        }
    }

    /// Live records.
    #[inline]
    pub(crate) fn live_records(&self) -> u32 {
        self.live_records
    }

    /// Marks live record `k` for restore this step.
    #[inline]
    pub(crate) fn mark_restore(&mut self, k: usize) {
        let mut records = self.records.build_view();
        let rec = &mut records.as_mut_slice()[k];
        if rec.live() {
            rec.flags |= HeldIsland::RESTORE;
        }
    }

    /// Marks every live record for restore (a flush, design 04 D9).
    pub(crate) fn mark_all(&mut self) {
        let mut records = self.records.build_view();
        for rec in records.as_mut_slice() {
            if rec.live() {
                rec.flags |= HeldIsland::RESTORE;
            }
        }
    }

    /// A Rows step (design 04 A1.2): every record's row table and root translated through
    /// `inv` (previous row → current row, [`NONE`] for a vanished body), and `of_row` rebuilt
    /// for `n_rows` current rows. A tombstoned record is translated too: a move-in of this step
    /// may read its copy of a cross pair.
    pub(crate) fn translate(&mut self, inv: &[u32], n_rows: usize) {
        let tr = |r: u32| if r == NONE { NONE } else { inv.get(r as usize).copied().unwrap_or(NONE) };
        {
            let mut rows = self.rows.build_view();
            for r in rows.as_mut_slice() {
                *r = tr(*r);
            }
        }
        {
            let mut records = self.records.build_view();
            for rec in records.as_mut_slice() {
                rec.root = tr(rec.root);
            }
        }
        self.rebuild_of_row(n_rows);
    }

    /// `of_row` for `n_rows` current rows: every live record's present members.
    pub(crate) fn rebuild_of_row(&mut self, n_rows: usize) {
        let records = self.records.as_read_slice();
        let rows = self.rows.as_read_slice();
        let mut of_row = self.of_row.build_view();
        of_row.clear();
        of_row.resize(n_rows, NONE);
        let of_row = of_row.as_mut_slice();
        for (k, rec) in records.iter().enumerate() {
            if !rec.live() {
                continue;
            }
            for &r in &rows[rec.members_range()] {
                if let Some(slot) = of_row.get_mut(r as usize) {
                    *slot = k as u32;
                }
            }
        }
    }

    /// Tombstones record `k` (it is restored this step): its members leave `of_row`, and the live
    /// counters drop its share. `order` is filtered separately
    /// ([`filter_order`](Self::filter_order)), once for every tombstone of the step.
    pub(crate) fn tombstone(&mut self, k: usize) {
        let rec = {
            let mut records = self.records.build_view();
            let rec = &mut records.as_mut_slice()[k];
            debug_assert!(rec.live(), "invariant: a record is tombstoned once");
            rec.flags = HeldIsland::DEAD;
            *rec
        };
        {
            let rows = self.rows.as_read_slice();
            let mut of_row = self.of_row.build_view();
            let of_row = of_row.as_mut_slice();
            for &r in &rows[rec.members_range()] {
                if let Some(slot) = of_row.get_mut(r as usize)
                    && *slot == k as u32
                {
                    *slot = NONE;
                }
            }
        }
        self.live_records -= 1;
        self.live_kept -= rec.kept_len;
        self.live_points -= rec.points;
        self.dead_kept += rec.kept_len;
        self.dead_records += 1;
    }

    /// Drops every tombstoned record's slots from `order` (design 04 A2.5), O(|order|).
    pub(crate) fn filter_order(&mut self) {
        let cur = usize::from(self.cur);
        let [o0, o1] = &mut self.order;
        let (src, dst) = if cur == 0 { (&*o0, o1) } else { (&*o1, o0) };
        let kept = self.kept.as_read_slice();
        let records = self.records.as_read_slice();
        let mut out = dst.build_view();
        out.clear();
        for &s in src.as_read_slice() {
            if records[kept[s as usize].record as usize].live() {
                out.push(s);
            }
        }
        drop(out);
        self.cur ^= 1;
        debug_assert_eq!(
            self.order[usize::from(self.cur)].len(),
            self.live_kept as usize,
            "invariant: order holds exactly the live kept slots"
        );
    }

    /// Where the next record's runs start: a move-in builds its record at the end of every
    /// column and [`rollback`](Self::rollback)s to here when an admission rule refuses it.
    #[inline]
    pub(crate) fn mark(&self) -> Mark {
        Mark {
            rows: self.rows.len(),
            kept: self.kept.len(),
            pairs: self.kept_pair.len(),
            reuse: self.kept_reuse.len(),
        }
    }

    /// Discards everything appended since `mark`.
    pub(crate) fn rollback(&mut self, mark: Mark) {
        self.rows.build_view().truncate(mark.rows);
        self.kept.build_view().truncate(mark.kept);
        self.kept_pair.build_view().truncate(mark.pairs);
        self.kept_reuse.build_view().truncate(mark.reuse);
    }

    /// Appends one row-table entry.
    #[inline]
    pub(crate) fn push_row(&mut self, row: u32) {
        self.rows.build_view().push(row);
    }

    /// The row table appended since `mark` (the record under construction).
    #[inline]
    pub(crate) fn rows_since(&self, mark: Mark) -> &[u32] {
        &self.rows.as_read_slice()[mark.rows..]
    }

    /// Sorts the rows appended since `mark` from row-table index `from` on (the anchors).
    #[inline]
    pub(crate) fn sort_rows_since(&mut self, mark: Mark, from: usize) {
        let mut view = self.rows.build_view();
        view.as_mut_slice()[mark.rows + from..].sort_unstable();
    }

    /// Drops adjacent duplicates of the rows appended since `mark` from row-table index `from`
    /// on (sorted anchors); returns how many remain there.
    pub(crate) fn dedup_rows_since(&mut self, mark: Mark, from: usize) -> usize {
        let mut view = self.rows.build_view();
        let run = &mut view.as_mut_slice()[mark.rows + from..];
        let mut w = 0;
        for i in 0..run.len() {
            if w == 0 || run[i] != run[w - 1] {
                run[w] = run[i];
                w += 1;
            }
        }
        let keep = mark.rows + from + w;
        view.truncate(keep);
        w
    }

    /// Appends one kept manifold.
    #[inline]
    pub(crate) fn push_kept(&mut self, kept: KeptManifold) {
        self.kept.build_view().push(kept);
    }

    /// Appends one kept pair, and its reuse record when it has one.
    #[inline]
    pub(crate) fn push_pair(&mut self, pair: KeptPair, record: Option<ReuseRecord>) {
        self.kept_pair.build_view().push(pair);
        if let Some(record) = record {
            self.kept_reuse.build_view().push(record);
        }
    }

    /// The kept manifolds (live and dead), counted: where the next move-in's slots start.
    #[inline]
    pub(crate) fn kept_len(&self) -> usize {
        self.kept.len()
    }

    /// Rewrites the kept manifolds and kept pairs appended since `mark` — which hold CURRENT rows
    /// until now — onto the record's row table, whose first `nm` rows are the members (sorted)
    /// and the rest the anchors (sorted): each row becomes its table index. Sets each pair's kept
    /// manifold (the kept run is sorted by ordinal, so a binary search) and sorts the pairs by
    /// `(la, lb)`.
    pub(crate) fn localize_since(&mut self, mark: Mark, nm: usize) {
        let table = &self.rows.as_read_slice()[mark.rows..];
        let (mem, anc) = table.split_at(nm);
        let local = |r: u32| -> u32 {
            match mem.binary_search(&r) {
                Ok(i) => i as u32,
                Err(_) => {
                    let i = anc
                        .binary_search(&r)
                        .expect("invariant: every kept row is in the record's row table");
                    (nm + i) as u32
                }
            }
        };
        let mut kept = self.kept.build_view();
        let kept = &mut kept.as_mut_slice()[mark.kept..];
        {
            let mut pairs = self.kept_pair.build_view();
            let pairs = &mut pairs.as_mut_slice()[mark.pairs..];
            for p in pairs.iter_mut() {
                let key = ord(p.la, p.lb);
                p.m_slot = kept
                    .binary_search_by_key(&key, |km| ord(km.m.body_a.0, km.m.body_b.0))
                    .map_or(NONE, |i| i as u32);
                debug_assert_eq!(
                    p.m_slot != NONE,
                    p.tag.has(PairTag::PUSHED),
                    "invariant: a kept pair has a kept manifold iff it emitted one"
                );
                p.la = local(p.la);
                p.lb = local(p.lb);
            }
            pairs.sort_unstable_by_key(|p| (p.la, p.lb));
        }
        for km in kept.iter_mut() {
            km.m.body_a = BodyIndex(local(km.m.body_a.0));
            if km.m.body_b != SDF_SENTINEL {
                km.m.body_b = BodyIndex(local(km.m.body_b.0));
            }
        }
    }

    /// Commits the record built since `mark` as `record` (its starts and lengths are taken from
    /// the mark), sets `of_row` for its members, and returns its index.
    pub(crate) fn commit(&mut self, mark: Mark, mut record: HeldIsland, n_rows: usize) -> u32 {
        record.rows_start = mark.rows as u32;
        record.kept_start = mark.kept as u32;
        record.kept_len = (self.kept.len() - mark.kept) as u32;
        record.pairs_start = mark.pairs as u32;
        record.pairs_len = (self.kept_pair.len() - mark.pairs) as u32;
        record.reuse_start = mark.reuse as u32;
        record.flags = 0;
        debug_assert_eq!(
            (record.n_members + record.n_anchors) as usize,
            self.rows.len() - mark.rows,
            "invariant: the row table is the members then the anchors"
        );
        let k = self.records.len() as u32;
        {
            let mut kept = self.kept.build_view();
            for km in &mut kept.as_mut_slice()[mark.kept..] {
                km.record = k;
            }
        }
        self.records.build_view().push(record);
        {
            let rows = self.rows.as_read_slice();
            let mut of_row = self.of_row.build_view();
            if of_row.len() < n_rows {
                of_row.resize(n_rows, NONE);
            }
            let of_row = of_row.as_mut_slice();
            for &r in &rows[record.members_range()] {
                of_row[r as usize] = k;
            }
        }
        self.live_records += 1;
        self.live_kept += record.kept_len;
        self.live_points += record.points;
        k
    }

    /// Merges the new live slots `new` — sorted by ordinal (`key`) — into `order` (design 04
    /// A2.4's `order` merge). O(|order| + |new|).
    pub(crate) fn merge_order(&mut self, new: &[RestoreSort]) {
        if new.is_empty() {
            return;
        }
        let cur = usize::from(self.cur);
        let view = ordinal_view(&self.records, &self.rows, &self.kept);
        let [o0, o1] = &mut self.order;
        let (src, dst) = if cur == 0 { (&*o0, o1) } else { (&*o1, o0) };
        let src = src.as_read_slice();
        let mut out = dst.build_view();
        out.clear();
        let (mut i, mut j) = (0, 0);
        while i < src.len() || j < new.len() {
            let take_old = j == new.len() || (i < src.len() && view.ordinal(src[i]) < new[j].key);
            if take_old {
                out.push(src[i]);
                i += 1;
            } else {
                out.push(new[j].slot);
                j += 1;
            }
        }
        drop(out);
        self.cur ^= 1;
        debug_assert!(
            {
                let v = self.view();
                v.order().windows(2).all(|w| v.ordinal(w[0]) < v.ordinal(w[1]))
            },
            "invariant: order is strictly increasing by ordinal"
        );
    }

    /// Whether compaction is due (design 06 B3, compaction at A1.0b): the tombstoned slots reach
    /// the live ones and 64, or the tombstoned records reach the live ones and 64 (a record may
    /// keep no manifold, and must not accumulate either).
    #[inline]
    pub(crate) fn compaction_due(&self) -> bool {
        self.dead_records > 0
            && (self.dead_kept >= self.live_kept.max(64)
                || self.dead_records >= self.live_records.max(64))
    }

    /// Compacts every tombstoned record away (A1.0b: only records that died on EARLIER steps
    /// exist here). The live records keep their order and their runs move down; `kept_rec`
    /// (the kept manifolds' warm records, owned by `SleepSets`) moves with `kept`. `order` is
    /// rebuilt by sorting the live slots with `sort_buf` as scratch, and `of_row` for the rows it
    /// was keyed by (the previous gather's; a Rows step translates it next). O(store), on
    /// compaction steps only.
    pub(crate) fn compact<W: Copy>(
        &mut self,
        kept_rec: &mut ScratchColumn<W>,
        sort_buf: &mut ScratchColumn<RestoreSort>,
    ) {
        let n_rows = self.of_row.len();
        {
            let mut records = self.records.build_view();
            let mut rows = self.rows.build_view();
            let mut kept = self.kept.build_view();
            let mut pairs = self.kept_pair.build_view();
            let mut reuse = self.kept_reuse.build_view();
            let mut warm = kept_rec.build_view();
            let (mut w_rec, mut w_rows, mut w_kept, mut w_pairs, mut w_reuse) = (0, 0, 0, 0, 0);
            let n = records.len();
            for k in 0..n {
                let rec = records.as_slice()[k];
                if !rec.live() {
                    continue;
                }
                let reuse_len = pairs.as_slice()[rec.pairs_range()]
                    .iter()
                    .filter(|p| p.r_slot != NONE)
                    .count();
                rows.as_mut_slice().copy_within(rec.rows_range(), w_rows);
                kept.as_mut_slice().copy_within(rec.kept_range(), w_kept);
                warm.as_mut_slice().copy_within(rec.kept_range(), w_kept);
                pairs.as_mut_slice().copy_within(rec.pairs_range(), w_pairs);
                let r_lo = rec.reuse_start as usize;
                reuse.as_mut_slice().copy_within(r_lo..r_lo + reuse_len, w_reuse);
                for km in &mut kept.as_mut_slice()[w_kept..w_kept + rec.kept_len as usize] {
                    km.record = w_rec as u32;
                }
                let moved = HeldIsland {
                    rows_start: w_rows as u32,
                    kept_start: w_kept as u32,
                    pairs_start: w_pairs as u32,
                    reuse_start: w_reuse as u32,
                    ..rec
                };
                w_rows += rec.rows_range().len();
                w_kept += rec.kept_len as usize;
                w_pairs += rec.pairs_len as usize;
                w_reuse += reuse_len;
                records.as_mut_slice()[w_rec] = moved;
                w_rec += 1;
            }
            records.truncate(w_rec);
            rows.truncate(w_rows);
            kept.truncate(w_kept);
            warm.truncate(w_kept);
            pairs.truncate(w_pairs);
            reuse.truncate(w_reuse);
            debug_assert_eq!(w_kept, self.live_kept as usize, "invariant: live slots survive compaction");
        }
        self.dead_kept = 0;
        self.dead_records = 0;
        // `order` by sorting the live slots with their ordinals, in place, no heap.
        {
            let view = ordinal_view(&self.records, &self.rows, &self.kept);
            let mut buf = sort_buf.build_view();
            buf.clear();
            for s in 0..self.live_kept {
                buf.push(RestoreSort::new(view.ordinal(s), s));
            }
            buf.as_mut_slice().sort_unstable();
        }
        {
            let mut out = self.order[usize::from(self.cur)].build_view();
            out.clear();
            for e in sort_buf.as_read_slice() {
                out.push(e.slot);
            }
        }
        self.rebuild_of_row(n_rows);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    //! L10 C3b's store gates (design 04 "Unit and property tests", 06 §7): the held store against
    //! a `Vec` model through move-ins, restores, Rows-step translations and compactions, and the
    //! logical manifold view against a merged `Vec`.

    use boyko_ecs::ecs::core::component::scratch::ScratchColumn;
    use proptest::prelude::*;

    use super::{HeldIsland, HeldStore, KeptManifold, KeptPair, NONE, RestoreSort};
    use crate::manifold::{BodyIndex, Manifold, SDF_SENTINEL};
    use crate::narrowphase::carry::PairTag;
    use crate::narrowphase::reuse::ReuseRecord;
    use crate::resources::{HELD_BASE, ManifoldsView};
    use crate::scratch_ids::{register_sleep_sets_layouts, sleep_restore_sort_id, sleep_scratch_id};
    use crate::solver::warm_records::ord;

    /// A xorshift64 stream: the model's choices from one proptest seed.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n.max(1)
        }
    }

    /// One record of the model, in current rows.
    #[derive(Clone, Debug)]
    pub(crate) struct ModelRecord {
        pub(crate) members: Vec<u32>,
        /// `(manifold, model id)`, sorted by ordinal.
        pub(crate) kept: Vec<(Manifold, u64)>,
        /// `(a, b, tag, record)` with `a < b`.
        pub(crate) pairs: Vec<(u32, u32, PairTag, Option<ReuseRecord>)>,
        pub(crate) live: bool,
    }

    /// A manifold between rows `a < b` (or `a` and the SDF sentinel) with `count` points and a
    /// normal naming its model id.
    pub(crate) fn manifold(a: u32, b: u32, count: u8, id: u64) -> Manifold {
        let mut m = Manifold::new(BodyIndex(a), BodyIndex(b));
        m.count = count;
        m.normal.x = id as f32;
        m
    }

    /// Builds `rec` in `store` exactly as the move-in does (members, kept manifolds and pairs in
    /// current rows, the anchors sorted and deduplicated, localized, committed) and merges its
    /// slots into `order`; `ids` is the parallel model-id column. Returns the record index.
    pub(crate) fn move_in(
        store: &mut HeldStore,
        ids: &mut ScratchColumn<u64>,
        rec: &ModelRecord,
        n_rows: usize,
    ) -> u32 {
        let first = store.kept_len();
        let mark = store.mark();
        for &m in &rec.members {
            store.push_row(m);
        }
        let nm = rec.members.len();
        let is_member = |r: u32| rec.members.binary_search(&r).is_ok();
        let mut points = 0;
        for (m, _) in &rec.kept {
            for r in [m.body_a.0, m.body_b.0] {
                if r != SDF_SENTINEL.0 && !is_member(r) {
                    store.push_row(r);
                }
            }
            points += u32::from(m.count);
            store.push_kept(KeptManifold { m: *m, record: NONE });
        }
        let mut n_reuse = 0;
        for &(a, b, tag, record) in &rec.pairs {
            for r in [a, b] {
                if !is_member(r) {
                    store.push_row(r);
                }
            }
            let r_slot = if record.is_some() {
                n_reuse += 1;
                n_reuse - 1
            } else {
                NONE
            };
            store.push_pair(KeptPair { la: a, lb: b, m_slot: NONE, r_slot, tag, _p: 0 }, record);
        }
        store.sort_rows_since(mark, nm);
        let na = store.dedup_rows_since(mark, nm);
        store.localize_since(mark, nm);
        let island = HeldIsland {
            root: rec.members[0],
            n_members: nm as u32,
            n_anchors: na as u32,
            points,
            ..HeldIsland::default()
        };
        let k = store.commit(mark, island, n_rows);
        {
            let mut v = ids.build_view();
            for (_, id) in &rec.kept {
                v.push(*id);
            }
        }
        let view = store.view();
        let mut new: Vec<RestoreSort> =
            (first..store.kept_len()).map(|s| RestoreSort::new(view.ordinal(s as u32), s as u32)).collect();
        new.sort_unstable();
        store.merge_order(&new);
        k
    }

    fn bytes(m: &Manifold) -> [u8; 152] {
        // SAFETY: `Manifold` is `#[repr(C)]` with no implicit padding (152 B, asserted in
        // `manifold.rs`), so all its bytes are initialised; the slice borrows `m` read-only for
        // the copy below.
        let s = unsafe { core::slice::from_raw_parts((m as *const Manifold).cast::<u8>(), 152) };
        let mut out = [0u8; 152];
        out.copy_from_slice(s);
        out
    }

    /// Compares the store with the model: the live counts, `order` against the model's live kept
    /// manifolds sorted by ordinal (manifold and model id), each live record's members,
    /// `record_of_row`, and every kept pair through `kept_tag_of`, `find_pair` and `reuse_of`.
    fn check(store: &HeldStore, ids: &ScratchColumn<u64>, model: &[ModelRecord]) {
        let view = store.view();
        let live: Vec<&ModelRecord> = model.iter().filter(|r| r.live).collect();
        assert_eq!(store.live_records() as usize, live.len(), "live records");
        let mut want: Vec<(u64, [u8; 152], u64)> = live
            .iter()
            .flat_map(|r| r.kept.iter().map(|(m, id)| (ord(m.body_a.0, m.body_b.0), bytes(m), *id)))
            .collect();
        want.sort_unstable_by_key(|e| e.0);
        let got: Vec<(u64, [u8; 152], u64)> = view
            .order()
            .iter()
            .map(|&s| {
                let m = view.manifold(s);
                (ord(m.body_a.0, m.body_b.0), bytes(&m), ids.as_read_slice()[s as usize])
            })
            .collect();
        assert_eq!(got, want, "order holds the live kept manifolds by ordinal, in current rows");
        assert_eq!(view.live_kept() as usize, want.len(), "live kept");
        for r in &live {
            let k = view.record_of_row(r.members[0]);
            assert_ne!(k, NONE, "a live member maps to its record");
            let rec = view.records()[k as usize];
            assert!(rec.live(), "record_of_row names a live record");
            assert_eq!(view.rows(rec.members_range()), &r.members[..], "the record's members");
            for &m in &r.members {
                assert_eq!(view.record_of_row(m), k, "every member maps to the record");
            }
            for &(a, b, tag, record) in &r.pairs {
                assert_eq!(view.kept_tag_of(a, b), Some(tag), "the kept pair ({a}, {b})");
                let la = view.local_of(&rec, a).expect("a kept pair's row is in the table");
                let lb = view.local_of(&rec, b).expect("a kept pair's row is in the table");
                let kp = view.find_pair(&rec, la, lb).expect("find_pair finds the kept pair");
                assert_eq!(
                    view.reuse_of(&rec, kp).map(|r| r.words()),
                    record.map(|r| r.words()),
                    "its reuse record"
                );
            }
        }
    }

    /// A fresh record over rows not yet a live member: 1..=4 members, a kept manifold between
    /// consecutive members, one to an anchor (a static row below 8) and an SDF one; a kept pair
    /// per body-body manifold (tag `PUSHED`, every other one `REC`) and one separated pair to a
    /// second anchor.
    fn fresh(rng: &mut Rng, taken: &mut Vec<u32>, n_rows: u32, next_id: &mut u64) -> Option<ModelRecord> {
        let n = 1 + rng.below(4) as usize;
        let mut members = Vec::new();
        for _ in 0..40 {
            if members.len() == n {
                break;
            }
            let r = 8 + rng.below(u64::from(n_rows - 8)) as u32;
            if !taken.contains(&r) && !members.contains(&r) {
                members.push(r);
            }
        }
        if members.is_empty() {
            return None;
        }
        members.sort_unstable();
        taken.extend(&members);
        let mut kept = Vec::new();
        let mut id = || {
            *next_id += 1;
            *next_id
        };
        for w in members.windows(2) {
            let i = id();
            kept.push((manifold(w[0], w[1], 2, i), i));
        }
        let anchor = rng.below(8) as u32;
        let i = id();
        kept.push((manifold(anchor, members[0], 4, i), i));
        let i = id();
        kept.push((manifold(members[0], SDF_SENTINEL.0, 1, i), i));
        kept.sort_unstable_by_key(|(m, _)| ord(m.body_a.0, m.body_b.0));
        let mut pairs = Vec::new();
        for (i, (m, _)) in kept.iter().enumerate() {
            if m.body_b == SDF_SENTINEL {
                continue;
            }
            let rec = i % 2 == 0;
            let tag = if rec { PairTag::box_recorded(i % 15, true, false) } else { PairTag::box_contact(i % 15, true) };
            let record = rec.then(|| ReuseRecord::UNWRITTEN.with_parity(i % 3 == 0));
            pairs.push((m.body_a.0, m.body_b.0, tag, record));
        }
        let other = (anchor + 1) % 8;
        let (a, b) = (other.min(members[n.min(members.len()) - 1]), other.max(members[n.min(members.len()) - 1]));
        if !pairs.iter().any(|p| (p.0, p.1) == (a, b)) {
            pairs.push((a, b, PairTag::box_separated(3, false), None));
        }
        pairs.sort_unstable_by_key(|p| (p.0, p.1));
        Some(ModelRecord { members, kept, pairs, live: true })
    }

    proptest! {
        // `failure_persistence: None`: no regression file is read or written, so the test runs
        // under Miri's default isolation (the crate's convention, `broadphase_tree/tests.rs`).
        #![proptest_config(ProptestConfig {
            cases: if cfg!(miri) { 2 } else { 64 },
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        /// Move-ins, restores (tombstone + `order` filter), monotone Rows-step translations and
        /// compactions, against the `Vec` model.
        #[test]
        fn the_store_matches_a_vec_model(seed in 1u64..u64::MAX) {
            register_sleep_sets_layouts();
            let mut rng = Rng(seed);
            let mut store = HeldStore::with_capacity(0);
            let mut ids: ScratchColumn<u64> = ScratchColumn::new(sleep_scratch_id(), 1 << 12);
            let mut sort_buf: ScratchColumn<RestoreSort> =
                ScratchColumn::new(sleep_restore_sort_id(), 1 << 12);
            let mut model: Vec<ModelRecord> = Vec::new();
            let mut index: Vec<u32> = Vec::new();
            let mut n_rows = 64u32;
            let mut next_id = 0u64;
            let ops = if cfg!(miri) { 6 } else { 40 };
            for _ in 0..ops {
                match rng.below(4) {
                    0 | 1 => {
                        let mut taken: Vec<u32> =
                            model.iter().filter(|r| r.live).flat_map(|r| r.members.clone()).collect();
                        if let Some(rec) = fresh(&mut rng, &mut taken, n_rows, &mut next_id) {
                            let k = move_in(&mut store, &mut ids, &rec, n_rows as usize);
                            model.push(rec);
                            index.push(k);
                        }
                    }
                    2 => {
                        let live: Vec<usize> = (0..model.len()).filter(|&i| model[i].live).collect();
                        if !live.is_empty() {
                            let i = live[rng.below(live.len() as u64) as usize];
                            store.tombstone(index[i] as usize);
                            store.filter_order();
                            model[i].live = false;
                        }
                    }
                    _ => {
                        // A monotone Rows step: `grow` new rows inserted at a random point.
                        let at = rng.below(u64::from(n_rows)) as u32;
                        let grow = 1 + rng.below(3) as u32;
                        let map = |r: u32| if r >= at { r + grow } else { r };
                        let inv: Vec<u32> = (0..n_rows).map(map).collect();
                        n_rows += grow;
                        store.translate(&inv, n_rows as usize);
                        for rec in &mut model {
                            for m in &mut rec.members {
                                *m = map(*m);
                            }
                            for (m, _) in &mut rec.kept {
                                m.body_a = BodyIndex(map(m.body_a.0));
                                if m.body_b != SDF_SENTINEL {
                                    m.body_b = BodyIndex(map(m.body_b.0));
                                }
                            }
                            for p in &mut rec.pairs {
                                p.0 = map(p.0);
                                p.1 = map(p.1);
                            }
                        }
                    }
                }
                if store.compaction_due() || rng.below(8) == 0 {
                    store.compact(&mut ids, &mut sort_buf);
                    // Compaction renumbers the live records in their order.
                    let mut k = 0u32;
                    for (i, rec) in model.iter().enumerate() {
                        if rec.live {
                            index[i] = k;
                            k += 1;
                        }
                    }
                }
                check(&store, &ids, &model);
            }
        }
    }

    /// The logical manifold view against a merged `Vec`: `iter`, `next_back`, `get` and `len`
    /// over a stream and a store whose ordinals interleave, and `slot_of_handle` for every kept
    /// handle.
    #[test]
    fn the_manifold_view_is_the_merge_by_ordinal() {
        register_sleep_sets_layouts();
        let mut store = HeldStore::with_capacity(0);
        let mut ids: ScratchColumn<u64> = ScratchColumn::new(sleep_scratch_id(), 1 << 10);
        let rec = |members: Vec<u32>, kept: Vec<Manifold>| ModelRecord {
            members,
            kept: kept.into_iter().map(|m| (m, u64::from(m.normal.x as u32))).collect(),
            pairs: Vec::new(),
            live: true,
        };
        let first = rec(
            vec![3, 4],
            vec![manifold(0, 3, 4, 1), manifold(3, 4, 2, 2), manifold(4, SDF_SENTINEL.0, 1, 3)],
        );
        move_in(&mut store, &mut ids, &first, 16);
        move_in(&mut store, &mut ids, &rec(vec![9], vec![manifold(0, 9, 4, 4)]), 16);
        let stream = [
            manifold(0, 1, 4, 10),
            manifold(1, 2, 4, 11),
            manifold(5, 6, 3, 12),
            manifold(2, SDF_SENTINEL.0, 1, 13),
        ];
        let view = ManifoldsView::new(&stream, store.view());
        let mut want: Vec<Manifold> = stream.to_vec();
        want.extend(store.view().order().iter().map(|&s| store.view().manifold(s)));
        want.sort_unstable_by_key(|m| ord(m.body_a.0, m.body_b.0));
        let ids_of = |v: &[Manifold]| v.iter().map(|m| m.normal.x as u32).collect::<Vec<_>>();
        assert_eq!(view.len(), want.len(), "len");
        assert_eq!(ids_of(&view.iter().collect::<Vec<_>>()), ids_of(&want), "iter is the merge");
        let mut back: Vec<Manifold> = view.iter().rev().collect();
        back.reverse();
        assert_eq!(ids_of(&back), ids_of(&want), "next_back is the merge reversed");
        for (pos, m) in want.iter().enumerate() {
            assert_eq!(view.get(pos).map(|g| g.normal.x as u32), Some(m.normal.x as u32), "get({pos})");
        }
        assert!(view.get(want.len()).is_none(), "get past the end");
        let held = store.view();
        for &s in held.order() {
            assert_eq!(held.slot_of_handle(HELD_BASE + s), Some(s), "a kept handle resolves");
        }
        assert_eq!(held.slot_of_handle(HELD_BASE + 99), None, "a handle past the store");
    }
}
