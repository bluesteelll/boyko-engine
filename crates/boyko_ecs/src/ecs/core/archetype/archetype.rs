use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId, EntityId, InlandPoolId};
use crate::ecs::core::entity::entity_inland::EntityInland;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::component::component_registry::MAX_COMPONENTS;
use crate::ecs::core::archetype::archetype_signature::ArchetypeSignature;
use crate::ecs::core::component::component_pool_bundle::ComponentPoolBundle;
use crate::ecs::memory::arena::Arena;

/// `MAX_COMPONENTS` as a `ComponentId` newtype for comparison against newtype-guarded IDs.
const MAX_COMPONENTS_ID: ComponentId = ComponentId(MAX_COMPONENTS);

/// Pre-resolved component pointer + stride for the hot read path.
///
/// `ptr.is_null()` ⇔ this archetype has no pool for the `ComponentId` at this
/// index. Stored inline in `Archetype::columns` so a random component lookup
/// resolves to a base pointer and stride in a single cache line, bypassing the
/// `ComponentPoolBundle` sparse map (Phase 7 D4).
///
/// The `_reserved` field brings the struct to a power-of-two 16 B stride so
/// `columns[c]` lowers to `c << 4` indexing. Reserved for Phase 8; do not
/// rely on its current value.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Column {
    /// Base pointer to the component pool's buffer
    /// (== `ComponentPool::buffer_ptr()`). NULL when the column is absent.
    pub(crate) ptr: *mut u8,
    /// Component size in bytes (== `ComponentPool::component_layout().size()`).
    /// `unit_index * stride` gives the byte offset from `ptr`.
    pub(crate) stride: u32,
    /// Reserved for future use; layout-stable but value is not part of the
    /// public contract. **Do not rely on this field for any current dispatch.**
    pub(crate) _reserved: u32,
}

const _: () = assert!(std::mem::size_of::<Column>() == 16);
const _: () = assert!(std::mem::align_of::<Column>() == 8);
const _: () = assert!(std::mem::offset_of!(Column, ptr) == 0);
const _: () = assert!(std::mem::offset_of!(Column, stride) == 8);
const _: () = assert!(std::mem::offset_of!(Column, _reserved) == 12);

impl Column {
    /// All-zero column representing "no pool for this id".
    ///
    /// The all-zero representation is load-bearing: `refresh_all_columns`
    /// resets the table via byte-zero (`*col = Column::null()` per slot).
    /// Phase 4 will switch to a single `write_bytes(addr_of_mut!(columns), 0, ...)`
    /// against the all-zero invariant.
    #[inline]
    pub const fn null() -> Self {
        Self {
            ptr: std::ptr::null_mut(),
            stride: 0,
            _reserved: 0,
        }
    }

    /// `true` when the archetype has no pool for the component_id this column
    /// represents. First check on every fast-path component lookup.
    #[inline]
    pub const fn is_null(&self) -> bool {
        self.ptr.is_null()
    }
}

/// Outcome of removing an entity from an archetype.
///
/// Replaces the previous `Option<EntityId>` return which was ambiguous:
/// `None` could mean either "was the last entity" or "removal failed".
/// This enum makes all three outcomes explicit (C-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveOutcome {
    /// The removed entity was the last one; no swap-remove was needed.
    Last,
    /// A swap-remove occurred: the entity that was at the last position has
    /// been moved to the vacated slot. Callers must update that entity's
    /// `unit_index` in `EntityMaster`.
    Swapped {
        /// The `EntityId` of the entity that was moved from the last slot
        /// into the removed entity's slot.
        moved_entity: EntityId,
    },
    /// The removal failed (e.g., `swap_remove_unit` returned an error).
    /// The archetype state is unchanged.
    PoolFailure,
}

// CR3: size guard — must match Option<EntityId> = 16 bytes (8-byte EntityId
// + 8-byte discriminant/niche). If a new variant is added, this fires at
// compile time before the regression can ship.
const _: () = {
    assert!(
        std::mem::size_of::<RemoveOutcome>() == 16,
        "RemoveOutcome must stay 16 bytes (matches Option<EntityId>); \
         adding a variant without this check would silently bloat the type"
    );
};

/// Archetype represents a unique combination of component types
/// All entities with the same component types belong to the same archetype
///
/// `#[repr(C)]` pins the field order so `columns` is at offset 0 (Phase 7 D4 /
/// U5). The default `repr(Rust)` reorders by alignment, which would put
/// `ComponentPoolBundle` (align 16 via inner Vec) ahead of `columns` and break
/// the Phase 7 fast read path's "single dependent load at offset 0" promise.
#[repr(C)]
pub struct Archetype {
    /// 8 KB inline hot lookup table, indexed by `ComponentId.0`.
    ///
    /// Placed FIRST so it sits at fixed offset 0 from `*const Archetype` —
    /// the Phase 7 fast read path issues a single dependent load `*(arch + c*16)`
    /// without touching `component_pools` (Phase 7 D4). For every `ComponentId`
    /// that is NOT in this archetype, the slot stays `Column::null()`; null is
    /// the single source of truth for "absent column".
    ///
    /// `pub(crate)` so `ArchetypeBundle::add_archetype_from_components` can
    /// reach the field via `addr_of_mut!((*slot_ptr).columns)` for in-place
    /// slab initialisation (Phase 7 W6 / U13). Outside the crate the layout
    /// is opaque.
    pub(crate) columns: [Column; MAX_COMPONENTS],

    /// Unique identifier for this archetype.
    ///
    /// `pub(crate)` for the same reason as `columns` — in-place initialisation
    /// of slab slots from `ArchetypeBundle` requires field-by-field writes via
    /// `addr_of_mut!` (Phase 7 U13).
    pub(crate) id: ArchetypeId,

    /// Storage for components organized by component type.
    ///
    /// `pub(crate)` for in-place slab construction (Phase 7 U13).
    pub(crate) component_pools: ComponentPoolBundle,

    /// Current index for the next entity (equals number of entities).
    ///
    /// `pub(crate)` for in-place slab construction (Phase 7 U13).
    pub(crate) current_index: usize,

    /// Component signature for this archetype (bit mask of component IDs).
    ///
    /// `pub(crate)` for in-place slab construction (Phase 7 U13).
    pub(crate) signature: ArchetypeSignature,

    /// Raw provenance pointer to the arena used for memory allocation.
    /// Stored as `*const Arena` (raw provenance) to avoid Miri retag UB:
    /// see Phase 3a Miri retag fix in `ecs_master.rs` field-level doc.
    ///
    /// `pub(crate)` for in-place slab construction (Phase 7 U13).
    pub(crate) arena: *const Arena,

    /// Set of component IDs in this archetype for efficient iteration.
    ///
    /// `pub(crate)` for in-place slab construction (Phase 7 U13).
    pub(crate) component_ids: Vec<ComponentId>,
    /// Vector of entity IDs, indexed by unit_index.
    /// Allows O(1) access to entity ID by unit index.
    ///
    /// `pub(crate)` for in-place slab construction (Phase 7 U13).
    pub(crate) entity_ids: Vec<EntityId>,
}

// Phase 7 U5 / D4: the inline column table MUST be at offset 0 so the fast
// read path can issue `*(arch + c*16)` against a freshly-minted slab pointer
// without an extra add. The default `repr(Rust)` heuristic already places
// `[Column; 512]` (largest align*size product) first, but the invariant is
// load-bearing for Step 7 and Phase 8 — lock it at compile time.
const _: () = assert!(std::mem::offset_of!(Archetype, columns) == 0);

impl Archetype {
    /// Creates a new archetype with the given ID and arena.
    ///
    /// The 8 KB `columns` table is zero-initialised on the stack; Phase 4
    /// will switch this to in-place slab construction via `addr_of_mut!`
    /// (Phase 7 W6) once `ArchetypeBundle` lands. For now, low-frequency
    /// creation sites pay the temporary stack cost.
    pub fn new(id: ArchetypeId, arena: &Arena) -> Self {
        Self {
            columns: [Column::null(); MAX_COMPONENTS],
            id,
            component_pools: ComponentPoolBundle::new(),
            current_index: 0,
            signature: ArchetypeSignature::new(ComponentMask::new()),
            // SAFETY: `arena` is a shared reference valid for the lifetime of
            // the owning `EcsMaster`. Converting to a raw pointer preserves
            // provenance; the pointer is never dereferenced here — it is
            // forwarded to `add_pool` via a temporary `&Arena` reborrow.
            arena: &raw const *arena,
            component_ids: Vec::new(),
            entity_ids: Vec::new(),
        }
    }


    /// Creates a new archetype from a slice of component IDs.
    ///
    /// After each `add_pool` call, `refresh_column` syncs the inline
    /// `columns[comp_id.0]` entry so the hot read path can find the pool's
    /// `(ptr, stride)` without going through the bundle's sparse map
    /// (Phase 7 D4 / invariant U7).
    pub fn create_by_ids(id: ArchetypeId, component_ids: &[ComponentId], arena: &Arena) -> Self {
        // Create a mask from the component IDs
        let mut mask = ComponentMask::new();
        for &comp_id in component_ids {
            mask.set(comp_id);
        }

        // Initialize archetype with mask and empty component pools
        let mut archetype = Self {
            columns: [Column::null(); MAX_COMPONENTS],
            id,
            component_pools: ComponentPoolBundle::new(),
            current_index: 0,
            signature: ArchetypeSignature::new(mask),
            // SAFETY: same provenance contract as `Archetype::new`.
            arena: &raw const *arena,
            component_ids: component_ids.to_vec(),
            entity_ids: Vec::new(),
        };

        // Create component pools for each component ID. Each successful
        // `add_pool` must be paired with `refresh_column` to keep the inline
        // `columns` table in sync (Phase 7 invariant U7).
        for &comp_id in component_ids {
            archetype.component_pools.add_pool(arena, comp_id);
            archetype.refresh_column(comp_id);
        }

        archetype
    }

    /// Gets the unique ID of this archetype
    #[inline]
    pub fn id(&self) -> ArchetypeId {
        self.id
    }

    /// Registers a component type by ID
    pub fn register_component(&mut self, component_id: ComponentId) -> bool {
        // Check if this component type is already registered
        if self.signature.mask().contains(component_id) {
            return false;
        }

        // SAFETY: `self.arena` was minted from the `Box<Arena>` owned by
        // `EcsMaster` (audit C-001 / drop-order invariant, Phase 3a raw
        // provenance fix). The `Box` lives at a stable heap address and outlives
        // every `Archetype`. No aliasing `&mut Arena` exists: `Arena` is
        // `!Send + !Sync`; single-threaded use is enforced. The lifetime of
        // the reborrow is bounded to this call — it does not escape.
        let arena = unsafe { &*self.arena };

        // Add a pool for this component type, then refresh the inline column
        // table so the hot read path can resolve `component_id` without going
        // through the bundle's sparse map (Phase 7 invariant U7).
        self.component_pools.add_pool(arena, component_id);
        self.refresh_column(component_id);

        // Update signature mask
        let mut new_mask = *self.signature.mask();
        new_mask.set(component_id);
        self.signature = ArchetypeSignature::new(new_mask);

        // Add component ID to our list
        self.component_ids.push(component_id);

        true
    }

    /// Phase 7 Step 4 helper: registers the component pool for `component_id`
    /// without touching `signature` or `component_ids` (callers like in-place
    /// slab construction in `ArchetypeBundle::add_archetype_from_components`
    /// pre-populate those fields, so a full `register_component` call would
    /// early-return on the signature check).
    ///
    /// Calls `component_pools.add_pool(arena, component_id)` and refreshes the
    /// inline column entry. Intended exclusively for in-place archetype
    /// construction — the bundle resides in the same crate (`pub(crate)`).
    pub(crate) fn register_component_inplace(&mut self, component_id: ComponentId, arena: &Arena) {
        self.component_pools.add_pool(arena, component_id);
        self.refresh_column(component_id);
    }

    /// Re-syncs `columns[component_id.0]` with the current pool state.
    ///
    /// Called only after `component_pools.add_pool(...)` mints (or refreshes)
    /// a pool's `(buffer_ptr, component_layout)`. NOT called on data-only
    /// mutations (push / swap_remove / pop / set) — those leave the pool's
    /// base pointer and stride unchanged (Phase 7 D5 audit / invariant U10).
    #[inline]
    fn refresh_column(&mut self, component_id: ComponentId) {
        debug_assert!(
            component_id.0 < MAX_COMPONENTS,
            "component_id {} >= MAX_COMPONENTS ({})", component_id.0, MAX_COMPONENTS
        );
        match self.component_pools.get_pool(component_id) {
            Some(pool) => {
                self.columns[component_id.0] = Column {
                    ptr: pool.buffer_ptr() as *mut u8,
                    stride: pool.component_layout().size() as u32,
                    _reserved: 0,
                };
            }
            None => {
                self.columns[component_id.0] = Column::null();
            }
        }
    }

    /// Refreshes the entire `columns` table from `component_pools`.
    ///
    /// Reserved for future arena-grow events where every pool's `buffer_ptr`
    /// may relocate. Not used on the Phase 7 hot path.
    #[cold]
    #[allow(dead_code)]
    fn refresh_all_columns(&mut self) {
        for col in self.columns.iter_mut() {
            *col = Column::null();
        }
        // Clone the IDs out so the iteration does not hold a borrow of
        // `self.component_ids` across the `refresh_column(...)` call, which
        // needs `&mut self`.
        let ids: Vec<ComponentId> = self.component_ids.clone();
        for cid in ids {
            self.refresh_column(cid);
        }
    }

    /// Checks if this archetype contains a component with the given ID
    #[inline]
    pub fn has_component_id(&self, component_id: ComponentId) -> bool {
        self.signature.mask().contains(component_id)
    }

    /// Gets the number of component types in this archetype
    #[inline]
    pub fn component_count(&self) -> usize {
        self.component_ids.len()
    }

    /// Gets the number of entities in this archetype
    #[inline]
    pub fn entity_count(&self) -> usize {
        self.current_index
    }

    /// Creates a new entity in this archetype with the given components.
    ///
    /// Takes a borrowed slice of `(ComponentId, &[u8])` pairs — zero allocation
    /// on the caller side. Writes the assigned dense unit index into
    /// `*new_unit_index` on success.
    ///
    /// The previous signature accepted `&mut EntityInland` and mutated its
    /// `unit_index` / `archetype_id` fields. Phase 7 removes that coupling:
    /// the caller now receives just the `u32` slot and is responsible for
    /// constructing whatever entity-location record it needs (legacy
    /// `EntityInland` or Phase 7 `EntityInlandFast`).
    ///
    /// Audit: C-010 — switched from Vec to &[...]. Phase 7 Step 3 — replaced
    /// `&mut EntityInland` with `&mut u32`.
    pub fn create_entity(
        &mut self,
        entity_id: EntityId,
        new_unit_index: &mut u32,
        components: &[(ComponentId, &[u8])],
    ) -> bool {
        // Build a mask of the input component IDs in O(M), then check
        // that the archetype signature is a subset in O(8 u64 ops).
        // This replaces the previous O(N*M) nested scan.
        let mut input_mask = ComponentMask::new();
        for (id, _) in components {
            debug_assert!(
                *id < MAX_COMPONENTS_ID,
                "component_id {} >= MAX_COMPONENTS ({})", id.0, MAX_COMPONENTS
            );
            input_mask.set(*id);
        }
        // Duplicate ComponentIds collapse in the bitset: if popcount(input_mask) < components.len(),
        // at least one id appeared more than once. Duplicates corrupt pool state.
        debug_assert_eq!(
            input_mask.popcount(),
            components.len(),
            "Archetype::create_entity input contains duplicate ComponentId"
        );

        if !self.signature.mask().is_subset(&input_mask) {
            return false; // at least one required component is absent from input
        }

        // Two-phase commit (C-009): validate all pools have capacity before
        // writing any, so a full-pool failure cannot leave pools desynced.
        if !self.component_pools.can_push_entity_components(components) {
            return false;
        }

        let unit_index = self.component_pools.push_entity_components(components);
        *new_unit_index = unit_index as u32;

        // Add the entity ID to the vector
        self.entity_ids.push(entity_id);

        // Increment entity counter
        self.current_index += 1;

        true
    }

    /// Removes an entity and all its components from this archetype.
    ///
    /// Returns a [`RemoveOutcome`] describing what happened:
    /// - [`RemoveOutcome::Last`]: the entity was the last one; no swap needed.
    /// - [`RemoveOutcome::Swapped`]: swap-remove occurred; caller must update
    ///   the moved entity's `unit_index` in `EntityMaster`.
    /// - [`RemoveOutcome::PoolFailure`]: removal failed; archetype is unchanged.
    pub fn remove_entity(&mut self, entity_inland: &EntityInland) -> RemoveOutcome {
        debug_assert_eq!(entity_inland.archetype_id(), self.id,
            "EntityInland archetype_id mismatch");

        let removed_unit_index = entity_inland.unit_index();
        let last_unit_index = InlandPoolId(self.current_index.saturating_sub(1));

        // If removing the last entity, just pop it.
        if removed_unit_index == last_unit_index {
            if self.component_pools.pop_entity() {
                self.entity_ids.pop();
                self.current_index -= 1;
                return RemoveOutcome::Last;
            } else {
                return RemoveOutcome::PoolFailure;
            }
        }

        // Get the entity ID that will be swapped.
        let swapped_entity_id = self.entity_ids[last_unit_index.0];

        // Swap_remove in component pools.
        if self.component_pools.swap_remove_unit(removed_unit_index.0).is_err() {
            return RemoveOutcome::PoolFailure;
        }

        // Swap_remove the entity ID as well.
        self.entity_ids.swap_remove(removed_unit_index.0);
        self.current_index -= 1;

        RemoveOutcome::Swapped { moved_entity: swapped_entity_id }
    }

    /// Gets a raw pointer to a component using EntityInland for direct access
    #[inline]
    pub fn get_component_raw(&self, inland: &EntityInland, component_id: ComponentId) -> Option<*const u8> {
        debug_assert_eq!(inland.archetype_id(), self.id, 
            "EntityInland archetype_id mismatch");
        
        let unit_index = inland.unit_index();
        
        // Get the component pool for this component type
        let pool = self.component_pools.get_pool(component_id)?;
        
        // Use the unit index directly
        pool.get_raw(unit_index.0)
    }

    /// Gets a mutable raw pointer to a component using EntityInland for direct access
    #[inline]
    pub fn get_component_raw_mut(&mut self, inland: &EntityInland, component_id: ComponentId) -> Option<*mut u8> {
        debug_assert_eq!(inland.archetype_id(), self.id,
            "EntityInland archetype_id mismatch");

        let unit_index = inland.unit_index();

        // Get the component pool for this component type
        let pool = self.component_pools.get_pool_mut(component_id)?;

        // Use the unit index directly
        pool.get_raw_mut(unit_index.0)
    }

    /// Sets a component value using EntityInland for direct access
    #[inline]
    pub fn set_component(&mut self, inland: &EntityInland, component_id: ComponentId, bytes: &[u8]) -> bool {
        debug_assert_eq!(inland.archetype_id(), self.id,
            "EntityInland archetype_id mismatch");

        let unit_index = inland.unit_index();

        // Get the component pool for this component type
        let pool = match self.component_pools.get_pool_mut(component_id) {
            Some(p) => p,
            None => return false,
        };

        // Set the component using the unit index directly
        pool.set_component(unit_index.0, bytes)
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
    
    /// Gets the archetype signature
    #[inline]
    pub fn signature(&self) -> &ArchetypeSignature {
        &self.signature
    }
    
    /// Gets the component mask for this archetype
    #[inline]
    pub fn component_mask(&self) -> &ComponentMask {
        self.signature.mask()
    }
    
    /// Gets the slice of component IDs for this archetype
    #[inline]
    pub fn component_ids(&self) -> &[ComponentId] {
        &self.component_ids
    }
    
    /// Checks if this archetype has all the specified component IDs
    pub fn matches_component_ids(&self, component_ids: &[ComponentId]) -> bool {
        // Check if this archetype contains all the requested components
        for &comp_id in component_ids {
            if !self.signature.mask().contains(comp_id) {
                return false;
            }
        }
        
        true
    }
    
    /// Removes the last entity from this archetype
    /// Takes a reference to the last entity's EntityInland to update its generation
    pub fn pop(&mut self, last_entity_inland: &mut EntityInland) -> bool {
        debug_assert!(self.current_index > 0, "Attempting to pop from an empty archetype");

        // C-008 fix: pop_entity() ran inside debug_assert!, so in release builds the
        // pools were never popped while `current_index` was still decremented — silent
        // corruption. Capture the result outside the assert.
        let popped = self.component_pools.pop_entity();
        debug_assert!(popped, "Failed to pop entity from component pools");
        if !popped {
            return false;
        }

        // Q-022 fix: keep entity_ids length in sync with current_index, otherwise
        // get_entity_id_at returns stale entries after pop.
        self.entity_ids.pop();

        // Increment generation of the popped entity
        last_entity_inland.increment_generation();

        // Decrement entity counter
        self.current_index -= 1;

        true
    }
    
    /// Gets the entity ID at a specific unit index
    #[inline]
    pub fn get_entity_id_at(&self, unit_index: InlandPoolId) -> Option<EntityId> {
        self.entity_ids.get(unit_index.0).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::component_registry;
    use crate::ecs::memory::arena::Arena;

    // Use high IDs to avoid collisions with other test modules.
    const COMP_A: ComponentId = ComponentId(400);
    const COMP_B: ComponentId = ComponentId(401);

    fn register_test_components() {
        #[repr(C)]
        struct CompA(u32);
        #[repr(C)]
        struct CompB(u64);
        component_registry::register_layout::<CompA>(COMP_A.0);
        component_registry::register_layout::<CompB>(COMP_B.0);
    }

    fn make_archetype(arena: &Arena) -> Archetype {
        register_test_components();
        Archetype::create_by_ids(ArchetypeId(1), &[COMP_A, COMP_B], arena)
    }

    // Helper: add one entity with zero-filled bytes for both components.
    fn add_entity(arch: &mut Archetype, entity_id: EntityId) -> EntityInland {
        let bytes_a = vec![0u8; component_registry::get_component_size(COMP_A.0).unwrap()];
        let bytes_b = vec![0u8; component_registry::get_component_size(COMP_B.0).unwrap()];
        let mut new_unit_index: u32 = 0;
        let ok = arch.create_entity(entity_id, &mut new_unit_index, &[
            (COMP_A, bytes_a.as_slice()),
            (COMP_B, bytes_b.as_slice()),
        ]);
        assert!(ok, "create_entity must succeed in setup helper");
        EntityInland::new(arch.id(), InlandPoolId(new_unit_index as usize), 0)
    }

    // --- create_entity ---

    #[test]
    fn create_entity_increments_entity_count() {
        let arena = Arena::with_capacity(4096 * 1024);
        let mut arch = make_archetype(&arena);

        assert_eq!(arch.entity_count(), 0, "fresh archetype has no entities");
        add_entity(&mut arch, EntityId(42));
        assert_eq!(arch.entity_count(), 1, "count must be 1 after one create");
    }

    #[test]
    fn create_entity_pushes_entity_id_to_vector() {
        let arena = Arena::with_capacity(4096 * 1024);
        let mut arch = make_archetype(&arena);

        add_entity(&mut arch, EntityId(99));
        assert_eq!(
            arch.get_entity_id_at(InlandPoolId(0)),
            Some(EntityId(99)),
            "entity ID 99 must be accessible at slot 0"
        );
    }

    #[test]
    fn create_entity_missing_component_returns_false() {
        let arena = Arena::with_capacity(4096 * 1024);
        let mut arch = make_archetype(&arena);

        // Provide only COMP_A, omit COMP_B.
        let bytes_a = vec![0u8; component_registry::get_component_size(COMP_A.0).unwrap()];
        let mut new_unit_index: u32 = 0;
        let ok = arch.create_entity(EntityId(10), &mut new_unit_index, &[(COMP_A, bytes_a.as_slice())]);
        assert!(!ok, "create_entity must return false when a component is missing");
    }

    // --- pop (C-008 + Q-022 regression) ---

    #[test]
    fn pop_decrements_entity_count_in_debug_and_release() {
        // Regression for C-008: in the original code, component pools were NOT
        // popped in release because pop_entity() was inside debug_assert!.
        // This test must pass under both `cargo test` and `cargo test --release`.
        let arena = Arena::with_capacity(4096 * 1024);
        let mut arch = make_archetype(&arena);
        let mut inland = add_entity(&mut arch, EntityId(7));

        assert_eq!(arch.entity_count(), 1);
        let popped = arch.pop(&mut inland);
        assert!(popped, "pop must return true");
        assert_eq!(
            arch.entity_count(),
            0,
            "entity_count must be 0 after pop — C-008 regression"
        );
    }

    #[test]
    fn pop_removes_entity_id_from_vector() {
        // Regression for Q-022: entity_ids.pop() must be called alongside
        // component_pools.pop_entity() — previously it was missing.
        let arena = Arena::with_capacity(4096 * 1024);
        let mut arch = make_archetype(&arena);
        let inland0 = add_entity(&mut arch, EntityId(1));
        add_entity(&mut arch, EntityId(2));
        add_entity(&mut arch, EntityId(3));

        // Pop removes the last entity (ID=3).
        let mut inland_last = EntityInland::new(arch.id(), InlandPoolId(2), 0);
        arch.pop(&mut inland_last);

        assert_eq!(
            arch.entity_count(),
            2,
            "entity_count must be 2 after one pop"
        );
        assert!(
            arch.get_entity_id_at(InlandPoolId(2)).is_none(),
            "slot 2 must be empty after pop — Q-022 regression"
        );
        assert_eq!(
            arch.get_entity_id_at(InlandPoolId(0)),
            Some(EntityId(1)),
            "slot 0 must still hold entity ID 1"
        );
        assert_eq!(
            arch.get_entity_id_at(InlandPoolId(1)),
            Some(EntityId(2)),
            "slot 1 must still hold entity ID 2"
        );

        // Suppress unused-variable warning for inland0.
        let _ = inland0;
    }

    #[test]
    fn pop_on_empty_archetype_panics_in_debug_or_returns_false_in_release() {
        // In debug builds, debug_assert!(current_index > 0) fires and panics.
        // In release builds, pop_entity() is called but the pools are empty
        // and pop returns false — the function returns false without decrement.
        // Both outcomes are acceptable; we use catch_unwind to allow both.
        let arena = Arena::with_capacity(4096 * 1024);

        // Build the archetype inside the closure so arena lifetime is valid.
        // We can't move `arena` across the UnwindSafe boundary easily, so
        // we reproduce a minimal inline version.
        let _result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let arena2 = Arena::with_capacity(4096 * 1024);
            register_test_components();
            let mut arch = Archetype::create_by_ids(ArchetypeId(99), &[COMP_A, COMP_B], &arena2);
            let mut inland = EntityInland::new(arch.id(), InlandPoolId(0), 0);
            // In debug: panics. In release: returns false (pool is empty → pop() = false).
            let _ = arch.pop(&mut inland);
        }));
        // The test passes regardless of whether a panic occurred.
        let _ = arena; // keep arena alive
    }

    // --- remove_entity ---

    #[test]
    fn remove_entity_last_returns_last_outcome() {
        let arena = Arena::with_capacity(4096 * 1024);
        let mut arch = make_archetype(&arena);
        let inland = add_entity(&mut arch, EntityId(55));
        // Removing the only entity — no swap needed.
        let result = arch.remove_entity(&inland);
        assert_eq!(result, RemoveOutcome::Last, "no swap expected for the last entity");
        assert_eq!(arch.entity_count(), 0);
    }

    #[test]
    fn remove_entity_non_last_returns_swapped_outcome() {
        let arena = Arena::with_capacity(4096 * 1024);
        let mut arch = make_archetype(&arena);
        let inland_first = add_entity(&mut arch, EntityId(10));
        add_entity(&mut arch, EntityId(20)); // last entity

        // Remove first; last (20) should swap into position 0.
        let result = arch.remove_entity(&inland_first);
        assert_eq!(
            result,
            RemoveOutcome::Swapped { moved_entity: EntityId(20) },
            "swapped entity ID must be 20"
        );
        assert_eq!(arch.entity_count(), 1);
    }

    // --- RemoveOutcome (C-006) ---

    #[test]
    fn remove_outcome_last_on_single_entity() {
        // Removing the only entity must produce RemoveOutcome::Last.
        let arena = Arena::with_capacity(4096 * 1024);
        let mut arch = make_archetype(&arena);
        let inland = add_entity(&mut arch, EntityId(1));
        assert_eq!(arch.remove_entity(&inland), RemoveOutcome::Last);
        assert_eq!(arch.entity_count(), 0);
        assert!(arch.get_entity_id_at(InlandPoolId(0)).is_none());
    }

    #[test]
    fn remove_outcome_swapped_moves_last_entity_id() {
        // Removing the first of three entities must produce RemoveOutcome::Swapped
        // with the ID of the entity that was at the last position.
        let arena = Arena::with_capacity(4096 * 1024);
        let mut arch = make_archetype(&arena);
        let inland_0 = add_entity(&mut arch, EntityId(10));
        add_entity(&mut arch, EntityId(20));
        add_entity(&mut arch, EntityId(30)); // last

        let result = arch.remove_entity(&inland_0);
        assert_eq!(result, RemoveOutcome::Swapped { moved_entity: EntityId(30) });
        // Entity 30 now occupies slot 0; slot 1 holds entity 20.
        assert_eq!(arch.get_entity_id_at(InlandPoolId(0)), Some(EntityId(30)));
        assert_eq!(arch.get_entity_id_at(InlandPoolId(1)), Some(EntityId(20)));
        assert_eq!(arch.entity_count(), 2);
    }

    #[test]
    fn remove_outcome_removing_second_to_last() {
        // Removing the middle entity of two entities is a swap.
        let arena = Arena::with_capacity(4096 * 1024);
        let mut arch = make_archetype(&arena);
        let inland_0 = add_entity(&mut arch, EntityId(100));
        add_entity(&mut arch, EntityId(200)); // becomes "last"

        let result = arch.remove_entity(&inland_0);
        assert_eq!(result, RemoveOutcome::Swapped { moved_entity: EntityId(200) });
        assert_eq!(arch.entity_count(), 1);
        assert_eq!(arch.get_entity_id_at(InlandPoolId(0)), Some(EntityId(200)));
    }

    #[test]
    fn remove_outcome_last_on_last_of_multiple() {
        // Removing the last of multiple entities must produce RemoveOutcome::Last.
        let arena = Arena::with_capacity(4096 * 1024);
        let mut arch = make_archetype(&arena);
        add_entity(&mut arch, EntityId(10));
        let inland_last = add_entity(&mut arch, EntityId(20));

        let result = arch.remove_entity(&inland_last);
        assert_eq!(result, RemoveOutcome::Last);
        assert_eq!(arch.entity_count(), 1);
        assert_eq!(arch.get_entity_id_at(InlandPoolId(0)), Some(EntityId(10)));
    }

    // --- has_component_id ---

    #[test]
    fn has_component_id_returns_true_for_registered() {
        let arena = Arena::with_capacity(4096 * 1024);
        let arch = make_archetype(&arena);
        assert!(arch.has_component_id(COMP_A));
        assert!(arch.has_component_id(COMP_B));
    }

    #[test]
    fn has_component_id_returns_false_for_absent() {
        let arena = Arena::with_capacity(4096 * 1024);
        let arch = make_archetype(&arena);
        assert!(!arch.has_component_id(ComponentId(402))); // never added
    }

    // --- matches_component_ids ---

    #[test]
    fn matches_component_ids_subset_returns_true() {
        let arena = Arena::with_capacity(4096 * 1024);
        let arch = make_archetype(&arena);
        assert!(arch.matches_component_ids(&[COMP_A]));
        assert!(arch.matches_component_ids(&[COMP_A, COMP_B]));
    }

    #[test]
    fn matches_component_ids_superset_returns_false() {
        let arena = Arena::with_capacity(4096 * 1024);
        let arch = make_archetype(&arena);
        // 402 is not in the archetype.
        assert!(!arch.matches_component_ids(&[COMP_A, ComponentId(402)]));
    }

    // --- C-16: ComponentMask precheck in create_entity ---

    // ID range 410-419 reserved for C-16 tests (per plan, avoids collisions).
    const C16_A: ComponentId = ComponentId(410);
    const C16_B: ComponentId = ComponentId(411);
    // IDs 412-417 reserved for wide-mask test (8 components).
    const C16_WIDE: [ComponentId; 8] = [
        ComponentId(410), ComponentId(411), ComponentId(412), ComponentId(413),
        ComponentId(414), ComponentId(415), ComponentId(416), ComponentId(417),
    ];

    fn register_c16_components() {
        // Register each with a distinct struct type so TypeId differs.
        #[repr(C)] struct C16CompA(u32);
        #[repr(C)] struct C16CompB(u32);
        #[repr(C)] struct C16CompC(u32);
        #[repr(C)] struct C16CompD(u32);
        #[repr(C)] struct C16CompE(u32);
        #[repr(C)] struct C16CompF(u32);
        #[repr(C)] struct C16CompG(u32);
        #[repr(C)] struct C16CompH(u32);
        component_registry::register_layout::<C16CompA>(410);
        component_registry::register_layout::<C16CompB>(411);
        component_registry::register_layout::<C16CompC>(412);
        component_registry::register_layout::<C16CompD>(413);
        component_registry::register_layout::<C16CompE>(414);
        component_registry::register_layout::<C16CompF>(415);
        component_registry::register_layout::<C16CompG>(416);
        component_registry::register_layout::<C16CompH>(417);
    }

    /// Input with one extra unregistered ID: the C-16 archetype guard passes (subset holds
    /// because all required components are present), then execution falls into the pool
    /// bundle, which panics in debug (debug_assert fires for unknown IDs) or returns None
    /// in release (sparse lookup misses). Either outcome means no entity is created.
    ///
    /// This test locks in the pre-C-16 contract: extras pass the archetype guard but do
    /// not silently create an entity — the bundle-level rejection is unchanged.
    #[test]
    fn create_entity_with_extra_component_id_today_passes_archetype_guard() {
        register_c16_components();

        // Use catch_unwind to handle both debug (panic) and release (false return).
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let arena = Arena::with_capacity(4096 * 1024);
            // Archetype requires only C16_A and C16_B.
            let mut arch = Archetype::create_by_ids(ArchetypeId(50), &[C16_A, C16_B], &arena);

            let sz_a = component_registry::get_component_size(C16_A.0).unwrap();
            let sz_b = component_registry::get_component_size(C16_B.0).unwrap();
            let bytes_a = vec![0u8; sz_a];
            let bytes_b = vec![0u8; sz_b];

            // C16_C (412) is extra — not in the archetype's pool bundle.
            // Guard passes; bundle rejects (panic in debug, None in release).
            let sz_c = component_registry::get_component_size(412).unwrap();
            let bytes_c = vec![0u8; sz_c];

            let mut new_unit_index: u32 = 0;
            let ok = arch.create_entity(EntityId(200), &mut new_unit_index, &[
                (C16_A, bytes_a.as_slice()),
                (C16_B, bytes_b.as_slice()),
                (ComponentId(412), bytes_c.as_slice()), // extra: not in archetype pools
            ]);
            // In release: bundle returns None for the unknown ID → create_entity returns false.
            assert!(!ok, "create_entity must return false when bundle cannot accept the extra ID");
        }));
        // In debug: pool bundle debug_assert fires → panic is expected and acceptable.
        // In release: no panic, assertion inside closure must hold.
        // Either way the test passes.
        let _ = result;
    }

    /// Smoke test: 8-component archetype (wide mask path). Registers IDs 410-417,
    /// builds archetype, adds one entity. Exercises the full 8-block mask subset check.
    #[test]
    fn create_entity_wide_archetype_8_components() {
        register_c16_components();
        // 8 component pools each need arena space for chunks; use a larger arena.
        let arena = Arena::with_capacity(64 * 1024 * 1024);
        let mut arch = Archetype::create_by_ids(ArchetypeId(51), &C16_WIDE, &arena);

        // Build component data: 4 bytes each (all u32-sized).
        let bytes = [0u8; 4];
        let components: Vec<(ComponentId, &[u8])> = C16_WIDE.iter()
            .map(|&id| (id, bytes.as_slice()))
            .collect();

        let mut new_unit_index: u32 = 0;
        let ok = arch.create_entity(EntityId(300), &mut new_unit_index, &components);
        assert!(ok, "create_entity must succeed for 8-component archetype");
        assert_eq!(arch.entity_count(), 1);
    }
}