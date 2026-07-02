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
//! `(min, max)` pair order, and for each pair it READS the stored axis then WRITES
//! back the freshly chosen one — a read-then-overwrite within the same frame. A
//! single in-place open-addressed table is therefore sufficient and deterministic:
//! the value a pair reads is always last frame's write for that exact key (a key is
//! touched once per frame), independent of any other pair's traffic. There are no
//! tombstones; a pair that vanishes simply leaves a stale entry that is never read
//! again (and if its key is reused, the stale axis is re-validated against the
//! current candidates by the SAT, so a stale value is at worst a one-frame miss,
//! never a soundness or determinism break).
//!
//! # Keying (the dense-row assumption, shared with warm-start)
//!
//! Keyed by `pack(body_a, body_b)` on the dense [`BodyIndex`]
//! row indices, which are stable frame-to-frame for a stable scene (no
//! spawn/despawn between frames — the stacking case). A structural change
//! reshuffles the dense rows, so the matched keys differ for one frame: a
//! one-frame hysteresis miss, never a determinism break.
//!
//! # Capacity and eviction
//!
//! Sized `next_pow2(2 · pair_count)` (load ≤ 0.5, short probe chains). Capacity is
//! reused across frames — [`begin_frame`](BoxAxisCache::begin_frame) grows the
//! backing `Vec` only when the pair count rises (principle 5).
//!
//! Because there are no tombstones, a pair that vanishes leaves a STALE live entry
//! behind (the doc above explains why a stale value is at worst a one-frame miss).
//! Under pair-set churn those stale entries accumulate, so occupancy would climb
//! monotonically and eventually saturate the table (every slot occupied), turning a
//! probe of an absent key into a full-table walk. To bound this,
//! [`begin_frame`](BoxAxisCache::begin_frame) CLEARS the whole table (dropping all
//! entries to [`EMPTY`]) whenever live occupancy has passed the load-≤-0.5 target
//! (`occupied > len / 2`) or whenever the table grows. A wholesale clear costs a
//! single frame of warm-start misses — the same one-frame cost the module already
//! documents as acceptable for a key remap — while keeping the steady-state load
//! bounded and every probe chain short.

use crate::manifold::BodyIndex;

/// The empty-slot sentinel key. A real packed key can never equal it: [`pack`]
/// places the two `u32` body indices in the high/low 32-bit halves, so producing
/// `u64::MAX` would need `body_a == body_b == u32::MAX` — a manifold never
/// self-pairs and never keys the `u32::MAX` SDF sentinel row.
const EMPTY: u64 = u64::MAX;

/// The 64-bit multiplicative-hash constant (Fibonacci hashing — `2^64 / φ`, odd),
/// matching the warm-start table so the two caches scramble keys identically.
const GOLDEN_64: u64 = 0x9E37_79B9_7F4A_7C15;

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
struct AxisEntry {
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

/// A flat open-addressed table mapping a body pair to its last-frame SAT-axis
/// index — the box-box reference-axis hysteresis store (P2 W4).
///
/// Embedded in [`Manifolds`](crate::resources::Manifolds) (narrowphase output), so
/// it needs no extra resource wiring. The backing `Vec` capacity is reused across
/// frames.
#[derive(Default)]
pub struct BoxAxisCache {
    /// The slots; length is always a power of two (`mask = len - 1`). Empty slots
    /// carry the [`EMPTY`] sentinel key.
    slots: Vec<AxisEntry>,
    /// `slots.len() - 1` — the power-of-two index mask for the probe.
    mask: usize,
    /// `64 - log2(len)` — the Fibonacci-hash right-shift, cached alongside `mask` so
    /// the per-probe [`home`](Self::home) avoids recomputing `trailing_zeros`.
    shift: u32,
    /// Live (non-[`EMPTY`]) slot count. Drives the load-based clear in
    /// [`begin_frame`](Self::begin_frame): without it, stale entries from vanished
    /// pairs would accumulate under churn until the table saturates.
    occupied: usize,
}

impl BoxAxisCache {
    /// Builds an empty cache pre-sized for up to `pairs` body pairs (no later
    /// realloc until the pair count exceeds twice this).
    pub fn with_capacity(pairs: usize) -> Self {
        let len = next_pow2(2 * pairs.max(1));
        Self {
            slots: vec![AxisEntry::empty(); len],
            mask: len - 1,
            shift: shift_for(len),
            occupied: 0,
        }
    }

    /// Prepares the table for a frame that will touch `pairs` body pairs, keeping
    /// the load factor `≤ 0.5` and bounding the live occupancy against churn.
    ///
    /// This is a SINGLE in-place table, so in the steady state THIS frame's
    /// [`get`](Self::get)s must see LAST frame's [`set`](Self::set)s: each pair is
    /// touched at most once per frame (read its stored axis, then overwrite with the
    /// freshly chosen one), so a pair's read always returns its own last-frame write.
    /// A pair that vanished leaves a STALE entry behind (never read again unless its
    /// key is reused, in which case the SAT re-validates it — a one-frame miss at
    /// worst). Under pair-set churn those stale entries accumulate, so the table is
    /// CLEARED whenever either:
    ///
    /// - it must grow to fit `pairs` (a fresh larger buffer starts empty anyway), or
    /// - live occupancy has passed the load-≤-0.5 target (`occupied > len / 2`).
    ///
    /// A clear drops every entry to [`EMPTY`], costing one frame of warm-start misses
    /// (the same one-frame cost the module already accepts for a key remap) in
    /// exchange for a bounded steady-state load and short probe chains. When neither
    /// trigger fires, the table is left in place and allocates nothing.
    pub fn begin_frame(&mut self, pairs: usize) {
        let len = next_pow2(2 * pairs.max(1));
        if len > self.slots.len() {
            // Grow to a fresh larger buffer; it starts empty, so occupancy resets.
            self.slots.clear();
            self.slots.resize(len, AxisEntry::empty());
            self.mask = len - 1;
            self.shift = shift_for(len);
            self.occupied = 0;
        } else if self.occupied > self.slots.len() / 2 {
            // Stale entries have pushed the load past 0.5; wholesale clear to bound
            // occupancy and keep probe chains short (a one-frame warm-start miss).
            for slot in &mut self.slots {
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
        let h = key.wrapping_mul(GOLDEN_64) >> self.shift;
        (h as usize) & self.mask
    }

    /// Looks up the SAT-axis index stored for body pair `(a, b)`, or `None` if the
    /// pair was not present last frame (a cold contact — the SAT then picks the
    /// global min axis with no hysteresis bias).
    ///
    /// Linear-probes from the `home` slot; the first `EMPTY` slot ends the
    /// chain (no tombstones, so a miss is unambiguous).
    #[inline]
    pub fn get(&self, a: BodyIndex, b: BodyIndex) -> Option<usize> {
        let key = pack(a, b);
        let mut i = self.home(key);
        let mut probes = 0usize;
        loop {
            let slot = self.slots[i];
            if slot.key == key {
                return Some(slot.axis as usize);
            }
            if slot.key == EMPTY {
                return None;
            }
            i = (i + 1) & self.mask;
            probes += 1;
            if probes > self.mask {
                // Full table with no match: only reachable on a sizing violation;
                // treat as a miss rather than loop.
                return None;
            }
        }
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
        let mut i = self.home(key);
        let mut probes = 0usize;
        loop {
            let slot = &mut self.slots[i];
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
            i = (i + 1) & self.mask;
            probes += 1;
            if probes > self.mask {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_round_trip() {
        let mut c = BoxAxisCache::with_capacity(8);
        c.begin_frame(8);
        c.set(BodyIndex(2), BodyIndex(5), 11);
        assert_eq!(c.get(BodyIndex(2), BodyIndex(5)), Some(11));
    }

    #[test]
    fn miss_returns_none() {
        let mut c = BoxAxisCache::with_capacity(8);
        c.begin_frame(8);
        c.set(BodyIndex(0), BodyIndex(1), 3);
        assert_eq!(c.get(BodyIndex(4), BodyIndex(7)), None);
    }

    #[test]
    fn overwrite_updates_axis() {
        let mut c = BoxAxisCache::with_capacity(8);
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
        let mut c = BoxAxisCache::with_capacity(count);
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
        c.slots.iter().filter(|s| s.key != EMPTY).count()
    }

    #[test]
    fn churn_keeps_occupancy_bounded_and_set_terminates() {
        // FIX 1 (t1): a FIXED pair budget per frame, but a DIFFERENT set of distinct
        // pairs every frame (full pair-set churn). Without eviction the stale entries
        // accumulate until the table saturates and `set` of an absent key spins
        // forever in release. With the load-based clear, occupancy stays bounded and
        // every `set` returns.
        let pairs_per_frame = 32usize;
        let mut c = BoxAxisCache::with_capacity(pairs_per_frame);
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
        let mut c = BoxAxisCache::with_capacity(4);
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
        let mut c = BoxAxisCache::with_capacity(8);
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
}
