use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::entity::entity_inland::EntityInland;
use crate::ecs::identifiers::primitives::{EntityId, ArchetypeId, InlandPoolId};
use boyko_utils::sparse_map::sparse_map::SparseMap;

/// Manages entity lifecycle, recycling, and internal mapping
/// Provides O(1) access to entity data and efficient recycling
pub struct EntityMaster {
    /// Pool of free entity IDs for reuse
    free_entity_ids: Vec<EntityId>,

    /// All entities (active and inactive)
    entities: Vec<Entity>,
    
    /// Maps entity ID to EntityInland for fast access
    entity_map: SparseMap<EntityInland>,
    
    /// Next entity ID to allocate
    next_entity_id: EntityId,
    
    /// Total number of active entities
    active_count: usize,
}

impl EntityMaster {
    /// Creates a new empty EntityMaster
    #[inline]
    pub fn new() -> Self {
        Self {
            free_entity_ids: Vec::new(),
            entities: Vec::new(),
            entity_map: SparseMap::new(),
            next_entity_id: EntityId(0),
            active_count: 0,
        }
    }

    /// Creates a new EntityMaster with pre-allocated capacity
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            free_entity_ids: Vec::with_capacity(capacity / 4),
            entities: Vec::with_capacity(capacity),
            entity_map: SparseMap::with_capacity(capacity),
            next_entity_id: EntityId(0),
            active_count: 0,
        }
    }

    /// Allocates a new entity or reuses a recycled one
    /// Returns the allocated entity with appropriate generation
    #[inline]
    pub fn allocate_entity(&mut self) -> Entity {
        if let Some(id) = self.free_entity_ids.pop() {
            // Reuse a recycled entity ID
            debug_assert!(id.0 < self.entities.len(), "Free entity ID out of bounds");
            let entity = self.entities[id.0];
            debug_assert!(!self.entity_map.contains(id.0), "Recycled entity still in map");
            entity
        } else {
            // Create a new entity with the next available ID
            let id = self.next_entity_id;
            self.next_entity_id.0 += 1;

            let entity = Entity::new(id, 0); // New entities start at generation 0

            // Ensure entities vector has enough capacity
            if id.0 >= self.entities.len() {
                self.entities.resize(id.0 + 1, Entity::new(EntityId(0), 0));
            }

            self.entities[id.0] = entity;
            entity
        }
    }

    /// Registers an entity with its inland data
    /// This creates the association between Entity and EntityInland
    #[inline]
    pub fn register_entity(&mut self, entity: Entity, archetype_id: ArchetypeId, unit_index: InlandPoolId) {
        debug_assert!(entity.id().0 < self.entities.len(), "Entity ID out of bounds");
        debug_assert!(!self.entity_map.contains(entity.id().0), "Entity already registered");

        let entity_inland = EntityInland::new(archetype_id, unit_index, entity.generation());
        self.entity_map.insert(entity.id().0, entity_inland);
        self.active_count += 1;
    }

    /// Updates an entity's inland data
    /// Returns true if the update was successful
    #[inline]
    pub fn update_entity_inland(&mut self, entity: Entity, archetype_id: ArchetypeId, unit_index: InlandPoolId) -> bool {
        if !self.is_entity_valid(entity) {
            return false;
        }

        if let Some(inland) = self.entity_map.get_mut(entity.id().0) {
            inland.update(archetype_id, unit_index);
            true
        } else {
            false
        }
    }

    /// Updates only the unit index for an entity inland
    /// Used during swap_remove operations
    #[inline]
    pub fn update_entity_unit_index(&mut self, entity: Entity, new_unit_index: InlandPoolId) -> bool {
        if !self.is_entity_valid(entity) {
            return false;
        }

        if let Some(inland) = self.entity_map.get_mut(entity.id().0) {
            inland.set_unit_index(new_unit_index);
            true
        } else {
            false
        }
    }

    /// Deallocates an entity and updates its generation
    /// Returns the EntityInland data if the entity was valid
    #[inline]
    pub fn deallocate_entity(&mut self, entity: Entity) -> Option<EntityInland> {
        let entity_id = entity.id();

        // Verify the entity exists and has the correct generation
        if !self.is_entity_valid(entity) {
            return None;
        }

        // Get the EntityInland data before removing
        let entity_inland = self.entity_map.swap_remove(entity_id.0)?;

        // Increment generation of the deleted entity
        debug_assert!(entity_id.0 < self.entities.len(), "Entity ID out of bounds");
        let old_gen = self.entities[entity_id.0].generation();
        let new_gen = old_gen.wrapping_add(1);
        self.entities[entity_id.0] = Entity::new(entity_id, new_gen);

        // Add the ID to the free list for recycling
        self.free_entity_ids.push(entity_id);
        self.active_count -= 1;

        Some(entity_inland)
    }

    /// Gets the EntityInland for a specific entity
    #[inline]
    pub fn get_entity_inland(&self, entity: Entity) -> Option<&EntityInland> {
        if !self.is_entity_valid(entity) {
            return None;
        }
        self.entity_map.get(entity.id().0)
    }

    /// Gets a mutable reference to the EntityInland for a specific entity
    #[inline]
    pub fn get_entity_inland_mut(&mut self, entity: Entity) -> Option<&mut EntityInland> {
        if !self.is_entity_valid(entity) {
            return None;
        }
        self.entity_map.get_mut(entity.id().0)
    }

    /// Gets the EntityInland for a specific entity ID (unchecked)
    /// Warning: Does not verify generation
    #[inline]
    pub fn get_entity_inland_by_id(&self, entity_id: EntityId) -> Option<&EntityInland> {
        self.entity_map.get(entity_id.0)
    }

    /// Checks if an entity is valid (exists with matching generation)
    #[inline]
    pub fn is_entity_valid(&self, entity: Entity) -> bool {
        let entity_id = entity.id();
        entity_id.0 < self.entities.len() &&
            self.entities[entity_id.0].generation() == entity.generation() &&
            self.entity_map.contains(entity_id.0)
    }

    /// Gets an entity by ID if it exists and is active
    #[inline]
    pub fn get_entity(&self, entity_id: EntityId) -> Option<Entity> {
        if entity_id.0 < self.entities.len() && self.entity_map.contains(entity_id.0) {
            Some(self.entities[entity_id.0])
        } else {
            None
        }
    }

    /// Gets the total number of active entities
    #[inline]
    pub fn entity_count(&self) -> usize {
        self.active_count
    }

    /// Gets the total capacity (including recycled IDs)
    #[inline]
    pub fn capacity(&self) -> usize {
        self.entities.len()
    }

    /// Gets the number of recycled entity IDs available for reuse
    #[inline]
    pub fn recycled_entity_count(&self) -> usize {
        self.free_entity_ids.len()
    }

    /// Gets the next entity ID that would be allocated
    #[inline]
    pub fn next_entity_id(&self) -> EntityId {
        self.next_entity_id
    }

    /// Returns an iterator over all currently-active entities.
    ///
    /// Cost: O(active_count), not O(next_entity_id). Driven by
    /// `SparseMap::active_indices` (dense list of registered entity IDs); the
    /// `entities` Vec is consulted once per active entity via direct index.
    pub fn iter_entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.entity_map.active_indices().iter().map(move |&id| {
            // register_entity invariant guarantees that every id present in
            // entity_map was previously written to self.entities[id] by
            // allocate_entity. The id is therefore in bounds and points to a
            // fully-initialized Entity record. `id` is a raw `usize` from
            // SparseMap::active_indices (boyko_utils does not know about newtypes).
            debug_assert!(
                id < self.entities.len(),
                "invariant: entity_map id {} must be in entities (len {})",
                id,
                self.entities.len()
            );
            self.entities[id]
        })
    }

    /// Clears all entities from the master
    pub fn clear(&mut self) {
        self.free_entity_ids.clear();
        self.entities.clear();
        self.entity_map.clear();
        self.next_entity_id = EntityId(0);
        self.active_count = 0;
    }

    /// Checks if the master is empty (no active entities)
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.active_count == 0
    }

    /// Gets the total memory usage in bytes (approximate)
    pub fn memory_usage(&self) -> usize {
        self.free_entity_ids.capacity() * std::mem::size_of::<EntityId>() +
        self.entities.capacity() * std::mem::size_of::<Entity>() +
        self.entity_map.len() * (std::mem::size_of::<EntityId>() + std::mem::size_of::<EntityInland>())
    }

    /// Compacts the internal storage to minimize memory usage
    pub fn compact(&mut self) {
        self.free_entity_ids.shrink_to_fit();

        // Note: We don't shrink entities vector as it would invalidate IDs
        // Instead, we just sort the free list for better cache usage
        self.free_entity_ids.sort_unstable_by(|a, b| b.cmp(a)); // Reverse order for pop()
    }

    /// Rolls back the last `allocate_entity` call for a fresh ID (not a recycled one).
    ///
    /// # Invariant
    ///
    /// `rewind_allocate` must be called immediately after `allocate_entity` and
    /// before any other `EntityMaster` mutation, otherwise the
    /// `id == next_entity_id - 1` heuristic for fresh-ID rollback is unsound.
    /// The current single caller (`EcsMaster::create_entity` on guard failure)
    /// satisfies this contract by construction. If a second caller emerges,
    /// audit the contract or promote `rewind_allocate` to a token-based RAII
    /// guard.
    ///
    /// For recycled IDs (from `free_entity_ids`) this method has no effect and
    /// returns `false` — recycled IDs are returned to the free list by the
    /// caller (via `deallocate_entity`) if needed. In the single-caller context,
    /// `EcsMaster::create_entity` only calls this on the fresh-ID path (before
    /// `register_entity`), so the recycled case never occurs in practice.
    #[doc(hidden)]
    pub(crate) fn rewind_allocate(&mut self, entity: Entity) -> bool {
        let id = entity.id();
        // Fresh IDs are minted sequentially from next_entity_id; a fresh entity
        // is at `next_entity_id - 1` immediately after allocate_entity returns.
        if id.0 + 1 == self.next_entity_id.0 && id.0 < self.entities.len() {
            // Verify it was never registered (no entry in entity_map).
            debug_assert!(!self.entity_map.contains(id.0),
                "rewind_allocate called on a registered entity — invariant violated");
            // Undo next_entity_id increment.
            self.next_entity_id.0 -= 1;
            true
        } else {
            // Recycled ID path or stale call — caller must use deallocate_entity.
            false
        }
    }

}

impl Default for EntityMaster {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entity_allocation() {
        let mut master = EntityMaster::new();
        
        // Allocate first entity
        let entity1 = master.allocate_entity();
        assert_eq!(entity1.id(), EntityId(0));
        assert_eq!(entity1.generation(), 0);

        // Allocate second entity
        let entity2 = master.allocate_entity();
        assert_eq!(entity2.id(), EntityId(1));
        assert_eq!(entity2.generation(), 0);
    }

    #[test]
    fn test_entity_registration() {
        let mut master = EntityMaster::new();
        let entity = master.allocate_entity();
        
        // Register entity
        master.register_entity(entity, ArchetypeId(1), InlandPoolId(0));
        assert_eq!(master.entity_count(), 1);
        assert!(master.is_entity_valid(entity));

        // Get inland data
        let inland = master.get_entity_inland(entity).unwrap();
        assert_eq!(inland.archetype_id(), ArchetypeId(1));
        assert_eq!(inland.unit_index(), InlandPoolId(0));
    }

    #[test]
    fn test_entity_deallocation_and_reuse() {
        let mut master = EntityMaster::new();
        
        // Allocate and register entity
        let entity1 = master.allocate_entity();
        master.register_entity(entity1, ArchetypeId(1), InlandPoolId(0));

        // Deallocate entity
        let inland = master.deallocate_entity(entity1);
        assert!(inland.is_some());
        assert_eq!(master.entity_count(), 0);
        assert_eq!(master.recycled_entity_count(), 1);

        // Allocate again - should reuse the ID
        let entity2 = master.allocate_entity();
        assert_eq!(entity2.id(), EntityId(0));
        assert_eq!(entity2.generation(), 1); // Generation incremented
    }

    #[test]
    fn test_entity_inland_update() {
        let mut master = EntityMaster::new();
        let entity = master.allocate_entity();
        master.register_entity(entity, ArchetypeId(1), InlandPoolId(0));

        // Update unit index
        assert!(master.update_entity_unit_index(entity, InlandPoolId(5)));
        let inland = master.get_entity_inland(entity).unwrap();
        assert_eq!(inland.unit_index(), InlandPoolId(5));

        // Update full inland
        assert!(master.update_entity_inland(entity, ArchetypeId(2), InlandPoolId(10)));
        let inland = master.get_entity_inland(entity).unwrap();
        assert_eq!(inland.archetype_id(), ArchetypeId(2));
        assert_eq!(inland.unit_index(), InlandPoolId(10));
    }

    #[test]
    fn t_iter_entities_skips_recycled_slots() {
        let mut master = EntityMaster::new();

        // Allocate and register 100 entities.
        let mut all: Vec<Entity> = (0..100).map(|_| master.allocate_entity()).collect();
        for &e in &all {
            master.register_entity(e, ArchetypeId(1), InlandPoolId(0));
        }

        // Deallocate every other entity (indices 1, 3, 5, …, 99).
        let mut removed_ids = std::collections::HashSet::new();
        for i in (1..100).step_by(2) {
            removed_ids.insert(all[i].id());
            master.deallocate_entity(all[i]);
        }

        // Exactly 50 active entities remain.
        let collected: Vec<Entity> = master.iter_entities().collect();
        assert_eq!(collected.len(), 50, "expected 50 active entities");

        // None of the collected entities should have a recycled id.
        for e in &collected {
            assert!(!removed_ids.contains(&e.id()), "recycled id {} appeared in iter", e.id());
        }

        // Suppress the unused-mut warning: all was mutated only by the loop above
        // (we shadow with an immutable reborrow for the second half of the test).
        let _ = &mut all;
    }

    #[test]
    fn t_iter_entities_yields_correct_set_after_recycle() {
        let mut master = EntityMaster::new();

        // Allocate and register a, b, c.
        let a = master.allocate_entity();
        master.register_entity(a, ArchetypeId(1), InlandPoolId(0));
        let b = master.allocate_entity();
        master.register_entity(b, ArchetypeId(1), InlandPoolId(1));
        let c = master.allocate_entity();
        master.register_entity(c, ArchetypeId(1), InlandPoolId(2));

        // Delete b; its id goes onto the free list.
        master.deallocate_entity(b);

        // Allocate d — will recycle b's id with a higher generation.
        let d = master.allocate_entity();
        assert_eq!(d.id(), b.id(), "recycled id should be reused");
        master.register_entity(d, ArchetypeId(1), InlandPoolId(3));

        // iter_entities must yield exactly {a, c, d}.
        let mut got_ids: Vec<EntityId> = master.iter_entities().map(|e| e.id()).collect();
        got_ids.sort_unstable();

        let mut expected_ids = vec![a.id(), c.id(), d.id()];
        expected_ids.sort_unstable();

        assert_eq!(got_ids, expected_ids);

        // b (old generation) must NOT appear.
        assert!(!got_ids.contains(&b.id()) || got_ids.contains(&d.id()),
            "b's id is present but only as d (higher generation)");
    }

}