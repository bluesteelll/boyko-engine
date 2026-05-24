use std::ops::{Index, IndexMut};
use boyko_utils::sparse_map::sparse_map::SparseMap;
use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::entity::entity_inland::EntityInland;
use crate::ecs::identifiers::primitives::{ArchetypeId, InlandArchetypeId, ComponentId};
use crate::ecs::memory::arena::Arena;

/// A collection of archetypes with efficient access by archetype ID
pub struct ArchetypeBundle {
    /// Stores the actual archetypes with direct indexing
    archetypes: Vec<Archetype>,

    /// Maps external archetype IDs to indices in the archetypes vector
    archetype_to_index: SparseMap<usize>,
}

impl Default for ArchetypeBundle {
    fn default() -> Self {
        Self::new()
    }
}

impl ArchetypeBundle {
    /// Creates a new empty ArchetypeBundle
    pub fn new() -> Self {
        Self {
            archetypes: Vec::new(),
            archetype_to_index: SparseMap::new(),
        }
    }

    /// Creates a new ArchetypeBundle with pre-allocated capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            archetypes: Vec::with_capacity(capacity),
            archetype_to_index: SparseMap::with_capacity(capacity),
        }
    }

    /// Gets a reference to an archetype by its ID
    pub fn get_archetype(&self, index: ArchetypeId) -> Option<&Archetype> {
        self.archetype_to_index.get(index.0)
            .and_then(|&inland_id| self.archetypes.get(inland_id))
    }

    /// Gets a mutable reference to an archetype by its ID
    pub fn get_archetype_mut(&mut self, index: ArchetypeId) -> Option<&mut Archetype> {
        if let Some(&inland_id) = self.archetype_to_index.get(index.0) {
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
        if let Some(&inland_id) = self.archetype_to_index.get(archetype_id.0) {
            // Replace the existing archetype
            self.archetypes[inland_id] = archetype;
            return InlandArchetypeId(inland_id);
        }

        // Add new archetype
        let inland_id = self.archetypes.len();
        self.archetypes.push(archetype);
        self.archetype_to_index.insert(archetype_id.0, inland_id);

        InlandArchetypeId(inland_id)
    }

    /// Creates a new archetype from a list of component IDs and adds it to the bundle
    /// Returns the internal index assigned to this archetype
    pub fn add_archetype_from_components(&mut self, archetype_id: ArchetypeId, component_ids: &[ComponentId], arena: &Arena) -> InlandArchetypeId {
        // Create a new archetype from component IDs
        let archetype = Archetype::create_by_ids(archetype_id, component_ids, arena);
        
        // Add the archetype to the bundle using the existing method
        self.add_archetype(archetype)
    }
    
    /// Removes an archetype from the bundle
    /// Returns true if the archetype was found and removed
    pub fn remove_archetype(&mut self, archetype_id: ArchetypeId) -> bool {
        // Find the inland ID for this archetype
        let inland_id = match self.archetype_to_index.get(archetype_id.0) {
            Some(&id) => id,
            None => return false, // Archetype not found
        };

        // Remove the mapping
        self.archetype_to_index.swap_remove(archetype_id.0);

        // If this wasn't the last archetype, we need to update the index mapping
        // for the archetype that was moved from the end
        if inland_id < self.archetypes.len() - 1 {
            // Get the ID of the archetype that will be moved from the end
            let moved_archetype_id = self.archetypes.last().unwrap().id();

            // Update its mapping to point to the new location
            if let Some(mapping) = self.archetype_to_index.get_mut(moved_archetype_id.0) {
                *mapping = inland_id;
            }
        }

        // Remove the archetype using swap_remove
        self.archetypes.swap_remove(inland_id);

        true
    }

    
    /// Gets the archetype for a specific entity using its inland data
    pub fn get_entity_archetype(&self, entity_inland: &EntityInland) -> Option<&Archetype> {
        let archetype_id = entity_inland.archetype_id();
        if let Some(&inland_id) = self.archetype_to_index.get(archetype_id.0) {
            self.archetypes.get(inland_id)
        } else {
            None
        }
    }

    /// Gets a mutable reference to the archetype for a specific entity
    pub fn get_entity_archetype_mut(&mut self, entity_inland: &EntityInland) -> Option<&mut Archetype> {
        let archetype_id = entity_inland.archetype_id();
        if let Some(&inland_id) = self.archetype_to_index.get(archetype_id.0) {
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
    
    /// Gets the internal index for an archetype ID
    pub fn get_inland_id(&self, archetype_id: ArchetypeId) -> Option<InlandArchetypeId> {
        self.archetype_to_index.get(archetype_id.0).copied().map(InlandArchetypeId)
    }

    /// Clears all archetypes from the bundle
    pub fn clear(&mut self) {
        self.archetypes.clear();
        self.archetype_to_index.clear();
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

impl Index<&EntityInland> for ArchetypeBundle {
    type Output = Archetype;

    fn index(&self, entity_inland: &EntityInland) -> &Self::Output {
        self.get_entity_archetype(entity_inland).expect("Entity not registered with any archetype")
    }
}

impl IndexMut<&EntityInland> for ArchetypeBundle {
    fn index_mut(&mut self, entity_inland: &EntityInland) -> &mut Self::Output {
        self.get_entity_archetype_mut(entity_inland).expect("Entity not registered with any archetype")
    }
}