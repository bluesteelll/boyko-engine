//! `EntityCloneMap` — the source→clone entity map for deep clones (Feature 3, D5).
//!
//! Backed by [`SparseMap`] (boyko_utils) for O(1) lookup, NO `HashMap`. Keyed by
//! `EntityId.0` (W4: boyko's `SparseMap<U>` is single-param — keyed by `usize`,
//! valued by `Entity`; `Entity: Copy`, so `swap_remove` / `Clone`-on-remove is
//! cheap). The map doubles as the visited-set that dedups an entity reachable via
//! two links in a diamond subtree.
//!
//! The map is a LOCAL owned by the executing deep-clone call — never shared, never
//! held across a structural op as a borrow (W6: children are snapshotted by value
//! into the worklist before any spawn).

use boyko_utils::sparse_map::sparse_map::SparseMap;

use crate::ecs::core::entity::entity::Entity;

/// Source→clone entity map for a deep clone (D5 / W4). Allocated only on the deep
/// path (a shallow clone never builds one).
pub struct EntityCloneMap {
    /// `EntityId.0` → cloned `Entity`. The presence of a key is also the
    /// visited-set membership (diamond dedup).
    sparse: SparseMap<Entity>,
}

impl Default for EntityCloneMap {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl EntityCloneMap {
    /// Creates an empty map.
    #[inline]
    pub fn new() -> Self {
        Self {
            sparse: SparseMap::new(),
        }
    }

    /// Creates a map pre-sized for `capacity` source entities (the deep-clone
    /// subtree size hint — avoids the first few `Vec` growths).
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            sparse: SparseMap::with_capacity(capacity),
        }
    }

    /// Returns the clone of `source`, or `None` if `source` was not part of the
    /// cloned subtree (an external parent reference kept verbatim).
    #[inline]
    pub fn get(&self, source: Entity) -> Option<Entity> {
        self.sparse.get(source.id().0).copied()
    }

    /// Records that `source` cloned to `clone`. Crate-internal: only the deep-clone
    /// walk inserts. Returns the previous mapping if `source` was already cloned
    /// (a diamond — should not happen because the walk checks [`Self::contains`]
    /// first; the return is the defensive overwrite value).
    #[inline]
    pub(crate) fn insert(&mut self, source: Entity, clone: Entity) -> Option<Entity> {
        self.sparse.insert(source.id().0, clone)
    }

    /// Visited-set membership: `true` if `source` has already been cloned (diamond
    /// dedup, R2 #17726).
    #[inline]
    pub(crate) fn contains(&self, source: Entity) -> bool {
        self.sparse.contains(source.id().0)
    }
}
