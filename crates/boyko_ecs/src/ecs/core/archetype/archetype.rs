use std::ptr::NonNull;
use std::collections::HashSet;
use std::any::TypeId;
use boyko_utils::identifiers::primitives::Generation;
use boyko_utils::sparse_map::sparse_map::SparseMap;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId, EntityId};
use crate::ecs::identifiers::id_unit::UnitId;
use crate::ecs::core::component::Component;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::memory::arena::Arena;
use crate::ecs::memory::component_pool::ComponentPool;
use crate::ecs::core::archetype::component_pool_bundle::{ComponentPoolBundle, ComponentTuple};
use crate::ecs::constants::INITIAL_ENTITY_CAPACITY;

/// Represents the storage location of an entity's components within the archetype
#[derive(Debug, Clone, Copy)]
struct EntityLocation {
    /// The chunk index where the entity's components are stored
    chunk_index: usize,

    /// The index within the chunk (the row in the component arrays)
    inland_index: usize,
}

impl EntityLocation {
    #[inline]
    fn new(chunk_index: usize, inland_index: usize) -> Self {
        Self { chunk_index, inland_index }
    }

    #[inline]
    fn to_unit_id(&self) -> UnitId {
        UnitId::new(self.chunk_index, self.inland_index)
    }
}

/// Archetype represents a unique combination of component types
/// All entities with the same component types belong to the same archetype
pub struct Archetype {
    /// Unique identifier for this archetype
    id: ArchetypeId,

    /// Storage for components organized by component type
    component_pools: ComponentPoolBundle,

    /// Maps entity IDs to (generation, location) pairs
    entity_to_location: SparseMap<(Generation, EntityLocation)>,

    /// Current index for the next entity (equals number of entities)
    current_index: usize,

    /// Set of component IDs in this archetype for quick signature checking
    component_types: HashSet<ComponentId>,

    /// Reference to the arena used for memory allocation
    arena: NonNull<Arena>,
}

impl Archetype {
    /// Creates a new archetype with the given ID and arena
    pub fn new(id: ArchetypeId, arena: &Arena) -> Self {
        Self {
            id,
            component_pools: ComponentPoolBundle::new(),
            entity_to_location: SparseMap::with_capacity(INITIAL_ENTITY_CAPACITY),
            current_index: 0,
            component_types: HashSet::new(),
            arena: NonNull::from(arena),
        }
    }

    /// Creates a new archetype with pre-registered component types
    pub fn with_components<T: ComponentTypeList>(id: ArchetypeId, arena: &Arena) -> Self {
        let mut archetype = Self::new(id, arena);
        T::register_components(&mut archetype);
        archetype
    }

    /// Gets the unique ID of this archetype
    #[inline]
    pub fn id(&self) -> ArchetypeId {
        self.id
    }

    /// Adds a component type to this archetype's signature
    pub fn register_component<T: Component>(&mut self) -> bool {
        let component_id = T::component_id();

        // Check if this component type is already registered
        if self.component_types.contains(&component_id) {
            return false;
        }

        // Get the arena for component pool creation
        let arena = unsafe { &*self.arena.as_ptr() };

        // Add a pool for this component type
        self.component_pools.add_pool::<T>(arena);
        self.component_types.insert(component_id);

        true
    }

    /// Checks if this archetype contains a specific component type
    #[inline]
    pub fn has_component<T: Component>(&self) -> bool {
        self.component_types.contains(&T::component_id())
    }

    /// Checks if this archetype contains a component with the given ID
    #[inline]
    pub fn has_component_id(&self, component_id: ComponentId) -> bool {
        self.component_types.contains(&component_id)
    }

    /// Gets the number of component types in this archetype
    #[inline]
    pub fn component_count(&self) -> usize {
        self.component_types.len()
    }

    /// Gets the number of entities in this archetype
    #[inline]
    pub fn entity_count(&self) -> usize {
        self.current_index
    }

    /// Creates a new entity in this archetype with the given components
    /// Returns the entity instance if successful
    pub fn create_entity<C: ComponentTuple>(&mut self, entity_id: EntityId, components: C) -> Option<Entity> {
        // Use the existing add_entity method from ComponentPoolBundle
        let unit_ids = self.component_pools.add_entity(components)?;

        if !unit_ids.is_empty() {
            // Use the first UnitId to determine component location
            let first_unit_id = unit_ids[0];
            let location = EntityLocation::new(first_unit_id.chunk_index(), first_unit_id.inland_index());

            // For new entities, start with generation 0
            let generation = 0;

            // Store the entity-to-location mapping with generation
            self.entity_to_location.insert(entity_id, (generation, location));

            // Increment current index
            self.current_index += 1;

            // Return an Entity instance
            return Some(Entity::new(entity_id, generation));
        }

        None
    }

    /// Removes an entity and all its components from this archetype
    /// Returns true if the entity was successfully removed
    pub fn remove_entity(&mut self, entity: Entity) -> bool {
        let entity_id = entity.id();
        let generation = entity.generation();

        // Get the location for this entity, checking generation in the process
        if let Some(&(stored_gen, location)) = self.entity_to_location.get(entity_id) {
            // Check if generations match
            if stored_gen != generation {
                return false; // Stale reference
            }

            // Generate UnitIds for all components of this entity
            let mut unit_ids = Vec::with_capacity(self.component_pools.len());
            for _ in 0..self.component_pools.len() {
                unit_ids.push(location.to_unit_id());
            }

            // Remove the components using ComponentPoolBundle's method
            let success = self.component_pools.remove_entity(unit_ids);

            if success {
                // Remove the entity-to-location mapping
                self.entity_to_location.remove(entity_id);

                // Decrement current index
                self.current_index -= 1;

                true
            } else {
                false
            }
        } else {
            false // Entity not found
        }
    }

    /// Gets a reference to a component for the specified entity
    pub fn get_component<T: Component>(&self, entity: Entity) -> Option<&T> {
        let entity_id = entity.id();
        let generation = entity.generation();

        // Get the location for this entity, checking generation in the process
        if let Some(&(stored_gen, location)) = self.entity_to_location.get(entity_id) {
            // Check if generations match
            if stored_gen != generation {
                return None; // Stale reference
            }

            // Get the component pool for this component type
            let pool = self.component_pools.get_pool::<T>()?;

            // Use the cached location to create the UnitId
            let unit_id = location.to_unit_id();

            // Get the component
            pool.get::<T>(unit_id)
        } else {
            None // Entity not found
        }
    }

    /// Gets a mutable reference to a component for the specified entity
    pub fn get_component_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        let entity_id = entity.id();
        let generation = entity.generation();

        // Get the location for this entity, checking generation in the process
        if let Some(&(stored_gen, location)) = self.entity_to_location.get(entity_id) {
            // Check if generations match
            if stored_gen != generation {
                return None; // Stale reference
            }

            // Get the component pool for this component type
            let pool = self.component_pools.get_pool_mut::<T>()?;

            // Use the cached location to create the UnitId
            let unit_id = location.to_unit_id();

            // Get the component
            pool.get_mut::<T>(unit_id)
        } else {
            None // Entity not found
        }
    }

    /// Sets the value of a component for the specified entity
    pub fn set_component<T: Component>(&mut self, entity: Entity, component: T) -> bool {
        let entity_id = entity.id();
        let generation = entity.generation();

        // Get the location for this entity, checking generation in the process
        if let Some(&(stored_gen, location)) = self.entity_to_location.get(entity_id) {
            // Check if generations match
            if stored_gen != generation {
                return false; // Stale reference
            }

            // Get the component pool for this component type
            if let Some(pool) = self.component_pools.get_pool_mut::<T>() {
                // Use the cached location to create the UnitId
                let unit_id = location.to_unit_id();

                // Use the set_component method on ComponentPool
                return pool.set_component(unit_id, component);
            }
        }

        false
    }

    /// Checks if an entity is in this archetype
    #[inline]
    pub fn has_entity(&self, entity: Entity) -> bool {
        let entity_id = entity.id();
        let generation = entity.generation();

        if let Some(&(stored_gen, _)) = self.entity_to_location.get(entity_id) {
            stored_gen == generation
        } else {
            false
        }
    }

    /// Gets a reference to the component pool bundle
    #[inline]
    pub fn component_pools(&self) -> &ComponentPoolBundle {
        &self.component_pools
    }

    /// Gets a mutable reference to the component pool bundle
    #[inline]
    pub fn component_pools_mut(&mut self) -> &mut ComponentPoolBundle {
        &mut self.component_pools
    }
}

// Trait implementations remain the same

// Trait implementations remain the same

// We don't need a custom ComponentTupleToData trait as we're using
// the existing ComponentTuple trait from component_pool_bundle

// Additional implementations for larger tuples would be added as needed

/// Trait for registering multiple component types with an archetype
pub trait ComponentTypeList {
    fn register_components(archetype: &mut Archetype);
}

// Implement for various tuples of component types
impl ComponentTypeList for () {
    fn register_components(_: &mut Archetype) {}
}

impl<T: Component> ComponentTypeList for (T,) {
    fn register_components(archetype: &mut Archetype) {
        archetype.register_component::<T>();
    }
}

impl<T1: Component, T2: Component> ComponentTypeList for (T1, T2) {
    fn register_components(archetype: &mut Archetype) {
        archetype.register_component::<T1>();
        archetype.register_component::<T2>();
    }
}

// Note: Additional tuple implementations would be added for more component types
// This pattern would continue for larger tuples as needed
