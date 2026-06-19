//! [`DenseRegistry`] — the per-world owner of every [`DenseStore`] (Dense plan
//! D2: the dense storage subsystem).
//!
//! One [`DenseStore`] exists per dense `ComponentId`; the registry owns them all
//! and creates each lazily on first use (the `Layout` comes from the
//! `ComponentRegistry` via `ComponentPool::new`, so a dense component must be
//! registered before its first insert — the `Component` derive registers it).
//!
//! # Why a field on `EcsMaster`, not a `Resource`
//!
//! A `DenseStore` owns a `ComponentPool` (a raw VM reservation) and is therefore
//! `!Send`; the engine's `Resource` slab requires `Send` (it is shared across
//! the parallel scheduler). So the registry lives as a dedicated `EcsMaster`
//! field among the storage subsystems, single-threaded behind `&mut EcsMaster`
//! on the structural path exactly like `archetype_master`.
//!
//! # 0%-gate
//!
//! A world that defines no dense component never creates a store: `dense_ids` is
//! empty, the lazy `SparseMap` holds nothing, and every despawn-path iteration
//! over `dense_ids` runs zero turns. Construction is alloc-free until the first
//! dense insert.

use crate::ecs::core::component::component_registry::{self, MAX_COMPONENTS};
use crate::ecs::identifiers::primitives::ComponentId;

use super::dense_store::DenseStore;

/// Default per-store reserve-row hint handed to [`DenseStore::new`] on lazy
/// creation. The backing `ComponentPool` reserves (does not commit) this many
/// rows of virtual address space; growth commits pages in place, so an
/// under-estimate only costs a later one-syscall slab commit, never a move.
const DENSE_STORE_RESERVE_ROWS: usize = 1024;

/// Per-world owner of the dense storage subsystem (Dense plan D2).
///
/// # Why not `SparseMap<DenseStore>`
///
/// `SparseMap<U>` requires `U: Clone` (its `swap_remove` value semantics); a
/// `DenseStore` owns a `ComponentPool` (a raw VM reservation) and is correctly
/// `!Clone`. So `slots` is a directly-indexed `Box<[Option<DenseStore>]>` of
/// `MAX_COMPONENTS` cells — O(1) `ComponentId`-keyed lookup, `None` until the id
/// is first touched.
///
/// # 0%-gate (lazy `slots`)
///
/// The `slots` array is itself behind an `Option<Box<…>>` and is `None` until the
/// FIRST dense insert in this world. A table-only world therefore pays ZERO
/// allocation (the `slots` array — `MAX_COMPONENTS` × `size_of::<Option<DenseStore>>()`
/// — is never allocated) and every despawn-path walk over `dense_ids` (empty)
/// runs zero turns.
pub struct DenseRegistry {
    /// `ComponentId.0 -> Option<DenseStore>`, directly indexed. `None` until the
    /// outer `Option` is materialised AND the id is first touched. Lazy: the
    /// whole boxed array is `None` for a table-only world.
    slots: Option<Box<[Option<DenseStore>]>>,

    /// Registration order of every dense id that has a live store in this world.
    /// Walked by despawn / clone to enumerate an entity's dense memberships
    /// without a `0..MAX_COMPONENTS` sweep. Push-only.
    dense_ids: Vec<ComponentId>,
}

impl DenseRegistry {
    /// Creates an empty registry. Alloc-free until the first dense insert (the
    /// 0%-gate: a table-only world never touches this).
    #[inline]
    pub fn new() -> Self {
        Self {
            slots: None,
            dense_ids: Vec::new(),
        }
    }

    /// Materialises the `slots` array on first dense use (cold, once per world).
    #[cold]
    #[inline(never)]
    fn alloc_slots() -> Box<[Option<DenseStore>]> {
        let mut v: Vec<Option<DenseStore>> = Vec::with_capacity(MAX_COMPONENTS);
        v.resize_with(MAX_COMPONENTS, || None);
        v.into_boxed_slice()
    }

    /// Returns the store for `component_id`, creating it lazily on first use.
    ///
    /// The component MUST be registered (the `ComponentPool::new` contract — the
    /// `Component` derive registers it before the id can reach here) AND
    /// classified `Dense` (debug-asserted, the missing-store guard from the
    /// plan's C1 §49).
    pub fn store_mut(&mut self, component_id: ComponentId) -> &mut DenseStore {
        debug_assert!(
            matches!(
                component_registry::storage_kind(component_id.0),
                component_registry::StorageKind::Dense
            ),
            "DenseRegistry::store_mut: component {component_id} is not classified Dense \
             (a non-dense id reached the dense routing path)"
        );
        debug_assert!(component_id.0 < MAX_COMPONENTS, "dense component id out of range");
        let slots = self.slots.get_or_insert_with(Self::alloc_slots);
        if slots[component_id.0].is_none() {
            slots[component_id.0] =
                Some(DenseStore::new(component_id, DENSE_STORE_RESERVE_ROWS));
            self.dense_ids.push(component_id);
        }
        slots[component_id.0]
            .as_mut()
            .expect("invariant: store just inserted above")
    }

    /// Returns the store for `component_id`, or `None` if no entity has ever been
    /// inserted into it in this world (no store created yet).
    #[inline]
    pub fn store(&self, component_id: ComponentId) -> Option<&DenseStore> {
        self.slots
            .as_ref()
            .and_then(|slots| slots.get(component_id.0).and_then(|s| s.as_ref()))
    }

    /// Mutable accessor for an existing store, or `None` if none exists yet.
    /// Unlike [`Self::store_mut`] this never creates a store — used by the
    /// remove / despawn paths, which are no-ops for an untouched dense id.
    #[inline]
    pub fn store_existing_mut(&mut self, component_id: ComponentId) -> Option<&mut DenseStore> {
        self.slots
            .as_mut()
            .and_then(|slots| slots.get_mut(component_id.0).and_then(|s| s.as_mut()))
    }

    /// Every dense id with a live store in this world, in registration order.
    /// The despawn / clone paths iterate this to find an entity's memberships.
    /// Empty for a table-only world (the 0%-gate).
    #[inline]
    pub fn dense_ids(&self) -> &[ComponentId] {
        &self.dense_ids
    }

    /// `true` iff this world has at least one live dense store. The despawn-path
    /// fast-out: a table-only world skips the dense membership walk entirely.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.dense_ids.is_empty()
    }
}

impl Default for DenseRegistry {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}
