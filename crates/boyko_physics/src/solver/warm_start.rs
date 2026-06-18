//! The cross-frame warm-start impulse cache (P2 W3 — the C3 rebuild-each-frame
//! flat table).
//!
//! TGS-Soft is a velocity-level sequential-impulse solver: each frame it must
//! re-converge every contact's accumulated impulse from a seed. Zero-seeding
//! (W2) re-discovers the resting impulse from scratch every step, so a stack of
//! spheres converges too slowly under the per-step substep budget and jitters
//! apart. **Warm-starting** seeds each contact with the impulse it converged to
//! last frame, so a resting stack starts at (or near) its steady state and stays
//! stable.
//!
//! # The C3 structure — rebuilt each frame, never tombstoned
//!
//! [`WarmStartTable`] is a flat open-addressed array keyed by
//! [`pack`]`(body_a, body_b, feature_id)`. It is **double-buffered**: a `read`
//! table (last frame's converged impulses) and a `write` table (this frame's).
//! Each frame the solver:
//!
//! 1. probes `read` to seed each contact (a [`miss`](WarmStartTable::get)
//!    returns a zero seed);
//! 2. after the solve, inserts each live contact's converged impulse into a
//!    FRESHLY-ZEROED `write` table, in deterministic manifold order;
//! 3. swaps `read` ↔ `write` at frame end.
//!
//! Because the write table is a pure function of *this frame's* contact set
//! (cleared then refilled in manifold order, no insertion history carried over),
//! it is bit-deterministic regardless of the previous frame's occupancy — the
//! determinism property the C3 resolution requires. There are NO tombstones and
//! NO stamp-based lazy deletion (those make occupancy depend on insertion
//! history, breaking determinism); a contact that vanished last frame simply has
//! no entry this frame.
//!
//! # Keying (the dense-row assumption, OQ-2)
//!
//! W3 keys on the dense [`BodyIndex`] row indices,
//! which are stable frame-to-frame for a stable scene (no spawn/despawn between
//! frames — the stacking case). A structural change reshuffles the dense rows,
//! so the matched keys differ for one frame: that is a warm-start MISS (a
//! one-frame convergence cost), never a determinism break or a soundness issue.
//! Entity-id keying for robustness across structural changes is the architect's
//! OQ-2 refinement, deliberately deferred past W3.
//!
//! # Capacity
//!
//! The table is sized `next_pow2(2 · contact_point_count)` so the load factor
//! stays ≤ 0.5 (open addressing degrades sharply past ~0.7; ≤ 0.5 keeps probe
//! chains short). Capacity is REUSED across frames — [`rebuild`](WarmStartTable::rebuild)
//! clears and (only if needed) grows the backing `Vec`, never allocating in the
//! steady state (principle 5).

use crate::manifold::{BodyIndex, SDF_SENTINEL};

/// The empty-slot sentinel key. A real packed key can never equal it: [`pack`]
/// lays out `body_a:24 | body_b:24 | feature_id:16`, so `u64::MAX` would require
/// `body_a == body_b == 0xFFFFFF` (both 24-bit fields all-ones) and
/// `feature_id == 0xFFFF`. A manifold always keys `body_a < body_b` (broadphase
/// emits the pair as `(min, max)`, D4), so the two body fields can never both be
/// `0xFFFFFF`; the sentinel is therefore structurally unreachable for any real
/// contact key (and `BodyIndex(u32::MAX)`, the C1 SDF sentinel, is never a
/// warm-start row anyway).
pub const EMPTY: u64 = u64::MAX;

/// The 64-bit multiplicative-hash constant (Fibonacci hashing — `2^64 / φ`,
/// odd). Multiplying the key by it and taking the high bits scrambles the dense,
/// low-entropy row-pair keys into well-spread slot indices.
const GOLDEN_64: u64 = 0x9E37_79B9_7F4A_7C15;

/// One warm-start cache entry — a contact's accumulated impulses keyed by
/// [`pack`] (P2 W3).
///
/// 24 bytes: an 8-byte key plus the three accumulated impulse scalars (one
/// normal, two tangent). `Copy` POD so the table is a flat reusable array.
#[derive(Clone, Copy, Debug)]
pub struct WarmEntry {
    /// The packed contact key ([`pack`]), or [`EMPTY`] for a free slot.
    pub key: u64,
    /// The converged accumulated normal impulse `λn ≥ 0`.
    pub normal_impulse: f32,
    /// The converged accumulated tangent impulses `(λt1, λt2)`.
    pub tangent_impulse: [f32; 2],
}

impl WarmEntry {
    /// An empty slot (the [`EMPTY`] sentinel key, zero impulses).
    #[inline]
    const fn empty() -> Self {
        Self {
            key: EMPTY,
            normal_impulse: 0.0,
            tangent_impulse: [0.0, 0.0],
        }
    }
}

/// The bit width of each body-index field in the packed key (24 bits = up to
/// 16M distinct bodies, far beyond any realistic per-step body count).
const BODY_BITS: u32 = 24;

/// The mask selecting one packed body-index field (low `BODY_BITS`).
const BODY_MASK: u64 = (1 << BODY_BITS) - 1;

/// The bit width of the feature-id field (16 bits — the full range the
/// narrowphase feature ids use; see [`feature_face_face`](crate::narrowphase::feature_face_face)).
const FEATURE_BITS: u32 = 16;

/// The mask selecting the packed feature-id field (low `FEATURE_BITS`).
const FEATURE_MASK: u64 = (1 << FEATURE_BITS) - 1;

/// Packs a contact POINT's identity into a single 64-bit key (P2 W3/W4).
///
/// The layout is **unconditionally injective** over realistic inputs:
/// `body_a:24 | body_b:24 | feature_id:16` (high → low). Distinct
/// `(body_a, body_b, feature_id)` triples therefore produce distinct keys for
/// any body index `< 2^24` (16M — far beyond a single step's body count) and any
/// `feature_id < 2^16` (the full narrowphase feature-id range; bit 15 is the
/// highest the class tags use). No field overlaps another, so non-zero W4 box
/// feature ids can never alias a body index — the per-point warm-start keys
/// (one key per contact POINT, not per manifold) rely on this.
///
/// The pair is keyed in the manifold's own `(body_a, body_b)` order — which
/// broadphase already emits as `(min, max)` (D4) — so the key is stable for a
/// stable scene without a re-sort here.
///
/// A real key never equals [`EMPTY`] (`u64::MAX`): the all-ones key needs both
/// body fields `== 0xFFFFFF`, but a manifold always keys `body_a < body_b`
/// (D4), so the two fields can never both be `0xFFFFFF` (and the `u32::MAX` SDF
/// sentinel row is never warm-started). See [`EMPTY`].
#[inline]
pub fn pack(body_a: BodyIndex, body_b: BodyIndex, feature_id: u32) -> u64 {
    // Non-overlapping fields: body_a in bits [40..64), body_b in [16..40),
    // feature_id in [0..16). Mask each field so an out-of-range input (a debug
    // bug) truncates into its own field rather than corrupting a neighbor.
    debug_assert!(body_a.0 as u64 <= BODY_MASK, "invariant: body_a fits in 24 bits");
    debug_assert!(body_b.0 as u64 <= BODY_MASK, "invariant: body_b fits in 24 bits");
    debug_assert!(feature_id as u64 <= FEATURE_MASK, "invariant: feature_id fits in 16 bits");
    let a = (body_a.0 as u64 & BODY_MASK) << (BODY_BITS + FEATURE_BITS);
    let b = (body_b.0 as u64 & BODY_MASK) << FEATURE_BITS;
    let f = feature_id as u64 & FEATURE_MASK;
    a | b | f
}

/// Packs an SDF-collision contact POINT's identity into a warm-start key (plan C1
/// — the sentinel `body_b` guard).
///
/// An SDF contact's `body_b` is [`SDF_SENTINEL`] (`u32::MAX`), which does NOT fit
/// the 24-bit `body_b` field — feeding it through [`pack`] would trip the field's
/// debug-assert and (in release) truncate `u32::MAX` to the all-ones 24-bit value,
/// which is also the value [`EMPTY`] needs in *both* body fields. This dedicated
/// path instead places the sentinel in `body_b` as the reserved all-ones 24-bit
/// tag `BODY_MASK` (`0xFFFFFF`):
///
/// - **No aliasing with a real pair:** a real body-body key's `body_b` is a dense
///   row index (a small body count, far below `0xFFFFFF`), so no real pair ever
///   produces `body_b == 0xFFFFFF`. SDF keys therefore occupy a disjoint slice of
///   the key space, keyed by `(body_a, feature_id)`.
/// - **Never [`EMPTY`]:** `EMPTY` (`u64::MAX`) requires BOTH 24-bit body fields
///   all-ones, but `body_a` here is a real dense row (`< 0xFFFFFF`), so the key
///   can never be all-ones.
/// - **Deterministic + injective:** distinct `(body_a, feature_id)` produce
///   distinct keys (non-overlapping fields), so a body's SDF contacts warm-start
///   independently and the table stays bit-deterministic (C3).
#[inline]
pub fn pack_sdf(body_a: BodyIndex, feature_id: u32) -> u64 {
    debug_assert_eq!(
        SDF_SENTINEL.0, u32::MAX,
        "invariant: the SDF sentinel is u32::MAX, the reserved out-of-range body_b"
    );
    // `body_b` field = the reserved all-ones 24-bit tag (a real dense row never
    // reaches it), so SDF keys never alias a body-body pair and never equal EMPTY.
    pack(body_a, BodyIndex(BODY_MASK as u32), feature_id)
}

/// The rounded-up power of two `≥ n`, with a floor of 1 (so an empty contact set
/// still yields a 1-slot table rather than a zero-length one that cannot be
/// masked). Used to size the table for a load factor `≤ 0.5`.
#[inline]
fn next_pow2(n: usize) -> usize {
    if n <= 1 {
        1
    } else {
        // The next power of two `≥ n`: `n.next_power_of_two()` is exact for the
        // contact counts a single step ever produces (far below `usize::MAX/2`).
        n.next_power_of_two()
    }
}

/// A flat open-addressed warm-start impulse table, rebuilt fresh each frame
/// (P2 W3 — C3).
///
/// One side of the double buffer (the solver owns a `read` + a `write`). The
/// backing `Vec` capacity is reused across frames; [`rebuild`](Self::rebuild)
/// clears and resizes it (growing only when the contact count rises), so the
/// steady state allocates nothing. Probing is a pure function of the key
/// (Fibonacci-hashed linear probe), so a fixed key set lands in fixed slots
/// regardless of insertion order — the determinism property.
#[derive(Default)]
pub struct WarmStartTable {
    /// The slots; length is always a power of two (`mask = len - 1`). Empty
    /// slots carry the [`EMPTY`] sentinel key.
    slots: Vec<WarmEntry>,
    /// `slots.len() - 1` — the power-of-two index mask for the probe.
    mask: usize,
}

impl WarmStartTable {
    /// Builds an empty table pre-sized for up to `contacts` contact points (no
    /// later realloc until the contact count exceeds twice this).
    pub fn with_capacity(contacts: usize) -> Self {
        let len = next_pow2(2 * contacts.max(1));
        Self {
            slots: vec![WarmEntry::empty(); len],
            mask: len - 1,
        }
    }

    /// Resizes the table to hold `contacts` contact points at load `≤ 0.5` and
    /// zeroes every slot for a fresh frame (C3 — no tombstones, no carried
    /// occupancy).
    ///
    /// Reuses the backing capacity: it only grows the `Vec` when the required
    /// power-of-two length exceeds the current one (and never shrinks it, to
    /// avoid churn), then fills every slot with the [`EMPTY`] sentinel. After
    /// this the table is empty and ready for in-manifold-order [`insert`s](Self::insert).
    pub fn rebuild(&mut self, contacts: usize) {
        let len = next_pow2(2 * contacts.max(1));
        if len > self.slots.len() {
            // Grow to the new power-of-two length (reusing the allocation when
            // the `Vec` already has the capacity).
            self.slots.resize(len, WarmEntry::empty());
        }
        self.mask = self.slots.len() - 1;
        // Zero every live slot (the whole buffer, including any slack past `len`
        // — `mask` covers the full current length).
        for slot in &mut self.slots {
            *slot = WarmEntry::empty();
        }
    }

    /// The first probe slot for `key` — `(key · GOLDEN_64) >> shift`. The linear
    /// `+1` probe step lives in [`insert`](Self::insert) / [`get`](Self::get),
    /// NOT here (folding it in would skip the true home slot).
    ///
    /// Pure function of `key` and the current table length, so the same key
    /// always starts at the same slot (the determinism property the C3
    /// resolution depends on).
    #[inline]
    fn home(&self, key: u64) -> usize {
        // High-bits multiplicative hash: the product's top `log2(len)` bits are
        // the well-mixed slot index. `shift = 64 - log2(len)`.
        let bits = (self.mask + 1).trailing_zeros();
        let h = key.wrapping_mul(GOLDEN_64) >> (64 - bits);
        (h as usize) & self.mask
    }

    /// Inserts (or overwrites) a contact's converged impulses under `key`, in the
    /// caller's deterministic manifold order (P2 W3 — C3).
    ///
    /// Linear-probes from the `home` slot to the first empty slot (or an
    /// existing entry for the same key, which it overwrites). Because the table
    /// is freshly zeroed each frame and the caller inserts each live key exactly
    /// once in a fixed order, the resulting occupancy is a pure function of the
    /// key set — independent of any previous frame.
    ///
    /// # Panics (debug only)
    ///
    /// `debug_assert!`s the table is not full (the caller sizes it for load
    /// `≤ 0.5`, so a full table is an invariant violation, not a runtime case);
    /// in release a full table would loop, which the `≤ 0.5` sizing prevents.
    #[inline]
    pub fn insert(&mut self, key: u64, normal_impulse: f32, tangent_impulse: [f32; 2]) {
        debug_assert_ne!(key, EMPTY, "invariant: a real contact key cannot be EMPTY");
        let mut i = self.home(key);
        // Bounded by the table length: with load ≤ 0.5 an empty slot is always
        // found well within `len` probes. The `debug_assert` guards a sizing bug.
        let mut probes = 0usize;
        loop {
            let slot = &mut self.slots[i];
            if slot.key == EMPTY || slot.key == key {
                slot.key = key;
                slot.normal_impulse = normal_impulse;
                slot.tangent_impulse = tangent_impulse;
                return;
            }
            i = (i + 1) & self.mask;
            probes += 1;
            debug_assert!(
                probes <= self.mask,
                "invariant: warm-start table is full (load > 1); size it for load ≤ 0.5"
            );
        }
    }

    /// Looks up the converged impulses stored under `key`, or `None` on a miss
    /// (P2 W3 — C3 seed lookup).
    ///
    /// Linear-probes from the `home` slot; the first `EMPTY` slot ends the
    /// chain (no tombstones, so a miss is unambiguous). A miss means the contact
    /// was not present last frame (a new or just-reformed contact) and the caller
    /// seeds it with zero impulses — a one-frame convergence cost, no error.
    #[inline]
    pub fn get(&self, key: u64) -> Option<WarmEntry> {
        debug_assert_ne!(key, EMPTY, "invariant: a real contact key cannot be EMPTY");
        let mut i = self.home(key);
        let mut probes = 0usize;
        loop {
            let slot = self.slots[i];
            if slot.key == key {
                return Some(slot);
            }
            if slot.key == EMPTY {
                // Hit an empty slot before the key: the key is absent (miss).
                return None;
            }
            i = (i + 1) & self.mask;
            probes += 1;
            if probes > self.mask {
                // Full table with no match — only reachable if the sizing
                // invariant was violated; treat as a miss rather than loop.
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Pure-function unit tests for the warm-start table (P2 W3). No solver, no
    //! schedule, no threads — just key packing, probe determinism, the
    //! rebuild-each-frame zero, and the order-independence (determinism)
    //! property. Run native and under Miri (zero `unsafe` here).

    use super::*;

    #[test]
    fn pack_is_injective_for_distinct_pairs() {
        // Distinct (a, b) pairs (and distinct feature ids) produce distinct keys.
        let k00 = pack(BodyIndex(0), BodyIndex(1), 0);
        let k10 = pack(BodyIndex(1), BodyIndex(0), 0);
        let k01 = pack(BodyIndex(0), BodyIndex(2), 0);
        let kf = pack(BodyIndex(0), BodyIndex(1), 1);
        assert_ne!(k00, k10, "pair order is significant");
        assert_ne!(k00, k01, "body_b is significant");
        assert_ne!(k00, kf, "feature_id is significant");
    }

    #[test]
    fn pack_is_injective_with_nonzero_features_across_realistic_range() {
        // The W4 injectivity property: distinct (body_a, body_b, feature_id)
        // triples produce distinct keys even with NON-ZERO feature ids and body
        // indices well past 2^16 — the regime the old XOR-fold aliased. Sample a
        // grid of body indices spanning the realistic range (including > 65536)
        // and the full feature-id range (face-face, edge-edge, vertex-face tags).
        let bodies = [0u32, 1, 7, 100, 65_535, 65_536, 70_000, 1_000_000, 16_777_214];
        let features = [0u32, 1, 0x2D, 0x8000, 0x8011, 0xC007, 0xFFFF];
        let mut seen = std::collections::HashSet::new();
        for &a in &bodies {
            for &b in &bodies {
                for &f in &features {
                    let key = pack(BodyIndex(a), BodyIndex(b), f);
                    assert!(
                        seen.insert(key),
                        "pack collision for (a={a}, b={b}, f={f:#x}) → {key:#x}"
                    );
                }
            }
        }
    }

    #[test]
    fn pack_sdf_never_aliases_a_body_body_pair_or_empty() {
        // C1: an SDF contact key (`pack_sdf`, sentinel body_b) must never collide
        // with a real body-body key (`pack` over realistic dense rows) nor equal
        // the EMPTY sentinel. The SDF key reserves the all-ones 24-bit body_b tag,
        // which a real dense row never reaches.
        let bodies = [0u32, 1, 7, 100, 65_536, 1_000_000, 16_777_213];
        let features = [0u32, 1, 0x8000, 0xC007, 0xFFFF];
        let mut body_body = std::collections::HashSet::new();
        for &a in &bodies {
            for &b in &bodies {
                for &f in &features {
                    // A real pair always keys body_b < BODY_MASK (a dense row never
                    // reaches the reserved all-ones tag), so skip the (unreal) case
                    // b == BODY_MASK; every realistic b here is far below it.
                    body_body.insert(pack(BodyIndex(a), BodyIndex(b), f));
                }
            }
        }
        let mut sdf_seen = std::collections::HashSet::new();
        for &a in &bodies {
            for &f in &features {
                let key = pack_sdf(BodyIndex(a), f);
                // Never the EMPTY sentinel (a real body_a is not all-ones).
                assert_ne!(key, EMPTY, "pack_sdf must not equal EMPTY (a={a}, f={f:#x})");
                // Never aliases a realistic body-body key.
                assert!(
                    !body_body.contains(&key),
                    "pack_sdf aliases a body-body key (a={a}, f={f:#x}) → {key:#x}"
                );
                // Distinct (body_a, feature_id) → distinct SDF keys (injective).
                assert!(
                    sdf_seen.insert(key),
                    "pack_sdf collision for (a={a}, f={f:#x}) → {key:#x}"
                );
            }
        }
    }

    #[test]
    fn pack_never_equals_empty_sentinel() {
        // A real key (with body_a < body_b, the broadphase D4 order) can never
        // collide with EMPTY across the realistic body range and the full
        // feature-id range — the two 24-bit body fields can never both be all-ones.
        let bodies = [0u32, 1, 7, 65_536, 1_000_000, 16_777_214];
        let features = [0u32, 1, 0x8000, 0xC007, 0xFFFF];
        for &a in &bodies {
            for &b in &bodies {
                for &f in &features {
                    assert_ne!(pack(BodyIndex(a), BodyIndex(b), f), EMPTY);
                }
            }
        }
    }

    #[test]
    fn insert_get_round_trip() {
        let mut t = WarmStartTable::with_capacity(8);
        t.rebuild(8);
        let key = pack(BodyIndex(3), BodyIndex(7), 0);
        t.insert(key, 1.25, [0.5, -0.25]);
        let got = t.get(key).expect("inserted key must be found");
        assert_eq!(got.normal_impulse, 1.25);
        assert_eq!(got.tangent_impulse, [0.5, -0.25]);
    }

    #[test]
    fn miss_returns_none() {
        let mut t = WarmStartTable::with_capacity(8);
        t.rebuild(8);
        t.insert(pack(BodyIndex(0), BodyIndex(1), 0), 1.0, [0.0, 0.0]);
        // A key that was never inserted misses (→ zero seed).
        assert!(t.get(pack(BodyIndex(2), BodyIndex(3), 0)).is_none());
    }

    #[test]
    fn home_is_deterministic_for_a_key() {
        // The probe home slot is a pure function of the key + table length.
        let t = WarmStartTable::with_capacity(16);
        let key = pack(BodyIndex(5), BodyIndex(9), 0);
        assert_eq!(t.home(key), t.home(key), "same key → same home slot");
    }

    #[test]
    fn rebuild_clears_previous_occupancy() {
        let mut t = WarmStartTable::with_capacity(8);
        t.rebuild(8);
        let key = pack(BodyIndex(1), BodyIndex(2), 0);
        t.insert(key, 9.0, [1.0, 2.0]);
        assert!(t.get(key).is_some());
        // A fresh frame: rebuild must zero the table (C3 — no carried occupancy).
        t.rebuild(8);
        assert!(t.get(key).is_none(), "rebuild must clear last frame's entries");
    }

    #[test]
    fn rebuild_is_order_independent_for_a_fixed_key_set() {
        // The C3 determinism property: a freshly-rebuilt table refilled with the
        // SAME key set produces identical lookups regardless of insertion order.
        let keys: [(u64, f32, [f32; 2]); 4] = [
            (pack(BodyIndex(0), BodyIndex(1), 0), 1.0, [0.1, 0.2]),
            (pack(BodyIndex(2), BodyIndex(5), 0), 2.0, [0.3, 0.4]),
            (pack(BodyIndex(3), BodyIndex(8), 0), 3.0, [0.5, 0.6]),
            (pack(BodyIndex(1), BodyIndex(9), 0), 4.0, [0.7, 0.8]),
        ];

        let mut forward = WarmStartTable::with_capacity(keys.len());
        forward.rebuild(keys.len());
        for &(k, n, t) in &keys {
            forward.insert(k, n, t);
        }

        let mut reverse = WarmStartTable::with_capacity(keys.len());
        reverse.rebuild(keys.len());
        for &(k, n, t) in keys.iter().rev() {
            reverse.insert(k, n, t);
        }

        // Every key resolves to the same stored value regardless of fill order.
        for &(k, n, t) in &keys {
            let f = forward.get(k).expect("forward has key");
            let r = reverse.get(k).expect("reverse has key");
            assert_eq!(f.normal_impulse, n);
            assert_eq!(r.normal_impulse, n);
            assert_eq!(f.tangent_impulse, t);
            assert_eq!(r.tangent_impulse, t);
        }
    }

    #[test]
    fn load_factor_stays_bounded_no_infinite_probe() {
        // Fill the table to its design load (≤ 0.5) and confirm every insert +
        // lookup terminates (no infinite probe) and every key round-trips.
        let count = 64usize;
        let mut t = WarmStartTable::with_capacity(count);
        t.rebuild(count);
        for i in 0..count {
            let key = pack(BodyIndex(i as u32), BodyIndex((i + 1000) as u32), 0);
            t.insert(key, i as f32, [0.0, 0.0]);
        }
        for i in 0..count {
            let key = pack(BodyIndex(i as u32), BodyIndex((i + 1000) as u32), 0);
            assert_eq!(
                t.get(key).expect("dense fill key must be found").normal_impulse,
                i as f32
            );
        }
    }

    #[test]
    fn overwrite_same_key_updates_value() {
        // Inserting the same key twice overwrites (last write wins), not duplicates.
        let mut t = WarmStartTable::with_capacity(8);
        t.rebuild(8);
        let key = pack(BodyIndex(4), BodyIndex(6), 0);
        t.insert(key, 1.0, [0.0, 0.0]);
        t.insert(key, 5.0, [1.0, 1.0]);
        let got = t.get(key).expect("key present");
        assert_eq!(got.normal_impulse, 5.0);
        assert_eq!(got.tangent_impulse, [1.0, 1.0]);
    }
}
