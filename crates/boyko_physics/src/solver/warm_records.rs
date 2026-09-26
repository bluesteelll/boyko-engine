//! The colored solver's warm store (L11 D1–D3): one 64 B record per manifold of
//! the stored step, indexed by that step's manifold index, beside a hot column of
//! stream ordinals. The next step finds each manifold's source record by a
//! merge-join over the ordinals (D2) and matches points by feature id inside the
//! record. There is no hash: the lookup of a sorted stream against a sorted store
//! is one compare per manifold, and the store is a write by index.
//!
//! # The layout
//!
//! A [`WarmRecords`] is one side of the solver's double buffer: `keys[mi]` is the
//! ordinal ([`ord`]) of manifold `mi`'s pair in the rows of the step that wrote it,
//! `recs[mi]` its stored points (the three accumulated impulses and the feature id
//! of each), and `strict` whether `keys` is strictly increasing — which it is for
//! every stream the narrowphase emits (body-body pairs sorted by `(a, b)`, then the
//! SDF manifolds in row order). A hand-built stream may be unsorted or repeat a
//! pair; the store records that, and the next step's lookups then go through the
//! cold sorted [`WarmIndex`] instead of the cursor search.
//!
//! # The lookup (D2, ruling W1)
//!
//! [`plan_sources_restored`] runs once per step in stream order: it writes this step's
//! ordinals into the write side and, for every manifold, searches the read side for
//! the ordinal of the rows its pair held when the read side was written
//! ([`RowRemap::manifold_pair`]). The result is a [`WarmRun`]: the whole run of
//! read-side positions whose key equals the translated ordinal. On a strict read
//! side that run is one record (or empty); on a non-strict one it is a run of
//! cold-index positions, which may name several records. The seed of a point and
//! the B1 carry both go through [`WarmLookup::seed`], the single routine that walks
//! a run: records by descending manifold index, points last to first, first
//! feature-id match wins.
//!
//! # Lemma W (why the values equal the per-point table's)
//!
//! The table this replaces held one entry per point, keyed by
//! `pack(a, b, fid & 0xFFFF)`, and its value for a key was the last insert of that
//! key in the order "solved manifolds ascending, then frozen manifolds ascending,
//! points in point order within a manifold". Five facts make the record lookup
//! return the same value and the same hit count:
//!
//! 1. `recs[mi]` holds exactly manifold `mi`'s inserts, in point order (the fill
//!    writes a solved record's shape and the store its impulses from the solved
//!    lanes, L11 C2; the carry compacts the hits in point order).
//! 2. `ord` is injective over pairs, so a run of equal keys is exactly the set of
//!    manifolds that keyed the same pair.
//! 3. Manifolds with equal pairs share an island, so they are all frozen or all
//!    solved; within one class ascending `mi` is the insert order, so "descending
//!    `mi`, last point first" walks the inserts of a key in reverse and the first
//!    feature-id match is the last insert.
//! 4. A carried point is the value the previous lookup returned, by induction on
//!    steps.
//! 5. The row translation is the table's own (`manifold_pair`); an untranslatable
//!    pair is a miss on both sides.

use boyko_ecs::ecs::core::component::scratch::{ScratchBuildView, ScratchColumn};
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

use crate::manifold::{Manifold, SDF_SENTINEL};
use crate::math::MAX_CONTACT_POINTS;
use crate::row_identity::RowRemap;
use crate::scratch_ids::{register_scratch_layouts, scratch_reserve_rows};

/// The feature-id bits a record keeps: the field width of the per-point table's
/// `pack`, so two points of one manifold that collide under the table's key collide
/// under the record's feature id as well.
pub(crate) const FEATURE_ID_MASK: u32 = 0xFFFF;

/// The stored feature id of point `p` of `m` (its feature id under
/// [`FEATURE_ID_MASK`]).
#[inline]
pub(crate) fn point_fid(m: &Manifold, p: usize) -> u16 {
    let f = m.points[p].feature_id;
    debug_assert!(
        f <= FEATURE_ID_MASK,
        "invariant: a feature id fits in 16 bits"
    );
    // Truncation is the point: the low 16 bits are the stored id.
    (f & FEATURE_ID_MASK) as u16
}

/// L10's D6 ordinal of a manifold's pair, monotone with the sorted stream order:
/// the body-body stream sorts by `(a, b)`, and the SDF manifolds (`body_b` the
/// sentinel) follow it in row order. Injective over pairs: bit 63 separates the two
/// classes, and a row never reaches 2³¹.
#[inline]
pub(crate) fn ord(a: u32, b: u32) -> u64 {
    debug_assert!(a >> 31 == 0, "invariant: a row index stays below 2^31");
    if b == SDF_SENTINEL.0 {
        (1u64 << 63) | u64::from(a)
    } else {
        (u64::from(a) << 32) | u64::from(b)
    }
}

/// One manifold's stored points: the accumulated impulses of each, its feature id,
/// and how many are stored. One cache line; index = the manifold index of the
/// writing step.
#[repr(C, align(64))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct WarmRecord {
    /// Accumulated normal impulse per stored point, point order.
    n: [f32; MAX_CONTACT_POINTS],
    /// Accumulated first-tangent impulse per stored point.
    t1: [f32; MAX_CONTACT_POINTS],
    /// Accumulated second-tangent impulse per stored point.
    t2: [f32; MAX_CONTACT_POINTS],
    /// Feature id per stored point ([`point_fid`]).
    fid: [u16; MAX_CONTACT_POINTS],
    /// Stored points `0..=MAX_CONTACT_POINTS`. A carried record holds its hits only,
    /// compacted.
    count: u8,
    /// Zero, so two records with equal contents are byte-equal.
    _pad: [u8; 7],
}

const _: () = assert!(
    size_of::<WarmRecord>() == 64 && align_of::<WarmRecord>() == 64,
    "a warm record is exactly one 64 B cache line"
);

const _: () = assert!(
    MAX_CONTACT_POINTS == 4,
    "the record's point arrays are sized for the manifold's four points"
);

impl WarmRecord {
    /// A record with no stored point, every field zero.
    pub(crate) const EMPTY: Self = Self {
        n: [0.0; MAX_CONTACT_POINTS],
        t1: [0.0; MAX_CONTACT_POINTS],
        t2: [0.0; MAX_CONTACT_POINTS],
        fid: [0; MAX_CONTACT_POINTS],
        count: 0,
        _pad: [0; 7],
    };

    /// The record of a SOLVED manifold from per-point impulse columns: its `count`
    /// points' feature ids beside the converged impulses of slots `base..base +
    /// count`. The G2 oracle's constructor; the solver writes a solved record in two
    /// halves — [`set_shape`](Self::set_shape) at the fill, [`set_impulses`](Self::set_impulses)
    /// at the store (L11 C2).
    #[cfg(test)]
    #[inline]
    pub(crate) fn solved(m: &Manifold, base: usize, count: usize, impulses: [&[f32]; 3]) -> Self {
        debug_assert!(
            count <= MAX_CONTACT_POINTS,
            "invariant: a manifold has at most four points"
        );
        let mut rec = Self::EMPTY;
        for p in 0..count {
            let s = base + p;
            rec.fid[p] = point_fid(m, p);
            rec.n[p] = impulses[0][s];
            rec.t1[p] = impulses[1][s];
            rec.t2[p] = impulses[2][s];
        }
        rec.count = count as u8;
        rec
    }

    /// The shape of a SOLVED manifold's record, written by the fill (L11 D3/D4): its
    /// live points' feature ids and count, the impulses zero until the store writes
    /// them. Every byte is written, so the padding is zero.
    #[inline]
    pub(crate) fn set_shape(&mut self, m: &Manifold) {
        let count = m.count as usize;
        debug_assert!(
            count <= MAX_CONTACT_POINTS,
            "invariant: a manifold has at most four points"
        );
        *self = Self::EMPTY;
        for p in 0..count {
            self.fid[p] = point_fid(m, p);
        }
        self.count = count as u8;
    }

    /// Writes stored point `p`'s converged impulses (the store, after
    /// [`set_shape`](Self::set_shape)).
    #[inline]
    pub(crate) fn set_impulses(&mut self, p: usize, seed: [f32; 3]) {
        debug_assert!(
            p < self.count as usize,
            "invariant: the store writes a stored point of the record's shape"
        );
        self.n[p] = seed[0];
        self.t1[p] = seed[1];
        self.t2[p] = seed[2];
    }

    /// Appends one point (the carry's compaction).
    #[inline]
    fn push(&mut self, fid: u16, seed: [f32; 3]) {
        let j = self.count as usize;
        debug_assert!(
            j < MAX_CONTACT_POINTS,
            "invariant: a record holds at most four points"
        );
        self.fid[j] = fid;
        self.n[j] = seed[0];
        self.t1[j] = seed[1];
        self.t2[j] = seed[2];
        self.count = (j + 1) as u8;
    }

    /// Stored points.
    #[inline]
    pub(crate) fn count(&self) -> u8 {
        self.count
    }

    /// The record's 64 bytes as sixteen little-endian words, field by field (the G3
    /// layout-bytes gate): the three impulse rows, the feature ids in pairs, then
    /// the count with its zero padding.
    #[cfg(test)]
    pub(crate) fn words(&self) -> [u32; 16] {
        let mut w = [0u32; 16];
        for p in 0..MAX_CONTACT_POINTS {
            w[p] = self.n[p].to_bits();
            w[4 + p] = self.t1[p].to_bits();
            w[8 + p] = self.t2[p].to_bits();
        }
        w[12] = u32::from(self.fid[0]) | (u32::from(self.fid[1]) << 16);
        w[13] = u32::from(self.fid[2]) | (u32::from(self.fid[3]) << 16);
        w[14] = u32::from(self.count)
            | (u32::from(self._pad[0]) << 8)
            | (u32::from(self._pad[1]) << 16)
            | (u32::from(self._pad[2]) << 24);
        w[15] = u32::from(self._pad[3])
            | (u32::from(self._pad[4]) << 8)
            | (u32::from(self._pad[5]) << 16)
            | (u32::from(self._pad[6]) << 24);
        w
    }

    /// The stored impulses of the LAST stored point whose feature id is `fid`, or
    /// `None` (Lemma W: the last insert of a key wins).
    #[inline]
    pub(crate) fn seed_of(&self, fid: u16) -> Option<[f32; 3]> {
        let live = &self.fid[..self.count as usize];
        live.iter()
            .rposition(|&f| f == fid)
            .map(|p| [self.n[p], self.t1[p], self.t2[p]])
    }
}

/// Where a manifold's warm seeds come from (D2, ruling W1): the run `[lo, hi)` of
/// read-side positions whose key is the manifold's translated ordinal — record
/// indices on a strict read side (one at most), cold-index positions otherwise. A
/// miss is the empty run.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct WarmRun {
    /// First position of the run.
    pub(crate) lo: u32,
    /// One past the last position of the run.
    pub(crate) hi: u32,
}

impl WarmRun {
    /// No source: the seeds are zero and the carry drops every point.
    pub(crate) const MISS: Self = Self { lo: 0, hi: 0 };
}

const _: () = assert!(size_of::<WarmRun>() == 8, "a plan entry is 8 B");

/// The strict-side search's result: the run, and whether the cursor could not
/// gallop forward to it (a descent of the translated ordinals, served by a binary
/// search over the prefix).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Found {
    /// The run holding the key (one record or empty).
    pub(crate) run: WarmRun,
    /// Whether the search went backward.
    pub(crate) backward: bool,
}

/// What [`plan_sources_restored`] counted for the G1 anti-vacuity counters.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PlanCounts {
    /// Lookups the strict-side cursor served by a backward binary search.
    pub(crate) backward_searches: u32,
    /// Whether the read side was non-strict and the cold index was built.
    pub(crate) cold_index: bool,
    /// Lookups that missed the read side and searched L10's restore source
    /// ([`plan_sources_restored`]).
    pub(crate) restore_searches: u32,
    /// Of those, the hits.
    pub(crate) restore_hits: u32,
}

/// One side of the colored solver's double-buffered warm store.
pub(crate) struct WarmRecords {
    /// HOT: the searched column. `keys[mi]` is [`ord`] of manifold `mi`'s pair in the
    /// rows of the step that wrote it.
    keys: ScratchColumn<u64>,
    /// Read on a hit, written by the store: `recs[mi]` is manifold `mi`'s points.
    recs: ScratchColumn<WarmRecord>,
    /// Whether `keys` is strictly increasing (the cursor search applies); else the
    /// cold index serves the lookups. `true` for an empty side.
    strict: bool,
}

impl WarmRecords {
    /// An empty side pre-sized for `manifolds` records, its two columns under
    /// `keys_id` and `recs_id` (two ids, so the two columns keep distinct cache-set
    /// staggers). Registers the scratch layouts (idempotent) first.
    pub(crate) fn with_capacity(
        keys_id: ComponentId,
        recs_id: ComponentId,
        manifolds: usize,
    ) -> Self {
        register_scratch_layouts();
        Self {
            keys: ScratchColumn::new(
                keys_id,
                manifolds.max(scratch_reserve_rows(size_of::<u64>())),
            ),
            recs: ScratchColumn::new(
                recs_id,
                manifolds.max(scratch_reserve_rows(size_of::<WarmRecord>())),
            ),
            strict: true,
        }
    }

    /// Stored manifolds (the G2 / G8 gates read it).
    #[cfg(test)]
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether the keys are strictly increasing.
    #[inline]
    pub(crate) fn strict(&self) -> bool {
        self.strict
    }

    /// The key column.
    #[inline]
    pub(crate) fn keys(&self) -> &[u64] {
        self.keys.as_read_slice()
    }

    /// The records.
    #[inline]
    pub(crate) fn recs(&self) -> &[WarmRecord] {
        self.recs.as_read_slice()
    }

    /// Sizes both columns to exactly `n` manifolds for a step that writes this side
    /// by index. Fills only on growth (O(1) in the steady state); every entry is then
    /// overwritten by [`plan_sources_restored`] (the keys) and the store (the records).
    pub(crate) fn resize(&mut self, n: usize) {
        self.keys.build_view().resize(n, 0);
        self.recs.build_view().resize(n, WarmRecord::EMPTY);
    }

    /// The key column for writing by index (after [`resize`](Self::resize)).
    #[inline]
    pub(crate) fn keys_mut(&mut self) -> ScratchBuildView<'_, u64> {
        self.keys.build_view()
    }

    /// The records for writing by index (after [`resize`](Self::resize)).
    #[inline]
    pub(crate) fn recs_mut(&mut self) -> ScratchBuildView<'_, WarmRecord> {
        self.recs.build_view()
    }

    /// Records whether the keys written this step are strictly increasing.
    #[inline]
    pub(crate) fn set_strict(&mut self, strict: bool) {
        self.strict = strict;
    }

    /// Writes `read` merged with `extra` into this side, by key: L10's D-H drain (ruling on
    /// rev 2.3, open question 2), which hands the warm records of the islands a flush restores
    /// to a solve that searches `read` alone. `extra` is strictly sorted and its keys are not
    /// in `read` (no stream manifold of the step that wrote `read` named a held row), so on a
    /// strict `read` the merge is strict and every lookup finds what a search of the two
    /// sources finds; on a non-strict `read` `extra` is appended and the cold index serves
    /// the lookups, whose runs are single records either way. O(|read| + |extra|), on a
    /// flush step only.
    pub(crate) fn merge_of(&mut self, read: &WarmRecords, extra: &WarmRecords) {
        let (rk, rr) = (read.keys(), read.recs());
        let (ek, er) = (extra.keys(), extra.recs());
        debug_assert!(extra.strict, "invariant: the drained source is strictly sorted");
        self.resize(rk.len() + ek.len());
        let mut keys = self.keys.build_view();
        let mut recs = self.recs.build_view();
        let (keys, recs) = (keys.as_mut_slice(), recs.as_mut_slice());
        if read.strict {
            let (mut i, mut j) = (0, 0);
            for (k, r) in keys.iter_mut().zip(recs.iter_mut()) {
                let take_read = j == ek.len() || (i < rk.len() && rk[i] < ek[j]);
                debug_assert!(
                    j == ek.len() || i == rk.len() || rk[i] != ek[j],
                    "invariant: the drained keys are not in the read side"
                );
                if take_read {
                    (*k, *r) = (rk[i], rr[i]);
                    i += 1;
                } else {
                    (*k, *r) = (ek[j], er[j]);
                    j += 1;
                }
            }
        } else {
            let (k0, k1) = keys.split_at_mut(rk.len());
            k0.copy_from_slice(rk);
            k1.copy_from_slice(ek);
            let (r0, r1) = recs.split_at_mut(rr.len());
            r0.copy_from_slice(rr);
            r1.copy_from_slice(er);
        }
        self.strict = read.strict;
    }

    /// D2's search on a STRICT read side. The cursor sits after the last result, so
    /// every key before it is at most the last key looked up: a key above the key
    /// just before the cursor lies at or after the cursor and is found by galloping
    /// forward; a key at or below it (a descent of the translated ordinals) lies
    /// before the cursor and is found by a binary search over `[0, cursor)`, which
    /// the result reports. The cursor then moves past the hit, or to the insertion
    /// point on a miss. A few compares on the steady-state Identity step, where every
    /// lookup is the key at the cursor.
    #[inline]
    pub(crate) fn find(&self, cursor: &mut usize, key: u64) -> Found {
        debug_assert!(
            self.strict,
            "invariant: the cursor search serves strict read sides only"
        );
        let keys = self.keys.as_read_slice();
        let len = keys.len();
        let cur = *cursor;
        let (i, backward) = if cur == 0 || keys[cur - 1] < key {
            // Gallop for the first position at or above `key`: every index below `lo`
            // holds a smaller key; the stride doubles until a probe reaches `key` or
            // the end, and the last bracket is bisected.
            let mut lo = cur;
            let mut hi = cur;
            let mut step = 1usize;
            while hi < len && keys[hi] < key {
                lo = hi + 1;
                hi += step;
                step <<= 1;
            }
            let hi = hi.min(len);
            (lo + keys[lo..hi].partition_point(|&k| k < key), false)
        } else {
            (keys[..cur].partition_point(|&k| k < key), true)
        };
        if i < len && keys[i] == key {
            *cursor = i + 1;
            Found {
                run: WarmRun {
                    lo: i as u32,
                    hi: i as u32 + 1,
                },
                backward,
            }
        } else {
            *cursor = i;
            Found {
                run: WarmRun::MISS,
                backward,
            }
        }
    }
}

/// The cold sorted index of a NON-STRICT read side: `(key, mi)` pairs sorted, so an
/// equal-key run is contiguous and ascending in `mi`. Built once per step that looks
/// such a side up (hand-built streams only), in place, no heap.
pub(crate) struct WarmIndex {
    /// The sorted pairs.
    sorted: ScratchColumn<(u64, u32)>,
}

impl WarmIndex {
    /// An empty index pre-sized for `manifolds` pairs under `id`.
    pub(crate) fn with_capacity(id: ComponentId, manifolds: usize) -> Self {
        register_scratch_layouts();
        Self {
            sorted: ScratchColumn::new(
                id,
                manifolds.max(scratch_reserve_rows(size_of::<(u64, u32)>())),
            ),
        }
    }

    /// Rebuilds the index over `keys`.
    pub(crate) fn build(&mut self, keys: &[u64]) {
        let mut view = self.sorted.build_view();
        view.resize(keys.len(), (0, 0));
        let sorted = view.as_mut_slice();
        for (i, (&k, e)) in keys.iter().zip(sorted.iter_mut()).enumerate() {
            *e = (k, i as u32);
        }
        // Tuple order: by key, then by manifold index — the run of a key is ascending
        // in `mi`, which the lookup walks in reverse. Unstable is exact here (the
        // pairs are distinct), and it is the in-place sort.
        sorted.sort_unstable();
    }

    /// The run of index positions whose key is `key`, [`WarmRun::MISS`] when none.
    #[inline]
    pub(crate) fn run(&self, key: u64) -> WarmRun {
        let sorted = self.sorted.as_read_slice();
        let lo = sorted.partition_point(|e| e.0 < key);
        let hi = lo + sorted[lo..].partition_point(|e| e.0 == key);
        if lo == hi {
            WarmRun::MISS
        } else {
            WarmRun {
                lo: lo as u32,
                hi: hi as u32,
            }
        }
    }

    /// The sorted pairs.
    #[inline]
    pub(crate) fn sorted(&self) -> &[(u64, u32)] {
        self.sorted.as_read_slice()
    }
}

/// The read side of a step, with the cold index that serves it when it is not
/// strict: the ONE lookup surface both the seed of a solved point and the B1 carry
/// go through (ruling W1).
#[derive(Clone, Copy)]
pub(crate) struct WarmLookup<'a> {
    /// The records the previous store wrote.
    pub(crate) read: &'a WarmRecords,
    /// The cold index over `read`'s keys, built by [`plan_sources_restored`] when `read` is
    /// not strict.
    pub(crate) index: &'a WarmIndex,
}

impl WarmLookup<'_> {
    /// The stored impulses for feature `fid` inside `run`: records by descending
    /// manifold index, points last to first, the first match wins (Lemma W). `None`
    /// when no stored point of the run carries `fid` (an empty run misses at once).
    #[inline]
    pub(crate) fn seed(self, run: WarmRun, fid: u16) -> Option<[f32; 3]> {
        let recs = self.read.recs();
        for pos in (run.lo..run.hi).rev() {
            let mi = if self.read.strict() {
                pos as usize
            } else {
                self.index.sorted()[pos as usize].1 as usize
            };
            if let Some(seed) = recs[mi].seed_of(fid) {
                return Some(seed);
            }
        }
        None
    }

    /// The record of a FROZEN manifold (B1): each of its `count` points looked up by
    /// feature id through [`seed`](Self::seed); the hits compact into the record in
    /// point order, a miss drops its point. Returns the record and the hit count.
    pub(crate) fn carry(self, run: WarmRun, m: &Manifold, count: usize) -> (WarmRecord, u32) {
        let mut rec = WarmRecord::EMPTY;
        for p in 0..count {
            let fid = point_fid(m, p);
            if let Some(seed) = self.seed(run, fid) {
                rec.push(fid, seed);
            }
        }
        let hits = u32::from(rec.count);
        (rec, hits)
    }
}

/// [`plan_sources_restored`] with no restore source: the entry the L11 gates drive.
#[cfg(test)]
pub(crate) fn plan_sources(
    manifolds: &[Manifold],
    remap: RowRemap<'_>,
    warm_start_enabled: bool,
    read: &WarmRecords,
    write: &mut WarmRecords,
    index: &mut WarmIndex,
    plan: &mut [WarmRun],
) -> PlanCounts {
    plan_sources_restored(
        manifolds,
        remap,
        warm_start_enabled,
        read,
        write,
        index,
        plan,
        None,
        |_| {},
    )
}

/// P-a (D4): the stream-order pass of the build. Writes every manifold's ordinal
/// into `write` (and its strictness), and the source run of every manifold into
/// `plan[mi]` from `read` through `remap`. With warm start disabled nothing is
/// written to `write` and every run is a miss; on a `Reset` the ordinals are still
/// written (the store follows) and every run is a miss without a search. Builds
/// `index` first when `read` is not strict and is looked up.
///
/// L10's second source (design 06 A3, 08 A3′): a manifold whose translated ordinal misses
/// `read` is searched in `restore` — the warm records of the kept manifolds restored this
/// step, keyed in the rows `read` is keyed in, strictly sorted — by the same D2 routine with a
/// cursor of its own, and `on_restore(mi)` marks a hit, whose run then names positions of
/// `restore` (the caller's `SRC_RESTORE`). The two key sets are disjoint (no stream manifold
/// of the step that wrote `read` named a held row), so a run lies in one source; a Reset or a
/// flipped pair misses both without a search.
///
/// `plan` and `write` must be sized for `manifolds.len()` entries.
#[allow(clippy::too_many_arguments)]
pub(crate) fn plan_sources_restored(
    manifolds: &[Manifold],
    remap: RowRemap<'_>,
    warm_start_enabled: bool,
    read: &WarmRecords,
    write: &mut WarmRecords,
    index: &mut WarmIndex,
    plan: &mut [WarmRun],
    restore: Option<&WarmRecords>,
    mut on_restore: impl FnMut(usize),
) -> PlanCounts {
    debug_assert_eq!(
        plan.len(),
        manifolds.len(),
        "invariant: one plan entry per manifold"
    );
    let mut counts = PlanCounts::default();
    if !warm_start_enabled {
        plan.fill(WarmRun::MISS);
        return counts;
    }
    debug_assert!(
        restore.is_none_or(WarmRecords::strict),
        "invariant: the restore source is strictly sorted"
    );
    let looks_up = !matches!(remap, RowRemap::Reset);
    if looks_up && !read.strict() {
        index.build(read.keys());
        counts.cold_index = true;
    }
    let mut strict = true;
    {
        let mut keys_w = write.keys_mut();
        let keys_w = keys_w.as_mut_slice();
        debug_assert_eq!(
            keys_w.len(),
            manifolds.len(),
            "invariant: one key per manifold"
        );
        let mut prev = 0u64;
        let mut cursor = 0usize;
        let mut restore_cursor = 0usize;
        for (mi, m) in manifolds.iter().enumerate() {
            let key = ord(m.body_a.0, m.body_b.0);
            strict &= mi == 0 || key > prev;
            prev = key;
            keys_w[mi] = key;
            // `manifold_pair` is `None` on a `Reset` and for an untranslatable pair:
            // both miss without a search.
            plan[mi] = match remap.manifold_pair(m) {
                Some((la, lb)) => {
                    let lookup = ord(la, lb);
                    let run = if read.strict() {
                        let found = read.find(&mut cursor, lookup);
                        counts.backward_searches += u32::from(found.backward);
                        found.run
                    } else {
                        index.run(lookup)
                    };
                    match restore {
                        Some(restore) if run == WarmRun::MISS => {
                            counts.restore_searches += 1;
                            let found = restore.find(&mut restore_cursor, lookup);
                            if found.run != WarmRun::MISS {
                                counts.restore_hits += 1;
                                on_restore(mi);
                            }
                            found.run
                        }
                        _ => run,
                    }
                }
                None => WarmRun::MISS,
            };
        }
    }
    write.set_strict(strict);
    counts
}
