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
