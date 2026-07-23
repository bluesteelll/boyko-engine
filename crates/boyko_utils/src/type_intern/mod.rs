//! [`TypeIntern`] — a lock-free, allocation-free `TypeId → dense id` intern table.
//!
//! # Why this exists
//!
//! Rust does not monomorphise a `static` declared inside a generic function body
//! ([rust#22991]): every instantiation of `fn id_for<T>()` shares ONE static, so the first `T`
//! mints id 0 and every other `T` reads that same id back. The engine's registries therefore
//! cannot memoize with a per-`T` `OnceLock` and must intern by [`TypeId`] instead.
//!
//! Four registries independently reached for the obvious intern — `OnceLock<Mutex<HashMap<TypeId,
//! Id>>>` — and each documented it as "cold, registration-only". The 2026-07 audit traced the
//! callers and found all four claims false: the memo lookup IS the locked map, so the lock was
//! acquired on EVERY call, not on the first. `query_type_id()` alone admitted "~20-30 ns (Mutex
//! lock + HashMap lookup) … called ~50 times per frame" — and under the parallel scheduler that is
//! not 20 ns, it is one process-global lock every worker thread contends for, inside the frame.
//!
//! Per Principle 0 the capability belongs in the kernel once, not as four per-crate adapters:
//! one primitive whose atomics get audited once, used uniformly.
//!
//! # Design
//!
//! An open-addressed, write-once table of [`OnceLock`] slots with linear probing:
//!
//! - **Lookup is lock-free** — hash, then 1–2 acquire loads in the common case. No lock, no
//!   allocation, no `unsafe`. Entries are never removed and never move, so a probe that reaches an
//!   empty slot proves absence.
//! - **Minting takes a cold spin gate**, claimed at most once per distinct key per process and
//!   never on a hit. It exists only to make "probe, then claim" atomic: without it two threads
//!   racing on the same new key would mint two ids for one type — [rust#22991]'s collapse in
//!   reverse.
//! - **Dense ids** come from a separate counter, so the id stays usable as an array index while
//!   the table stays sparse (load factor ≤ 0.5) for probe-length reasons.
//!
//! A linear scan over the published prefix was rejected: it is O(registered types) per call, and
//! with the ~100+ distinct query shapes a real scene registers that is slower than the `HashMap`
//! it replaces. Being lock-free is not enough — it also has to be faster.
//!
//! [rust#22991]: https://github.com/rust-lang/rust/issues/22991

use std::hash::{Hash, Hasher};
use std::hint;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// A cheap non-cryptographic mixer for keys that are ALREADY high-quality hashes.
///
/// [`TypeId`](std::any::TypeId) is a compiler-generated 128-bit hash, so running SipHash over it
/// (what [`DefaultHasher`](std::collections::hash_map::DefaultHasher) would do) buys no
/// distribution and costs ~10-20 ns on a path whose whole point is to be cheaper than a mutex.
/// This is the fxhash mix: rotate, xor, multiply — a few cycles per 8-byte word.
#[derive(Default)]
struct KeyHasher {
    state: u64,
}

impl KeyHasher {
    /// fxhash's odd 64-bit multiplier (the golden-ratio constant).
    const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

    #[inline]
    fn mix(&mut self, word: u64) {
        self.state = (self.state.rotate_left(5) ^ word).wrapping_mul(Self::SEED);
    }
}

impl Hasher for KeyHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.state
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks {
            self.mix(u64::from_ne_bytes(c.try_into().expect("invariant: chunks_exact(8) yields 8 bytes")));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rest.len()].copy_from_slice(rest);
            self.mix(u64::from_ne_bytes(buf));
        }
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.mix(i);
    }

    #[inline]
    fn write_u128(&mut self, i: u128) {
        self.mix(i as u64);
        self.mix((i >> 64) as u64);
    }
}

/// Releases [`TypeIntern::gate`] on scope exit, including while unwinding.
///
/// A bare store would leak the gate if the caller's exhaustion path panics, turning a clean
/// terminal panic into a process-wide livelock as every later minter spins forever.
struct GateGuard<'a>(&'a AtomicBool);

impl Drop for GateGuard<'_> {
    #[inline]
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// A process-global `K → dense u32` intern table with a lock-free lookup path.
///
/// `SLOTS` must be a power of two and at least twice the number of ids the caller intends to
/// mint; [`MAX_IDS`](Self::MAX_IDS) reports the resulting cap. Both are checked at construction.
pub struct TypeIntern<K: 'static, const SLOTS: usize> {
    /// Write-once `(key, id)` cells. `None` terminates a probe — sound because entries are never
    /// removed, so absence at an empty slot is permanent for that key's probe sequence.
    slots: [OnceLock<(K, u32)>; SLOTS],
    /// Dense-id dispenser. Also the count of live entries.
    next: AtomicU32,
    /// Cold mutual exclusion for minting only. Never taken on a lookup hit.
    gate: AtomicBool,
}

impl<K: Copy + Eq + Hash + 'static, const SLOTS: usize> TypeIntern<K, SLOTS> {
    /// Largest number of distinct keys this table accepts, capping the load factor at 0.5 so
    /// linear probing stays short.
    pub const MAX_IDS: u32 = (SLOTS / 2) as u32;

    /// Compile-time shape check: a power-of-two `SLOTS` turns the modulo into a mask, and fewer
    /// than 2 slots cannot hold a single entry under the 0.5 load-factor rule.
    const SHAPE_OK: () = assert!(
        SLOTS >= 2 && SLOTS.is_power_of_two(),
        "TypeIntern SLOTS must be a power of two >= 2"
    );

    /// Creates an empty table. `const`, so callers declare it as a plain `static`.
    #[allow(clippy::new_without_default, reason = "const fn; Default cannot be const")]
    pub const fn new() -> Self {
        // Force evaluation of the shape assertion.
        let () = Self::SHAPE_OK;
        Self {
            slots: [const { OnceLock::new() }; SLOTS],
            next: AtomicU32::new(0),
            gate: AtomicBool::new(false),
        }
    }

    /// Probe start index for `key`.
    #[inline]
    fn home(key: &K) -> usize {
        let mut h = KeyHasher::default();
        key.hash(&mut h);
        (h.finish() as usize) & (SLOTS - 1)
    }

    /// Returns the id already interned for `key`, or `None` if it has never been minted.
    ///
    /// Lock-free: a hash plus one acquire load per probe. Every published slot was written before
    /// its `OnceLock` released, so a reader that sees the cell sees a fully-initialised pair.
    #[inline]
    pub fn get(&self, key: &K) -> Option<u32> {
        let mut i = Self::home(key);
        for _ in 0..SLOTS {
            match self.slots[i].get() {
                Some((k, id)) if k == key => return Some(*id),
                Some(_) => i = (i + 1) & (SLOTS - 1),
                None => return None,
            }
        }
        None
    }

    /// Returns `key`'s id, minting a fresh dense id on first sight.
    ///
    /// `None` means the table is full (`> MAX_IDS` distinct keys). Callers turn that into their
    /// own `#[cold]` terminal panic so the message can name the cap and the feature that raises
    /// it, rather than this primitive guessing at their vocabulary.
    #[inline]
    pub fn get_or_mint(&self, key: K) -> Option<u32> {
        match self.get(&key) {
            Some(id) => Some(id),
            None => self.mint(key, |id| id),
        }
    }

    /// Like [`get_or_mint`](Self::get_or_mint), but the id comes from `mint_value` instead of
    /// this table's counter.
    ///
    /// For registries that already own a dispenser with its own exhaustion policy and test hooks
    /// — `query_type_registry`'s `register_new()` mints from `QUERY_NEXT_ID` and panics with a
    /// message naming the cargo feature that raises the cap. Replacing that with this table's
    /// anonymous `None` would lose the diagnosis, so the table interns the mapping and leaves the
    /// dispenser alone. `mint_value` runs at most once per key, under the mint gate; the gate is
    /// released even if it panics.
    #[inline]
    pub fn get_or_mint_with(&self, key: K, mint_value: impl FnOnce(u32) -> u32) -> Option<u32> {
        match self.get(&key) {
            Some(id) => Some(id),
            None => self.mint(key, mint_value),
        }
    }

    /// First-sight path: claim the gate, re-probe (a racer may have minted `key` while we spun),
    /// and only then claim a slot and an id.
    #[cold]
    #[inline(never)]
    fn mint(&self, key: K, mint_value: impl FnOnce(u32) -> u32) -> Option<u32> {
        while self
            .gate
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            hint::spin_loop();
        }
        let _gate = GateGuard(&self.gate);

        // Re-probe under the gate: the winner of a race published while we spun.
        if let Some(id) = self.get(&key) {
            return Some(id);
        }
        let occupancy = self.next.load(Ordering::Relaxed);
        if occupancy >= Self::MAX_IDS {
            return None;
        }
        // Runs before the slot is claimed so a panicking dispenser leaves no half-entry behind.
        let id = mint_value(occupancy);
        let mut i = Self::home(&key);
        for _ in 0..SLOTS {
            if self.slots[i].get().is_none() {
                // Cannot fail: we hold the gate, and this slot was empty one line ago.
                let _ = self.slots[i].set((key, id));
                // Release AFTER the slot is published so `len` never over-reports.
                self.next.store(occupancy + 1, Ordering::Release);
                return Some(id);
            }
            i = (i + 1) & (SLOTS - 1);
        }
        None
    }

    /// Number of distinct keys interned so far.
    #[inline]
    pub fn len(&self) -> u32 {
        self.next.load(Ordering::Acquire)
    }

    /// `true` while no key has been interned.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use super::*;

    static T: TypeIntern<TypeId, 64> = TypeIntern::new();

    #[test]
    fn distinct_types_get_distinct_ids() {
        // The rust#22991 regression: a shared static would hand every type id 0.
        let a = T.get_or_mint(TypeId::of::<u8>()).expect("capacity");
        let b = T.get_or_mint(TypeId::of::<u16>()).expect("capacity");
        let c = T.get_or_mint(TypeId::of::<String>()).expect("capacity");
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
    }

    #[test]
    fn repeat_lookups_are_stable() {
        let first = T.get_or_mint(TypeId::of::<i64>()).expect("capacity");
        for _ in 0..1000 {
            assert_eq!(T.get_or_mint(TypeId::of::<i64>()), Some(first));
            assert_eq!(T.get(&TypeId::of::<i64>()), Some(first));
        }
    }

    #[test]
    fn absent_key_reads_none_without_minting() {
        static E: TypeIntern<TypeId, 16> = TypeIntern::new();
        assert_eq!(E.get(&TypeId::of::<u8>()), None);
        assert_eq!(E.len(), 0);
    }

    #[test]
    fn exhaustion_reports_none_rather_than_panicking() {
        static S: TypeIntern<(u64, u64), 8> = TypeIntern::new();
        assert_eq!(<TypeIntern<(u64, u64), 8>>::MAX_IDS, 4);
        for i in 0..4u64 {
            assert_eq!(S.get_or_mint((i, i)), Some(i as u32));
        }
        assert_eq!(S.get_or_mint((99, 99)), None, "past MAX_IDS must report full");
        // A full table still resolves keys it already holds.
        assert_eq!(S.get(&(2, 2)), Some(2));
    }

    #[test]
    fn concurrent_mint_of_one_key_yields_one_id() {
        static C: TypeIntern<(u64, u64), 256> = TypeIntern::new();
        let ids: Vec<u32> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..8)
                .map(|_| s.spawn(|| C.get_or_mint((7, 7)).expect("capacity")))
                .collect();
            handles.into_iter().map(|h| h.join().expect("thread")).collect()
        });
        assert!(ids.windows(2).all(|w| w[0] == w[1]), "one key must mint exactly one id: {ids:?}");
        assert_eq!(C.len(), 1);
    }

    #[test]
    fn concurrent_distinct_keys_get_unique_ids() {
        static D: TypeIntern<(u64, u64), 256> = TypeIntern::new();
        let mut ids: Vec<u32> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..64u64)
                .map(|k| s.spawn(move || D.get_or_mint((k, k)).expect("capacity")))
                .collect();
            handles.into_iter().map(|h| h.join().expect("thread")).collect()
        });
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 64, "each distinct key must own a distinct id");
        assert_eq!(D.len(), 64);
    }

    #[test]
    fn probing_survives_a_dense_table() {
        // 32 keys into 64 slots is the documented 0.5 load factor — every one must still resolve.
        static P: TypeIntern<(u64, u64), 64> = TypeIntern::new();
        for i in 0..32u64 {
            P.get_or_mint((i, i * 31)).expect("capacity");
        }
        for i in 0..32u64 {
            assert!(P.get(&(i, i * 31)).is_some(), "key {i} lost to probing");
        }
    }
}
