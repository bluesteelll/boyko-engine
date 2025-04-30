use std::ptr::NonNull;
use std::any::TypeId;
use boyko_utils::identifiers::primitives::Generation;

/// Archetype represents a unique combination of component types
/// All entities with the same component types belong to the same archetype
pub struct Archetype {
    /// Unique identifier for this archetype
    id: ArchetypeId,

    /// Storage for components organized by component type
    component_pools: ComponentPoolBundle,


    /// Current index for the next entity (equals number of entities)
    current_index: usize,

    signature: ArchetypeSignature,
    /// Reference to the arena used for memory allocation
    arena: NonNull<Arena>,
}

impl Archetype {
    /// Creates a new archetype with the given ID and arena
    pub fn new(id: ArchetypeId, arena: &Arena) -> Self {
        Self {
            id,
            signature: ArchetypeSignature::new(mask),
            component_pools: ComponentPoolBundle::new(),
            current_index: 0,
            arena: NonNull::from(arena),
        }
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

            // Determine generation to use
            let generation = match self.entity_to_location.get(entity_id) {
                Some(info) => info.generation(), // Use existing generation
                None => 0, // For completely new entities, start with generation 0
            };

            // Create or update location info
            let location_info = EntityLocationInfo::with_location(generation, location);
            self.entity_to_location.insert(entity_id, location_info);

            self.current_index += 1;

            // Return an Entity instance
            return Some(Entity::new(entity_id, generation));
        }

        None
    }

    /// Removes an entity and all its components from this archetype
    /// Returns true if the entity was successfully removed
    // TODO: Change impl. Use sparse_remove of map
    pub fn remove_entity(&mut self, entity: Entity) -> bool {
        let entity_id = entity.id();
        let generation = entity.generation();

        // Get the location info for this entity
        if let Some(location_info) = self.entity_to_location.get(entity_id) {
            // Check if generations match
            if location_info.generation() != generation {
                return false; // Stale reference
            }

            let location = location_info.location();
            if let Some(location_info) = self.entity_to_location.get_mut(entity_id) {
                location_info.increment_generation();
            }

            // Remove the components using ComponentPoolBundle's method
            let success = self.component_pools.remove_entity(UnitId::new(location.chunk_index, location.inland_index));

            if success {
                // Increment the generation instead of removing the entity


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
        if let Some(location_info) = self.entity_to_location.get(entity_id) {
            // Check if generations match
            if location_info.generation() != generation {
                return None; // Stale reference
            }

            // Get the component pool for this component type
            let pool = self.component_pools.get_pool::<T>()?;

            // Use the cached location to create the UnitId
            let unit_id = location_info.location().to_unit_id();

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
        if let Some(location_info) = self.entity_to_location.get(entity_id) {
            // Check if generations match
            if location_info.generation() != generation {
                return None; // Stale reference
            }

            // Get the component pool for this component type
            let pool = self.component_pools.get_pool_mut::<T>()?;

            // Use the cached location to create the UnitId
            let unit_id = location_info.location().to_unit_id();

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
        if let Some(location_info) = self.entity_to_location.get(entity_id) {
            // Check if generations match
            if location_info.generation() != generation {
                return false; // Stale reference
            }

            // Get the component pool for this component type
            if let Some(pool) = self.component_pools.get_pool_mut::<T>() {
                // Use the cached location to create the UnitId
                let unit_id = location_info.location().to_unit_id();

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

        if let Some(location_info) = self.entity_to_location.get(entity_id) {
            location_info.generation() == generation
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

