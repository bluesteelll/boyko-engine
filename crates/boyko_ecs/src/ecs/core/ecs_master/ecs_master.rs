use crate::ecs::core::ecs_master::archetype_bundle::ArchetypeBundle;
use crate::ecs::core::archetype::archetype::{Archetype, ComponentTypeList};
use crate::ecs::core::archetype::component_pool_bundle::ComponentTuple;
use crate::ecs::core::component::Component;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::identifiers::primitives::{ArchetypeId, EntityId};
use crate::ecs::memory::arena::Arena;
use crate::ecs::constants::DEFAULT_ARENA_SIZE;

pub struct EcsMaster {
    /// Collection of archetypes
    pub archetypes: ArchetypeBundle,

    /// Pool of free entity IDs for reuse
    free_entity_ids: Vec<EntityId>,

    /// All entities (active and inactive)
    entities: Vec<Entity>,

    /// Memory arena for component allocation
    arena: Arena,
}

impl EcsMaster {
    /// Creates a new empty EcsMaster
    pub fn new() -> Self {
        Self {
            archetypes: ArchetypeBundle::new(),
            free_entity_ids: Vec::new(),
            entities: Vec::new(),
            arena: Arena::new(),
        }
    }

    /// Creates a new EcsMaster with pre-allocated capacity
    pub fn with_capacity(entity_capacity: usize, archetype_capacity: usize) -> Self {
        Self {
            archetypes: ArchetypeBundle::with_capacity(archetype_capacity),
            free_entity_ids: Vec::with_capacity(entity_capacity / 4),
            entities: Vec::with_capacity(entity_capacity),
            arena: Arena::with_capacity(DEFAULT_ARENA_SIZE),
        }
    }

    /// Creates a new archetype with the specified component types
    /// Returns the ID of the created archetype
    pub fn create_archetype<T: ComponentTypeList>(&mut self) -> ArchetypeId {
        let id = self.archetypes.len(); // Use length as next ID
        self.archetypes.create_archetype::<T>(id, &self.arena)
    }

    /// Creates a new entity with components in the specified archetype
    /// Returns the created entity if successful
    pub fn create_entity<C: ComponentTuple>(&mut self, archetype_id: ArchetypeId, components: C) -> Option<Entity> {
        // First, get or create an entity (borrowing self)
        let entity = self.next_entity();
        let entity_id = entity.id();

        // Then get the target archetype (borrowing self again, but after the first borrow is complete)
        let archetype = self.archetypes.get_archetype_mut(archetype_id)?;

        // Create the entity in the archetype with its components
        let created_entity = archetype.create_entity(entity_id, components)?;

        // Register the entity with the archetype for fast lookups
        self.archetypes.register_entity(entity, archetype_id);

        Some(created_entity)
    }
    /// Gets the next available entity, either from the free pool or by creating a new one
    #[inline]
    fn next_entity(&mut self) -> Entity {
        if let Some(id) = self.free_entity_ids.pop() {
            // Return the entity with its current (incremented) generation
            self.entities[id]
        } else {
            // Create a new entity with the next available ID
            let id = self.entities.len();
            let entity = Entity::new(id, 0); // New entities start at generation 0
            self.entities.push(entity);
            entity
        }
    }
    /// Gets a reference to a component for the specified entity
    pub fn delete_entity(&mut self, entity: Entity) -> bool {
        let entity_id = entity.id();

        // Verify the entity exists and has the correct generation
        if entity_id >= self.entities.len() || self.entities[entity_id].generation() != entity.generation() {
            return false;
        }

        // Find the archetype containing this entity
        if let Some(archetype) = self.archetypes.get_entity_archetype_mut(entity) {
            // Remove the entity from the archetype
            if archetype.remove_entity(entity) {
                // Unregister the entity from the archetype mapping
                self.archetypes.unregister_entity(entity);

                // Explicitly create a new Entity with incremented generation
                let old_gen = self.entities[entity_id].generation();
                let new_gen = old_gen.wrapping_add(1);
                self.entities[entity_id] = Entity::new(entity_id, new_gen);



                // Add the ID to the free list for recycling
                self.free_entity_ids.push(entity_id);

                return true;
            }
        }

        false
    }
    pub fn get_component<T: Component>(&self, entity: Entity) -> Option<&T> {
        // Verify the entity exists with matching generation
        let entity_id = entity.id();
        if entity_id >= self.entities.len() || self.entities[entity_id].generation() != entity.generation() {
            return None;
        }

        // Find the archetype containing the entity and get the component
        self.archetypes.get_entity_archetype(entity)?.get_component::<T>(entity)
    }

    /// Gets a mutable reference to a component for the specified entity
    pub fn get_component_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        // Verify the entity exists with matching generation
        let entity_id = entity.id();
        if entity_id >= self.entities.len() || self.entities[entity_id].generation() != entity.generation() {
            return None;
        }

        // Find the archetype containing the entity and get the component mutably
        self.archetypes.get_entity_archetype_mut(entity)?.get_component_mut::<T>(entity)
    }

    /// Sets the value of a component for the specified entity
    /// Returns true if the component was successfully set
    pub fn set_component<T: Component>(&mut self, entity: Entity, component: T) -> bool {
        // Verify the entity exists with matching generation
        let entity_id = entity.id();
        if entity_id >= self.entities.len() || self.entities[entity_id].generation() != entity.generation() {
            return false;
        }

        if let Some(archetype) = self.archetypes.get_entity_archetype_mut(entity) {
            archetype.set_component(entity, component)
        } else {
            false
        }
    }

    /// Checks if an entity exists with matching generation
    pub fn has_entity(&self, entity: Entity) -> bool {
        let entity_id = entity.id();
        entity_id < self.entities.len() &&
            self.entities[entity_id].generation() == entity.generation() &&
            !self.free_entity_ids.contains(&entity_id)
    }

    /// Gets an entity by ID if it exists and is active
    pub fn get_entity(&self, entity_id: EntityId) -> Option<Entity> {
        if entity_id < self.entities.len() && !self.free_entity_ids.contains(&entity_id) {
            Some(self.entities[entity_id])
        } else {
            None
        }
    }

    /// Checks if an entity has a specific component type
    pub fn has_component<T: Component>(&self, entity: Entity) -> bool {
        if !self.has_entity(entity) {
            return false;
        }

        if let Some(archetype) = self.archetypes.get_entity_archetype(entity) {
            archetype.has_component::<T>()
        } else {
            false
        }
    }

    /// Gets the archetype containing the specified entity
    pub fn get_entity_archetype(&self, entity: Entity) -> Option<&Archetype> {
        if !self.has_entity(entity) {
            return None;
        }

        self.archetypes.get_entity_archetype(entity)
    }

    /// Gets the ID of the archetype containing the specified entity
    pub fn get_entity_archetype_id(&self, entity: Entity) -> Option<ArchetypeId> {
        if !self.has_entity(entity) {
            return None;
        }

        self.archetypes.get_entity_archetype(entity).map(|archetype| archetype.id())
    }

    /// Gets the total number of active entities in the system
    pub fn entity_count(&self) -> usize {
        self.entities.len() - self.free_entity_ids.len()
    }

    /// Gets the number of archetypes in the system
    pub fn archetype_count(&self) -> usize {
        self.archetypes.len()
    }

    /// Gets the number of recycled entity IDs available for reuse
    pub fn recycled_entity_count(&self) -> usize {
        self.free_entity_ids.len()
    }

    /// Gets an iterator over all active entities
    pub fn iter_entities(&self) -> impl Iterator<Item = Entity> + '_ {
        let free_ids = &self.free_entity_ids;
        self.entities.iter()
            .enumerate()
            .filter(move |(idx, _)| !free_ids.contains(idx))
            .map(|(_, entity)| *entity)
    }
}