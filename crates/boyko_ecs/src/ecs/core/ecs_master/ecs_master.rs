use crate::ecs::core::archetype::archetype::{Archetype, RemoveOutcome};
use crate::ecs::core::archetype::archetype_master::ArchetypeMaster;
use crate::ecs::core::component::component_registry::MAX_COMPONENTS;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::entity::entity_inland::EntityInland;
use crate::ecs::core::entity::entity_master::EntityMaster;
use crate::ecs::core::events::event::Event;
use crate::ecs::core::events::event_config::EventConfig;
use crate::ecs::core::events::event_dispatcher::EventDispatcher;
use crate::ecs::core::resources::resource::Resource;
use crate::ecs::core::resources::resources::Resources;
use crate::ecs::core::system::{
    fn_once_system::FnOnceSystem, system::System, system_param::SystemParam,
    unsafe_ecs_cell::UnsafeEcsCell,
};
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId, EntityId, InlandPoolId};
use crate::ecs::memory::arena::Arena;
use crate::ecs::constants::DEFAULT_ARENA_SIZE;
use crate::ecs::error::{EcsError, EcsResult};

/// Main ECS manager that coordinates entities, archetypes, memory, and events.
///
/// # Field order (drop order — Phase 8a C5 RESOLUTION)
///
/// Fields are dropped in declaration order:
/// `resources → events → entity_master → archetype_master → arena`.
///
/// `resources: Resources` is the **first** field so it drops first. A
/// `Resource`'s `Drop` impl runs while every other subsystem is still alive;
/// if user code violates the [`Resource`] contract and touches the world from
/// `Drop`, the world is still fully valid. The most-defensive position
/// prevents the worst case from being UB.
///
/// `events: EventDispatcher` drops next. Event buffers live in their own
/// heap allocations (separate from the arena) and do not reference arena
/// memory.
///
/// `arena` **must** be last because `ArchetypeMaster`/`Archetype` store
/// `*const Arena` (raw provenance pointer — Phase 3a Miri retag fix; previously
/// `NonNull<Arena>`, audit finding C-001) and `ComponentPool`s store raw
/// pointers into the arena's backing buffer. Dropping the arena last guarantees
/// those pointers remain valid while child `Drop`s run.
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
///
/// [`Resource`]: crate::ecs::core::resources::Resource
pub struct EcsMaster {
    /// World-global resources slab.
    ///
    /// Dropped first per the Phase 8a C5 drop-order resolution. Public facade
    /// methods (`insert_resource`, `remove_resource`, `resource`,
    /// `resource_mut`) are deferred to Step 9; this minimal field addition
    /// unblocks Step 7's `Res<R>` / `ResMut<R>` `get_param` via
    /// `UnsafeEcsCell::resources()` / `resources_mut()`.
    pub(crate) resources: Resources,

    /// Event dispatcher — dropped after `resources` and before the entity /
    /// archetype subsystems. Event buffers live in their own heap allocations
    /// independent of the arena.
    events: EventDispatcher,

    /// Entity management system.
    entity_master: EntityMaster,

    /// Archetype management system.
    archetype_master: ArchetypeMaster,

    /// Memory arena for component allocation. `Box` provides a stable heap
    /// address shared by every `*const Arena` raw provenance pointer stored in
    /// child structures (`ArchetypeMaster`, `Archetype`, `ComponentPool`).
    arena: Box<Arena>,
}

impl EcsMaster {
    /// Creates a new empty EcsMaster.
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
        // EventDispatcher::new(1) validates 1 ∈ 1..=64 — never fails.
        let events = EventDispatcher::new(1)
            .expect("invariant: default thread_count=1 is always valid");
        Self {
            resources: Resources::new(),
            events,
            entity_master: EntityMaster::new(),
            archetype_master,
            arena,
        }
    }

    /// Creates a new EcsMaster with pre-allocated capacity.
    pub fn with_capacity(entity_capacity: usize, archetype_capacity: usize) -> Self {
        let arena: Box<Arena> = Box::new(Arena::with_capacity(DEFAULT_ARENA_SIZE));
        // SAFETY: same rationale as `EcsMaster::new`.
        let arena_ptr: *const Arena = unsafe {
            let box_ptr: *const Box<Arena> = std::ptr::addr_of!(arena);
            *(box_ptr.cast::<*const Arena>())
        };
        // SAFETY: same contract as `EcsMaster::new`.
        let archetype_master = unsafe { ArchetypeMaster::with_capacity(arena_ptr, archetype_capacity) };
        // EventDispatcher::new(1) validates 1 ∈ 1..=64 — never fails.
        let events = EventDispatcher::new(1)
            .expect("invariant: default thread_count=1 is always valid");
        Self {
            resources: Resources::new(),
            events,
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
    /// # W7 choreography (Phase 7)
    ///
    /// 1. Mint write-capable `*mut Archetype` via `archetype_ptr_for` under
    ///    `&mut self`. The raw pointer does not participate in borrow
    ///    checking, so it lives across the subsequent `&mut entity_master`
    ///    call without conflict.
    /// 2. Allocate the entity id.
    /// 3. Reborrow the raw pointer as `&mut Archetype` (scoped tightly to
    ///    the `create_entity` call) to push the new row.
    /// 4. On rejection, rewind the entity-id allocation (C-007 rewind path).
    /// 5. On success, register the entity in the Phase 7 fast store
    ///    (read path of `get_component_raw`, `has_entity`, and friends).
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

        // Step 1 of W7: mint write-capable *mut Archetype. The raw pointer is
        // not subject to borrow checking, so it can outlive the &mut borrow
        // on archetype_master that produced it — see U14.
        let archetype_ptr = self.archetype_master
            .archetype_ptr_for(archetype_id)
            .expect("invariant: archetype existed at guard check; single-threaded");

        // Step 2 of W7: allocate the entity id (fresh or recycled).
        let entity = self.entity_master.allocate_entity();

        // Step 3 of W7: reborrow archetype_ptr as &mut Archetype inside a
        // tight scope so the &mut reference is dropped before any further
        // entity_master mutation.
        let mut new_unit_index: u32 = 0;
        let pushed = {
            // SAFETY (U14, U1, U2):
            //   - U14: archetype_ptr was just minted via archetype_ptr_for
            //     under &mut self, so the provenance is write-capable; the
            //     bundle slab address is stable; no other live borrow into
            //     this slot exists (single-threaded EcsMaster).
            //   - U1/U2: slab address stable, slab slot lifetime ⊇
            //     EcsMaster lifetime (bundle invariants).
            //   - The reborrow is scoped to this block; once create_entity
            //     returns, the &mut Archetype is dropped before any further
            //     self.entity_master calls.
            let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };
            archetype.create_entity(entity.id(), &mut new_unit_index, components)
        };

        if !pushed {
            // Step 4 of W7: archetype rejected the push (capacity / signature
            // mismatch). Undo the allocation so the EntityId is not leaked.
            let rewound = self.entity_master.rewind_allocate(entity);
            if !rewound {
                // rewind_allocate returns false for recycled IDs; fall back
                // to the full deallocate path so the ID returns to the free
                // list.
                self.entity_master.deallocate_entity(entity);
            }
            return Err(EcsError::ArchetypeRejectedEntity { archetype_id });
        }

        // Step 5 of W7: register the entity in the Phase 7 fast store. This
        // is the read path consumed by get_component_raw, has_entity,
        // set_component_raw, and the typed get_component<T> /
        // get_component_mut<T> wrappers.
        self.entity_master.register_entity_with_ptr(entity, archetype_ptr, new_unit_index);

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
        // Resolve the fast inland by value. Copying 16 B releases the
        // entity_master borrow before we dereference the raw archetype_ptr.
        let inland: EntityInland = {
            let Some(slot) = self.entity_master.entities_inland.get(entity.id().0) else {
                return false;
            };
            if slot.is_null() || slot.generation() != entity.generation() {
                return false;
            }
            *slot
        };
        let removed_unit_index = InlandPoolId(inland.unit_index() as usize);

        // SAFETY (U1, U2, U11, U14): archetype_ptr was minted via
        //   archetype_ptr_for under &mut self at registration time; the
        //   bundle slab is heap-stable for the EcsMaster's lifetime;
        //   single-threaded &mut self gives exclusive access and no other
        //   live borrow into this slot exists.
        let archetype: &mut Archetype = unsafe { &mut *inland.archetype_ptr() };
        let outcome = archetype.remove_entity(removed_unit_index);

        match outcome {
            RemoveOutcome::Last => {
                self.entity_master.deallocate_entity(entity);
                true
            }
            RemoveOutcome::Swapped { moved_entity: swapped_entity_id } => {
                // The entity that moved into the vacated slot needs its
                // fast-store unit_index updated.
                if let Some(slot) = self.entity_master.entities_inland
                    .get_mut(swapped_entity_id.0)
                {
                    slot.set_unit_index(removed_unit_index.0 as u32);
                }
                self.entity_master.deallocate_entity(entity);
                true
            }
            RemoveOutcome::PoolFailure => false,
        }
    }

    /// Fast random access read: 3-4 cache lines, ~12-16 ns target.
    ///
    /// Lookup sequence:
    ///   1. `entity_master.entities_inland[entity.id().0]` — 1 line.
    ///   2. Null check + generation check (both fields in the same line as 1).
    ///   3. `(*archetype_ptr).columns[component_id.0]` — 1 line (`columns` at
    ///      offset 0; for `ComponentId.0 < 4` shares the line with the
    ///      archetype deref).
    ///   4. `column.ptr.add(unit_index * stride)` — arithmetic on the
    ///      cached pointer; final line is the component itself.
    ///
    /// Returns `None` for stale entities (generation mismatch), missing
    /// components (column is null), or never-registered entities
    /// (archetype_ptr is null).
    #[inline]
    pub fn get_component_raw(
        &self,
        entity: Entity,
        component_id: ComponentId,
    ) -> Option<*const u8> {
        // Line 1: entity_master.entities_inland[entity.id().0]
        let inland = self.entity_master.entities_inland.get(entity.id().0)?;
        // Null check (dead slot) + generation check (stale handle).
        // Order chosen so the null check covers never-registered IDs first.
        if inland.is_null() {
            return None;
        }
        if inland.generation() != entity.generation() {
            return None;
        }
        // SAFETY (U1, U2, U11): archetype_ptr was minted via raw arithmetic
        //   from the bundle slab (Step 4); slab heap address is stable for
        //   the EcsMaster's lifetime; &self gives shared access to the slab.
        let archetype = unsafe { &*inland.archetype_ptr() };

        debug_assert!(component_id.0 < MAX_COMPONENTS);
        // SAFETY (U4): columns is [Column; MAX_COMPONENTS]; bound checked above.
        let column = unsafe { archetype.columns.get_unchecked(component_id.0) };
        if column.ptr.is_null() {
            return None;
        }

        // SAFETY (U5, U6, U10):
        //   - U5: column.ptr / stride are set by refresh_column after add_pool.
        //   - U6: pool buffer pointer is write-once at add_pool (Phase 7 D5
        //     audit table).
        //   - U10: unit_index < archetype.current_index for any alive
        //     entity; multiplication fits because `stride * MAX_ENTITIES`
        //     ≤ pool buffer size, and `unit_index < MAX_ENTITIES`.
        Some(unsafe {
            column.ptr.add(inland.unit_index() as usize * column.stride as usize) as *const u8
        })
    }

    /// Mutable fast random access. `EntityInland` is `Copy`; we copy
    /// 16 B to drop the `EntityMaster` borrow before reborrowing the slab
    /// pointer as `&mut Archetype` (W4 / U14).
    #[inline]
    pub fn get_component_raw_mut(
        &mut self,
        entity: Entity,
        component_id: ComponentId,
    ) -> Option<*mut u8> {
        // Copy the inland by value to release the entity_master borrow.
        let inland: EntityInland = *self.entity_master.entities_inland
            .get(entity.id().0)?;
        if inland.is_null() {
            return None;
        }
        if inland.generation() != entity.generation() {
            return None;
        }
        debug_assert!(component_id.0 < MAX_COMPONENTS);

        // SAFETY (U1, U2, U11, U14):
        //   - U14: archetype_ptr is write-capable provenance (minted via
        //     archetype_ptr_for under &mut EcsMaster during create_entity);
        //     single-threaded &mut self gives exclusive access; no other
        //     live borrow into the slot exists.
        let archetype = unsafe { &mut *inland.archetype_ptr() };

        // SAFETY (U4): same as get_component_raw.
        let column = unsafe { archetype.columns.get_unchecked(component_id.0) };
        if column.ptr.is_null() {
            return None;
        }

        // SAFETY (U5, U6, U10): same as get_component_raw plus
        //   &mut self exclusivity ⇒ the returned *mut points to a uniquely
        //   accessible byte range.
        Some(unsafe {
            column.ptr.add(inland.unit_index() as usize * column.stride as usize)
        })
    }

    /// Fast component write: `~15-18 ns` target. Returns `false`
    /// for stale entities, missing components, or never-registered entities.
    /// On success, byte-copies the provided slice into the component slot.
    ///
    /// `component_bytes.len()` must equal the pool's stride; mismatched
    /// sizes produce undefined behavior in release. Callers should obtain
    /// the slice from a properly-sized `&T` for the target component type
    /// (see [`get_component_mut`] typed wrappers).
    #[inline]
    pub fn set_component_raw(
        &mut self,
        entity: Entity,
        component_id: ComponentId,
        component_bytes: &[u8],
    ) -> bool {
        let Some(dst) = self.get_component_raw_mut(entity, component_id) else {
            return false;
        };
        // Stride is not re-queried here; the size invariant lives at the
        // caller boundary (typed wrappers downcast from `&T` with
        // `size_of::<T>()`). A debug-assertable stride check would require
        // threading the column reference back out of the inner lookup,
        // which defeats the fast-path goal. The pool layer carries the
        // ultimate size guarantee through `Layout`.
        // SAFETY (U5, U6, U10):
        //   - dst is a valid *mut u8 to a byte range of size `stride` for
        //     the target component (U5/U6 — column resolved through the
        //     same fast path as get_component_raw_mut).
        //   - The caller's slice is sized to match by API contract; typed
        //     wrappers enforce this via `size_of::<T>()`.
        //   - Single-threaded &mut self ⇒ no concurrent reader.
        //   - copy_nonoverlapping is sound because the slice and the pool
        //     buffer live in disjoint allocations (slice is a caller-stack
        //     view; the pool buffer is arena-owned).
        unsafe {
            std::ptr::copy_nonoverlapping(component_bytes.as_ptr(), dst, component_bytes.len());
        }
        true
    }

    /// Typed read accessor. Returns a shared reference to the
    /// component of type `T` owned by `entity`, or `None` if the entity is
    /// stale, the archetype does not host `T`, or the entity was never
    /// registered.
    #[inline]
    pub fn get_component<T: crate::ecs::core::component::component::Component>(
        &self,
        entity: Entity,
    ) -> Option<&T> {
        let raw = self.get_component_raw(entity, T::component_id())?;
        // SAFETY: the pool was registered with T::component_id(), so the
        //   bytes at `raw` are a valid `T` (M-001 drop-fn / layout guarantee
        //   from the component registry). The lifetime of the returned
        //   reference is bounded by &self.
        Some(unsafe { &*(raw as *const T) })
    }

    /// Typed mutable accessor. Symmetric counterpart of
    /// [`get_component`] returning `&mut T`.
    #[inline]
    pub fn get_component_mut<T: crate::ecs::core::component::component::Component>(
        &mut self,
        entity: Entity,
    ) -> Option<&mut T> {
        let raw = self.get_component_raw_mut(entity, T::component_id())?;
        // SAFETY: same as get_component, plus &mut self ⇒ exclusive access.
        Some(unsafe { &mut *(raw as *mut T) })
    }

    /// Fast existence check: 1 cache line, ~5 ns target. Returns `true`
    /// iff the slot for `entity.id()` is live AND its stored generation
    /// matches the handle.
    #[inline]
    pub fn has_entity(&self, entity: Entity) -> bool {
        let Some(inland) = self.entity_master.entities_inland.get(entity.id().0) else {
            return false;
        };
        !inland.is_null() && inland.generation() == entity.generation()
    }

    /// Gets an entity by ID if it exists and is active
    #[inline]
    pub fn get_entity(&self, entity_id: EntityId) -> Option<Entity> {
        self.entity_master.get_entity(entity_id)
    }

    /// Checks if an entity has a specific component.
    ///
    /// Uses the fast inland + column lookup: a null `column.ptr` is the
    /// single source of truth for "archetype does not host this component".
    #[inline]
    pub fn has_component(&self, entity: Entity, component_id: ComponentId) -> bool {
        let Some(inland) = self.entity_master.entities_inland.get(entity.id().0) else {
            return false;
        };
        if inland.is_null() || inland.generation() != entity.generation() {
            return false;
        }
        // SAFETY (U1, U2, U11): archetype_ptr is stable slab provenance.
        let archetype = unsafe { &*inland.archetype_ptr() };
        if component_id.0 >= MAX_COMPONENTS {
            return false;
        }
        // SAFETY (U4): bounded by check above.
        !unsafe { archetype.columns.get_unchecked(component_id.0) }.ptr.is_null()
    }

    /// Gets the archetype ID containing the specified entity.
    ///
    /// Derives the id from the fast inland's slab pointer via
    /// [`Archetype::id`] — no SparseMap traversal.
    #[inline]
    pub fn get_entity_archetype_id(&self, entity: Entity) -> Option<ArchetypeId> {
        let inland = self.entity_master.entities_inland.get(entity.id().0)?;
        if inland.is_null() || inland.generation() != entity.generation() {
            return None;
        }
        // SAFETY (U1, U2, U11): same as get_component_raw.
        let archetype = unsafe { &*inland.archetype_ptr() };
        Some(archetype.id())
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

    /// Gets raw pointers to multiple components for an entity.
    ///
    /// Resolves the inland record once, then walks `component_ids` reading
    /// the cached `Column` table inline. Returns `(ComponentId, *const u8)`
    /// pairs only for components actually hosted by the entity's archetype.
    pub fn get_components_raw(
        &self,
        entity: Entity,
        component_ids: &[ComponentId],
    ) -> Vec<(ComponentId, *const u8)> {
        let mut result = Vec::with_capacity(component_ids.len());
        let Some(inland) = self.entity_master.entities_inland.get(entity.id().0) else {
            return result;
        };
        if inland.is_null() || inland.generation() != entity.generation() {
            return result;
        }
        // SAFETY (U1, U2, U11): archetype_ptr is stable slab provenance.
        let archetype = unsafe { &*inland.archetype_ptr() };
        let unit_index = inland.unit_index() as usize;
        for &component_id in component_ids {
            if component_id.0 >= MAX_COMPONENTS {
                continue;
            }
            // SAFETY (U4): bounded by check above.
            let column = unsafe { archetype.columns.get_unchecked(component_id.0) };
            if column.ptr.is_null() {
                continue;
            }
            // SAFETY (U5, U6, U10): same as get_component_raw.
            let ptr = unsafe { column.ptr.add(unit_index * column.stride as usize) } as *const u8;
            result.push((component_id, ptr));
        }
        result
    }

    /// Gets mutable raw pointers to multiple components for an entity.
    ///
    /// Mutable counterpart of [`get_components_raw`]; the inland is copied
    /// by value (16 B) to release the `entity_master` borrow before the
    /// `archetype_ptr` is reborrowed as `&mut Archetype` (W4 / U14).
    pub fn get_components_raw_mut(
        &mut self,
        entity: Entity,
        component_ids: &[ComponentId],
    ) -> Vec<(ComponentId, *mut u8)> {
        let mut result = Vec::with_capacity(component_ids.len());
        let inland: EntityInland = match self.entity_master.entities_inland
            .get(entity.id().0)
        {
            Some(i) => *i,
            None => return result,
        };
        if inland.is_null() || inland.generation() != entity.generation() {
            return result;
        }
        // SAFETY (U1, U2, U11, U14): write-capable slab provenance under
        //   &mut self; no other live borrow into this slot.
        let archetype = unsafe { &mut *inland.archetype_ptr() };
        let unit_index = inland.unit_index() as usize;
        for &component_id in component_ids {
            if component_id.0 >= MAX_COMPONENTS {
                continue;
            }
            // SAFETY (U4): bounded by check above.
            let column = unsafe { archetype.columns.get_unchecked(component_id.0) };
            if column.ptr.is_null() {
                continue;
            }
            // SAFETY (U5, U6, U10): same as get_component_raw_mut.
            let ptr = unsafe { column.ptr.add(unit_index * column.stride as usize) };
            result.push((component_id, ptr));
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

    // ── Event dispatch proxy methods (Phase 6) ──────────────────────────────

    /// Returns a shared reference to the event dispatcher.
    #[inline]
    pub fn events(&self) -> &EventDispatcher {
        &self.events
    }

    /// Returns a mutable reference to the event dispatcher.
    #[inline]
    pub fn events_mut(&mut self) -> &mut EventDispatcher {
        &mut self.events
    }

    /// Preregisters event type `E` with a custom config.
    ///
    /// Must be called before the first `send_event::<E>` or `events_of::<E>`.
    /// All write lanes and the reader buffer are allocated here; no allocation
    /// occurs during steady-state `send_event` or `update_events`.
    ///
    /// # Errors
    ///
    /// Forwards errors from [`EventDispatcher::preregister`].
    #[inline]
    pub fn preregister_event<E: Event>(&mut self, cfg: EventConfig) -> EcsResult<()> {
        self.events.preregister::<E>(cfg)
    }

    /// Preregisters event type `E` with default capacity and the dispatcher's
    /// validated `default_thread_count`.
    ///
    /// Equivalent to calling [`preregister_event`] with
    /// `EventConfig::default_for(self.events.default_thread_count())`.
    ///
    /// # Errors
    ///
    /// Forwards errors from [`EventDispatcher::preregister`].
    ///
    /// [`preregister_event`]: EcsMaster::preregister_event
    #[inline]
    pub fn preregister_event_default<E: Event>(&mut self) -> EcsResult<()> {
        let cfg = EventConfig::default_for(self.events.default_thread_count())
            .expect("invariant: default_thread_count was validated at EventDispatcher::new");
        self.events.preregister::<E>(cfg)
    }

    /// Sends a single event of type `E` to the lane for `thread_index`.
    ///
    /// # Errors
    ///
    /// Forwards errors from [`EventDispatcher::send`].
    #[inline]
    pub fn send_event<E: Event>(&self, thread_index: u32, event: E) -> EcsResult<()> {
        self.events.send::<E>(thread_index, event)
    }

    /// Returns the slice of events of type `E` from the previous frame.
    ///
    /// Returns an empty slice if `E` was not registered or if no events were
    /// sent last frame. Slice remains valid until the next `update_events` call.
    #[inline]
    pub fn events_of<E: Event>(&self) -> &[E] {
        self.events.events::<E>()
    }

    /// Advances the frame counter and flattens write lanes into reader buffers.
    ///
    /// Must be called once per frame. After this call, `events_of::<E>()` returns
    /// the events sent during the frame that just ended.
    #[inline]
    pub fn update_events(&mut self) {
        self.events.update_events();
    }

    // ── System execution (Phase 8a Step 8) ──────────────────────────────────

    /// Runs a single [`System`] once, end-to-end.
    ///
    /// Generic over `S: System` so the caller's system value survives across
    /// calls without virtual dispatch. Sequence:
    ///   1. [`System::initialize`] — idempotent two-phase init (state then
    ///      access surface); subsequent calls short-circuit so cross-call
    ///      `&mut S` reuse is supported.
    ///   2. [`UnsafeEcsCell::new_mutable`] — mints a write-capable cell
    ///      bound to the `&mut self` borrow scope.
    ///   3. [`System::run_unsafe`] — invokes the system body.
    ///
    /// Phase 9's scheduler will replace this method with a multi-system
    /// runner that resolves aliasing via the `Access` conflict graph; for
    /// now `&mut EcsMaster` enforces the S1 invariant trivially.
    ///
    /// [`System`]: crate::ecs::core::system::system::System
    /// [`System::initialize`]: crate::ecs::core::system::system::System::initialize
    /// [`System::run_unsafe`]: crate::ecs::core::system::system::System::run_unsafe
    /// [`UnsafeEcsCell::new_mutable`]: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::new_mutable
    pub fn run_system_once<S: System>(&mut self, system: &mut S) -> S::Out {
        system.initialize(self);
        // SAFETY (U_C1): `cell` does not outlive the `&mut self` borrow — it
        //   is consumed by `run_unsafe` on the next line and cannot escape.
        let cell = unsafe { UnsafeEcsCell::new_mutable(self) };
        // SAFETY (S1): `&mut self` is exclusive for the entire call ⇒ no
        //   other `System::run_unsafe` is in flight on this `EcsMaster`.
        //   The Phase 9 scheduler will replace this trivial enforcement
        //   with the `Access` conflict graph.
        unsafe { system.run_unsafe(cell) }
    }

    /// Convenience wrapper around [`run_system_once`] that materialises a
    /// [`FnOnceSystem`] from `body` (M5 RESOLUTION).
    ///
    /// # Turbofish requirement (W3)
    ///
    /// Closure-argument inference cannot deduce the [`SystemParam`] tuple
    /// `P` from the body alone, so callers must spell `P` out:
    ///
    /// ```ignore
    /// ecs.run_closure_once::<(Res<A>, ResMut<B>), _, _>(|(a, b)| { /* ... */ });
    /// ```
    ///
    /// Phase 8c's `IntoSystem` adapter removes the requirement by inferring
    /// `P` from the closure signature.
    ///
    /// [`run_system_once`]: EcsMaster::run_system_once
    /// [`FnOnceSystem`]: crate::ecs::core::system::fn_once_system::FnOnceSystem
    /// [`SystemParam`]: crate::ecs::core::system::system_param::SystemParam
    pub fn run_closure_once<P, F, O>(&mut self, body: F) -> O
    where
        P: SystemParam + 'static,
        F: for<'w, 's> FnMut(<P as SystemParam>::Item<'w, 's>) -> O + Send + Sync + 'static,
        O: 'static,
    {
        let mut sys = FnOnceSystem::<P, F, O>::new(body);
        self.run_system_once(&mut sys)
    }

    // ── Resources facade (Phase 8a Step 9) ───────────────────────────────────

    /// Inserts (or replaces) the world-global resource of type `R`.
    ///
    /// Cold path. Forwards to [`Resources::insert`]; see its docs for the
    /// clear-bit-first replace protocol (R4) that guards against panic-in-drop
    /// UB on the old value.
    ///
    /// [`Resources::insert`]: crate::ecs::core::resources::resources::Resources::insert
    #[cold]
    pub fn insert_resource<R: Resource>(&mut self, value: R) {
        self.resources.insert(value);
    }

    /// Removes the resource of type `R` from the world, returning the typed
    /// value if it was present.
    ///
    /// Cold path. Forwards to [`Resources::remove`]; see invariant R5 for the
    /// clear-bit-before-`Box::from_raw` ordering.
    ///
    /// [`Resources::remove`]: crate::ecs::core::resources::resources::Resources::remove
    #[cold]
    pub fn remove_resource<R: Resource>(&mut self) -> Option<R> {
        self.resources.remove::<R>()
    }

    /// Returns `true` iff the world currently holds a resource of type `R`.
    #[inline]
    pub fn contains_resource<R: Resource>(&self) -> bool {
        self.resources.contains::<R>()
    }

    /// Returns a shared reference to the resource of type `R`.
    ///
    /// # Panics
    ///
    /// Panics if no resource of type `R` has been inserted. Use
    /// [`try_resource`] for the non-panicking variant.
    ///
    /// [`try_resource`]: EcsMaster::try_resource
    #[inline]
    pub fn resource<R: Resource>(&self) -> &R {
        match self.resources.get_ptr::<R>() {
            Some(ptr) => {
                // SAFETY (R2): `get_ptr` returned `Some` ⇒ the slot is populated
                //   and the bytes at `ptr` form a valid `R` (the slot was
                //   inserted via `insert_resource::<R>` with this same TypeId
                //   binding; the cached `ResourceId` in the registry guarantees
                //   the type tag). The lifetime of the returned reference is
                //   tied to `&self`, so the pointer cannot outlive the borrow.
                unsafe { &*ptr }
            }
            None => missing_resource_panic_facade::<R>(),
        }
    }

    /// Returns an exclusive reference to the resource of type `R`.
    ///
    /// # Panics
    ///
    /// Panics if no resource of type `R` has been inserted. Use
    /// [`try_resource_mut`] for the non-panicking variant.
    ///
    /// [`try_resource_mut`]: EcsMaster::try_resource_mut
    #[inline]
    pub fn resource_mut<R: Resource>(&mut self) -> &mut R {
        match self.resources.get_mut_ptr::<R>() {
            Some(ptr) => {
                // SAFETY (R2, R4): `get_mut_ptr` returned `Some` ⇒ the slot is
                //   populated and the bytes at `ptr` form a valid `R`. `&mut
                //   self` gives exclusive access to the resources slab, so the
                //   `&mut R` produced here cannot alias any other reference
                //   into the same slot for the duration of the borrow.
                unsafe { &mut *ptr }
            }
            None => missing_resource_panic_facade::<R>(),
        }
    }

    /// Returns a shared reference to the resource of type `R`, or `None` if
    /// the resource has not been inserted. Non-panicking counterpart of
    /// [`resource`].
    ///
    /// [`resource`]: EcsMaster::resource
    #[inline]
    pub fn try_resource<R: Resource>(&self) -> Option<&R> {
        // SAFETY (R2): same as `resource` — `get_ptr` returns `Some` only when
        //   the slot is populated and holds a valid `R`. Lifetime is tied to
        //   `&self`.
        self.resources.get_ptr::<R>().map(|p| unsafe { &*p })
    }

    /// Returns an exclusive reference to the resource of type `R`, or `None`
    /// if the resource has not been inserted. Non-panicking counterpart of
    /// [`resource_mut`].
    ///
    /// [`resource_mut`]: EcsMaster::resource_mut
    #[inline]
    pub fn try_resource_mut<R: Resource>(&mut self) -> Option<&mut R> {
        // SAFETY (R2, R4): same as `resource_mut` — `get_mut_ptr` returns
        //   `Some` only when the slot is populated and holds a valid `R`.
        //   `&mut self` gives exclusive access for the returned borrow.
        self.resources.get_mut_ptr::<R>().map(|p| unsafe { &mut *p })
    }

    /// Clears all entities and archetypes from the system
    pub fn clear(&mut self) {
        self.entity_master.clear();
        self.archetype_master.clear();
        // Note: We don't clear the arena as it manages its own memory
    }
}

/// Cold-path panic helper for [`EcsMaster::resource`] / [`EcsMaster::resource_mut`].
///
/// Distinct from `params::diagnostics::missing_resource_panic` (which targets
/// the `SystemParam` `get_param` path) — the wording here points at the
/// direct-call API rather than the system runner.
#[cold]
#[inline(never)]
fn missing_resource_panic_facade<R: Resource>() -> ! {
    panic!(
        "Resource `{}` not registered. Call `EcsMaster::insert_resource::<{}>(...)` first.",
        R::debug_type_name(),
        R::debug_type_name()
    );
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

    // --- Phase 8a Step 8: `run_system_once` / `run_closure_once` smoke tests ---

    /// Test resource used by the `run_closure_once` smoke tests. Lives inside
    /// the `tests` module so its `ResourceId` is reserved on first use without
    /// colliding with other test modules.
    struct SystemTestRes(u32);

    impl crate::ecs::core::resources::resource::Resource for SystemTestRes {
        fn resource_id() -> crate::ecs::identifiers::primitives::ResourceId {
            use crate::ecs::core::resources::resource_registry::register_new;
            use crate::ecs::identifiers::primitives::ResourceId;
            use std::sync::OnceLock;
            static ID: OnceLock<ResourceId> = OnceLock::new();
            *ID.get_or_init(|| ResourceId(register_new::<Self>()))
        }
    }

    /// `run_closure_once::<(), _, _>` runs an empty closure and propagates
    /// its return value.
    #[test]
    fn run_system_once_with_empty_closure_runs_once() {
        let mut ecs = EcsMaster::new();
        // W3: turbofish on the param tuple is required in Phase 8a.
        let out: u32 = ecs.run_closure_once::<(), _, _>(|()| 42);
        assert_eq!(out, 42, "run_closure_once must propagate the closure's output");
    }

    /// `run_closure_once::<Res<TestRes>, _, _>` reads back a resource that
    /// was inserted via the `pub(crate)` `resources` field (the public
    /// `insert_resource` facade lands in Step 9).
    #[test]
    fn run_closure_once_with_res_reads_value() {
        use crate::ecs::core::system::Res;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        let mut ecs = EcsMaster::new();
        ecs.resources.insert(SystemTestRes(123));

        // The closure capture must be `Send + Sync` because the `System`
        // trait bound transitively requires it on the closure. `AtomicU32`
        // behind `Arc` satisfies the bound and serves as a probe channel
        // from inside the closure back to the outer test.
        let observed = Arc::new(AtomicU32::new(0));
        let probe = Arc::clone(&observed);
        // W3: turbofish on the param tuple is required in Phase 8a.
        // r: Res<SystemTestRes>; r.0: &SystemTestRes; r.0.0: u32 (the inner
        // newtype field, accessed via auto-deref through the shared borrow).
        ecs.run_closure_once::<Res<'_, SystemTestRes>, _, _>(move |r| {
            probe.store(r.0.0, Ordering::Relaxed);
        });
        assert_eq!(
            observed.load(Ordering::Relaxed),
            123,
            "Res<R> must round-trip the inserted value"
        );
    }
}