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
//! Keyed by `pack(body_a, body_b)` on the dense [`BodyIndex`](crate::manifold::BodyIndex)
//! row indices, which are stable frame-to-frame for a stable scene (no
//! spawn/despawn between frames — the stacking case). A structural change
//! reshuffles the dense rows, so the matched keys differ for one frame: a
//! one-frame hysteresis miss, never a determinism break.
//!
//! # Capacity
//!
//! Sized `next_pow2(2 · pair_count)` (load ≤ 0.5, short probe chains). Capacity is
//! reused across frames — [`begin_frame`](BoxAxisCache::begin_frame) clears in
//! place, growing only when the pair count rises (principle 5).

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
}

impl BoxAxisCache {
    /// Builds an empty cache pre-sized for up to `pairs` body pairs (no later
    /// realloc until the pair count exceeds twice this).
    pub fn with_capacity(pairs: usize) -> Self {
        let len = next_pow2(2 * pairs.max(1));
        Self {
            slots: vec![AxisEntry::empty(); len],
            mask: len - 1,
        }
    }

    /// Ensures the table can hold `pairs` body pairs at load `≤ 0.5`, growing the
    /// backing `Vec` only when the pair count rises (never shrinking, to avoid
    /// churn). Steady state allocates nothing.
    ///
    /// Deliberately does NOT clear: this is a SINGLE in-place table, so THIS
    /// frame's [`get`](Self::get)s must see LAST frame's [`set`](Self::set)s. Each
    /// pair is touched at most once per frame (read its stored axis, then overwrite
    /// with the freshly chosen one), so no clear is needed — a pair's read always
    /// returns its own last-frame write. A pair that vanished leaves a stale entry
    /// that is simply never read again (or, if its key is reused, re-validated by
    /// the SAT against the live candidate axes — a one-frame miss at worst).
    pub fn begin_frame(&mut self, pairs: usize) {
        let len = next_pow2(2 * pairs.max(1));
        if len > self.slots.len() {
            self.slots.resize(len, AxisEntry::empty());
            self.mask = self.slots.len() - 1;
        }
    }

    /// The first probe slot for `key` — `(key · GOLDEN_64) >> shift` (the same
    /// Fibonacci high-bits hash as the warm-start table). Pure function of `key`
    /// and the table length.
    #[inline]
    fn home(&self, key: u64) -> usize {
        let bits = (self.mask + 1).trailing_zeros();
        let h = key.wrapping_mul(GOLDEN_64) >> (64 - bits);
        (h as usize) & self.mask
    }

    /// Looks up the SAT-axis index stored for body pair `(a, b)`, or `None` if the
    /// pair was not present last frame (a cold contact — the SAT then picks the
    /// global min axis with no hysteresis bias).
    ///
    /// Linear-probes from [`home`](Self::home); the first [`EMPTY`] slot ends the
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
    /// Linear-probes from [`home`](Self::home) to the pair's slot (or the first
    /// empty one). Each pair is set at most once per frame (narrowphase visits a
    /// pair once), so the table never holds two live entries for one pair.
    ///
    /// # Panics (debug only)
    ///
    /// `debug_assert!`s the table is not full (the caller sizes it for load `≤
    /// 0.5`); a full table is a sizing-invariant violation, not a runtime case.
    #[inline]
    pub fn set(&mut self, a: BodyIndex, b: BodyIndex, axis: usize) {
        let key = pack(a, b);
        debug_assert_ne!(key, EMPTY, "invariant: a real body-pair key cannot be EMPTY");
        let mut i = self.home(key);
        let mut probes = 0usize;
        loop {
            let slot = &mut self.slots[i];
            if slot.key == EMPTY || slot.key == key {
                slot.key = key;
                slot.axis = axis as u32;
                return;
            }
            i = (i + 1) & self.mask;
            probes += 1;
            debug_assert!(
                probes <= self.mask,
                "invariant: box-axis cache is full (load > 1); size it for load ≤ 0.5"
            );
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
}
