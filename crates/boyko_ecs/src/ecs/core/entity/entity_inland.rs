use core::ptr;

/// Phase-7 entity location record (fast-path).
///
/// Replaces the legacy three-field `EntityInland` (archetype_id +
/// unit_index + generation) with a direct `*mut Archetype` slab pointer so
/// the hot `get_component_raw` path can dereference into the archetype
/// without a `SparseMap` indirection.
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
pub struct EntityInland {
    archetype_ptr: *mut crate::ecs::core::archetype::archetype::Archetype,
    unit_index: u32,
    generation: u32,
}

// Layout pinned for the 64-bit target (the engine's supported platform); the
// size/align/offsets encode an 8-byte raw pointer (`archetype_ptr`), so they are
// gated to 64-bit — see CLAUDE.md target platform. `offset_of(archetype_ptr) ==
// 0` is width-independent (a `#[repr(C)]` first field is at offset 0 on every
// target) and stays unconditional.
const _: () = assert!(std::mem::offset_of!(EntityInland, archetype_ptr) == 0);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<EntityInland>() == 16);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::align_of::<EntityInland>() == 8);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::offset_of!(EntityInland, unit_index) == 8);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::offset_of!(EntityInland, generation) == 12);

impl EntityInland {
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

    /// Constructs an entity-location record.
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

    /// Overwrites the generation tag. Used by tests and migration tooling;
    /// production code should never call this directly — generation bumps
    /// are owned by `EntityMaster::deallocate_entity`.
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

    /// Updates the unit_index field in place. Used by
    /// `EcsMaster::delete_entity` to fix up the swapped entity's row
    /// index when `RemoveOutcome::Swapped` is returned.
    #[inline]
    pub fn set_unit_index(&mut self, unit_index: u32) {
        self.unit_index = unit_index;
    }

    /// Test-only constructor producing a record with a **dangling but
    /// non-null** `archetype_ptr`. The pointer is never dereferenced;
    /// `is_null()` returns `false`, distinguishing a test "live but
    /// synthetic" inland from a real `NULL`-sentinel dead slot. Used by
    /// the Phase-7 step-10 migration recipe (M1) for unit tests that
    /// need an inland value but cannot realistically spin up an
    /// `ArchetypeBundle`.
    #[cfg(test)]
    #[allow(dead_code)] // Wired into M1 test migration at Phase 7 Step 10.
    pub(crate) fn dangling_for_test(unit_index: u32, generation: u32) -> Self {
        Self {
            archetype_ptr: ptr::NonNull::<
                crate::ecs::core::archetype::archetype::Archetype,
            >::dangling()
            .as_ptr(),
            unit_index,
            generation,
        }
    }
}
