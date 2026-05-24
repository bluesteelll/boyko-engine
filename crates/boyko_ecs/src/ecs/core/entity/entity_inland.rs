use core::ptr;

use crate::ecs::identifiers::primitives::{ArchetypeId, InlandPoolId, Generation};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityInland {
    archetype_id: ArchetypeId,
    unit_index: InlandPoolId,
    generation: Generation,
}

impl EntityInland {
    pub fn new(archetype_id: ArchetypeId, unit_index: InlandPoolId, generation: Generation) -> Self {
        Self { archetype_id, unit_index, generation }
    }
    
    #[inline]
    pub fn archetype_id(&self) -> ArchetypeId {
        self.archetype_id
    }
    
    #[inline]
    pub fn unit_index(&self) -> InlandPoolId {
        self.unit_index
    }
    
    #[inline]
    pub fn set_archetype_id(&mut self, archetype_id: ArchetypeId) {
        self.archetype_id = archetype_id;
    }
    
    #[inline]
    pub fn set_unit_index(&mut self, unit_index: InlandPoolId) {
        self.unit_index = unit_index;
    }

     #[inline]
    pub fn set_generation(&mut self, generation: Generation) {
        self.generation = generation;
    }
    
    #[inline]
    pub fn increment_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    #[inline]
    pub fn update(&mut self, archetype_id: ArchetypeId, unit_index: InlandPoolId) {
        self.archetype_id = archetype_id;
        self.unit_index = unit_index;
    }
}

/// Phase-7 fast-path entity location record.
///
/// Parallel to the legacy [`EntityInland`] during the Phase-7 migration
/// (steps 2 – 9 in `docs/plans/PHASE-07-fast-random-access.md`). Once the
/// shims are removed in step 9, this struct is renamed to `EntityInland`
/// and the legacy struct above is deleted.
///
/// Layout (asserted below): 16 B total, align 8.
/// - offset 0  : `archetype_ptr` (8 B) — raw provenance pointer into the
///   `ArchetypeBundle` slab. `NULL` ⇔ dead slot.
/// - offset 8  : `unit_index` (4 B) — row index into
///   `Archetype.entity_ids`.
/// - offset 12 : `generation` (4 B) — matches `Entity::generation`
///   natively (no truncation in the hot path).
///
/// Stored as `*mut Archetype` (not `*const`) so that a `&mut EcsMaster`
/// can transitively cast to `&mut Archetype` without provenance
/// laundering during `create_entity` (see plan decision D7 / SAFETY
/// invariant U14).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct EntityInlandFast {
    archetype_ptr: *mut crate::ecs::core::archetype::archetype::Archetype,
    unit_index: u32,
    generation: u32,
}

const _: () = assert!(std::mem::size_of::<EntityInlandFast>() == 16);
const _: () = assert!(std::mem::align_of::<EntityInlandFast>() == 8);
const _: () = assert!(std::mem::offset_of!(EntityInlandFast, archetype_ptr) == 0);
const _: () = assert!(std::mem::offset_of!(EntityInlandFast, unit_index) == 8);
const _: () = assert!(std::mem::offset_of!(EntityInlandFast, generation) == 12);

impl EntityInlandFast {
    /// Sentinel value for "dead" / never-registered slots.
    ///
    /// `archetype_ptr.is_null()` is the single source of truth for
    /// liveness; `unit_index` and `generation` are unspecified when
    /// `archetype_ptr` is null.
    pub const NULL: Self = Self {
        archetype_ptr: ptr::null_mut(),
        unit_index: 0,
        generation: 0,
    };

    /// Constructs a fast-path inland record.
    ///
    /// `archetype_ptr` must point into a stable `ArchetypeBundle` slab
    /// slot for the lifetime of the owning `EcsMaster` (plan invariants
    /// U1 / U2). This constructor performs no validation; the caller is
    /// responsible for honoring the invariants. The constructor itself
    /// is safe because it merely stores the pointer.
    #[inline]
    pub fn new(
        archetype_ptr: *mut crate::ecs::core::archetype::archetype::Archetype,
        unit_index: u32,
        generation: u32,
    ) -> Self {
        Self { archetype_ptr, unit_index, generation }
    }

    /// Raw pointer to the owning `Archetype`. May be null for dead
    /// slots; callers must check `is_null` before dereferencing.
    #[inline]
    pub fn archetype_ptr(&self) -> *mut crate::ecs::core::archetype::archetype::Archetype {
        self.archetype_ptr
    }

    /// Row index of this entity inside its archetype's column tables.
    #[inline]
    pub fn unit_index(&self) -> u32 {
        self.unit_index
    }

    /// Generation tag stored at registration time; must match
    /// `Entity::generation` for the handle to be considered live.
    #[inline]
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// `true` when the slot has never been registered or has been
    /// deallocated. Cheaper than a generation comparison and is the
    /// first check in every fast-path read.
    #[inline]
    pub fn is_null(&self) -> bool {
        self.archetype_ptr.is_null()
    }

    #[inline]
    pub fn set_generation(&mut self, generation: u32) {
        self.generation = generation;
    }

    /// Wrapping increment of the generation counter. Wrap window is
    /// 2^32 per slot — accepted in plan decision D2.
    #[inline]
    pub fn increment_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// Test-only constructor producing a record with a null
    /// `archetype_ptr`. Used by the Phase-7 step-10 migration recipe
    /// (M1) for unit tests that need an inland value but cannot
    /// realistically spin up an `ArchetypeBundle`.
    #[cfg(test)]
    pub(crate) fn dangling_for_test(generation: u32, unit_index: u32) -> Self {
        Self {
            archetype_ptr: ptr::null_mut(),
            unit_index,
            generation,
        }
    }
}
