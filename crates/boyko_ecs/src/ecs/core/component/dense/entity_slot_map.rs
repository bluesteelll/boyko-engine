//! [`EntitySlotMap`] — the [`DenseStore`]'s flat `EntityId -> slot` membership
//! oracle (audit F2).
//!
//! A dense store must answer "which slot does entity `e` occupy?" on the mixed
//! dense query path once (or, until the fetch/filter dedupe lands, twice) PER
//! ROW. The previous backing was a generic `SparseMap<u32>` — three heap `Vec`s
//! with a `Vec<Option<usize>>` sparse layer (16 B per addressable entity id at
//! `size_of::<Option<usize>> == 16`) and a sparse→dense→value double hop. The
//! reverse `slot -> EntityId` table and the dense iteration order that
//! `SparseMap` also carried are already served by `DenseStore::s2e` /
//! `for_each_live`, so the sparse map's `dense` / `indices` arrays were pure
//! redundancy on this path (`dense_registry.rs` documents this).
//!
//! This replacement is a single flat `Vec<u32>` indexed by `EntityId.0`, value =
//! slot, [`ABSENT`] (`u32::MAX`) = not a member: 4 B per addressable id and ONE
//! dependent load per probe (no sparse→dense indirection). It is the dense
//! storage's own bookkeeping — the legitimate `std::Vec` exception the Dense
//! plan grants this module (Principle 0). It is entity-id-indexed and grows
//! amortized on insert to the max id touched; it stays a plain heap `Vec` (NOT a
//! VM reservation) by design — the growth is cold relative to the per-row probe.
//!
//! [`DenseStore`]: super::dense_store::DenseStore

/// Sentinel slot value marking an id that is NOT a member of the store. `u32::MAX`
/// can never be a real slot: the column's row ceiling is `POOL_MAX_ROWS < u32::MAX`
/// (a const-asserted invariant in `constants.rs`), so a live slot is always
/// strictly below this value.
pub(crate) const ABSENT: u32 = u32::MAX;

/// Flat `EntityId.0 -> slot` map with a `u32::MAX` absence sentinel — the dense
/// store's membership oracle (`contains` / `slot_of`).
///
/// Addressed directly: `slots[entity_id]` is the entity's slot, or [`ABSENT`].
/// Ids beyond the current `slots.len()` are treated as absent (never inserted).
/// Growth is amortized doubling via `Vec::resize`, paid only when an id past the
/// current frontier is first inserted.
pub(crate) struct EntitySlotMap {
    /// `EntityId.0 -> slot`. `slots[i] == ABSENT` ⟺ id `i` is not a member.
    /// `slots.len()` is the addressable id ceiling; ids `>= len` are absent.
    slots: Vec<u32>,

    /// Number of currently-present ids (`slots[i] != ABSENT` count). Maintained
    /// incrementally so `len` / `is_empty` are O(1) and need no scan.
    present: usize,
}

impl EntitySlotMap {
    /// Creates a map pre-sized to address at least `ids` entity ids without a
    /// further allocation. All entries start [`ABSENT`].
    ///
    /// `ids` is a floor hint, not a ceiling: inserting an id `>= ids` grows the
    /// backing `Vec` amortized (this map is entity-id-indexed, so its ceiling is
    /// the max live entity id, not the column reserve).
    #[inline]
    pub(crate) fn with_capacity(ids: usize) -> Self {
        Self {
            slots: vec![ABSENT; ids],
            present: 0,
        }
    }

    /// Ensures id `entity` is addressable, growing the backing `Vec` with
    /// [`ABSENT`] fill if it lies beyond the current frontier.
    ///
    /// Cold relative to `slot_of` / `contains`: only the first insert of an id
    /// past the current `slots.len()` pays the (amortized) `Vec` growth.
    #[cold]
    #[inline(never)]
    fn grow_to(&mut self, id: usize) {
        // Amortized doubling: `Vec::resize` grows the allocation geometrically,
        // so a monotone id stream pays O(1) amortized per new id. `id + 1` is the
        // minimum length that makes `id` addressable.
        self.slots.resize(id + 1, ABSENT);
    }

    /// Records that `entity` occupies `slot`, growing the backing `Vec` if the id
    /// lies beyond the current addressable range.
    ///
    /// `slot` must not be [`ABSENT`] (`u32::MAX` is not a valid slot — the row
    /// ceiling is strictly below it); debug-asserted.
    #[inline]
    pub(crate) fn insert(&mut self, entity: usize, slot: u32) {
        debug_assert_ne!(slot, ABSENT, "EntitySlotMap::insert: slot must not be the ABSENT sentinel");
        if entity >= self.slots.len() {
            self.grow_to(entity);
        }
        // Only bump `present` on a genuine absent→present transition; a re-insert
        // onto a live id (the `insert_or_replace` path) keeps the count stable.
        if self.slots[entity] == ABSENT {
            self.present += 1;
        }
        self.slots[entity] = slot;
    }

    /// Removes `entity`, marking its id [`ABSENT`]. No-op (returns without
    /// touching `present`) if the id was already absent or beyond range.
    ///
    /// Never shrinks the backing `Vec` — the address-stable membership array is
    /// reused for a later re-insert of the same id (the dense store's steady
    /// state is insert/remove churn on a bounded id set).
    #[inline]
    pub(crate) fn remove(&mut self, entity: usize) {
        if entity < self.slots.len() && self.slots[entity] != ABSENT {
            self.slots[entity] = ABSENT;
            self.present -= 1;
        }
    }

    /// Returns the slot `entity` occupies, or `None` if it is not a member.
    ///
    /// The per-row membership probe: ONE bounds check + ONE dependent load, no
    /// sparse→dense hop.
    #[inline]
    pub(crate) fn slot_of(&self, entity: usize) -> Option<u32> {
        match self.slots.get(entity).copied() {
            Some(s) if s != ABSENT => Some(s),
            _ => None,
        }
    }

    /// `true` iff `entity` is a member of the store.
    #[inline]
    pub(crate) fn contains(&self, entity: usize) -> bool {
        matches!(self.slots.get(entity), Some(&s) if s != ABSENT)
    }

    /// Marks every id absent in O(len) without releasing the backing allocation.
    /// Used by `compact()` to rebuild membership from the canonical slot order.
    #[inline]
    pub(crate) fn clear(&mut self) {
        self.slots.iter_mut().for_each(|s| *s = ABSENT);
        self.present = 0;
    }

    /// The number of currently-present ids (O(1) — maintained incrementally).
    #[inline]
    #[allow(dead_code)] // Exercised by the unit tests; kept for diagnostics.
    pub(crate) fn len(&self) -> usize {
        self.present
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Insert/slot_of/contains round trip within the pre-sized range.
    #[test]
    fn insert_slot_of_contains_round_trip() {
        let mut m = EntitySlotMap::with_capacity(8);
        assert_eq!(m.len(), 0);
        assert!(!m.contains(0));
        assert_eq!(m.slot_of(0), None);

        m.insert(3, 7);
        m.insert(0, 1);
        assert!(m.contains(3) && m.contains(0));
        assert_eq!(m.slot_of(3), Some(7));
        assert_eq!(m.slot_of(0), Some(1));
        assert_eq!(m.len(), 2);
        assert!(!m.contains(1), "never-inserted id must be absent");
    }

    /// `remove` marks the id absent and decrements the present count; a
    /// second remove of the same id (or of a never-inserted / out-of-range
    /// id) is a counted no-op.
    #[test]
    fn remove_transitions_and_noop_paths() {
        let mut m = EntitySlotMap::with_capacity(4);
        m.insert(2, 5);
        assert_eq!(m.len(), 1);

        m.remove(2);
        assert!(!m.contains(2));
        assert_eq!(m.slot_of(2), None);
        assert_eq!(m.len(), 0);

        // Double-remove, absent-remove, and out-of-range remove: all no-ops
        // that must NOT underflow `present`.
        m.remove(2);
        m.remove(3);
        m.remove(1_000_000);
        assert_eq!(m.len(), 0);
    }

    /// Inserting an id past the current frontier regrows the backing with
    /// ABSENT fill: the new id is present, every id in the gap stays absent.
    #[test]
    fn regrow_past_frontier_fills_absent() {
        let mut m = EntitySlotMap::with_capacity(2);
        m.insert(100, 42); // far past the 2-slot frontier
        assert_eq!(m.slot_of(100), Some(42));
        assert_eq!(m.len(), 1);
        for gap in [0usize, 1, 2, 50, 99] {
            assert!(!m.contains(gap), "gap id {gap} must be absent after regrow");
        }
        // Out-of-range probes stay absent (never grown for reads).
        assert!(!m.contains(101));
        assert_eq!(m.slot_of(10_000), None);
    }

    /// Re-insert onto a LIVE id (the `insert_or_replace` path): the slot value
    /// updates in place and `present` stays stable (no double count).
    #[test]
    fn reinsert_onto_live_id_keeps_present_stable() {
        let mut m = EntitySlotMap::with_capacity(4);
        m.insert(1, 10);
        assert_eq!(m.len(), 1);

        m.insert(1, 20); // re-insert: present→present, slot value replaced
        assert_eq!(m.slot_of(1), Some(20));
        assert_eq!(m.len(), 1, "re-insert onto a live id must not bump present");

        // Full absent→present→absent→present cycle keeps the count exact.
        m.remove(1);
        assert_eq!(m.len(), 0);
        m.insert(1, 30);
        assert_eq!(m.slot_of(1), Some(30));
        assert_eq!(m.len(), 1);
    }

    /// `clear` marks every id absent and zeroes `present` while keeping the
    /// backing addressable (a later insert needs no regrow within range).
    #[test]
    fn clear_marks_all_absent() {
        let mut m = EntitySlotMap::with_capacity(4);
        m.insert(0, 1);
        m.insert(3, 2);
        m.insert(64, 3); // regrown band
        assert_eq!(m.len(), 3);

        m.clear();
        assert_eq!(m.len(), 0);
        for id in [0usize, 3, 64] {
            assert!(!m.contains(id), "id {id} must be absent after clear");
            assert_eq!(m.slot_of(id), None);
        }

        // Post-clear reuse: the map behaves like fresh.
        m.insert(3, 9);
        assert_eq!(m.slot_of(3), Some(9));
        assert_eq!(m.len(), 1);
    }
}
