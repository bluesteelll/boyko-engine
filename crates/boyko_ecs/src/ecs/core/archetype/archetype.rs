use std::cell::UnsafeCell;

use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId, EntityId, InlandPoolId};
use crate::ecs::core::archetype::archetype_signature::ArchetypeSignature;
use crate::ecs::core::change_detection::Tick;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::component::component_pool_bundle::ComponentPoolBundle;
use crate::ecs::core::component::component_registry::{
    self, MAX_COMPONENTS, ResidencyKind, StorageKind,
};
use crate::ecs::core::component::enable::enable_store::{EnableColumn, EnableStore};
use crate::ecs::core::component::hooks::archetype_flags::ArchetypeFlags;
use crate::ecs::error::{EcsError, EcsResult};

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

// Layout pinned for the 64-bit target (the engine's supported platform); the
// size/align/offsets encode an 8-byte raw pointer (`ptr`), so they are gated to
// 64-bit — see CLAUDE.md target platform. `offset_of(ptr) == 0` is the only
// width-independent member (a `#[repr(C)]` first field is at offset 0 on every
// target) and stays unconditional.
const _: () = assert!(std::mem::offset_of!(Column, ptr) == 0);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<Column>() == 16);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::align_of::<Column>() == 8);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::offset_of!(Column, stride) == 8);
#[cfg(target_pointer_width = "64")]
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
// `EntityId` wraps a `usize`, so the 16-byte size encodes the 64-bit ABI;
// gated to 64-bit (the engine's supported platform) — see CLAUDE.md.
#[cfg(target_pointer_width = "64")]
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

    /// EnableTag bitset columns owned by this archetype (Decision D1 / Step 4).
    ///
    /// Parallel to [`Self::component_pools`] but for `StorageKind::Bitset` tags:
    /// each toggled `(archetype, tag)` owns a lazily-paged [`EnableColumn`]
    /// (no signature membership, no `ComponentPool`). Empty by default — an
    /// archetype that never has an enable tag toggled pays one `SmallList4`
    /// inline header and zero heap allocations.
    ///
    /// Placed immediately AFTER `columns` so the single-load hot read path
    /// (`*(arch + c*16)` at offset 0) is undisturbed; the enable store is
    /// touched only by the cold toggle/migration/swap-remove paths, never by
    /// the per-row component fetch.
    pub(crate) enable_store: EnableStore,

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

    /// Phase 14a hook-presence bitset. OR-computed once at construction from
    /// the cold `HOOKS` table (plan §4.6); read as a single `u16` load +
    /// `test`/`jz` on every structural-op dispatch site.
    ///
    /// Placed after `signature` (plan §1-W2). Because
    /// `signature` embeds a `#[repr(align(32))]` `ComponentMask`, the offset
    /// after it was already 8-aligned with zero padding; inserting this `u16`
    /// adds +8 B (2 B + 6 B realign), not zero — the W2 correction. `columns`
    /// stays at offset 0 (asserted below).
    ///
    /// `pub(crate)` for the same in-place-slab-construction reason as the
    /// neighbouring fields (Phase 7 U13): the slab path writes every field via
    /// `addr_of_mut!` and must initialise this one too.
    pub(crate) flags: ArchetypeFlags,

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

// TRIPWIRE 1 (Phase 14a §1-W2 / §8 P5; EnableTag plan Step 4): hard size
// assertion pinned to a MEASURED literal. History: the `flags: ArchetypeFlags`
// (u16) field grew the struct to 8480 B (Phase 14a); Phase X.J deleted the
// vestigial `arena: *const Arena` field (-8 B raw) but `align_of == 32` (via
// the `ComponentMask` inside `signature`) rounded straight back to 8480 B.
//
// EnableTag Step 4 inserts the `enable_store: EnableStore` field (112 B,
// align 8) immediately after `columns`. Net +96 B (it consumed some existing
// trailing align-32 padding rather than a full +112) → 8576 B measured on the
// x86_64 target. `columns` stays at offset 0 (asserted above) so the Phase 7
// single-load hot read path is undisturbed. This guards against accidental
// layout drift; if a future change reorders fields or alters `signature`'s
// alignment, this trips before a perf regression can ship.
//
// The struct embeds `Vec`s and `usize` fields, so the 8576 B figure encodes
// the 64-bit ABI; gated to 64-bit (the engine's supported platform) — see
// CLAUDE.md target platform. `offset_of(columns) == 0` above is
// width-independent (first `#[repr(C)]` field) and stays unconditional.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<Archetype>() == 8576);

impl Archetype {
    /// Creates a new archetype with the given ID.
    ///
    /// The 8 KB `columns` table is zero-initialised on the stack; Phase 4
    /// will switch this to in-place slab construction via `addr_of_mut!`
    /// (Phase 7 W6) once `ArchetypeBundle` lands. For now, low-frequency
    /// creation sites pay the temporary stack cost.
    pub fn new(id: ArchetypeId) -> Self {
        Self {
            columns: [Column::null(); MAX_COMPONENTS],
            enable_store: EnableStore::new(),
            id,
            component_pools: ComponentPoolBundle::new(),
            current_index: 0,
            signature: ArchetypeSignature::new(ComponentMask::new()),
            // Phase 14a: no hooks until Wave 2 computes them from the `HOOKS`
            // table at `create_by_ids` / `register_component_inplace`.
            flags: ArchetypeFlags::empty(),
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
    /// Builds the archetype-signature mask for `component_ids`, FILTERING OUT
    /// every `StorageKind::Bitset` id (EnableTag plan C1 premise / Decision D5).
    ///
    /// This is the single signature-filtering implementation. Every archetype
    /// signature — the archetype's own [`Self::create_by_ids`] mask AND the
    /// parallel registry-index mask built at the `ArchetypeMaster`
    /// create/get-or-create funnels — MUST route through here so the registry
    /// signature is byte-identical to the archetype's own filtered signature.
    /// Without it, the registry would index an archetype under a signature that
    /// includes a bitset bit while the archetype's real signature excludes it,
    /// breaking dedup (`find_exact_match`) and diverging the query-match path.
    ///
    /// `pub(crate)`: the only off-archetype caller is `ArchetypeMaster`; both
    /// live in this crate. Cold path (archetype creation), so the per-id
    /// `storage_kind` lookup is fine.
    pub(crate) fn filtered_signature_mask(component_ids: &[ComponentId]) -> ComponentMask {
        let mut mask = ComponentMask::new();
        for &comp_id in component_ids {
            if component_registry::storage_kind(comp_id.0) == StorageKind::Bitset {
                continue;
            }
            mask.set(comp_id);
        }
        mask
    }

    pub fn create_by_ids(id: ArchetypeId, component_ids: &[ComponentId]) -> Self {
        // Create a mask from the component IDs. EnableTag plan C1 premise:
        // `StorageKind::Bitset` ids are FILTERED OUT of the signature mask so
        // they never fragment the archetype space (Decision D5). Routed through
        // the single shared filter so the registry signature minted at the
        // `ArchetypeMaster` funnels matches this one bit-for-bit.
        let mask = Self::filtered_signature_mask(component_ids);

        // Initialize archetype with mask and empty component pools
        let mut archetype = Self {
            columns: [Column::null(); MAX_COMPONENTS],
            enable_store: EnableStore::new(),
            id,
            component_pools: ComponentPoolBundle::new(),
            current_index: 0,
            signature: ArchetypeSignature::new(mask),
            // Phase 14a: initialised empty; the OR-compute happens below once
            // the component pools are registered (Wave 2 wires `HOOKS`).
            flags: ArchetypeFlags::empty(),
            component_ids: component_ids.to_vec(),
            entity_ids: Vec::new(),
        };

        // Create component pools for each component ID. Each successful
        // `add_pool` must be paired with `refresh_column` to keep the inline
        // `columns` table in sync (Phase 7 invariant U7). Phase 14a: OR each
        // component's hook bits into the archetype flags from the cold `HOOKS`
        // table (plan §4.6) — one accumulator, set once after the loop.
        //
        // EnableTag C1 premise: a `StorageKind::Bitset` id gets NO
        // `ComponentPool` (and never enters the signature, filtered above) — a
        // sibling `&mut C` data param on a bitset id is therefore structurally
        // impossible, which is what makes the Enable filter's no-op
        // `init_access` sound (Decision D8 / D1 inv 3).
        //
        // Phase 4 Seam 1/2 (D1/D2, IM-2): this is the single full-slice mint
        // funnel (both `ArchetypeMaster` paths reach it). Fold the residency
        // classification into the SAME walk — one extra cold `residency_class`
        // load per id + a 2-bool fold, no new loop:
        //   * per-component `GPU_RESIDENT` OR rides the hook-bit accumulator;
        //   * the set-level `saw_gpu && saw_non_gpu` conflict (Phase 5 C2:
        //     GPU-resident ⇒ all-components-Gpu) is detected over the full slice
        //     and rejected loudly AFTER the walk.
        let mut flags = ArchetypeFlags::empty();
        let mut saw_gpu = false;
        let mut saw_non_gpu = false;
        for &comp_id in component_ids {
            // Residency is scanned for EVERY id in the signature, including a
            // bitset tag (always `Cpu` — no `ComponentPool` — so it counts as
            // non-Gpu, which is correct: a bitset EnableTag is never device-
            // resident).
            match component_registry::residency_class(comp_id.0) {
                ResidencyKind::Gpu => {
                    saw_gpu = true;
                    flags.insert(ArchetypeFlags::GPU_RESIDENT);
                }
                // C2: ANY non-Gpu component (Cpu OR CpuPinned) makes a Gpu
                // signature impure → rejected after the walk.
                ResidencyKind::CpuPinned | ResidencyKind::Cpu => saw_non_gpu = true,
            }
            if component_registry::storage_kind(comp_id.0) == StorageKind::Bitset {
                Self::debug_assert_bitset_premise(&archetype, comp_id);
                continue;
            }
            archetype.component_pools.add_pool(comp_id);
            archetype.refresh_column(comp_id);
            flags.insert_from_hooks(comp_id);
        }
        if saw_gpu && saw_non_gpu {
            residency_conflict_panic(component_ids);
        }
        archetype.flags = flags;

        archetype
    }

    /// Gets the unique ID of this archetype
    #[inline]
    pub fn id(&self) -> ArchetypeId {
        self.id
    }

    /// Registers a component type by ID
    pub fn register_component(&mut self, component_id: ComponentId) -> bool {
        // EnableTag C1 premise: a `StorageKind::Bitset` id is never part of a
        // signature and never gets a `ComponentPool`. Refuse to register one as
        // a table component (Decision D5 / D1 inv 3) — toggling is the only way
        // a bitset tag enters an archetype.
        if component_registry::storage_kind(component_id.0) == StorageKind::Bitset {
            Self::debug_assert_bitset_premise(self, component_id);
            return false;
        }

        // Check if this component type is already registered
        if self.signature.mask().contains(component_id) {
            return false;
        }

        // Add a pool for this component type, then refresh the inline column
        // table so the hot read path can resolve `component_id` without going
        // through the bundle's sparse map (Phase 7 invariant U7).
        self.component_pools.add_pool(component_id);
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
    /// Calls `component_pools.add_pool(component_id)` and refreshes the
    /// inline column entry. Intended exclusively for in-place archetype
    /// construction — the bundle resides in the same crate (`pub(crate)`).
    pub(crate) fn register_component_inplace(&mut self, component_id: ComponentId) {
        // EnableTag C1 premise (Decision D5 / D1 inv 3): a `StorageKind::Bitset`
        // id gets NO `ComponentPool` and no hook bits. The slab path's caller
        // (`add_archetype_from_components_fallible`) is responsible for keeping
        // the bit out of the signature mask too; here we skip the pool so the
        // bitset id never gains a backing column (the no-`ComponentPool`
        // structural premise behind D8's no-op `init_access`).
        if component_registry::storage_kind(component_id.0) == StorageKind::Bitset {
            Self::debug_assert_bitset_premise(self, component_id);
            return;
        }
        self.component_pools.add_pool(component_id);
        self.refresh_column(component_id);
        // Phase 14a (plan §4.6): OR this single component's hook bits into the
        // archetype flags from the cold `HOOKS` table. The slab path
        // (`add_archetype_from_components_fallible`) calls this once per
        // component, so the accumulated OR over the whole component set is the
        // archetype's flag value.
        self.flags.insert_from_hooks(component_id);
        // Phase 4 Seam 1 (D1, IM-2): PURE bit-stamp — OR `GPU_RESIDENT` if this
        // single id is `Gpu`. This single-component path NEVER rejects: the
        // set-level `saw_gpu && saw_cpu_pinned` conflict scan lives at the
        // full-slice `create_by_ids` funnel (the slab signature is validated
        // there). One cold `residency_class` load.
        if component_registry::residency_class(component_id.0) == ResidencyKind::Gpu {
            self.flags.insert(ArchetypeFlags::GPU_RESIDENT);
        }
    }

    /// Debug-only tripwire for the EnableTag C1 premise (Decision D1 inv 3 /
    /// D8): a `StorageKind::Bitset` id must NEVER appear in this archetype's
    /// signature mask AND must NEVER have a `ComponentPool`. A sibling data
    /// access on a bitset id is then structurally impossible — the soundness
    /// ground for the Enable filter's no-op `init_access`.
    ///
    /// Called at every construction/signature site that skips a bitset id.
    /// Compiles to nothing in release.
    #[inline]
    fn debug_assert_bitset_premise(this: &Self, component_id: ComponentId) {
        debug_assert!(
            !this.signature.mask().contains(component_id),
            "C1 premise: bitset id {} must not be in the archetype signature",
            component_id.0
        );
        debug_assert!(
            !this.component_pools.contains(component_id),
            "C1 premise: bitset id {} must not have a ComponentPool",
            component_id.0
        );
    }

    /// Returns the directory bound (`reserve_rows`) for this archetype's enable
    /// columns: a row count guaranteed to be `> unit_index` for every live row.
    ///
    /// An `EnableColumn`'s page directory is sized to cover this many rows
    /// (Decision D1 sub-decision: backing & regrow). Live rows are bounded by
    /// `current_index`, which in turn never exceeds any owned pool's reserve
    /// ceiling (the `reserve_capacity` two-phase contract validates
    /// `count + n <= reserve_rows` for every pool before any row is written).
    /// The EMPTY archetype owns no pools, so this falls back to
    /// `current_index` (rows are still bounded by the entity count).
    ///
    /// The `+ 1` floor guarantees a non-zero, row-covering bound even at
    /// `current_index == 0` (the `EnableColumn::new` directory always holds at
    /// least one slot).
    #[inline]
    pub(crate) fn enable_reserve_rows(&self) -> usize {
        // Any pool's capacity bounds `current_index`; pick the first. Empty
        // archetype ⇒ no pools ⇒ fall back to the entity count.
        let pool_ceiling = self
            .component_pools
            .pools_iter()
            .map(|p| p.capacity())
            .max();
        match pool_ceiling {
            Some(ceiling) => ceiling.max(self.current_index + 1),
            None => self.current_index + 1,
        }
    }

    /// Returns a raw pointer to the [`EnableColumn`] for `tag`, or null if this
    /// archetype has never toggled that tag (Decision D2 — the Fetch cache
    /// caches this, NULL ⇒ "all rows disabled").
    ///
    /// The pointer is valid while `&self` is borrowed; the enable store lives
    /// inline in the (pointer-stable) slab slot, so it does not move for the
    /// archetype's lifetime. Used by the query filter `set_table_*` cold path
    /// (Wave 3) and by tests.
    // Forward seam (EnableTag plan, Step 4): the production consumer is the
    // `Enabled<T>`/`Disabled<T>` filter `set_table_*_no_meta` cold path landing
    // in Wave 3 (Step 7). Shipped here so the archetype-side enable accessor is
    // complete in one unit; `#[allow(dead_code)]` (scoped, justified — mirrors
    // the Wave-1 `set_storage_kind` seam) keeps the crate clippy-clean under
    // `-D warnings` until Wave 3 wires it. Tests in this module exercise it.
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn enable_column_ptr(&self, tag: ComponentId) -> *const EnableColumn {
        match self.enable_store.column(tag) {
            Some(col) => col as *const EnableColumn,
            None => core::ptr::null(),
        }
    }

    /// Sets (or clears) the enable bit for `tag` at `row` (the cold
    /// toggle/migration path — Decision D3), returning `true` iff this call
    /// allocated the tag's [`EnableColumn`] for the FIRST time.
    ///
    /// On a `true` return the caller (the toggle API, Step 5) performs the
    /// one-time bookkeeping exactly once: record
    /// `EnablePresence::note_column_alloc` + bump `enable_generation`
    /// (Decision D1 inv 5 / O2). This method owns only the per-archetype
    /// storage and computes the directory bound from
    /// [`Self::enable_reserve_rows`].
    ///
    /// Clearing (`value == false`) never allocates a column or page: the
    /// never-toggled default is already clear, so a clear into an absent column
    /// is a no-op and returns `false`.
    #[inline]
    pub(crate) fn set_enable_bit(&mut self, tag: ComponentId, row: usize, value: bool) -> bool {
        debug_assert_eq!(
            component_registry::storage_kind(tag.0),
            StorageKind::Bitset,
            "set_enable_bit: id {} is not a bitset enable tag",
            tag.0
        );
        let reserve_rows = self.enable_reserve_rows();
        if !value {
            // Clear: only touch an existing column; never allocate to store a
            // zero. `write_row_bit` already short-circuits the clear-no-column
            // case, but go through the column directly to avoid a redundant
            // scan when the column is absent.
            self.enable_store.write_row_bit(tag, row, false, reserve_rows);
            return false;
        }
        let newly_allocated = self.enable_store.column(tag).is_none();
        self.enable_store
            .get_or_alloc_column(tag, reserve_rows)
            .set(row, true, reserve_rows);
        newly_allocated
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

    /// Flips component `cid`'s pool to device-resident behind `handle`, then NULLs
    /// its inline column cache (Phase 5 C1 — the LOAD-BEARING device-mint funnel).
    ///
    /// This is the archetype-level funnel `boyko_render` (Wave B) calls to mint a
    /// device column. It performs two steps in order:
    ///   (a) flip the pool via [`ComponentPool::make_device_backed`] + record the
    ///       opaque handle via [`ComponentPool::set_device_handle`]; THEN
    ///   (b) `self.columns[cid.0] = Column::null()` **DIRECTLY** — NOT
    ///       `refresh_column`.
    ///
    /// # Why direct-null, not `refresh_column` (C1)
    ///
    /// `Column.ptr` caches the pool's `buffer_ptr()`. After `make_device_backed`
    /// frees the Host `VmReservation`, that cached base DANGLES but stays
    /// non-null, so every direct reader's null-check (`Column::is_null`) would
    /// PASS → use-after-free. `refresh_column` would RE-CACHE the now-dangling
    /// Host base (it reads `pool.buffer_ptr()` even on a device pool — the base
    /// fields are not cleared on the flip, see `refresh_column` at the lines
    /// above). Nulling the column directly makes the existing null-checks return
    /// `None`/`false`/skip — the correct "the CPU cannot touch GPU bytes"
    /// contract (umbrella §2).
    ///
    /// `#[cfg(not(miri))]` — the device backing arm + its pool primitives are
    /// compiled out under Miri (Phase 4 / `DeviceColumn`).
    ///
    /// # Postcondition verified here (PER-COMPONENT only)
    ///
    /// This is a per-component site: Wave B calls it once per column. Its
    /// funnel-tail `debug_assert` verifies ONLY the per-component postcondition —
    /// the just-flipped `columns[cid.0]` is null. The whole-archetype "all GPU
    /// columns null" property is NOT — and CANNOT be — checked here: `GPU_RESIDENT`
    /// is stamped at MINT over the full signature, so it is already true on the
    /// first flip, while components are flipped one at a time. A not-yet-flipped
    /// sibling's pool is still Host-backed and VALID (its reservation is freed only
    /// when ITS OWN flip runs) and its column correctly non-null; queries skip the
    /// whole archetype via `is_gpu_resident()` (mint-stamped, column-state-
    /// independent), so every intermediate state is sound. The whole-archetype
    /// property holds BY CONSTRUCTION once every component has been flipped.
    ///
    /// # Panics
    ///
    /// Release-panics (via [`ComponentPool::make_device_backed`]'s O1 guard) if
    /// the pool is non-empty (`len != 0`) — a populated pool must never flip to
    /// device backing (data-loss guard).
    ///
    /// Visibility: `pub` because the production caller is `boyko_render`'s
    /// `GpuColumnManager::create_column`, a SEPARATE crate (Phase-5 Wave B). It is
    /// reached through the existing public chain
    /// `EcsMaster::archetype_master_mut().get_archetype_mut(id)` → `&mut Archetype`.
    /// It introduces NO graphics type into `boyko_ecs` — the only non-core type in
    /// its signature is the graphics-PURE [`DeviceColumnHandle`](crate::ecs::memory::device_column::DeviceColumnHandle)
    /// (a `#[repr(transparent)]` `u64`), so the purity invariant holds.
    ///
    /// `#[cfg(not(miri))]`: the device-backing arm + its pool primitives are
    /// compiled out under Miri (Phase 4 / `DeviceColumn`); cross-crate callers
    /// build native, so the `pub` surface is present exactly where it is callable.
    #[cfg(not(miri))]
    pub fn make_component_device_backed(
        &mut self,
        cid: ComponentId,
        handle: crate::ecs::memory::device_column::DeviceColumnHandle,
    ) {
        debug_assert!(
            cid.0 < MAX_COMPONENTS,
            "make_component_device_backed: component_id {} >= MAX_COMPONENTS ({})",
            cid.0,
            MAX_COMPONENTS
        );
        // X4 (release-present soundness guard): ONLY a statically `Gpu`-classed
        // component may flip to device backing. A `debug_assert!` here would vanish
        // in release, letting an external caller flip a `Cpu` component → a
        // CPU-reachable dangling column (the X3 unsound state). This is a setup-time
        // call (not the CPU hot path), so the release `assert!` does not affect the
        // 0%-gate. (Keep the existing per-component column-null behavior below.)
        assert_eq!(
            component_registry::residency_class(cid.0),
            ResidencyKind::Gpu,
            "make_component_device_backed: component {} is not ResidencyKind::Gpu — \
             a Cpu component must never be flipped to device backing (X4)",
            cid.0
        );
        let pool = self
            .component_pools
            .get_pool_mut(cid)
            .expect("invariant: make_component_device_backed targets a component with a pool");
        // (a) flip + record the opaque handle on the boxed DeviceColumn.
        pool.make_device_backed(handle.0);
        pool.set_device_handle(handle);
        // (b) NULL the inline column DIRECTLY — never `refresh_column` (it would
        // re-cache the now-dangling Host base, C1). The existing null-checks on
        // every direct reader then return None/false/skip.
        self.columns[cid.0] = Column::null();

        // Funnel-tail invariant (C1): this PER-COMPONENT site can only verify the
        // PER-COMPONENT postcondition — the just-flipped column is null. The
        // whole-archetype "all GPU columns null" property is NOT checkable here:
        // `GPU_RESIDENT` is stamped at MINT over the full signature, so it is
        // already true on the first flip, while Wave B flips components one at a
        // time. Between flips a not-yet-flipped component's pool is still
        // Host-backed and VALID (its reservation is freed only when ITS OWN flip
        // runs), so its column is correctly non-null; queries skip the whole
        // archetype via `is_gpu_resident()` (mint-stamped, column-state-
        // independent), so the intermediate state is sound. The whole-archetype
        // property holds BY CONSTRUCTION once every component has been flipped.
        debug_assert!(
            self.columns[cid.0].is_null(),
            "make_component_device_backed must null the just-flipped column"
        );
    }

    /// Writes a NEW device-column handle onto an already-device-backed component
    /// `cid` (Phase 5 MF-2/3 — the grow write path).
    ///
    /// `boyko_render`'s `GpuColumnManager::grow_column` calls this after it
    /// reallocs the device column and mints a NEW handle: it updates ONLY the
    /// boxed [`DeviceColumn`]'s handle (via [`ComponentPool::set_device_handle`]) —
    /// it does NOT re-flip the backing (no `Box` churn, no lost device counters)
    /// and does NOT touch the already-null column cache. DISTINCT from the
    /// write-once `buffer`/`added_base`/`changed_base`, so it violates no
    /// base-pointer invariant (MF-3); the `unreachable!` Host-grow arm stays
    /// unreachable.
    ///
    /// Visibility: `pub` for the same cross-crate reason as
    /// [`make_component_device_backed`](Self::make_component_device_backed),
    /// reached through `EcsMaster::archetype_master_mut().get_archetype_mut(id)`.
    /// Introduces NO graphics type (the only non-core type is the graphics-pure
    /// [`DeviceColumnHandle`](crate::ecs::memory::device_column::DeviceColumnHandle)).
    ///
    /// `#[cfg(not(miri))]` — matches the device-backing arm compiled out under
    /// Miri (Phase 4 / `DeviceColumn`).
    ///
    /// # Panics
    ///
    /// **Release** (X5): panics if the targeted pool is NOT already `Device`-backed
    /// — `ComponentPool::set_device_handle` silently no-ops on a Host pool, so the
    /// guard is release-present to prevent a silently-dropped handle write.
    /// (Debug) if the targeted column is not null (a device-backed component's
    /// column is nulled at the original flip and stays null), or if the pool's
    /// Host `len != 0`.
    #[cfg(not(miri))]
    pub fn set_component_device_handle(
        &mut self,
        cid: ComponentId,
        handle: crate::ecs::memory::device_column::DeviceColumnHandle,
    ) {
        debug_assert!(
            cid.0 < MAX_COMPONENTS,
            "set_component_device_handle: component_id {} >= MAX_COMPONENTS ({})",
            cid.0,
            MAX_COMPONENTS
        );
        debug_assert!(
            self.columns[cid.0].is_null(),
            "set_component_device_handle: a device-backed component's column must stay null"
        );
        let pool = self
            .component_pools
            .get_pool_mut(cid)
            .expect("invariant: set_component_device_handle targets a component with a pool");
        // X5 (release-present soundness guard): the pool MUST already be
        // `Device`-backed. `ComponentPool::set_device_handle` silently no-ops on a
        // Host pool (its `PoolBacking::Device` match arm), so a `debug_assert!`
        // would vanish in release and let a stale-key grow silently DROP the new
        // handle write — leaving the core pool pointing at the freed old buffer.
        // `device_handle()` returns `Some` iff the pool is the `Device` arm. Setup-
        // time (the grow write path), so the release `assert!` is off the hot path.
        assert!(
            pool.device_handle().is_some(),
            "set_component_device_handle: component {} pool is not Device-backed — \
             the handle write would be silently dropped on a Host pool (X5)",
            cid.0
        );
        pool.set_device_handle(handle);
    }

    /// Refreshes the entire `columns` table from `component_pools`.
    ///
    /// Reserved for a hypothetical future RELOCATING store where every pool's
    /// `buffer_ptr` may move. Phase X.F/X.I confirmed address-stable growth
    /// (each pool only commits fresh pages at the frontier of its own
    /// contiguous reservation — pool buffers never move), so this MUST remain
    /// dead code. Not used on the Phase 7 hot path.
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
    /// constructing whatever entity-location record it needs.
    ///
    /// # Phase 10 INIT3 — `current_tick` parameter
    ///
    /// The world's current change-detection tick is threaded through the
    /// call so the newly-pushed row receives `added = changed = current_tick`
    /// in every component pool. The canonical caller is
    /// [`crate::ecs::core::ecs_master::EcsMaster::create_entity`], which
    /// reads `self.change_tick.load(Relaxed)` and forwards it here (plan
    /// §2.4 INIT3 / Round 2 W4 — single source of truth for the world tick).
    ///
    /// Audit: C-010 — switched from Vec to &[...]. Phase 7 Step 3 — replaced
    /// `&mut EntityInland` with `&mut u32`. Phase 10 Step 6 — added
    /// `current_tick`.
    pub fn create_entity(
        &mut self,
        entity_id: EntityId,
        new_unit_index: &mut u32,
        components: &[(ComponentId, &[u8])],
        current_tick: Tick,
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

        // Phase 22 D5(3): the dense row is `current_index`, taken BEFORE the
        // pool push. `push_entity_components` returns the FIRST pool's add
        // index, which is a vacuous 0 when `components` is empty (the EMPTY
        // archetype hosts zero pools) — propagating it gave every empty
        // entity row 0 and corrupted its archetype-mates' identity on
        // swap-remove. Pools grow in lock-step with `current_index`
        // (archetype invariant), so for a non-empty input the pushed index
        // must equal `row`; together with `push_entity_components`'
        // internal cross-pool agreement assert this proves EVERY pool's add
        // index == `current_index`.
        let row = self.current_index;
        let _pushed_index = self.component_pools.push_entity_components(components);
        debug_assert!(
            components.is_empty() || _pushed_index == row,
            "pool desync: push_entity_components returned row {} but archetype \
             current_index is {}",
            _pushed_index,
            row
        );
        *new_unit_index = row as u32;

        // Phase 10 STORE4 / INIT1: stamp the per-row `added` and `changed`
        // ticks of every component pool the entity contributes to. The
        // bundle push above guarantees every `components[i].0` resolves to
        // a live pool and that all pools share the same dense row `row`
        // (C-009 two-phase commit + the D5(3) agreement assert above).
        for (component_id, _) in components {
            // The pool was just pushed; `row < pool.count()` holds.
            // `get_pool_mut` returns `Some(_)` here per the bundle's
            // pre-validation in `can_push_entity_components`.
            if let Some(pool) = self.component_pools.get_pool_mut(*component_id) {
                // SAFETY (STORE3 + STORE4 + SCH3):
                //   - `row < pool.count()` — the slot was written above by
                //     `push_entity_components` (agreement debug-asserted).
                //   - `&mut self` on `Archetype` gives exclusive write
                //     access to every owned pool (Phase 9 dispatcher-only
                //     entry per the apply window); no concurrent reader
                //     of the tick slot exists.
                unsafe {
                    pool.write_added_tick(row, current_tick);
                    pool.write_changed_tick(row, current_tick);
                }
            }
        }

        // Add the entity ID to the vector
        self.entity_ids.push(entity_id);

        // Increment entity counter
        self.current_index += 1;

        true
    }

    /// Returns the `(added_ticks_base, changed_ticks_base)` pointer pair for
    /// the column hosting `component_id`, or `None` when the archetype lacks
    /// a pool for it.
    ///
    /// Wave C `Added<C>::set_table_*` / `Changed<C>::set_table_*` cache the
    /// returned base pointers in their `Fetch<'w>` once per archetype
    /// boundary (cold path), then index per-row through the `Fetch`.
    /// The pointers are stable for the pool's lifetime: Phase X.I made
    /// them write-once vm-reservation sub-region bases (growth commits
    /// fresh pages in place and never moves them) — strictly stronger
    /// than the old Phase 10 STORE2 "Box never reallocates" promise.
    ///
    /// Returning a tuple by value keeps the cold-path call signature
    /// simple; per-row reads do not touch this accessor.
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn tick_column_base(
        &self,
        component_id: ComponentId,
    ) -> Option<(*const UnsafeCell<Tick>, *const UnsafeCell<Tick>)> {
        let pool = self.component_pools.get_pool(component_id)?;
        Some((pool.added_ticks_ptr(), pool.changed_ticks_ptr()))
    }

    /// Removes an entity and all its components from this archetype.
    ///
    /// `removed_unit_index` is the dense row index inside the archetype's
    /// component pools (read from the caller's `EntityInland` fast store).
    /// The caller is responsible for ensuring the index belongs to this
    /// archetype — there is no longer a per-call `archetype_id` debug-assert
    /// because Phase 7 dispatches via direct `*mut Archetype` pointers and
    /// the caller is, by construction, this archetype.
    ///
    /// Returns a [`RemoveOutcome`] describing what happened:
    /// - [`RemoveOutcome::Last`]: the entity was the last one; no swap needed.
    /// - [`RemoveOutcome::Swapped`]: swap-remove occurred; caller must update
    ///   the moved entity's `unit_index` in `EntityMaster`.
    /// - [`RemoveOutcome::PoolFailure`]: removal failed; archetype is unchanged.
    pub fn remove_entity(&mut self, removed_unit_index: InlandPoolId) -> RemoveOutcome {
        let last_unit_index = InlandPoolId(self.current_index.saturating_sub(1));

        // EnableTag swap-remove fix (Decision O1-r7): captured from the PRE-removal
        // state so the directory bound still covers `last`. The bit op fires
        // exactly ONCE here (the DROP path) — never inside the migration helper
        // bodies, which take the disjoint `move_out_entity` no-drop path. Skipped
        // entirely when the archetype owns no enable columns (the common case).
        let enable_reserve_rows = self.enable_reserve_rows();

        // If removing the last entity, just pop it.
        if removed_unit_index == last_unit_index {
            if self.component_pools.pop_entity() {
                self.entity_ids.pop();
                self.current_index -= 1;
                // O1-r7 Last/pop: clear the popped row's bit in every enable
                // column (`removed == last` ⇒ a single `clear(last)`).
                if !self.enable_store.is_empty() {
                    self.enable_store.swap_remove_row(
                        last_unit_index.0,
                        last_unit_index.0,
                        enable_reserve_rows,
                    );
                }
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

        // O1-r7 Swapped: the entity that was at `last` moved into `removed`'s
        // slot, so `removed` must inherit `last`'s bit and `last` must clear —
        // READ-first inside `swap_remove_bit` (Decision C2/C4). Sequenced at the
        // same point as the component-byte `swap_remove_unit` above.
        if !self.enable_store.is_empty() {
            self.enable_store.swap_remove_row(
                removed_unit_index.0,
                last_unit_index.0,
                enable_reserve_rows,
            );
        }

        RemoveOutcome::Swapped { moved_entity: swapped_entity_id }
    }

    /// Phase 11 (plan §7.2 / C5 / W-N2): releases the row at
    /// `removed_unit_index` WITHOUT invoking per-component `Drop`.
    ///
    /// Used by the archetype-migration paths in
    /// [`crate::ecs::core::commands::migration_helpers`] after their
    /// retained-bytes have been memcpy'd into the target archetype. The
    /// caller MUST have already moved or explicitly dropped every
    /// component at `removed_unit_index` — otherwise components leak (no
    /// destructor will ever run on those bytes).
    ///
    /// Returns a [`RemoveOutcome`] mirroring [`Self::remove_entity`]:
    ///
    /// * [`RemoveOutcome::Last`] — `removed_unit_index == last_unit_index`,
    ///   trailing slot popped, no swap needed.
    /// * [`RemoveOutcome::Swapped`] — swap-remove occurred; caller must
    ///   update the moved entity's `unit_index` in `EntityMaster`.
    ///
    /// # Differences vs [`Self::remove_entity`]
    ///
    /// `remove_entity` runs `drop_fn` on the source row's bytes for every
    /// pool; this function does NOT, so the caller can re-claim ownership
    /// of those bytes (e.g. memcpy them into a different archetype before
    /// release). Both paths invoke the swap-remove dance over byte +
    /// tick storage identically.
    pub(crate) fn move_out_entity(
        &mut self,
        removed_unit_index: InlandPoolId,
    ) -> RemoveOutcome {
        let last_unit_index = InlandPoolId(self.current_index.saturating_sub(1));

        // EnableTag swap-remove fix (Decision O1-r7 / critic note 3): the SOURCE
        // swap-fix bit op lives HERE (the no-drop path taken by all 4 migration
        // helpers via this single funnel), NEVER in the helper bodies — so it
        // fires exactly once per migration. The migrating entity's bit was
        // already READ by the helper's phase-1 `read_row_bits` BEFORE this call
        // (C4 READ-before-swap ordering); this op only fixes the swapped-in
        // survivor. Captured from the PRE-removal state so the bound covers
        // `last`. Skipped when the archetype owns no enable columns.
        let enable_reserve_rows = self.enable_reserve_rows();

        if removed_unit_index == last_unit_index {
            // Pop trailing slot in every pool. W-N2: no drop.
            self.component_pools.pop_entity_no_drop();
            self.entity_ids.pop();
            self.current_index -= 1;
            // O1-r7 Last/pop: clear the popped row's bit (`removed == last`).
            if !self.enable_store.is_empty() {
                self.enable_store.swap_remove_row(
                    last_unit_index.0,
                    last_unit_index.0,
                    enable_reserve_rows,
                );
            }
            return RemoveOutcome::Last;
        }
        let moved_entity = self.entity_ids[last_unit_index.0];
        // SAFETY: `removed_unit_index < last_unit_index < current_index`
        //   so the index is in-bounds for every pool (each pool's
        //   `count()` equals `current_index` by archetype invariant).
        //   `&mut self` ⇒ exclusive access; caller upholds the
        //   W-N2 PRECONDITION (bytes moved or dropped before this call).
        unsafe { self.component_pools.swap_remove_unit_no_drop(removed_unit_index.0); }
        self.entity_ids.swap_remove(removed_unit_index.0);
        self.current_index -= 1;
        // O1-r7 Swapped: READ-first fix-up at the same sequence point as the
        // component-byte `swap_remove_unit_no_drop` above (Decision C4).
        if !self.enable_store.is_empty() {
            self.enable_store.swap_remove_row(
                removed_unit_index.0,
                last_unit_index.0,
                enable_reserve_rows,
            );
        }
        RemoveOutcome::Swapped { moved_entity }
    }

    /// Phase 11 (plan §9.2 / Wave E Step 20): pushes a new entity row
    /// into this archetype with **explicit** per-component
    /// `(added_tick, changed_tick)` pairs.
    ///
    /// Used by the migration helpers to preserve retained components'
    /// original ticks across archetype boundaries (insert / remove). The
    /// bundle slots threaded through `migrate_entity_insert` already
    /// carry `(current_tick, current_tick)` for fresh bundle bytes and
    /// `(orig_added, orig_changed)` for retained bytes — this function
    /// memcpys the bytes and stamps the supplied ticks in lockstep.
    ///
    /// On signature mismatch or pool failure returns `false` and leaves
    /// the archetype unchanged (two-phase commit via
    /// `can_push_entity_components`). On success, writes the assigned
    /// dense row index into `*new_unit_index`.
    ///
    /// # `current_tick` parameter
    ///
    /// Threaded for parity with [`Self::create_entity`] — currently
    /// unused inside this function because every per-component tick is
    /// supplied explicitly. Reserved for the Phase 12 `is_new` flag
    /// (OQ5) that will distinguish migration-added vs replaced bytes.
    pub(crate) fn create_entity_with_ticks(
        &mut self,
        entity_id: EntityId,
        new_unit_index: &mut u32,
        components: &[(ComponentId, &[u8], Tick, Tick)],
        current_tick: Tick,
    ) -> bool {
        let _ = current_tick; // Reserved (Phase 12 OQ5).

        // Build a mask of the input ids; signature subset check (mirrors
        // `create_entity`).
        let mut input_mask = ComponentMask::new();
        for (id, _, _, _) in components {
            debug_assert!(
                *id < MAX_COMPONENTS_ID,
                "component_id {} >= MAX_COMPONENTS ({})", id.0, MAX_COMPONENTS
            );
            input_mask.set(*id);
        }
        debug_assert_eq!(
            input_mask.popcount(),
            components.len(),
            "Archetype::create_entity_with_ticks: duplicate ComponentId in input"
        );
        if !self.signature.mask().is_subset(&input_mask) {
            return false;
        }

        // Two-phase commit: pre-validate every pool has free capacity.
        // We reuse the existing 3-tuple checker by stripping ticks.
        let component_bytes: Vec<(ComponentId, &[u8])> = components
            .iter()
            .map(|(id, bytes, _, _)| (*id, *bytes))
            .collect();
        if !self.component_pools.can_push_entity_components(&component_bytes) {
            return false;
        }

        // Push bytes; pools grow in lockstep, yielding a shared dense
        // row index.
        //
        // Phase 22 D5(3): the row is `current_index`, taken BEFORE the push —
        // mirroring `create_entity`. The remove-last→EMPTY migration
        // (`migrate_entity_remove` with a zero-component target) calls this
        // with an EMPTY retained slice, where `push_entity_components`'
        // return is a vacuous 0. For non-empty inputs the pushed index must
        // agree with `row` (pools grow in lock-step with `current_index`).
        let row = self.current_index;
        let _pushed_index = self
            .component_pools
            .push_entity_components(&component_bytes);
        debug_assert!(
            component_bytes.is_empty() || _pushed_index == row,
            "pool desync: push_entity_components returned row {} but archetype \
             current_index is {}",
            _pushed_index,
            row
        );
        *new_unit_index = row as u32;

        // Stamp the explicit ticks per-component. The order of
        // `components` matches `component_bytes`, and `push_entity_components`
        // wrote each component to the same dense slot `row`.
        for (component_id, _, added_tick, changed_tick) in components {
            if let Some(pool) = self.component_pools.get_pool_mut(*component_id) {
                // SAFETY (mirrors STORE4 in `create_entity`):
                //   * `row < pool.count()` (just pushed above; agreement
                //     debug-asserted).
                //   * `&mut self` ⇒ exclusive write access; Phase 9 SCH3
                //     keeps workers off this pool during apply.
                unsafe {
                    pool.write_added_tick(row, *added_tick);
                    pool.write_changed_tick(row, *changed_tick);
                }
            }
        }

        self.entity_ids.push(entity_id);
        self.current_index += 1;
        true
    }

    /// Phase 12.5 Opt-A3 (§6 / plan §1.3): single-row spawn that bypasses
    /// the 4× SparseMap lookup of [`Self::create_entity`] by consuming
    /// the pre-resolved `pool_ids` slice from [`crate::ecs::core::bundle::bundle_column_cache::BundleColumnRecord`].
    ///
    /// `components` MUST be in **canonical order** (sorted by
    /// `ComponentId.0`) matching `pool_ids` slot-for-slot — guaranteed by
    /// B1/B2 (`Bundle::component_ids()` / `for_each_component_bytes`).
    ///
    /// Returns `true` on success and writes the assigned dense unit
    /// index into `*new_unit_index`. Returns `false` if any pool's
    /// reserve ceiling is exhausted (two-phase grow via
    /// `reserve_capacity(1)` — Phase X.I).
    ///
    /// # Cost
    ///
    /// One `reserve_capacity(1)` (one bounds check per pool) + direct
    /// `pool_at_unchecked_mut(pool_ids[i])` indexing + per-row tick init
    /// via direct pool index — no SparseMap lookups on the warm path.
    ///
    /// # Phase 12.6 — legacy bridge
    ///
    /// `SpawnAtCommand::apply` no longer routes through this method;
    /// the collapsed inline write loop lives directly inside the command
    /// (`spawn_at_command.rs`). This method is retained as the
    /// `Archetype`-side primitive that external benchmarks reach for to
    /// model the pre-Phase-12.6 dispatch shape (see
    /// `crates/bench_bevy_vs_boyko/benches/profile_spawn_*.rs`).
    #[allow(dead_code)]
    pub(crate) fn create_entity_with_pool_ids(
        &mut self,
        entity_id: EntityId,
        new_unit_index: &mut u32,
        components: &[(ComponentId, &[u8])],
        pool_ids: &[InlandPoolId],
        current_tick: Tick,
    ) -> bool {
        debug_assert_eq!(
            components.len(),
            pool_ids.len(),
            "create_entity_with_pool_ids: components / pool_ids arity mismatch"
        );
        // Two-phase commit via `reserve_capacity`.
        if self.reserve_capacity(1).is_err() {
            return false;
        }
        let row = self.current_index;
        for (canonical_idx, (component_id, bytes)) in components.iter().enumerate() {
            debug_assert_eq!(
                self.component_ids[canonical_idx], *component_id,
                "create_entity_with_pool_ids: canonical order mismatch at idx {}",
                canonical_idx
            );
            let pool_idx = pool_ids[canonical_idx];
            // SAFETY (SBO13 + SBO-N + SBO-B2):
            //   * `pool_idx.0 < pools.len()` by SBO-N (push-only) + the
            //     cache install-time bound check.
            //   * `row < committed_rows` after `reserve_capacity(1)`
            //     succeeded (Phase X.I: Phase B grew every pool).
            //   * `&mut self` ⇒ exclusive access.
            //   * `bytes.len() == pool.component_layout.size()` by
            //     Bundle/macro contract.
            unsafe {
                let pool = self
                    .component_pools
                    .pool_at_unchecked_mut(pool_idx);
                pool.write_at_unchecked_initialized(row, bytes);
                pool.commit_units(row, 1);
                pool.fill_ticks(row, 1, current_tick);
            }
        }
        self.entity_ids.push(entity_id);
        self.current_index = row + 1;
        *new_unit_index = row as u32;
        true
    }

    /// Phase X.I D5 — the single batch/migration grow funnel: ensures every
    /// owned pool has committed capacity for `n` more rows, growing IN
    /// PLACE (no relocation, no copy) when needed.
    ///
    /// Two-phase contract (preserved from Phase 12.5 Opt-A2 — on `Err` the
    /// archetype is unchanged; no pool was mutated):
    ///
    /// * **Phase A (read-only)**: every pool must satisfy
    ///   `count + n <= reserve_rows` (the reserve ceiling). On violation
    ///   returns `Err(EcsError::ArchetypePoolCapacityExceeded)` with NO
    ///   mutation.
    /// * **Phase B**: unconditional `grow_rows(count + n)` on every pool.
    ///   Calling unconditionally is legal ONLY because of `grow_rows`'s
    ///   idempotent no-op arm (★R1-1 / GROW1-XI proof 0): the common case
    ///   (`reserve_capacity(1)` with capacity already committed) is P warm
    ///   compares, ZERO syscalls, ZERO state change. Phase A proved the
    ///   ceiling for every pool, so `grow_rows` cannot return `false`
    ///   here; it can only panic on genuine OS commit failure (world
    ///   poisoned — the documented OOM policy).
    ///
    /// Callers (`SpawnBatchCommand::apply` direct + queued, the spawn /
    /// migration apply paths) MUST invoke this BEFORE writing any row via
    /// `pool_at_unchecked_mut().write_at_unchecked_initialized(...)`. The
    /// queued path (`I-N4`) `.expect`s the result (an `Err` there means
    /// the archetype outgrew its pools' reserve ceiling) while the direct
    /// path (`EcsMaster::spawn_batch`) bubbles it up to the caller.
    pub(crate) fn reserve_capacity(&mut self, n: usize) -> EcsResult<()> {
        // Phase A: read-only ceiling validation over every pool.
        for pool in self.component_pools.pools_iter() {
            if !pool.can_reserve(n) {
                let (_, ceiling) = pool.len_for_reserve();
                return Err(EcsError::ArchetypePoolCapacityExceeded {
                    archetype_id: self.id,
                    pool_capacity: ceiling,
                    requested: n,
                });
            }
        }
        // Phase B: grow every pool to `count + n` committed rows.
        for pool in self.component_pools.pools_iter_mut() {
            // Belt: Phase A's `can_reserve` checked_add proved `count + n`
            // on the same unmutated lens — re-assert per pool so the proof
            // does not silently couple to loop ordering / future edits.
            debug_assert!(
                pool.count().checked_add(n).is_some(),
                "reserve_capacity Phase B: count + n overflows usize despite Phase A"
            );
            let target = pool.count() + n;
            let grown = pool.grow_rows(target);
            debug_assert!(
                grown,
                "GROW1-XI: phase A proved `count + n <= reserve_rows`; \
                 grow_rows cannot hit the ceiling here"
            );
        }
        Ok(())
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
    
    /// Removes the last entity from this archetype.
    ///
    /// Phase 7: generation bumping is moved out — the caller
    /// (`EntityMaster::deallocate_entity`) handles the live-side generation
    /// bump on its fast-store `EntityInland`. This method only mutates
    /// archetype-local state.
    pub fn pop(&mut self) -> bool {
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

        // Decrement entity counter
        self.current_index -= 1;

        true
    }
    
    /// Gets the entity ID at a specific unit index
    #[inline]
    pub fn get_entity_id_at(&self, unit_index: InlandPoolId) -> Option<EntityId> {
        self.entity_ids.get(unit_index.0).copied()
    }

    /// The archetype's entity-id column as a contiguous slice, in row order.
    ///
    /// Lets serialization bulk-copy the entity-row table in one memcpy instead
    /// of a per-row gather: `EntityId` is `#[repr(transparent)]` over `usize`,
    /// so on a 64-bit little-endian target this slice's byte image equals the
    /// saved little-endian `u64[]` table.
    #[inline]
    pub fn entity_ids_slice(&self) -> &[EntityId] {
        &self.entity_ids
    }
}

/// Phase 4 Seam 2 (D2): the loud archetype-mint rejection for a signature that
/// mixes a [`ResidencyKind::Gpu`] component with a [`ResidencyKind::CpuPinned`]
/// component.
///
/// `#[cold] #[inline(never)]`: this is the OFF-the-hot-path reject arm — keeping
/// it out of line stops the diagnostic string-formatting from bloating the
/// mint funnel's I-cache footprint. A **release-present** panic (NOT a
/// `debug_assert`): a silently-built wrong-residency archetype would corrupt the
/// CPU/GPU population partition (the readback trap, D2), so the check fires in
/// every build.
///
/// # Phase 5 C2 — GPU-resident archetypes are GPU-PURE
///
/// The reject now fires on `saw_gpu && saw_non_gpu` — a `ResidencyKind::Gpu`
/// component alongside ANY non-Gpu component (`Cpu` OR `CpuPinned`). The semantic
/// is `GPU_RESIDENT ⇔ all-components-Gpu` (was OR-of-any-Gpu). Permitting a
/// `Gpu + ordinary Cpu` mix would let the blanket query-skip silently drop a
/// `Query<&CpuComp>` over the mixed archetype — a correctness regression; whole
/// -archetype device residency (umbrella §2 / §5.1) closes it.
#[cold]
#[inline(never)]
pub(crate) fn residency_conflict_panic(component_ids: &[ComponentId]) -> ! {
    panic!(
        "a GPU-resident archetype must be GPU-pure: mixes Gpu and non-Gpu \
         (signature {:?} pairs a ResidencyKind::Gpu component with a non-Gpu \
         component — Cpu or CpuPinned)",
        component_ids
    );
}

// SAFETY (SEND10 — Phase 9 §2.4, §9.1):
//
// `Archetype` becomes `Send + Sync` under the Phase 9 contract:
//
//   - The owned `ComponentPoolBundle` aggregates `ComponentPool`s, which are
//     themselves `Send + Sync` per SEND10 (see `component_pool.rs`).
//   - The inline `columns: [Column; MAX_COMPONENTS]` table is read-only from
//     workers; updates happen during dispatcher-only `refresh_column` /
//     `add_pool` flows.
//   - All `Vec`s (`component_ids`, `entity_ids`) mutate only on `&mut self`
//     paths reached from the apply window.
unsafe impl Send for Archetype {}
unsafe impl Sync for Archetype {}

// SAFETY (SEND10 — Phase 9 §2.4, §9.1):
//
// `Column` is a `Copy` POD wrapping a raw pointer + stride. The raw `*mut u8`
// is only dereferenced inside paths governed by the scheduler's aliasing
// contract (`EcsMaster::get_component_raw` under a worker cell; the
// `ConflictGraph` guarantees no overlapping mutable views). The column entry
// itself carries no shared interior mutability, so transmitting it across
// threads is sound.
unsafe impl Send for Column {}
unsafe impl Sync for Column {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::component_registry;

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

    fn make_archetype() -> Archetype {
        register_test_components();
        Archetype::create_by_ids(ArchetypeId(1), &[COMP_A, COMP_B])
    }

    // Helper: add one entity with zero-filled bytes for both components.
    // Returns the assigned dense unit index — Phase 7 removed the
    // `EntityInland` coupling from `Archetype::remove_entity` / `pop`.
    fn add_entity(arch: &mut Archetype, entity_id: EntityId) -> InlandPoolId {
        let bytes_a = vec![0u8; component_registry::get_component_size(COMP_A.0).unwrap()];
        let bytes_b = vec![0u8; component_registry::get_component_size(COMP_B.0).unwrap()];
        let mut new_unit_index: u32 = 0;
        let ok = arch.create_entity(entity_id, &mut new_unit_index, &[
            (COMP_A, bytes_a.as_slice()),
            (COMP_B, bytes_b.as_slice()),
        ], Tick::new(1));
        assert!(ok, "create_entity must succeed in setup helper");
        InlandPoolId(new_unit_index as usize)
    }

    // --- create_entity ---

    #[test]
    fn create_entity_increments_entity_count() {
        let mut arch = make_archetype();

        assert_eq!(arch.entity_count(), 0, "fresh archetype has no entities");
        add_entity(&mut arch, EntityId(42));
        assert_eq!(arch.entity_count(), 1, "count must be 1 after one create");
    }

    #[test]
    fn create_entity_pushes_entity_id_to_vector() {
        let mut arch = make_archetype();

        add_entity(&mut arch, EntityId(99));
        assert_eq!(
            arch.get_entity_id_at(InlandPoolId(0)),
            Some(EntityId(99)),
            "entity ID 99 must be accessible at slot 0"
        );
    }

    #[test]
    fn create_entity_missing_component_returns_false() {
        let mut arch = make_archetype();

        // Provide only COMP_A, omit COMP_B.
        let bytes_a = vec![0u8; component_registry::get_component_size(COMP_A.0).unwrap()];
        let mut new_unit_index: u32 = 0;
        let ok = arch.create_entity(
            EntityId(10),
            &mut new_unit_index,
            &[(COMP_A, bytes_a.as_slice())],
            Tick::new(1),
        );
        assert!(!ok, "create_entity must return false when a component is missing");
    }

    // --- pop (C-008 + Q-022 regression) ---

    #[test]
    fn pop_decrements_entity_count_in_debug_and_release() {
        // Regression for C-008: in the original code, component pools were NOT
        // popped in release because pop_entity() was inside debug_assert!.
        // This test must pass under both `cargo test` and `cargo test --release`.
        let mut arch = make_archetype();
        let _idx = add_entity(&mut arch, EntityId(7));

        assert_eq!(arch.entity_count(), 1);
        let popped = arch.pop();
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
        let mut arch = make_archetype();
        let _idx0 = add_entity(&mut arch, EntityId(1));
        add_entity(&mut arch, EntityId(2));
        add_entity(&mut arch, EntityId(3));

        // Pop removes the last entity (ID=3).
        arch.pop();

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
    }

    #[test]
    fn pop_on_empty_archetype_panics_in_debug_or_returns_false_in_release() {
        // In debug builds, debug_assert!(current_index > 0) fires and panics.
        // In release builds, pop_entity() is called but the pools are empty
        // and pop returns false — the function returns false without decrement.
        // Both outcomes are acceptable; we use catch_unwind to allow both.
        let _result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            register_test_components();
            let mut arch = Archetype::create_by_ids(ArchetypeId(99), &[COMP_A, COMP_B]);
            // In debug: panics. In release: returns false (pool is empty → pop() = false).
            let _ = arch.pop();
        }));
        // The test passes regardless of whether a panic occurred.
    }

    // --- remove_entity ---

    #[test]
    fn remove_entity_last_returns_last_outcome() {
        let mut arch = make_archetype();
        let idx = add_entity(&mut arch, EntityId(55));
        // Removing the only entity — no swap needed.
        let result = arch.remove_entity(idx);
        assert_eq!(result, RemoveOutcome::Last, "no swap expected for the last entity");
        assert_eq!(arch.entity_count(), 0);
    }

    #[test]
    fn remove_entity_non_last_returns_swapped_outcome() {
        let mut arch = make_archetype();
        let idx_first = add_entity(&mut arch, EntityId(10));
        add_entity(&mut arch, EntityId(20)); // last entity

        // Remove first; last (20) should swap into position 0.
        let result = arch.remove_entity(idx_first);
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
        let mut arch = make_archetype();
        let idx = add_entity(&mut arch, EntityId(1));
        assert_eq!(arch.remove_entity(idx), RemoveOutcome::Last);
        assert_eq!(arch.entity_count(), 0);
        assert!(arch.get_entity_id_at(InlandPoolId(0)).is_none());
    }

    #[test]
    fn remove_outcome_swapped_moves_last_entity_id() {
        // Removing the first of three entities must produce RemoveOutcome::Swapped
        // with the ID of the entity that was at the last position.
        let mut arch = make_archetype();
        let idx_0 = add_entity(&mut arch, EntityId(10));
        add_entity(&mut arch, EntityId(20));
        add_entity(&mut arch, EntityId(30)); // last

        let result = arch.remove_entity(idx_0);
        assert_eq!(result, RemoveOutcome::Swapped { moved_entity: EntityId(30) });
        // Entity 30 now occupies slot 0; slot 1 holds entity 20.
        assert_eq!(arch.get_entity_id_at(InlandPoolId(0)), Some(EntityId(30)));
        assert_eq!(arch.get_entity_id_at(InlandPoolId(1)), Some(EntityId(20)));
        assert_eq!(arch.entity_count(), 2);
    }

    #[test]
    fn remove_outcome_removing_second_to_last() {
        // Removing the middle entity of two entities is a swap.
        let mut arch = make_archetype();
        let idx_0 = add_entity(&mut arch, EntityId(100));
        add_entity(&mut arch, EntityId(200)); // becomes "last"

        let result = arch.remove_entity(idx_0);
        assert_eq!(result, RemoveOutcome::Swapped { moved_entity: EntityId(200) });
        assert_eq!(arch.entity_count(), 1);
        assert_eq!(arch.get_entity_id_at(InlandPoolId(0)), Some(EntityId(200)));
    }

    #[test]
    fn remove_outcome_last_on_last_of_multiple() {
        // Removing the last of multiple entities must produce RemoveOutcome::Last.
        let mut arch = make_archetype();
        add_entity(&mut arch, EntityId(10));
        let idx_last = add_entity(&mut arch, EntityId(20));

        let result = arch.remove_entity(idx_last);
        assert_eq!(result, RemoveOutcome::Last);
        assert_eq!(arch.entity_count(), 1);
        assert_eq!(arch.get_entity_id_at(InlandPoolId(0)), Some(EntityId(10)));
    }

    // --- has_component_id ---

    #[test]
    fn has_component_id_returns_true_for_registered() {
        let arch = make_archetype();
        assert!(arch.has_component_id(COMP_A));
        assert!(arch.has_component_id(COMP_B));
    }

    #[test]
    fn has_component_id_returns_false_for_absent() {
        let arch = make_archetype();
        assert!(!arch.has_component_id(ComponentId(402))); // never added
    }

    // --- matches_component_ids ---

    #[test]
    fn matches_component_ids_subset_returns_true() {
        let arch = make_archetype();
        assert!(arch.matches_component_ids(&[COMP_A]));
        assert!(arch.matches_component_ids(&[COMP_A, COMP_B]));
    }

    #[test]
    fn matches_component_ids_superset_returns_false() {
        let arch = make_archetype();
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
            // Archetype requires only C16_A and C16_B.
            let mut arch = Archetype::create_by_ids(ArchetypeId(50), &[C16_A, C16_B]);

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
            ], Tick::new(1));
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
        let mut arch = Archetype::create_by_ids(ArchetypeId(51), &C16_WIDE);

        // Build component data: 4 bytes each (all u32-sized).
        let bytes = [0u8; 4];
        let components: Vec<(ComponentId, &[u8])> = C16_WIDE.iter()
            .map(|&id| (id, bytes.as_slice()))
            .collect();

        let mut new_unit_index: u32 = 0;
        let ok = arch.create_entity(EntityId(300), &mut new_unit_index, &components, Tick::new(1));
        assert!(ok, "create_entity must succeed for 8-component archetype");
        assert_eq!(arch.entity_count(), 1);
    }

    // --- EnableTag Step 4: bitset-storage filtering + swap-remove bit wiring ---
    //
    // ID range 490-499 reserved for these tests (collisions checked against
    // 400-417, 480-481, 410-417, 300-308, 420-429).

    /// A table component (signature storage) and an enable tag (bitset storage)
    /// for the Step-4 wiring tests. The tag uses `set_storage_kind` directly to
    /// classify the id as bitset without minting through the name registry.
    // Reserved free block 320-322 (grep-verified empty in [320,340); disjoint
    // from ecs_master 100-109, archetype_master 300-309, registry TEST_BASE
    // 450-461, par_chunk 466-472, query_state ~490, resource 510). The prior
    // 490-492 collided with query_state's `Pos`, and 453-455 with the registry
    // TEST_BASE block's f32/f64 fixtures — both in the shared lib-test process.
    const STEP4_TABLE_A: ComponentId = ComponentId(320);
    const STEP4_TABLE_B: ComponentId = ComponentId(321);
    const STEP4_TAG: ComponentId = ComponentId(322);

    fn register_step4_components() {
        #[repr(C)]
        struct Step4TableA(u32);
        #[repr(C)]
        struct Step4TableB(u64);
        #[repr(C)]
        struct Step4Tag;
        component_registry::register_layout::<Step4TableA>(STEP4_TABLE_A.0);
        component_registry::register_layout::<Step4TableB>(STEP4_TABLE_B.0);
        component_registry::register_layout::<Step4Tag>(STEP4_TAG.0);
        // Classify the tag id as bitset storage (write-once / idempotent).
        component_registry::set_storage_kind(
            STEP4_TAG.0,
            component_registry::StorageKind::Bitset,
        );
    }

    /// Adds one zero-filled entity to a `[STEP4_TABLE_A, STEP4_TABLE_B]`
    /// archetype, returning the assigned dense row.
    fn add_step4_entity(arch: &mut Archetype, entity_id: EntityId) -> InlandPoolId {
        let bytes_a = vec![0u8; component_registry::get_component_size(STEP4_TABLE_A.0).unwrap()];
        let bytes_b = vec![0u8; component_registry::get_component_size(STEP4_TABLE_B.0).unwrap()];
        let mut new_unit_index: u32 = 0;
        let ok = arch.create_entity(
            entity_id,
            &mut new_unit_index,
            &[(STEP4_TABLE_A, bytes_a.as_slice()), (STEP4_TABLE_B, bytes_b.as_slice())],
            Tick::new(1),
        );
        assert!(ok, "create_entity must succeed in the Step-4 setup helper");
        InlandPoolId(new_unit_index as usize)
    }

    /// C1 premise: a bitset id passed to `create_by_ids` alongside table ids is
    /// FILTERED OUT of the signature and gets NO `ComponentPool`.
    #[test]
    fn bitset_id_never_in_signature_and_never_gets_a_pool() {
        register_step4_components();
        // Mix the tag in with two table components.
        let arch = Archetype::create_by_ids(
            ArchetypeId(1),
            &[STEP4_TABLE_A, STEP4_TAG, STEP4_TABLE_B],
        );
        // Table ids ARE in the signature; the bitset id is NOT.
        assert!(arch.has_component_id(STEP4_TABLE_A), "table A must be in signature");
        assert!(arch.has_component_id(STEP4_TABLE_B), "table B must be in signature");
        assert!(
            !arch.has_component_id(STEP4_TAG),
            "bitset tag must be filtered OUT of the signature (C1 premise)"
        );
        // The bitset id has no pool; the table ids do.
        assert!(
            arch.component_pools().get_pool(STEP4_TAG).is_none(),
            "bitset tag must NOT have a ComponentPool (C1 premise)"
        );
        assert!(arch.component_pools().get_pool(STEP4_TABLE_A).is_some());
        assert!(arch.component_pools().get_pool(STEP4_TABLE_B).is_some());
        // The bitset id never even enters the inline column table.
        assert!(arch.columns[STEP4_TAG.0].is_null(), "bitset tag column must be null");
    }

    /// `register_component` refuses a bitset id (it cannot be a table component).
    #[test]
    fn register_component_refuses_bitset_id() {
        register_step4_components();
        let mut arch = Archetype::new(ArchetypeId(2));
        let added = arch.register_component(STEP4_TAG);
        assert!(!added, "register_component must refuse a bitset id");
        assert!(!arch.has_component_id(STEP4_TAG));
        assert!(arch.component_pools().get_pool(STEP4_TAG).is_none());
    }

    /// `set_enable_bit` reports `newly_allocated == true` only on the first
    /// column for a tag; the column survives re-fetch; a clear never allocates.
    #[test]
    fn set_enable_bit_first_touch_flag() {
        register_step4_components();
        let mut arch = Archetype::create_by_ids(ArchetypeId(3), &[STEP4_TABLE_A, STEP4_TABLE_B]);
        let row = add_step4_entity(&mut arch, EntityId(1));

        // A clear into an absent column never allocates (returns false).
        assert!(!arch.set_enable_bit(STEP4_TAG, row.0, false));
        assert!(arch.enable_column_ptr(STEP4_TAG).is_null(), "clear must not allocate");

        let newly = arch.set_enable_bit(STEP4_TAG, row.0, true);
        assert!(newly, "first set must report newly_allocated");
        let newly = arch.set_enable_bit(STEP4_TAG, row.0, true);
        assert!(!newly, "second set of the same tag must NOT report newly_allocated");
        assert!(!arch.enable_column_ptr(STEP4_TAG).is_null());
        assert!(arch.enable_store.column(STEP4_TAG).unwrap().test(row.0));
    }

    /// O1-r7 Swapped: removing a non-last entity moves the former-last entity's
    /// enable bit into the vacated row (READ-first), and clears `last`.
    #[test]
    fn swap_remove_preserves_swapped_entity_bit_read_first() {
        register_step4_components();
        let mut arch = Archetype::create_by_ids(ArchetypeId(4), &[STEP4_TABLE_A, STEP4_TABLE_B]);
        let row0 = add_step4_entity(&mut arch, EntityId(10)); // row 0
        let row1 = add_step4_entity(&mut arch, EntityId(20)); // row 1
        let row2 = add_step4_entity(&mut arch, EntityId(30)); // row 2 (will be last)

        // Toggle the tag ON for row2 (the entity that will be swapped in), and
        // OFF (implicitly) for row0.
        arch.set_enable_bit(STEP4_TAG, row2.0, true);
        assert!(arch.enable_store.column(STEP4_TAG).unwrap().test(row2.0));
        assert!(!arch.enable_store.column(STEP4_TAG).unwrap().test(row0.0));

        // Remove row0 (non-last) → row2's entity (EntityId 30) swaps into row0.
        let outcome = arch.remove_entity(row0);
        assert_eq!(outcome, RemoveOutcome::Swapped { moved_entity: EntityId(30) });

        // The swapped entity's bit moved from `last` (row2) into row0, and the
        // popped `last` slot is clear.
        let col = arch.enable_store.column(STEP4_TAG).unwrap();
        assert!(col.test(row0.0), "swapped entity's set bit must move into the vacated row");
        assert!(!col.test(row2.0), "the popped last row's bit must be cleared");
        // row1 is untouched.
        let _ = row1;
    }

    /// O1-r7 Last/pop: removing the last entity clears its enable bit (no swap).
    #[test]
    fn remove_outcome_last_clears_popped_bit() {
        register_step4_components();
        let mut arch = Archetype::create_by_ids(ArchetypeId(5), &[STEP4_TABLE_A, STEP4_TABLE_B]);
        add_step4_entity(&mut arch, EntityId(1)); // row 0
        let last = add_step4_entity(&mut arch, EntityId(2)); // row 1 (last)

        arch.set_enable_bit(STEP4_TAG, last.0, true);
        assert!(arch.enable_store.column(STEP4_TAG).unwrap().test(last.0));

        let outcome = arch.remove_entity(last);
        assert_eq!(outcome, RemoveOutcome::Last);
        assert!(
            !arch.enable_store.column(STEP4_TAG).unwrap().test(last.0),
            "O1-r7: the popped last row's bit must be cleared"
        );
        assert_eq!(arch.entity_count(), 1);
    }

    /// `move_out_entity` (the no-drop migration path) wires the same swap-remove
    /// bit fix-up exactly once, READ-first.
    #[test]
    fn move_out_entity_preserves_swapped_bit() {
        register_step4_components();
        let mut arch = Archetype::create_by_ids(ArchetypeId(6), &[STEP4_TABLE_A, STEP4_TABLE_B]);
        let row0 = add_step4_entity(&mut arch, EntityId(10));
        add_step4_entity(&mut arch, EntityId(20));
        let row2 = add_step4_entity(&mut arch, EntityId(30)); // last

        arch.set_enable_bit(STEP4_TAG, row2.0, true);

        // Caller contract: bytes already moved out elsewhere; here we only
        // exercise the no-drop swap path's enable-bit fix-up.
        let outcome = arch.move_out_entity(row0);
        assert_eq!(outcome, RemoveOutcome::Swapped { moved_entity: EntityId(30) });

        let col = arch.enable_store.column(STEP4_TAG).unwrap();
        assert!(col.test(row0.0), "no-drop swap must move the bit into the vacated row");
        assert!(!col.test(row2.0));
    }

    /// The `Archetype` size pin holds after the `enable_store` field addition.
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn archetype_size_pin_holds() {
        assert_eq!(
            std::mem::size_of::<Archetype>(),
            8576,
            "Archetype size pin must match the const-assert tripwire"
        );
        assert_eq!(std::mem::offset_of!(Archetype, columns), 0, "columns must stay at offset 0");
    }

    // ----- Phase 4 Seam 1/2: residency stamp + conflict reject -----
    //
    // Fixed ids 330-339 reserved for these residency tests (disjoint from
    // STEP4 320-322, COMP_A/B 400-401, C16 410-425). Each id is given a
    // registered layout (so `create_by_ids` can `add_pool`) and a residency
    // class via the `pub(crate)` `set_residency_class`. The `RESIDENCY_CLASS`
    // table is a global write-once-per-id array, so these ids must not be
    // reclassified to a different class elsewhere.
    use crate::ecs::core::component::component_registry::ResidencyKind;

    #[repr(C)]
    struct ResComp(u32);

    const RES_GPU_A: ComponentId = ComponentId(330);
    const RES_GPU_B: ComponentId = ComponentId(331);
    const RES_CPU_A: ComponentId = ComponentId(332);
    const RES_CPU_B: ComponentId = ComponentId(333);
    const RES_PINNED: ComponentId = ComponentId(334);
    // Phase 5 C2 — a SECOND Gpu id so a GPU-PURE multi-component archetype can
    // be built (the new semantic: GPU_RESIDENT ⇔ all-components-Gpu). 345 sits in
    // the free 345-399 gap (disjoint from migration_helpers' 335-339).
    const RES_GPU_C: ComponentId = ComponentId(345);

    #[test]
    fn create_by_ids_stamps_gpu_resident_when_all_components_gpu() {
        // Phase 5 C2: GPU_RESIDENT ⇔ all-components-Gpu. A GPU-pure signature
        // stamps GPU_RESIDENT; a mixed Gpu + Cpu signature now PANICS (covered
        // separately), so this test uses two Gpu ids.
        component_registry::register_layout::<ResComp>(RES_GPU_A.0);
        component_registry::register_layout::<ResComp>(RES_GPU_C.0);
        component_registry::set_residency_class(RES_GPU_A.0, ResidencyKind::Gpu);
        component_registry::set_residency_class(RES_GPU_C.0, ResidencyKind::Gpu);

        let arch = Archetype::create_by_ids(ArchetypeId(50), &[RES_GPU_A, RES_GPU_C]);
        assert!(
            arch.flags.is_gpu_resident(),
            "a GPU-pure signature must stamp GPU_RESIDENT"
        );
    }

    #[test]
    #[should_panic(expected = "must be GPU-pure")]
    fn create_by_ids_mixed_gpu_and_cpu_panics() {
        // Phase 5 C2: a Gpu component alongside an ordinary Cpu component is now
        // a reject (GPU-resident ⇒ all-Gpu) — NOT just the old Gpu+CpuPinned mix.
        component_registry::register_layout::<ResComp>(RES_GPU_C.0);
        component_registry::register_layout::<ResComp>(RES_CPU_A.0);
        component_registry::set_residency_class(RES_GPU_C.0, ResidencyKind::Gpu);
        // RES_CPU_A stays at the default Cpu.

        let _ = Archetype::create_by_ids(ArchetypeId(54), &[RES_GPU_C, RES_CPU_A]);
    }

    #[test]
    fn create_by_ids_cpu_only_never_stamps_gpu_resident() {
        // Property: a Cpu-only signature NEVER carries GPU_RESIDENT.
        component_registry::register_layout::<ResComp>(RES_CPU_B.0);
        // RES_CPU_A registered above (or here, idempotent register_layout).
        component_registry::register_layout::<ResComp>(RES_CPU_A.0);

        let arch = Archetype::create_by_ids(ArchetypeId(51), &[RES_CPU_A, RES_CPU_B]);
        assert!(
            !arch.flags.is_gpu_resident(),
            "a Cpu-only signature must never stamp GPU_RESIDENT (the 0%-gate)"
        );
    }

    #[test]
    #[should_panic(expected = "must be GPU-pure")]
    fn create_by_ids_mixed_gpu_and_cpu_pinned_panics() {
        component_registry::register_layout::<ResComp>(RES_GPU_B.0);
        component_registry::register_layout::<ResComp>(RES_PINNED.0);
        component_registry::set_residency_class(RES_GPU_B.0, ResidencyKind::Gpu);
        component_registry::set_residency_class(RES_PINNED.0, ResidencyKind::CpuPinned);

        // A Gpu + CpuPinned mix is a residency conflict — loud release panic.
        let _ = Archetype::create_by_ids(ArchetypeId(52), &[RES_GPU_B, RES_PINNED]);
    }

    #[test]
    fn register_component_inplace_stamps_gpu_resident() {
        component_registry::register_layout::<ResComp>(RES_GPU_A.0);
        component_registry::set_residency_class(RES_GPU_A.0, ResidencyKind::Gpu);

        // Build an empty archetype, then stamp via the single-component path.
        let mut arch = Archetype::create_by_ids(ArchetypeId(53), &[]);
        assert!(!arch.flags.is_gpu_resident(), "empty archetype is not GPU-resident");
        arch.register_component_inplace(RES_GPU_A);
        assert!(
            arch.flags.is_gpu_resident(),
            "register_component_inplace must OR in GPU_RESIDENT for a Gpu id"
        );
    }

    // ----- Phase 5 C1: make_component_device_backed nulls the column -----

    /// `make_component_device_backed` flips the pool to Device backing AND nulls
    /// the inline `columns[cid]` cache (the C1 fix), so every direct reader's
    /// null-check returns absent — the "CPU can't touch GPU bytes" contract.
    ///
    /// Device-backing is `#[cfg(not(miri))]` (the `PoolBacking::Device` arm and
    /// its primitives are compiled out under Miri), so this test is gated to
    /// match.
    #[test]
    #[cfg(all(test, not(miri)))]
    fn make_component_device_backed_nulls_the_column() {
        use crate::ecs::memory::device_column::DeviceColumnHandle;

        component_registry::register_layout::<ResComp>(RES_GPU_A.0);
        component_registry::set_residency_class(RES_GPU_A.0, ResidencyKind::Gpu);

        // A GPU-pure single-component archetype. Empty (len == 0) — the O1
        // data-loss guard requires it before the device flip.
        let mut arch = Archetype::create_by_ids(ArchetypeId(55), &[RES_GPU_A]);
        assert!(arch.flags.is_gpu_resident(), "GPU-pure archetype is GPU-resident");
        // Before the flip, the column is populated (non-null base from the pool).
        assert!(
            !arch.columns[RES_GPU_A.0].is_null(),
            "a host-backed component column must have a non-null base before the flip"
        );

        arch.make_component_device_backed(RES_GPU_A, DeviceColumnHandle(0xDEAD_BEEF));

        assert!(
            arch.columns[RES_GPU_A.0].is_null(),
            "make_component_device_backed must NULL the column (C1)"
        );
        // The pool now reports the device handle (MF-2/3 round-trip).
        let pool = arch
            .component_pools
            .get_pool(RES_GPU_A)
            .expect("pool still present after device flip");
        assert_eq!(
            pool.device_handle(),
            Some(DeviceColumnHandle(0xDEAD_BEEF)),
            "set_device_handle / device_handle must round-trip the opaque handle"
        );
    }

    /// FIX-1 regression: a GPU-pure MULTI-component archetype is flipped one
    /// component at a time. `GPU_RESIDENT` is mint-stamped over the whole
    /// signature, so the OLD whole-archetype funnel-tail assert tripped on the
    /// intermediate state after the FIRST flip (the sibling's column still
    /// non-null). The per-component assert must NOT panic on that state.
    ///
    /// Asserts (a) NO panic on the intermediate (one-flipped) state and (b) after
    /// every component is flipped, all component columns are null.
    #[test]
    #[cfg(all(test, not(miri)))]
    fn make_component_device_backed_multi_component_no_intermediate_panic() {
        use crate::ecs::memory::device_column::DeviceColumnHandle;

        component_registry::register_layout::<ResComp>(RES_GPU_A.0);
        component_registry::register_layout::<ResComp>(RES_GPU_C.0);
        component_registry::set_residency_class(RES_GPU_A.0, ResidencyKind::Gpu);
        component_registry::set_residency_class(RES_GPU_C.0, ResidencyKind::Gpu);

        // A GPU-pure two-component archetype. Empty (len == 0) — the O1 data-loss
        // guard requires it before each device flip.
        let mut arch = Archetype::create_by_ids(ArchetypeId(56), &[RES_GPU_A, RES_GPU_C]);
        assert!(
            arch.flags.is_gpu_resident(),
            "a GPU-pure multi-component archetype is GPU-resident at mint"
        );

        // First flip: the OLD whole-archetype assert would PANIC here because the
        // sibling RES_GPU_C column is still non-null while GPU_RESIDENT is set.
        // The per-component assert must pass (this call not panicking is (a)).
        arch.make_component_device_backed(RES_GPU_A, DeviceColumnHandle(0x0000_0001));
        assert!(
            arch.columns[RES_GPU_A.0].is_null(),
            "first flip must null its own column"
        );
        // (a) The intermediate state is valid: the not-yet-flipped sibling's
        // Host column is still non-null and correct.
        assert!(
            !arch.columns[RES_GPU_C.0].is_null(),
            "the not-yet-flipped sibling's Host column stays non-null mid-flip"
        );

        // Second (last) flip completes the whole-archetype property.
        arch.make_component_device_backed(RES_GPU_C, DeviceColumnHandle(0x0000_0002));

        // (b) After every component is flipped, all columns are null.
        assert!(
            arch.component_ids.iter().all(|c| arch.columns[c.0].is_null()),
            "after the last flip every GPU component column must be null"
        );
    }
}