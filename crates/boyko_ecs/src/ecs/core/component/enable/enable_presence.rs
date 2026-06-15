//! Per-tag archetype-presence bitset — the EnableTag query cull *oracle*
//! (Decision D1 / D2, Step 3).
//!
//! For every EnableTag id, [`EnablePresence`] records the set of
//! `ArchetypeId`s that currently own an allocated `EnableColumn` for that tag.
//! A column exists iff the tag has ever been toggled into that archetype, so
//! the presence bit answers the cull's single question in O(1):
//!
//! > "Does archetype `A` have *any* enabled-or-once-toggled row for tag `T`?"
//!
//! `Enabled<T>` drops an archetype from the (already bounded) matched set when
//! `!contains(T, A)` — no column means every row is disabled. This is the
//! coarse cull; the per-row bit test refines it during iteration.
//!
//! # Why a bitset, not a driver (Decision D2 / critique C2)
//!
//! `EnablePresence` is consulted ONLY as the O(1) `contains` oracle over a set
//! the *required positive archetypal term* already bounds. It is **never** a
//! query driver: there is deliberately no `for_each_present` / `present_count`
//! (a presence-driven enumeration would have to shrink the full live-archetype
//! set, which is the unbounded sole-`Enabled` path the plan compile-rejects).
//! The set-walk seam for the deferred D7 "sole-flag / entity-disabling scan"
//! extension would add such an accessor; v1 does not.
//!
//! # Lock-free epoch read (Phase-22.1 `term_list` discipline)
//!
//! A column allocation ([`EnablePresence::note_column_alloc`]) sets one bit and
//! bumps a per-world epoch. A cache (e.g. a `QueryState` that culled an
//! enable-bearing query) snapshots [`EnablePresence::epoch`] and re-checks it
//! before reusing a culled set: an epoch change means "some archetype gained a
//! column since I last culled" — invalidate and re-cull. The read path is
//! purely atomic loads (no lock, no `Mutex`/`RwLock`), mirroring the
//! `TermScratch` epoch-stamp memo.
//!
//! The per-tag word arrays are lazily published through `AtomicPtr` (NonNull is
//! not used: the slot's *empty* state is the null pointer, and a raw
//! `AtomicPtr` keeps the publish/adopt protocol identical to `term_list`'s —
//! Phase 9.3c discipline). The bit words themselves are `AtomicU64`, so a read
//! never tears against a concurrent set even though v1 toggling is `&mut self`
//! exclusive (the atomics are the forward seam for D7 worker-marking, exactly
//! as the toggle bit is).

// Phase 9.1 lesson C1 / Phase 22.1: loom must drive the REAL atomics. The
// `cfg(loom)` aliases let a loom harness exercise the publish/read protocol
// verbatim; production builds use `core::sync::atomic`.
#[cfg(not(loom))]
use core::sync::atomic::{AtomicPtr, AtomicU64, Ordering};
#[cfg(loom)]
use loom::sync::atomic::{AtomicPtr, AtomicU64, Ordering};

use crate::ecs::core::component::component_registry::MAX_COMPONENTS;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

/// `u64` words per tag's archetype bitset. `16 * 64 = 1024` archetype ids of
/// presence capacity per tag — one `Box<[AtomicU64; 16]>` = 128 B, allocated
/// lazily on the tag's first column.
pub(crate) const PRESENCE_WORDS: usize = 16;

/// Highest `ArchetypeId` (exclusive) a presence bitset can record:
/// `PRESENCE_WORDS * 64`. An `ArchetypeId` at or beyond this would overflow the
/// per-tag word array — a `debug_assert` pins it (release: silently ignored,
/// matching the "no column ⇒ all-disabled ⇒ dropped" cull default, never UB).
pub(crate) const PRESENCE_CAPACITY: usize = PRESENCE_WORDS * 64;

/// A single tag's lazily-published archetype bitset: 16 `AtomicU64` words
/// (128 B). The pointer is published once via `AtomicPtr`; the words are then
/// set in place with `fetch_or`.
type PresenceWords = [AtomicU64; PRESENCE_WORDS];

/// Per-world per-tag archetype-presence bitset — the EnableTag cull
/// oracle (Decision D1 / D2).
///
/// One lazily-allocated `Box<[AtomicU64; 16]>` per EnableTag id, indexed by the
/// tag's [`ComponentId`]; a bit per `ArchetypeId`. `contains` is O(1) (one
/// pointer load + one word load + one bit test); `note_column_alloc` sets the
/// bit and bumps the lock-free epoch.
///
/// Auto `Send + Sync` (an array of `AtomicPtr` plus an `AtomicU64`); the
/// soundness of the lazy publish rests on the same release/acquire pairing the
/// `term_list` protocol uses (see module doc).
pub(crate) struct EnablePresence {
    /// One slot per possible EnableTag id; `null` until the tag's first column
    /// is allocated, then a published `Box<PresenceWords>` raw pointer.
    tags: [AtomicPtr<PresenceWords>; MAX_COMPONENTS],
    /// Monotonic counter bumped once per column allocation. A cache snapshots
    /// it before culling and re-checks it before reuse: a change means "an
    /// archetype gained a column" ⇒ re-cull (the lock-free invalidation seam,
    /// mirroring `TermScratch`'s generation stamps).
    epoch: AtomicU64,
}

impl EnablePresence {
    /// A fresh oracle — no tag has any allocated column (`epoch == 0`).
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            // `AtomicPtr::new(null)` is not `Copy`, so the array cannot be
            // built with `[expr; N]`; `from_fn` constructs each slot.
            tags: core::array::from_fn(|_| AtomicPtr::new(core::ptr::null_mut())),
            epoch: AtomicU64::new(0),
        }
    }

    /// Records that archetype `arch` now owns an allocated `EnableColumn` for
    /// `tag`, then bumps the epoch. Called once per column allocation (the
    /// toggle's first-touch-of-a-tag-in-an-archetype path) — exactly the site
    /// that also bumps `ArchetypeMaster::enable_generation` (Decision D1 inv 5).
    ///
    /// `&self`: the per-tag word array is lazily published through `AtomicPtr`,
    /// so no `&mut` is required even on first touch.
    ///
    /// # Panics (debug only)
    ///
    /// `debug_assert`s that the call is a genuine first column for
    /// `(tag, arch)` (the caller does not already hold the bit). In release the
    /// bit set is idempotent, so a redundant call is harmless apart from an
    /// extra epoch bump.
    #[inline]
    pub(crate) fn note_column_alloc(&self, tag: ComponentId, arch: ArchetypeId) {
        debug_assert!(
            tag.0 < MAX_COMPONENTS,
            "invariant: EnableTag id {} exceeds the {MAX_COMPONENTS}-slot id space",
            tag.0,
        );
        debug_assert!(
            arch.0 < PRESENCE_CAPACITY,
            "invariant: ArchetypeId {} exceeds the {PRESENCE_CAPACITY}-archetype \
             presence capacity",
            arch.0,
        );
        debug_assert!(
            !self.contains(tag, arch),
            "invariant: note_column_alloc must be called only on a genuine first \
             column for (tag {}, arch {}) — the caller already holds the bit",
            tag.0,
            arch.0,
        );

        if tag.0 >= MAX_COMPONENTS || arch.0 >= PRESENCE_CAPACITY {
            return;
        }

        let words = self.get_or_alloc_words(tag.0);
        let word = arch.0 >> 6;
        let bit = 1u64 << (arch.0 & 63);
        // Release: publishes the bit so an Acquire epoch read that observes the
        // matching epoch bump also observes this set bit. Pairs with the
        // Acquire word load in `contains`.
        words[word].fetch_or(bit, Ordering::Release);

        // Release: matches the Acquire load in `epoch()` — a reader that sees
        // the bumped epoch is guaranteed to see the set bit published above.
        self.epoch.fetch_add(1, Ordering::Release);
    }

    /// `true` iff archetype `arch` owns an allocated `EnableColumn` for `tag`.
    /// The O(1) cull oracle: one pointer load, one word load, one bit test.
    ///
    /// A never-allocated tag (null slot) or an out-of-range archetype returns
    /// `false` — both mean "no column ⇒ every row disabled ⇒ drop the
    /// archetype", the correct cull default.
    #[inline]
    pub(crate) fn contains(&self, tag: ComponentId, arch: ArchetypeId) -> bool {
        if tag.0 >= MAX_COMPONENTS || arch.0 >= PRESENCE_CAPACITY {
            return false;
        }
        // Acquire: pairs with the publishing Release CAS in `get_or_alloc_words`
        // — if the slot pointer is non-null we are guaranteed to read the fully
        // published word array.
        let ptr = self.tags[tag.0].load(Ordering::Acquire);
        if ptr.is_null() {
            return false;
        }
        // SAFETY: `ptr` is non-null, so it was published by `get_or_alloc_words`
        //   via `Box::into_raw` (or a CAS-loser adopting the winner's pointer);
        //   either way it points to a live `PresenceWords` that is never freed
        //   until `self` is dropped (see `Drop`). The Acquire load above
        //   synchronises with the publishing Release CAS, so the array is fully
        //   initialised. Only a shared `&` reborrow is taken, and the words are
        //   `AtomicU64`, so a concurrent `fetch_or` cannot tear this read.
        let words = unsafe { &*ptr };
        let word = arch.0 >> 6;
        // Acquire: pairs with the `fetch_or` Release in `note_column_alloc`.
        (words[word].load(Ordering::Acquire) >> (arch.0 & 63)) & 1 == 1
    }

    /// Current presence epoch — bumped once per column allocation. A cache
    /// snapshots this before culling an enable-bearing query and re-checks it
    /// before reuse: a change means re-cull (the lock-free invalidation seam).
    ///
    /// Acquire: a reader that observes a given epoch is guaranteed to observe
    /// every bit published before that epoch's bump (pairs with the Release
    /// `fetch_add` in `note_column_alloc`).
    #[inline]
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// Lazily publishes (or adopts) the per-tag word array for `tag_idx`,
    /// returning a shared reference valid for the call.
    ///
    /// Fast path: one Acquire load; if already published, return it. Slow path:
    /// allocate a zeroed `Box<PresenceWords>`, CAS-publish it; a racing loser
    /// frees its own candidate and adopts the winner — no spinning (mirrors
    /// `TermScratch::rebuild_publish`, protocol P1).
    #[inline]
    fn get_or_alloc_words(&self, tag_idx: usize) -> &PresenceWords {
        let slot = &self.tags[tag_idx];
        // Acquire: pairs with the publishing Release CAS below.
        let current = slot.load(Ordering::Acquire);
        if !current.is_null() {
            // SAFETY: non-null ⇒ published via `Box::into_raw`; never freed
            //   until `Drop`; Acquire-synchronised with the publish ⇒ fully
            //   initialised. Shared reborrow only.
            return unsafe { &*current };
        }
        self.alloc_words_slow(tag_idx)
    }

    /// Cold tail of [`Self::get_or_alloc_words`]: allocate and CAS-publish.
    #[cold]
    #[inline(never)]
    fn alloc_words_slow(&self, tag_idx: usize) -> &PresenceWords {
        let slot = &self.tags[tag_idx];
        let candidate: Box<PresenceWords> =
            Box::new(core::array::from_fn(|_| AtomicU64::new(0)));
        let raw: *mut PresenceWords = Box::into_raw(candidate);

        // Publish. Release (success) pairs with the Acquire load on every
        // reader; Acquire (failure) makes the winner pointer ready to deref.
        match slot.compare_exchange(
            core::ptr::null_mut(),
            raw,
            Ordering::Release,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                // SAFETY: we just published `raw` (a fresh `Box::into_raw`); it
                //   is the live word array for this tag, never freed until
                //   `Drop`. Shared reborrow only.
                unsafe { &*raw }
            }
            Err(winner) => {
                // A racer published first. Our candidate was never installed —
                // sole ownership — so free it.
                // SAFETY: `raw` came from `Box::into_raw` above and the CAS
                //   failed, so it was never installed in `slot` and no other
                //   thread can observe it; reconstructing the Box frees it
                //   exactly once.
                unsafe { drop(Box::from_raw(raw)) };
                // SAFETY: `winner` is non-null (a successful CAS installed it)
                //   and the failure ordering is Acquire, pairing with that
                //   publish ⇒ fully initialised. Never freed until `Drop`.
                unsafe { &*winner }
            }
        }
    }
}

impl Default for EnablePresence {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for EnablePresence {
    fn drop(&mut self) {
        // Exclusive `&mut self` — no concurrent access; plain loads suffice.
        // Frees every published per-tag word array.
        for slot in &self.tags {
            let ptr = slot.load(Ordering::Relaxed);
            if !ptr.is_null() {
                // SAFETY: `ptr` was published via `Box::into_raw` and is never
                //   freed elsewhere (`contains` / `get_or_alloc_words` only
                //   read). `&mut self` guarantees no live reader. Sole
                //   ownership ⇒ freed exactly once.
                unsafe { drop(Box::from_raw(ptr)) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[inline]
    fn tag(id: usize) -> ComponentId {
        ComponentId(id)
    }

    #[inline]
    fn arch(id: usize) -> ArchetypeId {
        ArchetypeId(id)
    }

    #[test]
    fn fresh_presence_contains_nothing() {
        let p = EnablePresence::new();
        assert!(!p.contains(tag(0), arch(0)));
        assert!(!p.contains(tag(7), arch(3)));
        assert_eq!(p.epoch(), 0);
    }

    #[test]
    fn contains_reflects_column_alloc() {
        let p = EnablePresence::new();
        p.note_column_alloc(tag(5), arch(1));
        assert!(p.contains(tag(5), arch(1)));
        // A different archetype for the same tag is still absent.
        assert!(!p.contains(tag(5), arch(2)));
        // A different tag at the same archetype is still absent.
        assert!(!p.contains(tag(6), arch(1)));
    }

    #[test]
    fn multiple_archetypes_same_tag() {
        let p = EnablePresence::new();
        p.note_column_alloc(tag(3), arch(1));
        p.note_column_alloc(tag(3), arch(64)); // crosses into word 1
        p.note_column_alloc(tag(3), arch(1023)); // last bit of word 15
        assert!(p.contains(tag(3), arch(1)));
        assert!(p.contains(tag(3), arch(64)));
        assert!(p.contains(tag(3), arch(1023)));
        assert!(!p.contains(tag(3), arch(2)));
        assert!(!p.contains(tag(3), arch(63)));
    }

    #[test]
    fn enable_generation_style_epoch_bumps_once_per_column() {
        let p = EnablePresence::new();
        assert_eq!(p.epoch(), 0);
        p.note_column_alloc(tag(1), arch(1));
        assert_eq!(p.epoch(), 1);
        p.note_column_alloc(tag(1), arch(2));
        assert_eq!(p.epoch(), 2);
        p.note_column_alloc(tag(2), arch(1));
        assert_eq!(p.epoch(), 3);
    }

    #[test]
    fn epoch_changes_on_new_archetype_for_same_tag() {
        let p = EnablePresence::new();
        p.note_column_alloc(tag(0), arch(5));
        let before = p.epoch();
        p.note_column_alloc(tag(0), arch(6));
        assert!(p.epoch() > before, "a new column must bump the epoch");
    }

    #[test]
    fn out_of_range_archetype_returns_false_not_panic() {
        let p = EnablePresence::new();
        // PRESENCE_CAPACITY is the first out-of-range id.
        assert!(!p.contains(tag(0), arch(PRESENCE_CAPACITY)));
        assert!(!p.contains(tag(0), arch(PRESENCE_CAPACITY + 100)));
    }

    #[test]
    fn out_of_range_tag_returns_false_not_panic() {
        let p = EnablePresence::new();
        assert!(!p.contains(tag(MAX_COMPONENTS), arch(0)));
        assert!(!p.contains(tag(MAX_COMPONENTS + 5), arch(0)));
    }

    #[test]
    fn lazy_alloc_is_idempotent_across_tags() {
        let p = EnablePresence::new();
        // Touch many distinct tags; each lazily allocates its own word array.
        for t in 0..32 {
            p.note_column_alloc(tag(t), arch(t));
        }
        for t in 0..32 {
            assert!(p.contains(tag(t), arch(t)));
            assert!(!p.contains(tag(t), arch(t + 1)));
        }
        assert_eq!(p.epoch(), 32);
    }
}
