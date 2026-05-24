use std::ops::{Index, IndexMut};
use crate::identifiers::slot::Slot;
use super::sparse_collection::SparseCollection;

/// High-performance sparse set with per-index generation tracking
/// for ABA-prevention on slot reuse (audit M-016 fix).
///
/// # Storage layout
///
/// `sparse: Vec<Option<Slot>>` uses a triple-state encoding per external index:
///
/// - `None` — pristine. The next allocation must come with `generation == 0`.
/// - `Some(Slot { index: usize::MAX, generation: g })` — **tombstone**.
///   The slot was previously occupied, then freed; the next allocation must come
///   with `generation == g` (the bumped successor of the prior occupant's
///   generation). The `usize::MAX` sentinel for `index` distinguishes a tombstone
///   from an occupied slot — `usize::MAX` is never a real dense index, since
///   reaching `2^64 - 1` simultaneously live entries is impossible in practice
///   (debug-asserted in `insert`).
/// - `Some(Slot { index: dense_idx, generation: g })` with `dense_idx < usize::MAX`
///   — **occupied**. `dense[dense_idx]` holds the value; the current generation
///   the caller must match is `g`.
///
/// # ABA prevention
///
/// `remove` writes a tombstone with `generation.wrapping_add(1)`, so any stale
/// `Slot` value the caller still holds — whose generation matches the
/// pre-remove occupant — is rejected by `contains` / `get` / `get_mut` (the
/// stored generation is now strictly greater). A subsequent `create_slot`
/// reads the tombstone's bumped generation, so the next valid allocation
/// carries that fresh generation: an attacker reusing the stale slot value
/// after one remove + reinsert cycle still sees a generation mismatch.
///
/// This closes the bug the audit recorded as M-016: previously the bumped
/// generation was computed (`let _new_generation = ...`) but immediately
/// dropped without being written anywhere, so the next `create_slot(idx)`
/// returned `Slot { idx, generation: 0 }` identical to the very first
/// allocation — a textbook ABA failure.
pub struct SparseSlotMap<U> {
    sparse: Vec<Option<Slot>>,
    dense: Vec<U>,
    /// External indices indexed by dense position — reverse lookup for `swap_remove`.
    indices: Vec<usize>,
}

/// Sentinel value packed into `Slot::index` to mark a tombstone entry in the
/// sparse array. A real dense index can never reach `usize::MAX`, so the
/// sentinel never collides with a live occupant.
const TOMBSTONE_DENSE_IDX: usize = usize::MAX;

impl<U> Default for SparseSlotMap<U> {
    fn default() -> Self {
        Self::new()
    }
}

impl<U> SparseSlotMap<U> {
    /// Creates a new empty SparseSlotMap.
    #[inline]
    pub fn new() -> Self {
        Self {
            sparse: Vec::new(),
            dense: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Creates a SparseSlotMap with pre-allocated capacity.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            sparse: Vec::with_capacity(capacity),
            dense: Vec::with_capacity(capacity),
            indices: Vec::with_capacity(capacity),
        }
    }

    /// Returns the `Slot` the next `insert` for `index` must be called with.
    ///
    /// - First allocation at `index`: returns `Slot { index, generation: 0 }`.
    /// - After a `remove`: returns `Slot { index, generation: bumped }` where
    ///   `bumped = prior_generation.wrapping_add(1)`.
    /// - Currently occupied: returns `Slot` matching the current occupant
    ///   (calling `insert` with this slot replaces the value).
    #[inline]
    pub fn create_slot(&self, index: usize) -> Slot {
        if index < self.sparse.len()
            && let Some(stored) = &self.sparse[index]
        {
            // Tombstone OR occupied — the next valid allocation/replacement
            // uses the generation currently recorded.
            return Slot::new(index, stored.generation());
        }
        Slot::new(index, 0)
    }

    /// Inserts `value` for `slot`.
    ///
    /// Returns:
    /// - `Some(old)` when `slot` matches an occupied entry — the old value
    ///   is replaced and returned.
    /// - `None` when the entry was pristine or a tombstone with matching
    ///   generation — a fresh dense allocation is performed.
    /// - `None` when `slot` carries a stale generation (mismatching the
    ///   stored one) — the call is rejected and no mutation occurs.
    #[inline]
    pub fn insert(&mut self, slot: Slot, value: U) -> Option<U> {
        let idx = slot.index();
        let caller_gen = slot.generation();

        if idx >= self.sparse.len() {
            self.sparse.resize(idx + 1, None);
        }

        match self.sparse[idx] {
            None => {
                // Pristine — only generation 0 is valid for the first allocation.
                if caller_gen != 0 {
                    return None;
                }
                self.push_dense(idx, value, 0);
                None
            }
            Some(stored) => {
                if stored.generation() != caller_gen {
                    // Stale slot — generation mismatch.
                    return None;
                }
                if stored.index() == TOMBSTONE_DENSE_IDX {
                    // Tombstone with matching gen → fresh allocation.
                    self.push_dense(idx, value, caller_gen);
                    None
                } else {
                    // Occupied with matching gen → replace value in dense slot.
                    let dense_idx = stored.index();
                    Some(std::mem::replace(&mut self.dense[dense_idx], value))
                }
            }
        }
    }

    /// Removes the entry for `slot` if the generation matches.
    ///
    /// On success, writes a tombstone with the bumped generation so any stale
    /// `Slot` the caller still holds is permanently rejected by future
    /// `contains` / `get` / `get_mut` / `insert` calls.
    #[inline]
    pub fn remove(&mut self, slot: Slot) -> Option<U> {
        let idx = slot.index();
        let caller_gen = slot.generation();

        if idx >= self.sparse.len() {
            return None;
        }

        let stored = self.sparse[idx]?;
        // Tombstones are never "occupied" — reject removal attempts on them.
        if stored.index() == TOMBSTONE_DENSE_IDX {
            return None;
        }
        if stored.generation() != caller_gen {
            return None; // Stale reference.
        }

        let dense_idx = stored.index();
        let last_idx = self.dense.len() - 1;

        // Pop the value out, swapping the last element down if necessary.
        let value = if dense_idx == last_idx {
            self.indices.pop();
            self.dense.pop().expect("dense was non-empty: occupied stored slot implies non-empty dense")
        } else {
            // After `Vec::swap_remove(dense_idx)`, the element that previously
            // lived at `last_idx` migrates into position `dense_idx`. Its
            // external index is whatever sat at `indices[last_idx]` *before*
            // the swap — capture it now.
            let moved_external = *self
                .indices
                .last()
                .expect("dense non-empty implies indices non-empty");

            let value = self.dense.swap_remove(dense_idx);
            // Return value of `indices.swap_remove` is the just-removed
            // external index (== `idx`, by the invariant that `indices[k]`
            // and `dense[k]` form a parallel pair). Discard it — we only
            // need the side effect of the swap.
            let _removed_external = self.indices.swap_remove(dense_idx);

            // The moved element now lives at dense_idx; redirect its sparse entry.
            if let Some(moved_slot) = self.sparse[moved_external] {
                debug_assert_ne!(
                    moved_slot.index(),
                    TOMBSTONE_DENSE_IDX,
                    "invariant: an occupant in `indices` must have an occupied sparse entry, not a tombstone"
                );
                let moved_gen = moved_slot.generation();
                self.sparse[moved_external] = Some(Slot::new(dense_idx, moved_gen));
            }
            value
        };

        // Tombstone with bumped generation — this is the M-016 fix proper.
        let bumped = caller_gen.wrapping_add(1);
        self.sparse[idx] = Some(Slot::new(TOMBSTONE_DENSE_IDX, bumped));

        Some(value)
    }

    /// Checks whether an entry exists for `slot` AND its generation matches.
    /// Tombstones and stale generations both return `false`.
    #[inline]
    pub fn contains(&self, slot: Slot) -> bool {
        let idx = slot.index();
        if idx >= self.sparse.len() {
            return false;
        }
        match self.sparse[idx] {
            Some(stored) => {
                stored.index() != TOMBSTONE_DENSE_IDX
                    && stored.generation() == slot.generation()
            }
            None => false,
        }
    }

    /// Returns a reference to the value at `slot` if it matches an occupied entry.
    #[inline]
    pub fn get(&self, slot: Slot) -> Option<&U> {
        let idx = slot.index();
        if idx >= self.sparse.len() {
            return None;
        }
        let stored = self.sparse[idx]?;
        if stored.index() == TOMBSTONE_DENSE_IDX || stored.generation() != slot.generation() {
            return None;
        }
        Some(&self.dense[stored.index()])
    }

    /// Mutable variant of [`get`].
    #[inline]
    pub fn get_mut(&mut self, slot: Slot) -> Option<&mut U> {
        let idx = slot.index();
        if idx >= self.sparse.len() {
            return None;
        }
        let stored = self.sparse[idx]?;
        if stored.index() == TOMBSTONE_DENSE_IDX || stored.generation() != slot.generation() {
            return None;
        }
        Some(&mut self.dense[stored.index()])
    }

    /// `true` when no live entry exists.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }

    /// Clears every live entry and discards all tombstones — the sparse array
    /// is reset to the pristine state (all `None`).
    ///
    /// **Note**: this also drops every generation counter, so a stale `Slot`
    /// minted before `clear` would erroneously match a future `create_slot(idx)`
    /// returning generation 0. Treat `clear` as a "factory reset" — never call
    /// it while any external `Slot` values are still considered live by user code.
    #[inline]
    pub fn clear(&mut self) {
        self.sparse.iter_mut().for_each(|v| *v = None);
        self.dense.clear();
        self.indices.clear();
    }

    /// Pushes `value` into the dense array, recording its position in `sparse`
    /// with the supplied generation. Common path shared by the two fresh-allocation
    /// arms of `insert` (pristine + tombstone-with-matching-gen).
    #[inline]
    fn push_dense(&mut self, external_idx: usize, value: U, generation: usize) {
        let dense_idx = self.dense.len();
        debug_assert!(
            dense_idx < TOMBSTONE_DENSE_IDX,
            "invariant: dense_idx must never reach usize::MAX (collides with tombstone sentinel)"
        );
        self.dense.push(value);
        self.indices.push(external_idx);
        self.sparse[external_idx] = Some(Slot::new(dense_idx, generation));
    }
}

impl<U> Index<Slot> for SparseSlotMap<U> {
    type Output = U;

    fn index(&self, slot: Slot) -> &Self::Output {
        self.get(slot).expect("Slot not found or generation mismatch")
    }
}

impl<U> IndexMut<Slot> for SparseSlotMap<U> {
    fn index_mut(&mut self, slot: Slot) -> &mut Self::Output {
        self.get_mut(slot).expect("Slot not found or generation mismatch")
    }
}

impl<U> SparseCollection<Slot, U> for SparseSlotMap<U> {
    fn len(&self) -> usize {
        self.dense.len()
    }

    fn sparse_len(&self) -> usize {
        self.sparse.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pristine_insert_with_nonzero_gen_is_rejected() {
        let mut map = SparseSlotMap::<u32>::new();
        // Caller fabricates a Slot with non-zero gen for an index that was never used.
        let bogus = Slot::new(0, 7);
        assert_eq!(map.insert(bogus, 42), None, "non-zero gen on pristine must be rejected");
        // The sparse slot stays pristine, and a fresh create_slot returns gen=0.
        let fresh = map.create_slot(0);
        assert_eq!(fresh.generation(), 0);
        assert_eq!(map.insert(fresh, 42), None, "first real insert returns None");
        assert_eq!(map.get(fresh), Some(&42));
    }

    #[test]
    fn insert_then_replace_returns_old() {
        let mut map = SparseSlotMap::<u32>::new();
        let slot = map.create_slot(3);
        assert_eq!(map.insert(slot, 100), None);
        // Same Slot reinserted → replace, old value returned.
        assert_eq!(map.insert(slot, 200), Some(100));
        assert_eq!(map.get(slot), Some(&200));
    }

    #[test]
    fn aba_stale_slot_rejected_after_remove_reinsert_cycle() {
        // The M-016 regression test proper.
        let mut map = SparseSlotMap::<u32>::new();
        let stale = map.create_slot(5);
        assert_eq!(map.insert(stale, 111), None);

        // Caller saves `stale` somewhere, then removes the entry.
        assert_eq!(map.remove(stale), Some(111));

        // Stale slot must not be usable anymore.
        assert!(!map.contains(stale), "stale Slot must not be contained after remove");
        assert_eq!(map.get(stale), None, "stale Slot must not resolve after remove");

        // A fresh insert at the same index must yield a *different* generation.
        let fresh = map.create_slot(5);
        assert_ne!(fresh.generation(), stale.generation(),
            "create_slot after remove must return a bumped generation");
        assert_eq!(map.insert(fresh, 222), None);

        // ABA check: the saved `stale` slot from BEFORE the remove must still
        // refuse to read the newly inserted value.
        assert!(!map.contains(stale), "ABA: stale Slot must not read fresh value");
        assert_eq!(map.get(stale), None, "ABA: stale Slot must not resolve to fresh value");

        // The fresh slot reads the new value, as expected.
        assert_eq!(map.get(fresh), Some(&222));
    }

    #[test]
    fn remove_on_tombstone_returns_none() {
        let mut map = SparseSlotMap::<u32>::new();
        let slot = map.create_slot(0);
        map.insert(slot, 1);
        assert_eq!(map.remove(slot), Some(1));
        // Second remove with the (now stale) slot must not yield the value again.
        assert_eq!(map.remove(slot), None);
        // And the freshly-minted slot for the same index can't remove a tombstone either.
        let fresh = map.create_slot(0);
        assert_eq!(map.remove(fresh), None, "tombstone cannot be removed; nothing to remove");
    }

    #[test]
    fn swap_remove_updates_moved_slot_sparse_entry() {
        // Two entries; remove the first; verify the second is still reachable
        // by its original Slot (and the moved Slot points to the new dense index).
        let mut map = SparseSlotMap::<u32>::new();
        let s_a = map.create_slot(10);
        let s_b = map.create_slot(20);
        map.insert(s_a, 1);
        map.insert(s_b, 2);
        assert_eq!(map.remove(s_a), Some(1));
        // s_b's *user-visible* Slot is unchanged; its sparse pointer was redirected
        // internally to whatever dense slot it occupies post swap_remove.
        assert_eq!(map.get(s_b), Some(&2), "b must remain reachable via its original Slot");
        assert!(!map.contains(s_a));
    }

    #[test]
    fn insert_into_tombstone_with_wrong_gen_is_rejected() {
        let mut map = SparseSlotMap::<u32>::new();
        let s = map.create_slot(0);
        map.insert(s, 1);
        map.remove(s);
        // A made-up Slot with the OLD generation must not slip through.
        assert_eq!(map.insert(s, 999), None, "insert with stale gen must be rejected");
        // And no value was written.
        let fresh = map.create_slot(0);
        assert!(map.get(fresh).is_none(), "no value should be visible — insert was rejected");
    }
}
