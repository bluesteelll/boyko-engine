//! The box-box reference-axis hysteresis store (P2 W3/W4 — the resting-stack
//! feature-id flicker guard).
//!
//! A near-parallel resting box pair has two (or more) SAT axes whose penetration
//! depths are equal to within FP noise. Which one the [`sat`](super::box_box) min
//! selects can flip frame to frame on the last bit, and because the feature ids
//! are derived from the chosen reference face, that flip is a warm-start MISS — a
//! lost support impulse — every frame, so the stack jitters apart.
//!
//! [`BoxAxisCache`] persists, per body pair, the SAT-axis index chosen last frame.
//! The box-box generator feeds it back as the hysteresis bias: if the current best
//! axis is no deeper than `HYSTERESIS_RATIO ×` last frame's axis, last frame's axis
//! is kept, so the reference face — hence the feature ids — stays put.
//!
//! # Why a single in-place table (not double-buffered like warm-start)
//!
//! Each frame narrowphase visits each body pair at most once, in deterministic
//! `(min, max)` pair order, and for each pair it READS the stored axis then, when the
//! pair produces a contact, WRITES back the freshly chosen one — a read-then-overwrite
//! within the same frame. A single in-place open-addressed table is therefore
//! sufficient and deterministic while the rows are unchanged: the value a pair reads is
//! the last write for that exact key (a key is touched once per frame), independent of
//! any other pair's traffic. There are no tombstones; a pair that vanishes leaves a
//! stale entry behind, and if a later pair reuses its key the stale axis is read as
//! that pair's hint. The SAT keeps a hint only while it is still a valid overlapping
//! candidate within the hysteresis ratio of the best depth, exactly like a legitimate
//! hint, so a stale
//! value can tip the choice between near-equal axes but is never a soundness or
//! determinism break. That bound comes from the SAT, not from time: an entry is
//! overwritten only on a step that produces a contact for its key.
//!
//! # Keying (row keys, carried across row moves)
//!
//! Keyed by `pack(body_a, body_b)` on the dense [`BodyIndex`] row indices. Rows are
//! not stable: a despawn swap-removes, a spawn appends, and a component insert or
//! remove shifts every later row. The table stays keyed by the rows of the step that
//! wrote it, so a step whose rows changed cannot read in the loop — when rows shift up
//! by one, pair `(a, b)` would read key `(a − 1, b − 1)`, which an earlier pair has
//! already overwritten this step. Instead `BoxAxisCache::begin_frame_synced` pre-reads
//! every box pair's previous axis through the gather's row identity map
//! (`row_identity.rs`, interim; U7's `PairCache` replaces it) before any write. A pair
//! whose row order flipped, or that names a new body, reads no bias for that step.
//!
//! # Capacity and eviction
//!
//! Sized `next_pow2(2 · pair_count)` (load ≤ 0.5, short probe chains). Capacity is
//! reused across frames — [`begin_frame`](BoxAxisCache::begin_frame) grows the
//! backing `Vec` only when the pair count rises (principle 5).
//!
//! Because there are no tombstones, a pair that vanishes leaves a STALE live entry
//! behind (the doc above explains what a stale value can and cannot do).
//! Under pair-set churn those stale entries accumulate, so occupancy would climb
//! monotonically and eventually saturate the table (every slot occupied), turning a
//! probe of an absent key into a full-table walk. To bound this,
//! [`begin_frame`](BoxAxisCache::begin_frame) CLEARS the whole table (dropping all
//! entries to [`EMPTY`]) whenever live occupancy has passed the load-≤-0.5 target
//! (`occupied > len / 2`) or whenever the table grows. A wholesale clear costs a
//! single frame of warm-start misses — the same one-frame cost a pair whose row order
//! flipped, or that names a new body, takes after a row move — while keeping the
//! steady-state load bounded and every probe chain short.

use boyko_ecs::ecs::core::component::scratch::ScratchColumn;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

use crate::components::ColliderShape;
use crate::manifold::BodyIndex;
use crate::resources::BodyState;
use crate::row_identity::{RemapCursor, RowIdentity, RowRemap};
use crate::scratch_ids::{axis_remap_id, register_narrowphase_column_layouts, scratch_reserve_rows};

/// The empty-slot sentinel key. A real packed key can never equal it: [`pack`]
/// places the two `u32` body indices in the high/low 32-bit halves, so producing
/// `u64::MAX` would need `body_a == body_b == u32::MAX` — a manifold never
/// self-pairs and never keys the `u32::MAX` SDF sentinel row.
const EMPTY: u64 = u64::MAX;

/// The 64-bit multiplicative-hash constant (Fibonacci hashing — `2^64 / φ`, odd),
/// matching the warm-start table so the two caches scramble keys identically.
const GOLDEN_64: u64 = 0x9E37_79B9_7F4A_7C15;

/// The carried-axis sentinel: no previous axis for this pair. SAT axis indices are `0..15`.
const AXIS_NONE: u8 = u8::MAX;

/// Packs a body pair `(body_a, body_b)` into a 64-bit key (the two dense row
/// indices in the high/low 32-bit halves).
///
/// No feature id rides here — the hysteresis is per BODY PAIR, not per feature
/// (the whole point is to keep the *feature choice* stable). The pair is keyed in
/// the manifold's `(body_a, body_b)` order, which broadphase emits as `(min, max)`
/// (D4), so the key is stable for a stable scene.
#[inline]
fn pack(body_a: BodyIndex, body_b: BodyIndex) -> u64 {
    ((body_a.0 as u64) << 32) | (body_b.0 as u64)
}

/// One cache slot — a packed body-pair key and the SAT-axis index chosen last
/// frame for that pair.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AxisEntry {
    /// The packed body-pair key ([`pack`]), or [`EMPTY`] for a free slot.
    key: u64,
    /// The canonical SAT-axis index (`0..15`) chosen for this pair last frame.
    axis: u32,
}

impl AxisEntry {
    /// An empty slot (the [`EMPTY`] sentinel key).
    #[inline]
    const fn empty() -> Self {
        Self {
            key: EMPTY,
            axis: 0,
        }
    }
}

/// The next power of two `≥ n`, with a floor of 1 (so an empty pair set still
/// yields a maskable 1-slot table).
#[inline]
fn next_pow2(n: usize) -> usize {
    if n <= 1 { 1 } else { n.next_power_of_two() }
}

/// The Fibonacci-hash right-shift for a table of `len` slots (`len` a power of two):
/// `64 - log2(len)`, so the multiplicative hash keeps the top `log2(len)` bits.
#[inline]
fn shift_for(len: usize) -> u32 {
    64 - len.trailing_zeros()
}

/// The first probe slot for `key` in a table with index mask `mask` and hash shift
/// `shift` — `(key · GOLDEN_64) >> shift`, the same Fibonacci high-bits hash as the
/// warm-start table.
#[inline]
fn home_slot(key: u64, mask: usize, shift: u32) -> usize {
    let h = key.wrapping_mul(GOLDEN_64) >> shift;
    (h as usize) & mask
}

/// Looks up the SAT-axis index stored under `key` in `slots`, or `None` on a miss.
///
/// A free function over the table geometry rather than a method, so the pre-read can
/// probe the slots while it holds the carried-axis column's refill view (disjoint
/// borrows). Linear-probes from the home slot; the first `EMPTY` slot ends the chain (no
/// tombstones, so a miss is unambiguous).
#[inline]
fn probe(slots: &[AxisEntry], mask: usize, shift: u32, key: u64) -> Option<usize> {
    let mut i = home_slot(key, mask, shift);
    let mut probes = 0usize;
    loop {
        let slot = slots[i];
        if slot.key == key {
            return Some(slot.axis as usize);
        }
        if slot.key == EMPTY {
            return None;
        }
        i = (i + 1) & mask;
        probes += 1;
        if probes > mask {
            // Full table with no match: only reachable on a sizing violation;
            // treat as a miss rather than loop.
            return None;
        }
    }
}

/// A flat open-addressed table mapping a body pair to its last-frame SAT-axis
/// index — the box-box reference-axis hysteresis store (P2 W4).
///
/// Embedded in [`Manifolds`](crate::resources::Manifolds) (narrowphase output), so
/// it needs no extra resource wiring. The backing column's capacity is reused
/// across frames.
pub struct BoxAxisCache {
    /// The slots; length is always a power of two (`mask = len - 1`). Empty slots
    /// carry the [`EMPTY`] sentinel key. Backed by a [`ScratchColumn`] (engine
    /// storage, address-stable) rather than a `std::Vec` side allocation
    /// (audit Stage 4).
    slots: ScratchColumn<AxisEntry>,
    /// `slots.len() - 1` — the power-of-two index mask for the probe.
    mask: usize,
    /// `64 - log2(len)` — the Fibonacci-hash right-shift, cached alongside `mask` so
    /// the per-probe [`home`](Self::home) avoids recomputing `trailing_zeros`.
    shift: u32,
    /// Live (non-[`EMPTY`]) slot count. Drives the load-based clear in
    /// [`begin_frame`](Self::begin_frame): without it, stale entries from vanished
    /// pairs would accumulate under churn until the table saturates.
    occupied: usize,
    /// Per candidate pair `k`, the axis its bodies' pair chose when the table was last
    /// keyed, read through the row identity map before any write of a step whose rows
    /// changed (`AXIS_NONE` when there is none). Valid only on a frame
    /// `begin_frame_synced` pre-read. 1 B per pair (defect A, interim; U7 deletes it).
    remapped: ScratchColumn<u8>,
    /// The table's place in the gather sequence (defect A, interim; U7 deletes it).
    cursor: RemapCursor,
}

impl BoxAxisCache {
    /// Builds an empty cache pre-sized for up to `pairs` body pairs, backed by the
    /// kernel column registered under `id`.
    ///
    /// Registers the scratch layouts (idempotent) before creating the column.
    pub(crate) fn with_capacity(id: ComponentId, pairs: usize) -> Self {
        register_narrowphase_column_layouts();
        let len = next_pow2(2 * pairs.max(1));
        // The reserve is a HARD ceiling for a `ScratchColumn`, and `begin_frame`
        // grows the table with the pair count, so the floor is the same budget every
        // other scratch column gets rather than this frame's length.
        let reserve = len.max(scratch_reserve_rows(size_of::<AxisEntry>()));
        let mut slots = ScratchColumn::new(id, reserve);
        slots.build_view().resize(len, AxisEntry::empty());
        Self {
            slots,
            mask: len - 1,
            shift: shift_for(len),
            occupied: 0,
            remapped: ScratchColumn::new(
                axis_remap_id(),
                pairs.max(scratch_reserve_rows(size_of::<u8>())),
            ),
            cursor: RemapCursor::default(),
        }
    }

    /// Prepares the table for a frame that will touch `pairs` body pairs, keeping
    /// the load factor `≤ 0.5` and bounding the live occupancy against churn.
    ///
    /// This is a SINGLE in-place table, so in the steady state THIS frame's
    /// [`get`](Self::get)s must see LAST frame's [`set`](Self::set)s: each pair is
    /// touched at most once per frame (read its stored axis, then overwrite it with the
    /// freshly chosen one if the pair produces a contact), so while the rows are
    /// unchanged a pair's read returns the last write under its key. A pair that
    /// vanished leaves a STALE entry behind (read again only if a later pair reuses its
    /// key, in which case the SAT re-validates it as that pair's hint). Under pair-set
    /// churn those stale entries accumulate, so
    /// the table is CLEARED whenever either:
    ///
    /// - it must grow to fit `pairs` (a fresh larger buffer starts empty anyway), or
    /// - live occupancy has passed the load-≤-0.5 target (`occupied > len / 2`).
    ///
    /// A clear drops every entry to [`EMPTY`], costing one frame of warm-start misses
    /// (the same one-frame cost that a pair whose row order flipped, or that names a
    /// new body, takes after a row move) in exchange for a bounded steady-state load
    /// and short probe chains. When neither
    /// trigger fires, the table is left in place and allocates nothing.
    pub fn begin_frame(&mut self, pairs: usize) {
        let len = next_pow2(2 * pairs.max(1));
        if len > self.slots.len() {
            // Grow to a fresh larger table; it starts empty, so occupancy resets.
            // `clear` before `resize` is what makes it fresh — the surviving prefix
            // would otherwise keep last frame's keys.
            let mut view = self.slots.build_view();
            view.clear();
            view.resize(len, AxisEntry::empty());
            self.mask = len - 1;
            self.shift = shift_for(len);
            self.occupied = 0;
        } else if self.occupied > self.slots.len() / 2 {
            // Stale entries have pushed the load past 0.5; wholesale clear to bound
            // occupancy and keep probe chains short (a one-frame warm-start miss).
            for slot in self.slots.build_view().as_mut_slice() {
                slot.key = EMPTY;
            }
            self.occupied = 0;
        }
    }

    /// The first probe slot for `key` — `(key · GOLDEN_64) >> shift` (the same
    /// Fibonacci high-bits hash as the warm-start table). Pure function of `key`
    /// and the table length; `shift` is cached alongside `mask` so no per-probe
    /// `trailing_zeros` is needed.
    #[inline]
    fn home(&self, key: u64) -> usize {
        home_slot(key, self.mask, self.shift)
    }

    /// Looks up the SAT-axis index stored for body pair `(a, b)`, or `None` if the
    /// pair was not present last frame (a cold contact — the SAT then picks the
    /// global min axis with no hysteresis bias).
    ///
    /// Linear-probes from the `home` slot; the first `EMPTY` slot ends the
    /// chain (no tombstones, so a miss is unambiguous).
    #[inline]
    pub fn get(&self, a: BodyIndex, b: BodyIndex) -> Option<usize> {
        probe(self.slots.as_read_slice(), self.mask, self.shift, pack(a, b))
    }

    /// Stores the SAT-axis index `axis` chosen this frame for body pair `(a, b)`,
    /// overwriting any prior entry for the same pair.
    ///
    /// Linear-probes from the `home` slot to the pair's slot (or the first empty
    /// one). Each pair is set at most once per frame (narrowphase visits a pair
    /// once), so the table never holds two live entries for one pair; a probe that
    /// fills an empty slot bumps the live `occupied` count that
    /// [`begin_frame`](Self::begin_frame) uses to decide when to evict.
    ///
    /// If the probe chain reaches the full table length without finding a home (a
    /// sizing-invariant violation — the load-≤-0.5 sizing plus the `begin_frame`
    /// eviction make this unreachable in practice), the store simply DECLINES rather
    /// than looping forever: a cache may refuse an entry, and a missing hysteresis
    /// bias is only a one-frame warm-start miss. `debug_assert!` still flags the
    /// violation in debug builds.
    #[inline]
    pub fn set(&mut self, a: BodyIndex, b: BodyIndex, axis: usize) {
        let key = pack(a, b);
        debug_assert_ne!(key, EMPTY, "invariant: a real body-pair key cannot be EMPTY");
        // The probe geometry goes into locals BEFORE the refill view is taken:
        // `home` borrows all of `&self`, while the view holds `&mut self.slots`.
        let mut i = self.home(key);
        let mask = self.mask;
        let mut view = self.slots.build_view();
        let slots = view.as_mut_slice();
        let mut probes = 0usize;
        loop {
            let slot = &mut slots[i];
            if slot.key == key {
                slot.axis = axis as u32;
                return;
            }
            if slot.key == EMPTY {
                slot.key = key;
                slot.axis = axis as u32;
                self.occupied += 1;
                return;
            }
            i = (i + 1) & mask;
            probes += 1;
            if probes > mask {
                // Full table with no home for `key`: only reachable on a sizing
                // violation (mirrors `get`'s release-mode probe escape). Decline to
                // cache rather than loop — a cache may refuse an entry.
                debug_assert!(
                    false,
                    "invariant: box-axis cache is full (load > 1); size it for load ≤ 0.5"
                );
                return;
            }
        }
    }

    /// Prepares the table for a frame through the gather's row identity map (defect A,
    /// interim): classifies this consumer, pre-reads every box pair's previous axis when
    /// the rows changed, runs [`begin_frame`](Self::begin_frame), and stamps the cursor.
    /// Returns whether the pre-read ran — the `prefetched` flag `read_hint` takes.
    ///
    /// The pre-read is required on a step whose rows changed: when rows shift up by one,
    /// pair `(a, b)` reads key `(a − 1, b − 1)`, which an earlier pair in `(min, max)`
    /// order has already overwritten this step. It runs BEFORE `begin_frame`, whose clear
    /// on growth or high occupancy would otherwise erase the axes being carried; it needs
    /// only the pair list and the table's current mask and shift. On `Reset` it fills
    /// `AXIS_NONE` and skips the probes.
    pub(crate) fn begin_frame_synced(
        &mut self,
        pairs: &[(BodyIndex, BodyIndex)],
        bodies: &[BodyState],
        rows: &RowIdentity,
    ) -> bool {
        let remap = self.cursor.remap(rows);
        let prefetched = !matches!(remap, RowRemap::Identity);
        if prefetched {
            self.prefetch_remapped(pairs, bodies, remap);
            debug_assert_eq!(
                self.remapped.len(),
                pairs.len(),
                "invariant: one carried axis per candidate pair on a pre-read frame"
            );
        }
        self.begin_frame(pairs.len());
        // Every current pair's previous axis is captured, and every later write this step
        // is `set(a, b)` in current rows: the table is keyed by this gather from here on.
        self.cursor.stamp(rows);
        prefetched
    }

    /// Fills the carried-axis column: for every box-box candidate pair, the axis stored
    /// under the rows its bodies held when the table was keyed, or `AXIS_NONE`.
    #[cold]
    #[inline(never)]
    fn prefetch_remapped(
        &mut self,
        pairs: &[(BodyIndex, BodyIndex)],
        bodies: &[BodyState],
        remap: RowRemap<'_>,
    ) {
        let slots = self.slots.as_read_slice();
        let (mask, shift) = (self.mask, self.shift);
        let mut view = self.remapped.build_view();
        view.clear();
        view.resize(pairs.len(), AXIS_NONE);
        if matches!(remap, RowRemap::Reset) {
            return;
        }
        for (carried, &(a, b)) in view.as_mut_slice().iter_mut().zip(pairs) {
            let box_pair = matches!(bodies[a.0 as usize].shape, ColliderShape::Box { .. })
                && matches!(bodies[b.0 as usize].shape, ColliderShape::Box { .. });
            if !box_pair {
                continue;
            }
            let Some((pa, pb)) = remap.pair(a.0, b.0) else {
                continue;
            };
            if let Some(axis) = probe(slots, mask, shift, pack(BodyIndex(pa), BodyIndex(pb))) {
                debug_assert!(axis < AXIS_NONE as usize, "invariant: SAT axis indices are 0..15");
                *carried = axis as u8;
            }
        }
    }

    /// The previous axis carried for candidate pair `k` by the last pre-read, or `None`.
    #[inline]
    pub(crate) fn remapped_axis(&self, k: usize) -> Option<usize> {
        let axis = self.remapped.as_read_slice()[k];
        (axis != AXIS_NONE).then_some(axis as usize)
    }

    /// The hysteresis hint for candidate pair `k` = `(a, b)`: the carried axis on a
    /// pre-read frame, otherwise the table's own entry. The narrowphase and the unit
    /// tests share this one read.
    #[inline]
    pub(crate) fn read_hint(
        &self,
        prefetched: bool,
        k: usize,
        a: BodyIndex,
        b: BodyIndex,
    ) -> Option<usize> {
        if prefetched { self.remapped_axis(k) } else { self.get(a, b) }
    }

    /// Diagnostic: `Reset` classifications of this cache's cursor after its first stamp.
    /// Each one is a frame whose hysteresis hints were dropped instead of carried, so it
    /// stays flat while the narrowphase runs every step — a structural liveness gate.
    #[inline]
    pub fn remap_resets(&self) -> u64 {
        self.cursor.resets()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch_ids::box_axis_cache_id;

    #[test]
    fn set_get_round_trip() {
        let mut c = BoxAxisCache::with_capacity(box_axis_cache_id(), 8);
        c.begin_frame(8);
        c.set(BodyIndex(2), BodyIndex(5), 11);
        assert_eq!(c.get(BodyIndex(2), BodyIndex(5)), Some(11));
    }

    #[test]
    fn miss_returns_none() {
        let mut c = BoxAxisCache::with_capacity(box_axis_cache_id(), 8);
        c.begin_frame(8);
        c.set(BodyIndex(0), BodyIndex(1), 3);
        assert_eq!(c.get(BodyIndex(4), BodyIndex(7)), None);
    }

    #[test]
    fn overwrite_updates_axis() {
        let mut c = BoxAxisCache::with_capacity(box_axis_cache_id(), 8);
        c.begin_frame(8);
        c.set(BodyIndex(1), BodyIndex(2), 5);
        c.set(BodyIndex(1), BodyIndex(2), 9);
        assert_eq!(c.get(BodyIndex(1), BodyIndex(2)), Some(9));
    }

    #[test]
    fn pack_never_equals_empty() {
        for a in 0..8u32 {
            for b in 0..8u32 {
                assert_ne!(pack(BodyIndex(a), BodyIndex(b)), EMPTY);
            }
        }
    }

    #[test]
    fn distinct_pairs_independent() {
        // Dense fill at the design load (≤ 0.5): every pair round-trips, no probe
        // chain corrupts a neighbor (the read/write protocol the hysteresis needs).
        let count = 64usize;
        let mut c = BoxAxisCache::with_capacity(box_axis_cache_id(), count);
        c.begin_frame(count);
        for i in 0..count {
            c.set(BodyIndex(i as u32), BodyIndex((i + 1) as u32), i % 15);
        }
        for i in 0..count {
            assert_eq!(
                c.get(BodyIndex(i as u32), BodyIndex((i + 1) as u32)),
                Some(i % 15)
            );
        }
    }

    /// Count live (non-`EMPTY`) slots directly — the eviction invariant is about
    /// physical occupancy, not the logical view through `get`.
    fn live_slots(c: &BoxAxisCache) -> usize {
        c.slots.as_read_slice().iter().filter(|s| s.key != EMPTY).count()
    }

    #[test]
    fn churn_keeps_occupancy_bounded_and_set_terminates() {
        // FIX 1 (t1): a FIXED pair budget per frame, but a DIFFERENT set of distinct
        // pairs every frame (full pair-set churn). Without eviction the stale entries
        // accumulate until the table saturates and `set` of an absent key spins
        // forever in release. With the load-based clear, occupancy stays bounded and
        // every `set` returns.
        let pairs_per_frame = 32usize;
        let mut c = BoxAxisCache::with_capacity(box_axis_cache_id(), pairs_per_frame);
        let capacity = c.slots.len();
        // Many more frames than the table could ever hold if entries never evicted.
        let frames = 200usize;
        for frame in 0..frames {
            c.begin_frame(pairs_per_frame);
            // Disjoint key ranges each frame ⇒ every pair is a brand-new key.
            let base = (frame * pairs_per_frame) as u32;
            for j in 0..pairs_per_frame as u32 {
                let a = base + j;
                // `set` must terminate every time (the release-mode hang guard).
                c.set(BodyIndex(a), BodyIndex(a + 1), (j as usize) % 15);
            }
            // Occupancy never exceeds the table length, and the load stays bounded:
            // the clear fires before saturation, so an absent key always finds an
            // EMPTY slot within the probe bound.
            assert!(
                live_slots(&c) <= capacity,
                "frame {frame}: occupancy {} exceeded capacity {capacity}",
                live_slots(&c)
            );
            assert_eq!(c.occupied, live_slots(&c), "frame {frame}: occupied counter desynced from physical live slots");
        }
        // After all the churn the table has not grown (fixed per-frame budget) and
        // load is still ≤ 1 — the saturation the old code hit is gone.
        assert_eq!(c.slots.len(), capacity, "table must not grow under fixed-budget churn");
        assert!(c.occupied <= capacity);
    }

    #[test]
    fn grow_leaves_no_unreachable_duplicates() {
        // FIX 1 (t2): the grow path must NOT `resize`-preserve entries into a table
        // whose `mask` changed (that strands old entries at unreachable homes and
        // duplicates keys). The clear-on-grow policy makes the grown table empty, so
        // every key that re-round-trips does so through exactly one slot.
        let mut c = BoxAxisCache::with_capacity(box_axis_cache_id(), 4);
        c.begin_frame(4);
        for i in 0..4u32 {
            c.set(BodyIndex(i), BodyIndex(i + 100), i as usize % 15);
        }
        let small = c.slots.len();

        // Force a grow: a larger pair count needs a bigger table.
        c.begin_frame(64);
        assert!(c.slots.len() > small, "begin_frame(64) must grow the table");
        // The grown table starts empty (clear-on-grow), so no stale/duplicate keys
        // survive at wrong homes.
        assert_eq!(live_slots(&c), 0, "grown table must be empty (no preserved duplicates)");
        assert_eq!(c.occupied, 0);

        // Re-populate and confirm each key maps to exactly one live slot (a duplicate
        // would show up as two live slots for one key range).
        for i in 0..4u32 {
            c.set(BodyIndex(i), BodyIndex(i + 100), i as usize % 15);
        }
        assert_eq!(live_slots(&c), 4, "each distinct key occupies exactly one slot");
        for i in 0..4u32 {
            assert_eq!(c.get(BodyIndex(i), BodyIndex(i + 100)), Some(i as usize % 15));
        }
    }

    #[test]
    fn load_based_clear_fires_before_saturation() {
        // Fill to just past the load-≤-0.5 target within one logical epoch, then a
        // fresh begin_frame(same budget) must clear (occupancy resets) rather than
        // grow — proving the eviction trigger is the load, not only the grow.
        let mut c = BoxAxisCache::with_capacity(box_axis_cache_id(), 8);
        let capacity = c.slots.len();
        c.begin_frame(8);
        // Insert enough distinct keys to push occupancy past len/2.
        let mut inserted = 0usize;
        let mut a = 0u32;
        while c.occupied <= capacity / 2 {
            c.set(BodyIndex(a), BodyIndex(a + 1), 0);
            a += 2;
            inserted += 1;
            assert!(inserted <= capacity, "should pass load 0.5 before filling the table");
        }
        assert!(c.occupied > capacity / 2);
        // Same budget ⇒ no grow, but the load trigger clears the table.
        c.begin_frame(8);
        assert_eq!(c.slots.len(), capacity, "must not grow on a same-budget frame");
        assert_eq!(c.occupied, 0, "load-based clear must reset occupancy");
        assert_eq!(live_slots(&c), 0);
    }

    // —— Defect A interim fix: the axis carry (T5, T5b, T10) ——————————————————

    use boyko_ecs::ecs::identifiers::primitives::EntityId;

    use crate::components::{Collider, RigidBody, RigidBodyMass};
    use crate::math::{Mat3, Quat, Vec3};

    /// `n` unit-mass unit cubes at the origin: every candidate pair among them is a box pair.
    fn box_bodies(n: usize) -> Vec<BodyState> {
        let body = RigidBody {
            position: Vec3::ZERO,
            linear_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
        };
        let mass = RigidBodyMass {
            inv_inertia: Mat3::IDENTITY,
            inv_mass: 1.0,
            restitution: 0.0,
            friction: 0.5,
        };
        let collider = Collider {
            shape: ColliderShape::Box {
                half_extents: Vec3::new(0.5, 0.5, 0.5),
            },
            layer: 1,
            mask: 1,
        };
        (0..n)
            .map(|_| BodyState::from_columns(&body, &mass, &collider, false, true, false))
            .collect()
    }

    /// Feeds one scripted gather into `rows`.
    fn gather(rows: &mut RowIdentity, ids: &[usize], added: &[u32]) {
        rows.begin_gather();
        {
            let (mut cur, mut add) = rows.gather_views();
            for &id in ids {
                cur.push(EntityId(id));
            }
            for &r in added {
                add.push(r);
            }
        }
        rows.finish_gather();
    }

    /// T5: when a body is inserted ahead of every row (`prev_row[r] = r - 1`), current pair
    /// (2, 3) writes key (2, 3) before current pair (3, 4) reads its previous key (2, 3). The
    /// pre-read must hand both pairs the axis they chose one step earlier. Red under M15 (the
    /// read translated inside the loop: pair (3, 4) would read pair (2, 3)'s fresh write 13).
    #[test]
    fn axis_prefetch_reads_every_moved_pair_before_the_loop_writes() {
        let mut rows = RowIdentity::with_capacity(0);
        let bodies = box_bodies(6);
        let mut c = BoxAxisCache::with_capacity(box_axis_cache_id(), 8);

        // Step 1: rows [A, B, C, D, E]; pair (B, C) chooses axis 4, pair (C, D) axis 9.
        gather(&mut rows, &[20, 21, 22, 23, 24], &[0, 1, 2, 3, 4]);
        let pairs = [(BodyIndex(1), BodyIndex(2)), (BodyIndex(2), BodyIndex(3))];
        let prefetched = c.begin_frame_synced(&pairs, &bodies, &rows);
        for (k, (a, b), axis) in [(0, pairs[0], 4), (1, pairs[1], 9)] {
            let _ = c.read_hint(prefetched, k, a, b);
            c.set(a, b, axis);
        }

        // Step 2: X is inserted ahead of every row, so (B, C) is now (2, 3) and (C, D) is (3, 4).
        gather(&mut rows, &[30, 20, 21, 22, 23, 24], &[0]);
        let pairs = [(BodyIndex(2), BodyIndex(3)), (BodyIndex(3), BodyIndex(4))];
        let prefetched = c.begin_frame_synced(&pairs, &bodies, &rows);
        assert!(prefetched, "construction: the rows changed, so the pre-read must run");
        let first = c.read_hint(prefetched, 0, pairs[0].0, pairs[0].1);
        c.set(pairs[0].0, pairs[0].1, 13);
        let second = c.read_hint(prefetched, 1, pairs[1].0, pairs[1].1);
        c.set(pairs[1].0, pairs[1].1, 14);
        assert_eq!(
            c.get(BodyIndex(2), BodyIndex(3)),
            Some(13),
            "construction: pair (2, 3)'s write must have overwritten key (2, 3) before pair (3, 4) read"
        );
        assert_eq!(
            (first, second),
            (Some(4), Some(9)),
            "T5: pairs (B, C) and (C, D), moved up one row, must read the axes they chose last step \
             through the pre-read, in loop order"
        );
    }

    /// T5b: on a step whose rows shift down by one AND whose pair count grows past the table,
    /// `begin_frame` reallocates and clears. The moved pair's carried axis must survive,
    /// because the pre-read runs before that clear. Red under M11 (`begin_frame` before the
    /// pre-read: the probe finds an empty table).
    #[test]
    fn axis_prefetch_survives_a_grow_clear() {
        let mut rows = RowIdentity::with_capacity(0);
        let bodies = box_bodies(4);
        let mut c = BoxAxisCache::with_capacity(box_axis_cache_id(), 1);
        let small = c.slots.len();

        gather(&mut rows, &[40, 41, 42], &[0, 1, 2]);
        let pair = (BodyIndex(1), BodyIndex(2));
        let prefetched = c.begin_frame_synced(&[pair], &bodies, &rows);
        let _ = c.read_hint(prefetched, 0, pair.0, pair.1);
        c.set(pair.0, pair.1, 6);

        // X inserted ahead: (41, 42) moves from (1, 2) to (2, 3), and two pairs outgrow the table.
        gather(&mut rows, &[50, 40, 41, 42], &[0]);
        let pairs = [(BodyIndex(0), BodyIndex(1)), (BodyIndex(2), BodyIndex(3))];
        let prefetched = c.begin_frame_synced(&pairs, &bodies, &rows);
        assert!(prefetched, "construction: the rows changed, so the pre-read must run");
        assert!(
            c.slots.len() > small && live_slots(&c) == 0,
            "construction: two pairs must grow the {small}-slot table and clear it (slots {}, live {})",
            c.slots.len(),
            live_slots(&c)
        );
        assert_eq!(
            c.remapped_axis(1),
            Some(6),
            "T5b: remapped axis None after a grow-clear: the moved pair's axis must be carried \
             from before the clear"
        );
        assert_eq!(c.remapped_axis(0), None, "the pair naming the new body carries nothing");
    }

    /// One scripted narrowphase frame over the single pair (1, 2): classify and pre-read, read
    /// the hint the narrowphase would pass to the SAT, then store `axis`. Returns the hint.
    fn t10_frame(c: &mut BoxAxisCache, bodies: &[BodyState], rows: &RowIdentity, axis: usize) -> Option<usize> {
        let pair = (BodyIndex(1), BodyIndex(2));
        let prefetched = c.begin_frame_synced(&[pair], bodies, rows);
        let hint = c.read_hint(prefetched, 0, pair.0, pair.1);
        c.set(pair.0, pair.1, axis);
        hint
    }

    /// T10: the axis cache's cursor leaves `Reset` after one consumption and stays out of it
    /// while the narrowphase runs every gather. Ids `[10, 11, 12]` every gather, nothing
    /// added. Red under M9 (stamp only when the pre-read ran): gather 2 is `Identity`, so it
    /// does not stamp, and gather 3 classifies `Reset` (resets 1, read None).
    #[test]
    fn axis_consumer_leaves_reset() {
        const IDS: [usize; 3] = [10, 11, 12];
        let mut rows = RowIdentity::with_capacity(0);
        let bodies = box_bodies(3);
        let mut c = BoxAxisCache::with_capacity(box_axis_cache_id(), 1);

        /// One scripted gather: whether the narrowphase runs the cache on it, the axis the
        /// frame stores, and the expected hint and reset count.
        struct Gather {
            called: bool,
            store: usize,
            read: Option<usize>,
            resets: u64,
            what: &'static str,
        }
        let frame = |store, read, resets, what| Gather {
            called: true,
            store,
            read,
            resets,
            what,
        };
        let script = [
            frame(3, None, 0, "Rows (first gather, fresh cursor)"),
            frame(3, Some(3), 0, "Identity"),
            frame(3, Some(3), 0, "Identity (two consecutive gathers without a miss)"),
            Gather {
                called: false,
                store: 0,
                read: None,
                resets: 0,
                what: "gathered, cache not called",
            },
            frame(5, None, 1, "Reset (gather 4 was missed)"),
            frame(5, Some(5), 1, "Identity after the Reset"),
            frame(5, Some(5), 1, "Identity (liveness)"),
        ];
        for (index, step) in script.iter().enumerate() {
            let g = index + 1;
            gather(&mut rows, &IDS, &[]);
            if !step.called {
                continue;
            }
            let read = t10_frame(&mut c, &bodies, &rows, step.store);
            assert_eq!(
                (read, c.remap_resets()),
                (step.read, step.resets),
                "T10 gather {g} ({}): (read, remap_resets) - an axis cursor that resets after \
                 consecutive gathers without a miss drops the carried hint",
                step.what
            );
        }
    }
}
