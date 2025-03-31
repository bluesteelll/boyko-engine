use std::ptr::NonNull;
use std::collections::HashSet;
use std::any::TypeId;
use boyko_utils::identifiers::slot::Slot;
use boyko_utils::sparse_map::sparse_slot_map::SparseSlotMap;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId, EntityId};
use crate::ecs::identifiers::id_unit::UnitId;
use crate::ecs::core::component::Component;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::memory::arena::Arena;
use crate::ecs::memory::component_pool::ComponentPool;
use crate::ecs::core::archetype::component_pool_bundle::ComponentPoolBundle;
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
    /// Each component pool stores components at aligned indices corresponding to entity indices
    component_pools: ComponentPoolBundle,

    /// Maps entity slots to their location in the component arrays
    /// Using SparseSlotMap ensures generation checking for entities
    entity_to_location: SparseSlotMap<EntityLocation>,

    /// Maps row indices to entity slots (reverse lookup)
    /// When an entity is deleted, its row is filled by moving the last entity to this position
    location_to_entity: Vec<Slot>,

    /// Current number of entities in this archetype
    entity_count: usize,

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
            entity_to_location: SparseSlotMap::with_capacity(INITIAL_ENTITY_CAPACITY),
            location_to_entity: Vec::with_capacity(INITIAL_ENTITY_CAPACITY),
            entity_count: 0,
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
        self.entity_count
    }

    /// Creates a new entity in this archetype with the given components
    /// Returns the entity's slot if successful
    pub fn create_entity<C: ComponentTuple>(&mut self, entity_id: EntityId, components: C) -> Option<Slot> {
        // Create a slot for this entity
        let slot = self.entity_to_location.create_slot(entity_id);

        // Use the existing add_entity method from ComponentPoolBundle
        // This handles all the component insertion with proper type safety
        let unit_ids = self.component_pools.add_entity(components)?;

        // If we have component IDs, we got exactly one UnitId per component pool
        // We need to ensure they're all in the same chunk and have the same inland index
        if !unit_ids.is_empty() {
            // For now, just use the first UnitId as our location
            // In a real implementation, we'd ensure all components share the same location
            // or we'd need to handle components potentially being in different chunks
            let first_unit_id = unit_ids[0];
            let location = EntityLocation::new(first_unit_id.chunk_index(), first_unit_id.inland_index());

            // Store the entity-to-location mapping
            self.entity_to_location.insert(slot, location);

            // Store the location-to-entity mapping
            let location_index = self.entity_count; // For now, use entity count as index
            if location_index >= self.location_to_entity.len() {
                self.location_to_entity.push(slot);
            } else {
                self.location_to_entity[location_index] = slot;
            }

            // Increment entity count
            self.entity_count += 1;

            return Some(slot);
        }

        None
    }

    /// Removes an entity and all its components from this archetype
    /// Returns true if the entity was successfully removed
    pub fn remove_entity(&mut self, entity: Entity) -> bool {
        let slot: Slot = entity.into();

        // Get the location for this entity, checking generation in the process
        let location = match self.entity_to_location.remove(slot) {
            Some(loc) => loc,
            None => return false, // Entity not found or generation mismatch
        };

        // Generate UnitIds for all components of this entity using the cached location
        let mut unit_ids = Vec::with_capacity(self.component_pools.len());

        for _ in 0..self.component_pools.len() {
            unit_ids.push(location.to_unit_id());
        }

        // Remove the components using ComponentPoolBundle's method
        let success = self.component_pools.remove_entity(unit_ids);

        if success {
            // If this isn't the last entity, move the last entity to this location
            let location_index = self.entity_count - 1; // For now, assume index matches entity count

            if location_index > 0 {
                let last_entity = self.location_to_entity[location_index];

                // Update the moved entity's location
                if let Some(loc) = self.entity_to_location.get_mut(last_entity) {
                    *loc = location;
                }

                // Update location_to_entity mapping
                let target_index = 0; // Would need proper indexing in a real implementation
                self.location_to_entity[target_index] = last_entity;
            }

            // Decrement entity count
            self.entity_count -= 1;

            true
        } else {
            // If component removal failed, reinsert the entity mapping
            self.entity_to_location.insert(slot, location);
            false
        }
    }

    /// Gets a reference to a component for the specified entity
    pub fn get_component<T: Component>(&self, entity: Entity) -> Option<&T> {
        let slot: Slot = entity.into();

        // Get the location for this entity, checking generation in the process
        let location = self.entity_to_location.get(slot)?;

        // Get the component pool for this component type
        let pool = self.component_pools.get_pool::<T>()?;

        // Use the cached location to create the UnitId
        let unit_id = location.to_unit_id();

        // Get the component
        pool.get::<T>(unit_id)
    }

    /// Gets a mutable reference to a component for the specified entity
    pub fn get_component_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        let slot: Slot = entity.into();

        // Get the location for this entity, checking generation in the process
        let location = self.entity_to_location.get(slot)?;

        // Get the component pool for this component type
        let pool = self.component_pools.get_pool_mut::<T>()?;

        // Use the cached location to create the UnitId
        let unit_id = location.to_unit_id();

        // Get the component
        pool.get_mut::<T>(unit_id)
    }

    /// Sets the value of a component for the specified entity
    pub fn set_component<T: Component>(&mut self, entity: Entity, component: T) -> bool {
        let slot: Slot = entity.into();

        // Get the location for this entity, checking generation in the process
        if let Some(location) = self.entity_to_location.get(slot) {
            // Get the component pool for this component type
            if let Some(pool) = self.component_pools.get_pool_mut::<T>() {
                // Use the cached location to create the UnitId
                let unit_id = location.to_unit_id();

                // Get the raw pointer to the component
                if let Some(ptr) = pool.raw_get_mut(unit_id) {
                    unsafe {
                        // Copy the component data directly to the existing memory location
                        std::ptr::copy_nonoverlapping(
                            &component as *const T as *const u8,
                            ptr,
                            std::mem::size_of::<T>()
                        );

                        return true;
                    }
                }
            }
        }

        false
    }

    /// Checks if an entity is in this archetype
    #[inline]
    pub fn has_entity(&self, entity: Entity) -> bool {
        let slot: Slot = entity.into();
        self.entity_to_location.contains(slot)
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

/// Re-export ComponentTuple from component_pool_bundle
pub use crate::ecs::core::archetype::component_pool_bundle::ComponentTuple;