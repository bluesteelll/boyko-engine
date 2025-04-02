use std::ops::{Index, IndexMut};
use boyko_utils::sparse_map::sparse_map::SparseMap;
use crate::ecs::core::archetype::archetype::{Archetype, ComponentTypeList};
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::identifiers::primitives::{ArchetypeId, EntityId, InlandArchetypeId};
use crate::ecs::memory::arena::Arena;

/// A collection of archetypes with efficient entity-to-archetype mapping
pub struct ArchetypeBundle {
    /// Maps entity IDs to archetype indices for fast entity lookup
    entity_to_archetype: SparseMap<InlandArchetypeId>,

    /// Stores the actual archetypes with direct indexing by ArchetypeId
    archetypes: Vec<Archetype>,

    /// Maps external archetype IDs to indices in the archetypes vector
    archetype_to_index: SparseMap<usize>,
}

impl ArchetypeBundle {
    /// Creates a new empty ArchetypeBundle
    pub fn new() -> Self {
        Self {
            entity_to_archetype: SparseMap::new(),
            archetypes: Vec::new(),
            archetype_to_index: SparseMap::new(),
        }
    }

    /// Creates a new ArchetypeBundle with pre-allocated capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entity_to_archetype: SparseMap::with_capacity(capacity * 64), // Estimate entities
            archetypes: Vec::with_capacity(capacity),
            archetype_to_index: SparseMap::with_capacity(capacity),
        }
    }

    /// Gets a reference to an archetype by its ID
    pub fn get_archetype(&self, index: ArchetypeId) -> Option<&Archetype> {
        self.archetype_to_index.get(index)
            .and_then(|&inland_id| self.archetypes.get(inland_id))
    }

    /// Gets a mutable reference to an archetype by its ID
    pub fn get_archetype_mut(&mut self, index: ArchetypeId) -> Option<&mut Archetype> {
        if let Some(&inland_id) = self.archetype_to_index.get(index) {
            self.archetypes.get_mut(inland_id)
        } else {
            None
        }
    }

    /// Adds an existing archetype to the bundle
    /// Returns the internal index assigned to this archetype
    pub fn add_archetype(&mut self, archetype: Archetype) -> InlandArchetypeId {
        let archetype_id = archetype.id();

        // Check if archetype with this ID already exists
        if let Some(&inland_id) = self.archetype_to_index.get(archetype_id) {
            // Replace the existing archetype
            self.archetypes[inland_id] = archetype;
            return inland_id;
        }

        // Add new archetype
        let inland_id = self.archetypes.len();
        self.archetypes.push(archetype);
        self.archetype_to_index.insert(archetype_id, inland_id);

        inland_id
    }

    /// Creates a new archetype with the specified component types and adds it to the bundle
    /// Returns the ID of the new archetype
    pub fn create_archetype<T: ComponentTypeList>(&mut self, id: ArchetypeId, arena: &Arena) -> ArchetypeId {
        let archetype = Archetype::with_components::<T>(id, arena);
        self.add_archetype(archetype);
        id
    }

    /// Registers an entity with a specific archetype for fast lookup
    /// This should be called when an entity is created or moved to a different archetype
    pub fn register_entity(&mut self, entity: Entity, archetype_id: ArchetypeId) -> bool {
        if let Some(&inland_id) = self.archetype_to_index.get(archetype_id) {
            self.entity_to_archetype.insert(entity.id(), inland_id);
            true
        } else {
            false
        }
    }

    /// Unregisters an entity, removing it from the lookup map
    /// This should be called when an entity is destroyed
    /// Returns true if the entity was found and removed from the lookup map
    pub fn unregister_entity(&mut self, entity: Entity) -> bool {
        // We store entity lookups by their ID, not by the full Entity struct
        let entity_id = entity.id();

        // Use the remove method on SparseMap to remove the entry
        let result = self.entity_to_archetype.remove(entity_id).is_some();

        // Return whether the removal was successful
        result
    }

    /// Gets the archetype for a specific entity
    pub fn get_entity_archetype(&self, entity: Entity) -> Option<&Archetype> {
        // Get the entity ID
        let entity_id = entity.id();

        // Look up the archetype index in the entity-to-archetype map
        self.entity_to_archetype.get(entity_id)
            .and_then(|&inland_id| self.archetypes.get(inland_id))
    }

    /// Gets a mutable reference to the archetype for a specific entity
    pub fn get_entity_archetype_mut(&mut self, entity: Entity) -> Option<&mut Archetype> {
        // Get the entity ID
        let entity_id = entity.id();

        // Look up the archetype index in the entity-to-archetype map
        if let Some(&inland_id) = self.entity_to_archetype.get(entity_id) {
            self.archetypes.get_mut(inland_id)
        } else {
            None
        }
    }

    /// Gets the number of archetypes in the bundle
    #[inline]
    pub fn len(&self) -> usize {
        self.archetypes.len()
    }

    /// Checks if the bundle is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.archetypes.is_empty()
    }

    /// Returns an iterator over all archetypes for efficient traversal
    pub fn iter(&self) -> impl Iterator<Item = &Archetype> {
        self.archetypes.iter()
    }

    /// Returns a mutable iterator over all archetypes
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Archetype> {
        self.archetypes.iter_mut()
    }
}

impl Index<ArchetypeId> for ArchetypeBundle {
    type Output = Archetype;

    fn index(&self, index: ArchetypeId) -> &Self::Output {
        self.get_archetype(index).expect("Archetype not found")
    }
}

impl IndexMut<ArchetypeId> for ArchetypeBundle {
    fn index_mut(&mut self, index: ArchetypeId) -> &mut Self::Output {
        self.get_archetype_mut(index).expect("Archetype not found")
    }
}

impl Index<Entity> for ArchetypeBundle {
    type Output = Archetype;

    fn index(&self, entity: Entity) -> &Self::Output {
        self.get_entity_archetype(entity).expect("Entity not registered with any archetype")
    }
}

impl IndexMut<Entity> for ArchetypeBundle {
    fn index_mut(&mut self, entity: Entity) -> &mut Self::Output {
        self.get_entity_archetype_mut(entity).expect("Entity not registered with any archetype")
    }
}