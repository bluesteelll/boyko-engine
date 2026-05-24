use crate::ecs::core::archetype::archetype_master::ArchetypeMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::entity::entity_master::EntityMaster;
use crate::ecs::core::entity::entity_inland::EntityInland;
use crate::ecs::identifiers::primitives::{ArchetypeId, EntityId, ComponentId};
use crate::ecs::memory::arena::Arena;
use crate::ecs::constants::DEFAULT_ARENA_SIZE;
use anyhow::{Result, bail};

/// Main ECS manager that coordinates entities, archetypes, and memory allocation.
///
/// # Field order (drop order)
///
/// Fields are dropped in declaration order (`entity_master`, `archetype_master`,
/// `arena`). The arena **must** be last because `ArchetypeMaster`/`Archetype`
/// store `*const Arena` (raw provenance pointer — Phase 3a Miri retag fix;
/// previously `NonNull<Arena>`, audit finding C-001) and `ComponentPool`s store
/// raw pointers into the arena's backing buffer. Dropping the arena last
/// guarantees those pointers remain valid while child `Drop`s run.
///
/// The arena lives behind `Box<Arena>` so its address stays stable across
/// moves of the owning `EcsMaster`: without that, the original code on
/// `master` and `ecs` constructed `Arena` on the stack, stored
/// `NonNull::from(&arena)`, and then moved `arena` into `self` — a textbook
/// dangling-pointer construction (C-001).
///
/// # Raw provenance (`*const Arena`) — Miri retag fix
///
/// Child structures (`ArchetypeMaster`, `Archetype`, `ComponentPool`) store the
/// arena address as `*const Arena` rather than `NonNull<Arena>`. This eliminates
/// the Stacked Borrows retag UB that Miri reported when multiple `NonNull`s were
/// derived from the same `Box<Arena>` in the same borrow scope: under Stacked
/// Borrows, each `NonNull::from(&*arena_box)` re-activates the `&`-read tag and
/// can invalidate earlier derived pointers on reborrow. A raw `*const` pointer
/// minted via `&raw const *arena_box` carries the box's provenance but does not
/// participate in the Stacked Borrows tag stack — Miri accepts it as a shared,
/// read-only view of the allocation (audit finding C-001 / Phase 3a).
pub struct EcsMaster {
    /// Entity management system
    entity_master: EntityMaster,

    /// Archetype management system
    archetype_master: ArchetypeMaster,

    /// Memory arena for component allocation. `Box` provides a stable heap
    /// address shared by every `*const Arena` raw provenance pointer stored in
    /// child structures (`ArchetypeMaster`, `Archetype`, `ComponentPool`).
    arena: Box<Arena>,
}

impl EcsMaster {
    /// Creates a new empty EcsMaster
    ///
    /// Uses two-phase construction to avoid Miri Stacked Borrows retag UB:
    /// the arena `Box` is written to its final struct field first; the raw
    /// `*const Arena` pointer is then minted by reading the Box's inner pointer
    /// representation directly — without creating a `&Arena` reference (which
    /// would create a SharedReadOnly tag that the subsequent `Unique` retag on
    /// move would invalidate). Phase 3a Miri retag fix.
    pub fn new() -> Self {
        let arena: Box<Arena> = Box::default();
        // SAFETY: `Box<Arena>` is guaranteed to have the same in-memory
        // representation as `*mut Arena` (a single non-null pointer). We read
        // the Box's inner raw pointer without constructing a `&Arena` reference,
        // so no SharedReadOnly tag is created in the Stacked Borrows model.
        // The arena's heap address is stable for the lifetime of the Box.
        let arena_ptr: *const Arena = unsafe {
            // addr_of!(&arena as Box<Arena>) → *const Box<Arena>
            // Cast to *const *const Arena → read the inner pointer.
            // This is safe because Box<T> is repr-equivalent to *mut T.
            let box_ptr: *const Box<Arena> = std::ptr::addr_of!(arena);
            *(box_ptr.cast::<*const Arena>())
        };
        // SAFETY: `arena_ptr` points to the heap allocation owned by `arena`.
        // `arena` (and therefore `arena_ptr`) outlives `archetype_master` by
        // field drop order (arena is declared last in EcsMaster). Arena is
        // `!Send + !Sync`; single-threaded use is enforced.
        let archetype_master = unsafe { ArchetypeMaster::new(arena_ptr) };
        Self {
            entity_master: EntityMaster::new(),
            archetype_master,
            arena,
        }
    }

    /// Creates a new EcsMaster with pre-allocated capacity
    pub fn with_capacity(entity_capacity: usize, archetype_capacity: usize) -> Self {
        let arena: Box<Arena> = Box::new(Arena::with_capacity(DEFAULT_ARENA_SIZE));
        // SAFETY: same rationale as `EcsMaster::new`.
        let arena_ptr: *const Arena = unsafe {
            let box_ptr: *const Box<Arena> = std::ptr::addr_of!(arena);
            *(box_ptr.cast::<*const Arena>())
        };
        // SAFETY: same contract as `EcsMaster::new`.
        let archetype_master = unsafe { ArchetypeMaster::with_capacity(arena_ptr, archetype_capacity) };
        Self {
            entity_master: EntityMaster::with_capacity(entity_capacity),
            archetype_master,
            arena,
        }
    }

    /// Creates a new archetype with the specified component IDs
    /// Returns the ID of the created archetype
    #[inline]
    pub fn create_archetype(&mut self, component_ids: &[ComponentId]) -> ArchetypeId {
        self.archetype_master.create_archetype(component_ids)
    }

    /// Gets or creates an archetype with the specified component IDs
    /// Returns the ID of the archetype
    #[inline]
    pub fn get_or_create_archetype(&mut self, component_ids: &[ComponentId]) -> ArchetypeId {
        self.archetype_master.get_or_create_archetype(component_ids)
    }

    /// Creates a new entity with components in the specified archetype
    /// Takes a slice of (ComponentId, &[u8]) pairs for component data
    /// Returns the created entity if successful
    pub fn create_entity(
        &mut self, 
        archetype_id: ArchetypeId, 
        components: Vec<(ComponentId, &[u8])>
    ) -> Result<Entity> {
        // Allocate a new entity
        let entity = self.entity_master.allocate_entity();
        let generation = entity.generation();

        // Create EntityInland with initial values
        let mut entity_inland = EntityInland::new(
            archetype_id, 
            0, // Unit index will be set by archetype
            generation
        );

        // Get the target archetype
        let archetype = self.archetype_master.get_archetype_mut(archetype_id)
            .ok_or_else(|| anyhow::anyhow!("Archetype {} not found", archetype_id))?;

        // Initialize the EntityInland with archetype info
        archetype.init_entity_inland(&mut entity_inland);

        // Create the entity in the archetype with its components
        if !archetype.create_entity(entity.id(), &mut entity_inland, components) {
            // Failed to create - recycle the entity
            self.entity_master.deallocate_entity(entity);
            bail!("Failed to create entity in archetype");
        }

        // Register the entity with its inland data
        self.entity_master.register_entity(entity, archetype_id, entity_inland.unit_index());

        Ok(entity)
    }

/// Deletes an entity and all its components from the system
pub fn delete_entity(&mut self, entity: Entity) -> bool {
    // Get the EntityInland data
    let entity_inland = match self.entity_master.get_entity_inland(entity) {
        Some(inland) => *inland,
        None => return false,
    };

    let archetype_id = entity_inland.archetype_id();
    let removed_unit_index = entity_inland.unit_index();

    // Find the archetype containing this entity
    if let Some(archetype) = self.archetype_master.get_archetype_mut(archetype_id) {
        // Store the entity count before removal for verification
        let entity_count_before = archetype.entity_count();
        
        // Remove the entity and check if a swap occurred
        match archetype.remove_entity(&entity_inland) {
            Some(swapped_entity_id) => {
                // A swap occurred - update the swapped entity's inland
                if let Some(swapped_entity) = self.entity_master.get_entity(swapped_entity_id) {
                    // The swapped entity now occupies the removed entity's position
                    self.entity_master.update_entity_unit_index(
                        swapped_entity, 
                        removed_unit_index
                    );
                }
                
                // Deallocate the deleted entity
                self.entity_master.deallocate_entity(entity);
                true
            }
            None => {
                // No swap occurred - this could mean:
                // 1. The entity was the last one and was removed successfully
                // 2. The removal failed
                
                // Check if the entity count decreased
                if archetype.entity_count() < entity_count_before {
                    // The entity was successfully removed (it was the last one)
                    self.entity_master.deallocate_entity(entity);
                    true
                } else {
                    // Removal failed
                    false
                }
            }
        }
    } else {
        false
    }
}
    /// Gets a raw pointer to a component for the specified entity
    #[inline]
    pub fn get_component_raw(&self, entity: Entity, component_id: ComponentId) -> Option<*const u8> {
        // Get EntityInland from the master
        let entity_inland = self.entity_master.get_entity_inland(entity)?;

        // Get the archetype and component
        let archetype = self.archetype_master.get_archetype(entity_inland.archetype_id())?;
        archetype.get_component_raw(entity_inland, component_id)
    }

    /// Gets a mutable raw pointer to a component for the specified entity
    #[inline]
    pub fn get_component_raw_mut(&mut self, entity: Entity, component_id: ComponentId) -> Option<*mut u8> {
        // Get EntityInland from the master
        let entity_inland = *self.entity_master.get_entity_inland(entity)?;

        // Get the archetype and component
        let archetype = self.archetype_master.get_archetype_mut(entity_inland.archetype_id())?;
        archetype.get_component_raw_mut(&entity_inland, component_id)
    }

    /// Sets the value of a component for the specified entity
    /// Returns true if the component was successfully set
    #[inline]
    pub fn set_component_raw(
        &mut self, 
        entity: Entity, 
        component_id: ComponentId, 
        component_bytes: &[u8]
    ) -> bool {
        // Get EntityInland from the master
        if let Some(entity_inland) = self.entity_master.get_entity_inland(entity).copied()
            && let Some(archetype) = self.archetype_master.get_archetype_mut(entity_inland.archetype_id())
        {
            return archetype.set_component(&entity_inland, component_id, component_bytes);
        }
        false
    }

    /// Checks if an entity exists with matching generation
    #[inline]
    pub fn has_entity(&self, entity: Entity) -> bool {
        self.entity_master.is_entity_valid(entity)
    }

    /// Gets an entity by ID if it exists and is active
    #[inline]
    pub fn get_entity(&self, entity_id: EntityId) -> Option<Entity> {
        self.entity_master.get_entity(entity_id)
    }

    /// Checks if an entity has a specific component
    #[inline]
    pub fn has_component(&self, entity: Entity, component_id: ComponentId) -> bool {
        if let Some(entity_inland) = self.entity_master.get_entity_inland(entity)
            && let Some(archetype) = self.archetype_master.get_archetype(entity_inland.archetype_id())
        {
            return archetype.has_component_id(component_id);
        }
        false
    }

    /// Gets the archetype ID containing the specified entity
    #[inline]
    pub fn get_entity_archetype_id(&self, entity: Entity) -> Option<ArchetypeId> {
        self.entity_master.get_entity_inland(entity)
            .map(|inland| inland.archetype_id())
    }

    /// Gets the total number of active entities in the system
    #[inline]
    pub fn entity_count(&self) -> usize {
        self.entity_master.entity_count()
    }

    /// Gets the number of archetypes in the system
    #[inline]
    pub fn archetype_count(&self) -> usize {
        self.archetype_master.archetype_count()
    }

    /// Gets the number of recycled entity IDs available for reuse
    #[inline]
    pub fn recycled_entity_count(&self) -> usize {
        self.entity_master.recycled_entity_count()
    }

    /// Gets an iterator over all active entities
    #[inline]
    pub fn iter_entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.entity_master.iter_entities()
    }

    /// Queries entities that have all specified components
    pub fn query_entities(&self, component_ids: &[ComponentId]) -> Vec<Entity> {
        let archetype_ids = self.archetype_master.find_archetypes_with_components(component_ids);
        let mut result = Vec::new();
        
        for archetype_id in archetype_ids {
            if let Some(archetype) = self.archetype_master.get_archetype(archetype_id) {
                // Get all entity IDs from this archetype
                for unit_index in 0..archetype.entity_count() {
                    if let Some(entity_id) = archetype.get_entity_id_at(unit_index)
                        && let Some(entity) = self.entity_master.get_entity(entity_id)
                    {
                        result.push(entity);
                    }
                }
            }
        }
        
        result
    }

    /// Gets raw pointers to multiple components for an entity
    /// Returns a vector of (ComponentId, *const u8) pairs
    pub fn get_components_raw(
        &self, 
        entity: Entity, 
        component_ids: &[ComponentId]
    ) -> Vec<(ComponentId, *const u8)> {
        let mut result = Vec::with_capacity(component_ids.len());
        
        if let Some(entity_inland) = self.entity_master.get_entity_inland(entity)
            && let Some(archetype) = self.archetype_master.get_archetype(entity_inland.archetype_id())
        {
            for &component_id in component_ids {
                if let Some(ptr) = archetype.get_component_raw(entity_inland, component_id) {
                    result.push((component_id, ptr));
                }
            }
        }

        result
    }

    /// Gets mutable raw pointers to multiple components for an entity
    /// Returns a vector of (ComponentId, *mut u8) pairs
    pub fn get_components_raw_mut(
        &mut self,
        entity: Entity,
        component_ids: &[ComponentId]
    ) -> Vec<(ComponentId, *mut u8)> {
        let mut result = Vec::with_capacity(component_ids.len());

        if let Some(entity_inland) = self.entity_master.get_entity_inland(entity).copied()
            && let Some(archetype) = self.archetype_master.get_archetype_mut(entity_inland.archetype_id())
        {
            for &component_id in component_ids {
                if let Some(ptr) = archetype.get_component_raw_mut(&entity_inland, component_id) {
                    result.push((component_id, ptr));
                }
            }
        }

        result
    }

    /// Gets a reference to the EntityMaster
    #[inline]
    pub fn entity_master(&self) -> &EntityMaster {
        &self.entity_master
    }

    /// Gets a mutable reference to the EntityMaster
    #[inline]
    pub fn entity_master_mut(&mut self) -> &mut EntityMaster {
        &mut self.entity_master
    }

    /// Gets a reference to the ArchetypeMaster
    #[inline]
    pub fn archetype_master(&self) -> &ArchetypeMaster {
        &self.archetype_master
    }

    /// Gets a mutable reference to the ArchetypeMaster
    #[inline]
    pub fn archetype_master_mut(&mut self) -> &mut ArchetypeMaster {
        &mut self.archetype_master
    }

    /// Gets a reference to the Arena
    #[inline]
    pub fn arena(&self) -> &Arena {
        &self.arena
    }

    /// Clears all entities and archetypes from the system
    pub fn clear(&mut self) {
        self.entity_master.clear();
        self.archetype_master.clear();
        // Note: We don't clear the arena as it manages its own memory
    }
}

impl Default for EcsMaster {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::component_registry;

    // Define test components with their IDs.
    //
    // Each test module owns its own ComponentId range to avoid inter-test
    // pollution through the global `OnceLock<ComponentLayout>` registry —
    // `OnceLock::set` fixes the first registration and silently ignores
    // subsequent ones, so two test modules registering different types under
    // the same ID end up with a layout mismatch (see audit C-003 — Phase 1b).
    //   ecs_master  : 100-109
    //   query       : 200-209
    //   archetype_master : 300-309
    //   archetype (unit) : 400-409
    const POSITION_ID: ComponentId = 100;
    const VELOCITY_ID: ComponentId = 101;
    const HEALTH_ID: ComponentId = 102;

    #[repr(C)]
    struct Position { x: f32, y: f32, z: f32 }

    #[repr(C)]
    struct Velocity { x: f32, y: f32, z: f32 }

    #[repr(C)]
    struct Health { value: i32 }

    fn register_test_components() {
        // Register components in the global registry
        component_registry::register_layout::<Position>(POSITION_ID);
        component_registry::register_layout::<Velocity>(VELOCITY_ID);
        component_registry::register_layout::<Health>(HEALTH_ID);
    }

    #[test]
    fn test_ecs_master_creation() {
        register_test_components();
        
        let ecs = EcsMaster::new();
        assert_eq!(ecs.entity_count(), 0);
        assert_eq!(ecs.archetype_count(), 0);
    }

    #[test]
    fn test_entity_creation_and_deletion() {
        register_test_components();
        
        let mut ecs = EcsMaster::new();
        
        // Create an archetype
        let archetype_id = ecs.create_archetype(&[POSITION_ID, VELOCITY_ID]);
        
        // Create entities
        let pos = Position { x: 1.0, y: 2.0, z: 3.0 };
        let vel = Velocity { x: 4.0, y: 5.0, z: 6.0 };
        
        let pos_bytes = unsafe {
            std::slice::from_raw_parts(&pos as *const _ as *const u8, std::mem::size_of::<Position>())
        };
        let vel_bytes = unsafe {
            std::slice::from_raw_parts(&vel as *const _ as *const u8, std::mem::size_of::<Velocity>())
        };
        
        let entity1 = ecs.create_entity(archetype_id, vec![
            (POSITION_ID, pos_bytes),
            (VELOCITY_ID, vel_bytes),
        ]).unwrap();
        
        assert_eq!(ecs.entity_count(), 1);
        assert!(ecs.has_entity(entity1));
        
        // Delete entity
        assert!(ecs.delete_entity(entity1));
        assert_eq!(ecs.entity_count(), 0);
        assert!(!ecs.has_entity(entity1));
    }

    #[test]
    fn test_query_entities() {
        register_test_components();
        
        let mut ecs = EcsMaster::new();
        
        // Create archetypes
        let arch1 = ecs.create_archetype(&[POSITION_ID, VELOCITY_ID]);
        let arch2 = ecs.create_archetype(&[POSITION_ID, HEALTH_ID]);
        
        // Create entities (simplified - using dummy data)
        let dummy_bytes = [0u8; 64];
        
        let _entity1 = ecs.create_entity(arch1, vec![
            (POSITION_ID, &dummy_bytes[..12]),
            (VELOCITY_ID, &dummy_bytes[..12]),
        ]).unwrap();
        
        let _entity2 = ecs.create_entity(arch2, vec![
            (POSITION_ID, &dummy_bytes[..12]),
            (HEALTH_ID, &dummy_bytes[..4]),
        ]).unwrap();
        
        // Query entities with Position
        let entities_with_position = ecs.query_entities(&[POSITION_ID]);
        assert_eq!(entities_with_position.len(), 2);
        
        // Query entities with Position and Velocity
        let entities_with_pos_vel = ecs.query_entities(&[POSITION_ID, VELOCITY_ID]);
        assert_eq!(entities_with_pos_vel.len(), 1);
    }
}