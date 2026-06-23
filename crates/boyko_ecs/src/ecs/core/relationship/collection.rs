//! `RelationshipSourceCollection` — the reverse-index container abstraction
//! (Relations API, Decision 2).
//!
//! The cardinality (one-to-many vs 1:1) and the backing store (`Vec` today,
//! dense/arena later) are a TYPE choice on a [`RelationshipTarget`], not a new
//! trait. The generic hooks operate through this trait, so a future backing is
//! an `impl` that touches no hook body.
//!
//! [`RelationshipTarget`]: super::RelationshipTarget

use crate::ecs::core::entity::entity::Entity;

/// A container of source [`Entity`]s on the reverse-index side of a relation.
///
/// Implemented by the field type of a [`RelationshipTarget`](super::RelationshipTarget)
/// (v1: only `Vec<Entity>`, the one-to-many default). The generic
/// link/unlink/cascade hooks read and mutate the reverse index exclusively
/// through these methods, so the cardinality and backing are decoupled from the
/// hook logic.
///
/// # 1:1 reservation
///
/// [`source_to_evict_before_add`](Self::source_to_evict_before_add) is the
/// RESERVED hook for the v1.1 one-to-one (`Entity`) collection: it returns the
/// currently-held source that the generic `on_insert` must evict before adding a
/// new one. The default (`None`) makes the v1 `Vec` collection skip eviction
/// entirely, and the eviction branch in the generic hook folds away under
/// monomorphization.
pub trait RelationshipSourceCollection {
    /// The borrowing iterator over the contained source entities.
    type Iter<'a>: Iterator<Item = Entity>
    where
        Self: 'a;

    /// Constructs an empty collection with room for `cap` sources.
    fn with_capacity(cap: usize) -> Self;

    /// Pushes `e`. Returns `true` iff it was newly added — a set-backed
    /// collection deduplicates (returning `false` for a present entity); the v1
    /// `Vec` collection always returns `true` (no dedup, EC9).
    fn add(&mut self, e: Entity) -> bool;

    /// Removes `e`. Returns `true` iff it was present. The `Vec` impl removes via
    /// `swap_remove` (O(1), order-perturbing — the Phase-19 sibling-order
    /// contract).
    fn remove(&mut self, e: Entity) -> bool;

    /// Iterates the contained source entities in unspecified order.
    fn iter(&self) -> Self::Iter<'_>;

    /// Returns the source at `index`, or `None` if out of bounds. Backs the
    /// cascade's WIDE path, which re-derives `&Self` per turn and reads ONE
    /// source by index (the Phase-19 `as_slice()[i]` access, generalized) — so it
    /// MUST be O(1) for the `Vec` collection to keep the wide cascade O(n).
    fn get(&self, index: usize) -> Option<Entity>;

    /// Number of contained sources.
    fn len(&self) -> usize;

    /// Clears the collection (drops all sources, keeps the allocation).
    fn clear(&mut self);

    /// 1:1 ONLY (RESERVED for v1.1, W1/O3): `Some(prev)` iff a prior source must
    /// be evicted before the next [`add`](Self::add). The future `Entity`
    /// collection returns the currently-held entity; the v1 `Vec` collection
    /// keeps the default `None` (no eviction), so the generic `on_insert`
    /// eviction branch is dead in v1 and folds away under monomorphization.
    #[inline]
    fn source_to_evict_before_add(&self) -> Option<Entity> {
        None
    }

    /// `true` when the collection holds no sources.
    #[inline]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl RelationshipSourceCollection for Vec<Entity> {
    type Iter<'a> = core::iter::Copied<core::slice::Iter<'a, Entity>>;

    #[inline]
    fn with_capacity(cap: usize) -> Self {
        Vec::with_capacity(cap)
    }

    #[inline]
    fn add(&mut self, e: Entity) -> bool {
        // No dedup (EC9): the `Vec` one-to-many collection always appends. Push
        // is O(1) amortized, preserving the Phase-19 O(1) relate.
        self.push(e);
        true
    }

    #[inline]
    fn remove(&mut self, e: Entity) -> bool {
        // `swap_remove` (O(1), order-perturbing) — the last source fills the gap.
        // This is the Phase-19 `Children::swap_remove_entity` behavior verbatim.
        if let Some(idx) = self.as_slice().iter().position(|&c| c == e) {
            self.swap_remove(idx);
            true
        } else {
            false
        }
    }

    #[inline]
    fn iter(&self) -> Self::Iter<'_> {
        self.as_slice().iter().copied()
    }

    #[inline]
    fn get(&self, index: usize) -> Option<Entity> {
        self.as_slice().get(index).copied()
    }

    #[inline]
    fn len(&self) -> usize {
        self.as_slice().len()
    }

    #[inline]
    fn clear(&mut self) {
        Vec::clear(self);
    }
}

/// The 1:1 reverse-slot collection (Relations v1.1): a target held by AT MOST ONE
/// source. Declared purely by the [`RelationshipTarget::Collection`] field type
/// being `Exclusive` (vs `Vec<Entity>` for one-to-many) — no new derive key.
///
/// Linking a new source `B` to a target already held by `A` EVICTS `A`: the
/// eviction is detected at APPLY time via [`source_to_evict_before_add`] in
/// `LinkCommand::apply`, which overwrites the slot, fires `OnUnlink{A}` once, and
/// enqueues a deferred remove of `A`'s now-dangling foreign key. The `Vec`
/// one-to-many path is unaffected: it inherits the default `None` override, so the
/// eviction branch const-folds away (byte-identical machine code).
///
/// [`source_to_evict_before_add`]: RelationshipSourceCollection::source_to_evict_before_add
/// [`RelationshipTarget::Collection`]: super::RelationshipTarget::Collection
///
/// `Default` (an empty slot) is derived so a `#[derive(RelationshipTarget, Default)]`
/// 1:1 target works without a hand-written `Default` (mirrors `Vec<Entity>`, which is
/// `Default`; `RelationshipTarget` requires the `Default` supertrait).
#[repr(transparent)]
#[derive(Default)]
pub struct Exclusive(Option<Entity>);

// A `#[repr(transparent)]` newtype is exactly its inner type's layout. The slot is
// a single `Option<Entity>` (no heap), strictly cheaper than `Vec`'s 24 B + heap
// allocation for the at-most-one case.
const _: () = assert!(
    core::mem::size_of::<Exclusive>() == core::mem::size_of::<Option<Entity>>(),
    "Exclusive must be a transparent newtype over Option<Entity>"
);

impl RelationshipSourceCollection for Exclusive {
    type Iter<'a> = core::option::IntoIter<Entity>;

    #[inline]
    fn with_capacity(_cap: usize) -> Self {
        Self(None)
    }

    #[inline]
    fn add(&mut self, e: Entity) -> bool {
        // W2/Q3: `false` on an identical re-add so the caller can suppress a
        // spurious `OnLink` re-fire for a 1:1 re-link of the SAME source.
        let changed = self.0 != Some(e);
        self.0 = Some(e);
        changed
    }

    #[inline]
    fn remove(&mut self, e: Entity) -> bool {
        // W3 KEYSTONE: `false` unless the current occupant IS `e`. After an
        // eviction overwrites the slot to `B`, the deferred remove of the evicted
        // `A` reaches here, finds `B != A`, and returns `false` — the
        // `UnlinkCommand` `if removed` gate then suppresses a second `OnUnlink`.
        if self.0 == Some(e) {
            self.0 = None;
            true
        } else {
            false
        }
    }

    #[inline]
    fn iter(&self) -> Self::Iter<'_> {
        self.0.into_iter()
    }

    #[inline]
    fn get(&self, index: usize) -> Option<Entity> {
        if index == 0 { self.0 } else { None }
    }

    #[inline]
    fn len(&self) -> usize {
        self.0.is_some() as usize
    }

    #[inline]
    fn clear(&mut self) {
        // RETAIN_EMPTY: the component is retained, only the slot is cleared.
        self.0 = None;
    }

    /// The 1:1 override: the currently-held source must be evicted before adding a
    /// new one. (`Vec` keeps the trait default `None`, so its eviction branch
    /// const-folds away.)
    #[inline]
    fn source_to_evict_before_add(&self) -> Option<Entity> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    //! Category A — `Exclusive` 1:1 collection CONFORMANCE (Relations v1.1).
    //!
    //! Pure data-structure unit tests for the `RelationshipSourceCollection` impl on
    //! `Exclusive`: the slot-occupancy semantics every production-eviction invariant
    //! (B/C/D categories, the `relations_exclusive*` integration suites) builds on.
    //! The W3 keystone (`remove(other) == false` after an eviction overwrite) is
    //! pinned at this layer, isolated from the command/hook machinery.

    use super::*;
    use crate::ecs::identifiers::primitives::EntityId;

    /// A bare `Entity` handle with `generation == 0` (these tests never touch live
    /// storage — only the slot's `==` semantics matter).
    #[inline]
    fn ent(id: usize) -> Entity {
        Entity::with_id(EntityId(id))
    }

    // ── with_capacity / is_empty / len on a fresh slot ─────────────────────────

    #[test]
    fn exclusive_with_capacity_starts_empty() {
        let c = Exclusive::with_capacity(8);
        assert_eq!(c.len(), 0, "a fresh 1:1 slot holds zero sources");
        assert!(c.is_empty(), "a fresh 1:1 slot is empty");
        assert_eq!(c.get(0), None, "get(0) on an empty slot is None");
        assert_eq!(
            c.source_to_evict_before_add(),
            None,
            "an empty slot has nothing to evict",
        );
    }

    #[test]
    fn exclusive_with_capacity_ignores_cap() {
        // `Exclusive` is a single `Option<Entity>` slot; `cap` is irrelevant — it
        // must never pre-fill or panic for any cap (incl. 0 and a large value).
        for cap in [0usize, 1, 7, 1024] {
            let c = Exclusive::with_capacity(cap);
            assert!(c.is_empty(), "cap={cap} must still produce an empty slot");
        }
    }

    // ── add: empty slot, change, identical re-add ──────────────────────────────

    #[test]
    fn exclusive_add_to_empty_returns_true_and_occupies() {
        let mut c = Exclusive::with_capacity(1);
        let a = ent(5);

        let changed = c.add(a);

        assert!(changed, "add to an empty slot is a change → true");
        assert_eq!(c.len(), 1, "len is 1 after the first add");
        assert!(!c.is_empty(), "occupied slot is not empty");
        assert_eq!(c.get(0), Some(a), "get(0) returns the added source");
    }

    #[test]
    fn exclusive_add_identical_returns_false_no_change() {
        // W2/Q3: an identical re-add is the no-op that lets `LinkCommand::apply`
        // suppress a spurious `OnLink` re-fire for a 1:1 re-link of the SAME source.
        let mut c = Exclusive::with_capacity(1);
        let a = ent(5);
        assert!(c.add(a), "first add changes the slot");

        let changed = c.add(a);

        assert!(!changed, "an identical re-add does NOT change the slot → false");
        assert_eq!(c.len(), 1, "len stays 1 on an identical re-add");
        assert_eq!(c.get(0), Some(a), "the slot still holds the same source");
    }

    #[test]
    fn exclusive_add_distinct_overwrites_and_returns_true() {
        // The slot is overwritten in place — this is the eviction primitive
        // (`LinkCommand::apply` overwrites to the new source `B` under the live
        // borrow). `add` does NOT itself fire/defer; it only reports the change.
        let mut c = Exclusive::with_capacity(1);
        let (a, b) = (ent(5), ent(6));
        c.add(a);

        let changed = c.add(b);

        assert!(changed, "overwriting with a distinct source is a change → true");
        assert_eq!(c.len(), 1, "1:1 slot len is still 1 after an overwrite");
        assert_eq!(c.get(0), Some(b), "the slot now holds the NEW source");
    }

    // ── remove: the W3 KEYSTONE (occupant-identity gate) ───────────────────────

    #[test]
    fn exclusive_remove_matching_occupant_returns_true_and_clears() {
        let mut c = Exclusive::with_capacity(1);
        let a = ent(5);
        c.add(a);

        let removed = c.remove(a);

        assert!(removed, "removing the current occupant returns true");
        assert_eq!(c.len(), 0, "the slot is empty after removing the occupant");
        assert_eq!(c.get(0), None, "get(0) is None after the remove");
    }

    #[test]
    fn exclusive_remove_non_occupant_returns_false_keeps_slot() {
        // THE W3 KEYSTONE. After an eviction overwrites the slot to `B`, the deferred
        // remove of the evicted `A` reaches here, finds the slot holds `B != A`, and
        // returns `false` — the `UnlinkCommand` `if removed` gate then suppresses a
        // SECOND `OnUnlink`. This is the single-fire guarantee at the data layer.
        let mut c = Exclusive::with_capacity(1);
        let (a, b) = (ent(5), ent(6));
        c.add(b); // slot now holds B (post-eviction state)

        let removed = c.remove(a); // the evicted A's deferred FK clear

        assert!(
            !removed,
            "remove(A) when the slot holds B != A returns FALSE (W3 keystone)",
        );
        assert_eq!(c.len(), 1, "a non-matching remove must NOT clear the slot");
        assert_eq!(c.get(0), Some(b), "the incumbent B is untouched by remove(A)");
    }

    #[test]
    fn exclusive_remove_from_empty_returns_false() {
        let mut c = Exclusive::with_capacity(1);

        let removed = c.remove(ent(5));

        assert!(!removed, "remove on an empty 1:1 slot returns false");
        assert_eq!(c.len(), 0, "an empty slot stays empty");
    }

    // ── get: index 0 vs out-of-bounds ──────────────────────────────────────────

    #[test]
    fn exclusive_get_index_zero_is_slot_else_none() {
        let mut c = Exclusive::with_capacity(1);
        let a = ent(9);
        c.add(a);

        assert_eq!(c.get(0), Some(a), "get(0) is the slot occupant");
        assert_eq!(c.get(1), None, "get(1) is out of bounds for a 1:1 slot");
        assert_eq!(c.get(usize::MAX), None, "any index > 0 is None");
    }

    // ── iter: 0 or 1 element ───────────────────────────────────────────────────

    #[test]
    fn exclusive_iter_empty_yields_nothing() {
        let c = Exclusive::with_capacity(1);
        let v: Vec<Entity> = c.iter().collect();
        assert!(v.is_empty(), "iterating an empty 1:1 slot yields no sources");
    }

    #[test]
    fn exclusive_iter_occupied_yields_exactly_one() {
        let mut c = Exclusive::with_capacity(1);
        let a = ent(7);
        c.add(a);
        let v: Vec<Entity> = c.iter().collect();
        assert_eq!(v, vec![a], "iterating an occupied 1:1 slot yields exactly the source");
    }

    // ── clear: RETAIN_EMPTY — slot cleared, no panic, re-usable ────────────────

    #[test]
    fn exclusive_clear_empties_slot() {
        let mut c = Exclusive::with_capacity(1);
        c.add(ent(3));

        c.clear();

        assert_eq!(c.len(), 0, "clear empties the slot");
        assert!(c.is_empty(), "the slot is empty after clear");
        assert_eq!(c.get(0), None, "no occupant after clear");
        // The component is RETAIN_EMPTY at the relationship layer; the collection's
        // `clear` only zeroes the slot (the `Option` allocation is the `Exclusive`
        // itself — nothing to free), and the slot is immediately re-usable.
        let a = ent(4);
        assert!(c.add(a), "the cleared slot accepts a new source");
        assert_eq!(c.get(0), Some(a), "re-add after clear occupies the slot");
    }

    // ── source_to_evict_before_add: occupied vs empty ──────────────────────────

    #[test]
    fn exclusive_source_to_evict_reports_occupant() {
        let mut c = Exclusive::with_capacity(1);
        assert_eq!(
            c.source_to_evict_before_add(),
            None,
            "nothing to evict when empty",
        );
        let a = ent(11);
        c.add(a);
        assert_eq!(
            c.source_to_evict_before_add(),
            Some(a),
            "the occupied slot reports its current source as the eviction candidate",
        );
        c.remove(a);
        assert_eq!(
            c.source_to_evict_before_add(),
            None,
            "after the occupant is removed, there is nothing to evict",
        );
    }

    // ── transparent layout invariant (re-pins the module-level const assert) ───

    #[test]
    fn exclusive_is_transparent_over_option_entity() {
        assert_eq!(
            core::mem::size_of::<Exclusive>(),
            core::mem::size_of::<Option<Entity>>(),
            "Exclusive must stay a transparent newtype over Option<Entity> (no heap)",
        );
    }
}
