use crate::ecs::core::archetype::archetype::RemoveOutcome;
use crate::ecs::core::archetype::archetype_master::ArchetypeMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::entity::entity_master::EntityMaster;
use crate::ecs::core::entity::entity_inland::EntityInland;
use crate::ecs::identifiers::primitives::{ArchetypeId, EntityId, ComponentId, InlandPoolId};
use crate::ecs::memory::arena::Arena;
use crate::ecs::constants::DEFAULT_ARENA_SIZE;
use crate::ecs::error::{EcsError, EcsResult};

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

    /// Creates a new entity with components in the specified archetype.
    ///
    /// Takes a borrowed slice of `(ComponentId, &[u8])` pairs for component data —
    /// zero allocation per call on the components argument. Returns the created
    /// entity if successful.
    ///
    /// # Guard pattern (C-007)
    ///
    /// Preconditions are validated **before** `allocate_entity` so that
    /// no `EntityId` is leaked if the archetype lookup fails. Specifically:
    /// 1. `has_archetype(archetype_id)` is checked first.
    /// 2. Only then is `allocate_entity` called.
    /// 3. If `create_entity` fails, `rewind_allocate` undoes the allocation
    ///    (fresh-ID path) so the ID is not silently wasted.
    ///
    /// Audit: C-010 — switched from Vec to &[...].
    pub fn create_entity(
        &mut self,
        archetype_id: ArchetypeId,
        components: &[(ComponentId, &[u8])],
    ) -> EcsResult<Entity> {
        // Guard: validate archetype exists BEFORE allocating an EntityId.
        // Previously, allocate_entity() was called first, and if the archetype
        // lookup subsequently failed the ID was permanently leaked (C-007).
        if !self.archetype_master.has_archetype(archetype_id) {
            return Err(EcsError::ArchetypeNotFound(archetype_id));
        }

        // Allocate a new entity — only reached if the guard passed.
        let entity = self.entity_master.allocate_entity();
        let generation = entity.generation();

        // Create EntityInland with initial values.
        let mut entity_inland = EntityInland::new(
            archetype_id,
            InlandPoolId(0), // unit index will be set by create_entity
            generation,
        );

        // SAFETY of the get_archetype_mut call: we verified existence above.
        // This cannot return None unless the archetype was removed between the
        // guard and this call, which is impossible in a single-threaded context.
        let archetype = self.archetype_master
            .get_archetype_mut(archetype_id)
            .expect("invariant: archetype existed at guard check; single-threaded");

        archetype.init_entity_inland(&mut entity_inland);

        if !archetype.create_entity(entity.id(), &mut entity_inland, components) {
            // Archetype rejected the entity (e.g. can_push failed). Undo the
            // allocation so the EntityId is not leaked (C-007 rewind path).
            let rewound = self.entity_master.rewind_allocate(entity);
            if !rewound {
                // rewind_allocate returns false for recycled IDs; fall back to
                // the full deallocate path to put the ID back on the free list.
                self.entity_master.deallocate_entity(entity);
            }
            return Err(EcsError::ArchetypeRejectedEntity { archetype_id });
        }

        // Register the entity with its inland data.
        self.entity_master.register_entity(entity, archetype_id, entity_inland.unit_index());

        Ok(entity)
    }

    /// Type-safe wrapper around [`create_entity`] for a single component (Phase 2e — Q-024 follow-up).
    ///
    /// The caller supplies the value by move; this function reads its bytes via
    /// `std::slice::from_raw_parts` and forwards to `create_entity`. No heap
    /// allocation, no `Vec` materialisation, no manual `ComponentId` lookup.
    ///
    /// ```ignore
    /// // Before:
    /// ecs.create_entity(arch_id, &[(Position::component_id(), &pos_bytes)])
    /// // After:
    /// ecs.spawn_one(arch_id, Position { x: 1.0, y: 2.0, z: 3.0 })
    /// ```
    ///
    /// # Drop discipline
    ///
    /// On success, `a` is byte-copied into the pool by `ComponentPool::add`
    /// (`ptr::copy_nonoverlapping`) and the pool's registered `drop_fn` (set up
    /// by `register_layout::<A>`, M-001) becomes the new drop owner. The local
    /// `a` value must NOT run its destructor — `std::mem::forget(a)` suppresses
    /// the local drop only on the Ok path.
    ///
    /// On failure, NO bytes were copied into the pool (the failure modes are
    /// either an early `ArchetypeNotFound` guard or a pool rejection that
    /// rewinds without writing). `a` retains its full identity and runs its
    /// destructor at function-exit scope as usual — no leak, no double-free.
    ///
    /// Mirrors `Query::iter_one` (Phase 2d) on the spawn side: bounded 1-arity
    /// API today, generic tuple version is Phase 2e-extension.
    pub fn spawn_one<A: crate::ecs::core::component::component::Component>(
        &mut self,
        archetype_id: ArchetypeId,
        a: A,
    ) -> EcsResult<Entity> {
        // SAFETY: `a` is a valid, fully-initialised `A` living on the caller's
        // stack; we read `size_of::<A>()` bytes out of it as `&[u8]`. The slice
        // borrow is scoped to this call.
        let bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(
                std::ptr::addr_of!(a) as *const u8,
                std::mem::size_of::<A>(),
            )
        };
        let result = self.create_entity(archetype_id, &[(A::component_id(), bytes)]);
        if result.is_ok() {
            // Bytes are now in the pool; pool's drop_fn is the new owner.
            std::mem::forget(a);
        }
        // On Err: no bytes copied; `a` drops normally at scope exit.
        result
    }

    /// Type-safe two-component spawn — see [`spawn_one`] for rationale.
    ///
    /// Mirrors `Query::iter_two` (Phase 2d) on the spawn side. Bounded 2-arity.
    ///
    /// # Drop discipline
    ///
    /// Same as [`spawn_one`]: on Ok, both `a` and `b` are byte-copied into
    /// their respective pools and `mem::forget`'d locally so their pool
    /// `drop_fn`s become the new owners. On Err, NEITHER value was copied
    /// (either the archetype guard fired before any copy, or the pool's
    /// `can_push_entity_components` rejected the batch before any pool was
    /// mutated — two-phase commit, C-009), so both values drop normally
    /// at function-exit scope.
    pub fn spawn_two<
        A: crate::ecs::core::component::component::Component,
        B: crate::ecs::core::component::component::Component,
    >(
        &mut self,
        archetype_id: ArchetypeId,
        a: A,
        b: B,
    ) -> EcsResult<Entity> {
        // SAFETY: same rationale as `spawn_one`, applied to both inputs.
        // The two slices view distinct stack locals — no aliasing.
        let bytes_a: &[u8] = unsafe {
            std::slice::from_raw_parts(
                std::ptr::addr_of!(a) as *const u8,
                std::mem::size_of::<A>(),
            )
        };
        let bytes_b: &[u8] = unsafe {
            std::slice::from_raw_parts(
                std::ptr::addr_of!(b) as *const u8,
                std::mem::size_of::<B>(),
            )
        };
        let result = self.create_entity(
            archetype_id,
            &[
                (A::component_id(), bytes_a),
                (B::component_id(), bytes_b),
            ],
        );
        if result.is_ok() {
            std::mem::forget(a);
            std::mem::forget(b);
        }
        result
    }

    /// Deletes an entity and all its components from the system.
    ///
    /// Returns `true` on success, `false` if the entity does not exist or if
    /// archetype removal fails (`RemoveOutcome::PoolFailure`). The explicit
    /// [`RemoveOutcome`] enum (C-006) replaces the previous fragile
    /// `Option<EntityId>`-based logic.
    pub fn delete_entity(&mut self, entity: Entity) -> bool {
        let entity_inland = match self.entity_master.get_entity_inland(entity) {
            Some(inland) => *inland,
            None => return false,
        };

        let archetype_id = entity_inland.archetype_id();
        let removed_unit_index = entity_inland.unit_index();

        let archetype = match self.archetype_master.get_archetype_mut(archetype_id) {
            Some(arch) => arch,
            None => return false,
        };

        match archetype.remove_entity(&entity_inland) {
            RemoveOutcome::Last => {
                self.entity_master.deallocate_entity(entity);
                true
            }
            RemoveOutcome::Swapped { moved_entity: swapped_entity_id } => {
                // Update the swapped entity's unit_index to the vacated slot.
                if let Some(swapped_entity) = self.entity_master.get_entity(swapped_entity_id) {
                    self.entity_master.update_entity_unit_index(
                        swapped_entity,
                        removed_unit_index,
                    );
                }
                self.entity_master.deallocate_entity(entity);
                true
            }
            RemoveOutcome::PoolFailure => false,
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
                    if let Some(entity_id) = archetype.get_entity_id_at(InlandPoolId(unit_index))
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
    const POSITION_ID: ComponentId = ComponentId(100);
    const VELOCITY_ID: ComponentId = ComponentId(101);
    const HEALTH_ID: ComponentId = ComponentId(102);

    #[repr(C)]
    struct Position { x: f32, y: f32, z: f32 }

    #[repr(C)]
    struct Velocity { x: f32, y: f32, z: f32 }

    #[repr(C)]
    struct Health { value: i32 }

    // Component impls mirror what `#[derive(Component)]` generates — needed
    // because Phase 2e `spawn_one` / `spawn_two` are bounded by `Component`,
    // and the test types must satisfy that bound to exercise the spawn path.
    use crate::ecs::core::component::component::Component;

    impl Component for Position {
        fn component_id() -> ComponentId { POSITION_ID }
    }
    impl Component for Velocity {
        fn component_id() -> ComponentId { VELOCITY_ID }
    }
    impl Component for Health {
        fn component_id() -> ComponentId { HEALTH_ID }
    }

    fn register_test_components() {
        // Register components in the global registry
        component_registry::register_layout::<Position>(POSITION_ID.0);
        component_registry::register_layout::<Velocity>(VELOCITY_ID.0);
        component_registry::register_layout::<Health>(HEALTH_ID.0);
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
        
        let entity1 = ecs.create_entity(archetype_id, &[
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

        let _entity1 = ecs.create_entity(arch1, &[
            (POSITION_ID, &dummy_bytes[..12]),
            (VELOCITY_ID, &dummy_bytes[..12]),
        ]).unwrap();

        let _entity2 = ecs.create_entity(arch2, &[
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

    // C-007 guard tests: validate that create_entity never leaks EntityIds.
    //
    // The guard sequence is:
    //   1. has_archetype() checked BEFORE allocate_entity()
    //   2. If archetype not found → bail! (no EntityId consumed)
    //   3. On post-allocation failure → rewind_allocate() undoes fresh-ID
    //      allocation, or deallocate_entity() recycles an existing one.

    /// Creating an entity in a non-existent archetype must fail and must not
    /// consume an EntityId from the allocator.
    #[test]
    fn test_create_entity_nonexistent_archetype_no_id_leak() {
        register_test_components();

        let mut ecs = EcsMaster::new();

        let dummy_bytes = [0u8; 12];

        // Attempt to create an entity in archetype 999 (never created).
        let result = ecs.create_entity(ArchetypeId(999), &[(POSITION_ID, &dummy_bytes)]);
        // C-019: caller can pattern-match on the concrete EcsError variant
        // (not just `is_err`) — the whole point of switching off `anyhow`.
        assert!(
            matches!(result, Err(EcsError::ArchetypeNotFound(ArchetypeId(999)))),
            "expected Err(ArchetypeNotFound(ArchetypeId(999))), got {:?}",
            result
        );

        // No EntityId must have been allocated: next fresh id stays at 0.
        assert_eq!(ecs.entity_master().next_entity_id(), EntityId(0),
            "EntityId must not be consumed when the guard fires");

        // No active entities and no recycled slots.
        assert_eq!(ecs.entity_count(), 0);
        assert_eq!(ecs.recycled_entity_count(), 0);
    }

    /// Consecutive failed guard calls must not accumulate leaked EntityIds.
    #[test]
    fn test_repeated_guard_failures_do_not_leak_ids() {
        register_test_components();

        let mut ecs = EcsMaster::new();
        let dummy_bytes = [0u8; 12];

        for _ in 0..5 {
            let _ = ecs.create_entity(ArchetypeId(42), &[(POSITION_ID, &dummy_bytes)]);
        }

        // After 5 failed guard calls the fresh-id counter must still be 0.
        assert_eq!(ecs.entity_master().next_entity_id(), EntityId(0));
        assert_eq!(ecs.entity_count(), 0);
        assert_eq!(ecs.recycled_entity_count(), 0);
    }

    /// A successful create_entity followed by a delete_entity returns the
    /// EntityId to the free list. A subsequent create_entity in a bad
    /// archetype must NOT consume that recycled slot.
    #[test]
    fn test_guard_does_not_consume_recycled_slot() {
        register_test_components();

        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[POSITION_ID, VELOCITY_ID]);

        let pos_bytes = [0u8; 12];
        let vel_bytes = [0u8; 12];

        // Create and immediately delete an entity.
        let entity = ecs.create_entity(arch, &[
            (POSITION_ID, &pos_bytes),
            (VELOCITY_ID, &vel_bytes),
        ]).unwrap();
        assert!(ecs.delete_entity(entity));
        assert_eq!(ecs.recycled_entity_count(), 1);

        // A guard-failing call must not touch the free list.
        let _ = ecs.create_entity(ArchetypeId(999), &[(POSITION_ID, &pos_bytes)]);
        assert_eq!(ecs.recycled_entity_count(), 1,
            "free list must not be consumed when guard fires before allocate_entity");
        assert_eq!(ecs.entity_count(), 0);
    }

    /// `rewind_allocate` is the internal mechanism backing the C-007 guard.
    /// Exercise it directly through entity_master() to verify the invariant:
    /// rewinding a fresh (non-registered) entity decrements next_entity_id.
    #[test]
    fn test_rewind_allocate_restores_fresh_id() {
        register_test_components();

        let mut ecs = EcsMaster::new();
        let entity_master = ecs.entity_master_mut();

        // Allocate a fresh entity without registering it.
        let entity = entity_master.allocate_entity();
        assert_eq!(entity.id(), EntityId(0));
        assert_eq!(entity_master.next_entity_id(), EntityId(1));

        // Rewind must succeed and restore next_entity_id to 0.
        let rewound = entity_master.rewind_allocate(entity);
        assert!(rewound, "fresh-ID rewind must succeed");
        assert_eq!(entity_master.next_entity_id(), EntityId(0),
            "next_entity_id must be restored after rewind");
        assert_eq!(entity_master.entity_count(), 0);
    }

    /// After a successful create_entity in a valid archetype the entity count
    /// must be 1 and the EntityId must be stable across the rewind path
    /// (i.e., the rewind path is never taken when creation succeeds).
    #[test]
    fn test_successful_create_entity_no_rewind() {
        register_test_components();

        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[POSITION_ID, VELOCITY_ID]);

        let pos = Position { x: 1.0, y: 0.0, z: 0.0 };
        let vel = Velocity { x: 0.0, y: 1.0, z: 0.0 };
        let pos_bytes = unsafe {
            std::slice::from_raw_parts(&pos as *const _ as *const u8, std::mem::size_of::<Position>())
        };
        let vel_bytes = unsafe {
            std::slice::from_raw_parts(&vel as *const _ as *const u8, std::mem::size_of::<Velocity>())
        };

        let entity = ecs.create_entity(arch, &[
            (POSITION_ID, pos_bytes),
            (VELOCITY_ID, vel_bytes),
        ]).unwrap();

        assert!(ecs.has_entity(entity));
        assert_eq!(ecs.entity_count(), 1);
        // next_entity_id was advanced to 1 and not rewound.
        assert_eq!(ecs.entity_master().next_entity_id(), EntityId(1));
        assert_eq!(ecs.recycled_entity_count(), 0);
    }

    // --- Phase 2e: spawn_one / spawn_two ergonomic wrappers ---

    /// `spawn_one` is equivalent to a 1-component `create_entity` call with
    /// auto-derived `ComponentId` and zero-alloc byte slicing.
    #[test]
    fn spawn_one_creates_entity_with_component() {
        register_test_components();

        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[POSITION_ID]);

        let entity = ecs.spawn_one(arch, Position { x: 1.5, y: 2.5, z: 3.5 })
            .expect("spawn_one in valid archetype must succeed");

        assert!(ecs.has_entity(entity), "spawned entity must be reachable");
        assert_eq!(ecs.entity_count(), 1);
    }

    /// `spawn_two` packs two components in archetype-defined order; result
    /// must be a fully-formed entity.
    #[test]
    fn spawn_two_creates_entity_with_both_components() {
        register_test_components();

        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[POSITION_ID, VELOCITY_ID]);

        let entity = ecs.spawn_two(
            arch,
            Position { x: 10.0, y: 20.0, z: 30.0 },
            Velocity { x: 1.0, y: 2.0, z: 3.0 },
        ).expect("spawn_two in valid archetype must succeed");

        assert!(ecs.has_entity(entity));
        assert_eq!(ecs.entity_count(), 1);
    }

    /// `spawn_one` must propagate `ArchetypeNotFound` for a bogus archetype id
    /// AND must not consume an EntityId from the allocator (C-007 guard
    /// behaviour carries through the wrapper).
    #[test]
    fn spawn_one_unknown_archetype_returns_err_no_leak() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let result = ecs.spawn_one(ArchetypeId(999), Position { x: 1.0, y: 2.0, z: 3.0 });
        assert!(
            matches!(result, Err(EcsError::ArchetypeNotFound(ArchetypeId(999)))),
            "spawn_one must propagate the typed error variant unchanged"
        );
        assert_eq!(ecs.entity_master().next_entity_id(), EntityId(0),
            "no EntityId must be consumed when the archetype guard fires");
        assert_eq!(ecs.entity_count(), 0);
    }
}