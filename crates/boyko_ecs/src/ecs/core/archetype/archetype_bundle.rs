//! Fixed-size slab storage for [`Archetype`]s with stable pointer addresses.
//!
//! Phase 7 Step 4 rewrite (see `docs/plans/PHASE-07-fast-random-access.md`).
//! Replaces the previous `Vec<Archetype>` + `SparseMap` backing with a
//! `Box<[MaybeUninit<Archetype>; MAX_ARCHETYPES]>` slab plus a 1024-bit
//! occupancy bitset, an `id_to_slot` Vec, and a LIFO free-slot stack. The
//! load-bearing property of the slab is that the heap address of any
//! occupied slot is **stable for the lifetime of the bundle** — this is
//! invariant U1, the foundation that lets `EntityInland` store raw
//! `*mut Archetype` pointers in later Phase 7 steps without ever dangling
//! across `create_archetype` calls.

use core::ptr::{self, addr_of_mut};
use std::mem::MaybeUninit;
use std::ops::{Index, IndexMut};

use crate::ecs::core::archetype::archetype::{Archetype, Column};
use crate::ecs::core::archetype::archetype_signature::ArchetypeSignature;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::component::component_pool_bundle::ComponentPoolBundle;
use crate::ecs::core::component::component_registry::MAX_COMPONENTS;
use crate::ecs::core::entity::entity_inland::EntityInland;
use crate::ecs::core::iters::MAX_ARCHETYPES;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId, InlandArchetypeId};
use crate::ecs::memory::arena::Arena;

/// Number of `u64` words backing the occupancy bitset (`MAX_ARCHETYPES / 64`).
const SLAB_WORDS: usize = MAX_ARCHETYPES / 64;

// Compile-time bounds-checking against the macro layout. If `MAX_ARCHETYPES`
// is ever changed to a non-multiple-of-64 or grown past `u16::MAX`, these
// asserts trip before the bundle silently corrupts memory.
const _: () = assert!(MAX_ARCHETYPES.is_multiple_of(64), "MAX_ARCHETYPES must be a multiple of 64");
const _: () = assert!(SLAB_WORDS == 16, "SLAB_WORDS must equal 16 for MAX_ARCHETYPES=1024");
const _: () = assert!(MAX_ARCHETYPES <= u16::MAX as usize, "slot indices must fit in u16");

/// Sentinel marker in `id_to_slot` for "no slot for this archetype id".
const NO_SLOT: u16 = u16::MAX;

/// Returned by [`ArchetypeBundle::add_archetype_from_components_fallible`]
/// when the slab is full (1024 archetypes registered with no free slot).
///
/// Phase-7 callers (`ArchetypeMaster::create_archetype`,
/// `EcsMaster::create_archetype`) panic via
/// `expect("invariant: bundle below MAX_ARCHETYPES")`; a future
/// `try_create_archetype` API may surface the error directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundleFullError;

impl std::fmt::Display for BundleFullError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ArchetypeBundle is full (MAX_ARCHETYPES = {MAX_ARCHETYPES})")
    }
}

impl std::error::Error for BundleFullError {}

/// Slab-backed collection of [`Archetype`]s with stable pointer addresses.
///
/// # Layout & invariants (Phase 7 D1 / U1, U2, U8, U11, U12, U13)
///
/// - `slots` is a [`Box`] of `[MaybeUninit<Archetype>; MAX_ARCHETYPES]`.
///   The `Box` is allocated once in [`Self::new`] and **never reassigned**.
///   The slab base address is therefore stable for the bundle's lifetime
///   (U1) and outlives every pointer minted from it (U2).
/// - `occupied[w]` bit `b` is set iff slot `w * 64 + b` is fully initialised.
///   `Drop` walks the bitset and calls `drop_in_place` per occupied slot
///   exactly once (U12).
/// - Slot indices are minted from `free_slots` (LIFO recycled) or
///   `count` (next-fresh) and stay below `MAX_ARCHETYPES`.
/// - `id_to_slot` is sparse-indexed by `ArchetypeId.0`. Absent entries hold
///   `NO_SLOT = u16::MAX` (also used while the Vec is grown). The Vec is
///   resized lazily on the first access to a higher id.
pub struct ArchetypeBundle {
    /// Heap-allocated fixed-size slab. Private — never reassigned after `new()`.
    slots: Box<[MaybeUninit<Archetype>; MAX_ARCHETYPES]>,
    /// Occupancy bitset; bit `w*64 + b` is set iff `slots[w*64 + b]` is live.
    occupied: [u64; SLAB_WORDS],
    /// Sparse map `ArchetypeId.0 → slot index`. `NO_SLOT` (= `u16::MAX`)
    /// means the id has no slot. The Vec grows on demand only when a
    /// previously-unseen larger id is registered.
    id_to_slot: Vec<u16>,
    /// LIFO stack of slot indices freed by `remove_archetype` / `clear`.
    /// Recycled by `add_archetype_from_components_fallible` before bumping
    /// `count`.
    free_slots: Vec<u16>,
    /// Number of occupied slots. `popcount(occupied) == count` is the
    /// consistency invariant maintained across all mutating methods.
    count: usize,
}

impl Default for ArchetypeBundle {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl ArchetypeBundle {
    /// Allocates a fresh slab and returns an empty bundle.
    ///
    /// The slab is heap-allocated directly via `Box::new_uninit`, never
    /// constructed on the stack (the 8.4 MB `[MaybeUninit<Archetype>; 1024]`
    /// stack temporary would overflow Windows' default 1 MB main thread
    /// stack — W6 fix).
    #[cold]
    pub fn new() -> Self {
        // SAFETY (slab init / C3):
        //   `Box::<T>::new_uninit()` allocates space for `T` on the heap and
        //   returns `Box<MaybeUninit<T>>`. For `T = [MaybeUninit<Archetype>; N]`
        //   the resulting allocation is uninitialised memory of the correct
        //   size and alignment, sized via the heap allocator with no stack
        //   construction of the 8.4 MB temporary.
        //
        //   `assume_init()` is sound because the array element type is
        //   `MaybeUninit<Archetype>`: an array of `MaybeUninit<U>` is itself
        //   always "initialised" in the type-system sense (every element is
        //   `MaybeUninit`, which has no validity requirement). Per-slot
        //   initialisation is tracked separately via `self.occupied`.
        //
        //   `Box::new_uninit` is stable since Rust 1.82; boyko-engine targets
        //   Rust 2024 (≥ 1.93).
        let slots = unsafe {
            Box::<[MaybeUninit<Archetype>; MAX_ARCHETYPES]>::new_uninit().assume_init()
        };

        Self {
            slots,
            occupied: [0u64; SLAB_WORDS],
            id_to_slot: Vec::new(),
            free_slots: Vec::with_capacity(16),
            count: 0,
        }
    }

    /// Legacy shim preserving the previous bundle API.
    ///
    /// The slab is fixed-size — the `capacity` argument is ignored.
    /// Phase 7 Step 5 may drop this constructor entirely; for now it
    /// forwards to [`Self::new`] so existing callers
    /// (e.g. `ArchetypeMaster::with_capacity`) keep compiling.
    #[inline]
    #[cold]
    pub fn with_capacity(_capacity: usize) -> Self {
        Self::new()
    }

    /// Returns a shared reference to the archetype with `index`, if present.
    #[inline]
    pub fn get_archetype(&self, index: ArchetypeId) -> Option<&Archetype> {
        let ptr = self.get_archetype_ptr(index)?;
        // SAFETY (U1, U2, U8, U11):
        //   - U11: pointer was minted via raw arithmetic from the Box's
        //     heap base inside `get_archetype_ptr`; no `&MaybeUninit`
        //     reborrow was taken along the way. Going through
        //     `Index<...>` on `self.slots` would materialise a transient
        //     `&MaybeUninit<Archetype>` whose borrow-stack pop could
        //     later retag-conflict with raw `*mut Archetype` pointers
        //     held by `EntityInland` (Step 7).
        //   - U8: `get_archetype_ptr` consults `slot_for_id`, which only
        //     succeeds when the occupancy bit confirms the slot is
        //     fully initialised; `&*ptr` is therefore reading a live
        //     `Archetype`.
        //   - U1/U2: the slab is heap-stable for the bundle's lifetime;
        //     the `&self` borrow bounds the returned reference's lifetime
        //     to that of `self`, blocking concurrent mutation.
        Some(unsafe { &*ptr })
    }

    /// Returns a unique reference to the archetype with `index`, if present.
    #[inline]
    pub fn get_archetype_mut(&mut self, index: ArchetypeId) -> Option<&mut Archetype> {
        let ptr = self.get_archetype_ptr_mut(index)?;
        // SAFETY (U1, U2, U8, U11):
        //   - U11: `get_archetype_ptr_mut` mints `ptr` from
        //     `self.slots.as_mut_ptr()` (write-capable provenance) via
        //     raw arithmetic; no `&mut MaybeUninit<Archetype>` reborrow
        //     is ever created. Routing through the `&self`-flavoured
        //     `get_archetype_ptr` instead would yield `*const`-provenance
        //     (Frozen under Tree Borrows), and the `&mut *ptr` write
        //     access would be UB.
        //   - U8: occupancy bit verified inside the helper.
        //   - U1/U2: slab is heap-stable; `&mut self` gives exclusive
        //     access to the slab, so no other live borrow into this
        //     slot exists for the duration of the returned reference.
        Some(unsafe { &mut *ptr })
    }

    /// Returns a write-capable raw `*mut Archetype` pointer to the slot for
    /// `archetype_id`, or `None` if no slot is registered for that id.
    ///
    /// Phase 7 C4 / U11 + U1 — pointer minting recipe with write-capable
    /// provenance. The pointer is minted via raw arithmetic on
    /// `self.slots.as_mut_ptr()`; no `&mut MaybeUninit<Archetype>` reborrow
    /// is created along the way. The `as_mut_ptr()` mint is **load-bearing**:
    /// under Tree Borrows the `&self`-flavoured [`Self::get_archetype_ptr`]
    /// returns `SharedReadOnly`-tagged provenance, and a `*const → *mut`
    /// laundering cast would not grant write capability — child-writes
    /// through that laundered pointer trip retag UB. Callers needing a
    /// read-only pointer use [`Self::get_archetype_ptr`]; this method is
    /// reserved for the write path (Step 7's `EntityInland` storage, the
    /// `&mut Archetype` rematerialisation inside `EcsMaster::create_entity`,
    /// and the internal safe accessor [`Self::get_archetype_mut`]).
    ///
    /// Slab base is stable for the bundle's lifetime (U1), so the pointer
    /// remains valid until the slot is removed or the bundle is dropped.
    #[inline]
    pub fn get_archetype_ptr_mut(
        &mut self,
        archetype_id: ArchetypeId,
    ) -> Option<*mut Archetype> {
        let raw_id = archetype_id.0;
        if raw_id >= self.id_to_slot.len() {
            return None;
        }
        let slot_idx = self.id_to_slot[raw_id];
        if slot_idx == NO_SLOT {
            return None;
        }
        debug_assert!((slot_idx as usize) < MAX_ARCHETYPES);
        // SAFETY (U11 + U1): mint from `as_mut_ptr()` so the resulting
        //   pointer carries write-capable provenance; no `&mut MaybeUninit`
        //   reborrow is created along the way. Slab base is heap-stable
        //   for the bundle's lifetime (U1). `slot_idx < MAX_ARCHETYPES` is
        //   enforced by the `id_to_slot` invariant (only ever populated
        //   with indices emitted under that bound by `add_archetype_*`).
        let slab_base: *mut MaybeUninit<Archetype> = self.slots.as_mut_ptr();
        let slot_ptr_mu: *mut MaybeUninit<Archetype> =
            unsafe { slab_base.add(slot_idx as usize) };
        Some(slot_ptr_mu as *mut Archetype)
    }

    /// Returns a read-only raw `*const Archetype` pointer to the slot for
    /// `archetype_id`, or `None` if no slot is registered for that id.
    ///
    /// Phase 7 C4 / U11 — pointer minting recipe. The pointer is minted via
    /// raw arithmetic on `self.slots.as_ptr()` (read-only provenance); no
    /// `&MaybeUninit<Archetype>` reborrow is ever materialised. Under
    /// Stacked Borrows / Tree Borrows, materialising `&MaybeUninit<Archetype>`
    /// and casting through to a `*const Archetype` would retag against the
    /// reference's borrow-stack frame; the raw-arithmetic recipe sidesteps
    /// that by never producing a reference to the slot.
    ///
    /// # Provenance contract
    /// Callers may **only read** through the returned pointer. The pointer
    /// carries `SharedReadOnly` provenance under Tree Borrows; casting it
    /// to `*mut Archetype` and dereferencing for write is UB. For write
    /// access, obtain a fresh pointer via [`Self::get_archetype_ptr_mut`].
    ///
    /// Slab base is stable for the bundle's lifetime (U1), so the pointer
    /// remains valid until the slot is removed or the bundle is dropped.
    #[inline]
    pub fn get_archetype_ptr(&self, archetype_id: ArchetypeId) -> Option<*const Archetype> {
        let raw_id = archetype_id.0;
        if raw_id >= self.id_to_slot.len() {
            return None;
        }
        let slot_idx = self.id_to_slot[raw_id];
        if slot_idx == NO_SLOT {
            return None;
        }
        debug_assert!((slot_idx as usize) < MAX_ARCHETYPES);
        // SAFETY (U11 — pointer minting recipe / U1 — slab stability):
        //   `self.slots.as_ptr()` returns a `*const [MaybeUninit<Archetype>; N]`
        //   without creating any & or &mut reference to a slab element. We
        //   cast to `*const MaybeUninit<Archetype>` and use `.add(slot_idx)`
        //   arithmetic to land on the slot. The resulting `*const Archetype`
        //   carries Box's heap-allocation provenance directly with read-only
        //   capability; subsequent reads through it cannot retag against a
        //   stale `&MaybeUninit` borrow stack because no such borrow was
        //   ever created. Slab base is stable for the bundle's lifetime
        //   (U1). `slot_idx < MAX_ARCHETYPES` is enforced by the
        //   `id_to_slot` invariant.
        let slab_base: *const MaybeUninit<Archetype> = self.slots.as_ptr().cast();
        let slot_ptr_mu: *const MaybeUninit<Archetype> = unsafe { slab_base.add(slot_idx as usize) };
        Some(slot_ptr_mu as *const Archetype)
    }

    /// In-place slab construction of a new archetype.
    ///
    /// Phase 7 W6 / U13 — never builds the 8.4 KB `Archetype` on the stack;
    /// every field is initialised through `addr_of_mut!.write()` directly
    /// into the slab slot, and the inline `columns` array is zero-filled
    /// via `write_bytes` (the all-zero bit pattern equals `Column::null()`
    /// by const-assert in `archetype.rs`).
    ///
    /// Returns the assigned slot index on success, [`BundleFullError`] when
    /// the slab is full. Phase-7 upper layers panic on `BundleFullError`;
    /// the typed handle is preserved here for a future Result-returning API.
    pub fn add_archetype_from_components_fallible(
        &mut self,
        archetype_id: ArchetypeId,
        component_ids: &[ComponentId],
        arena: &Arena,
    ) -> Result<u16, BundleFullError> {
        let slot_idx: u16 = if let Some(idx) = self.free_slots.pop() {
            idx
        } else {
            if self.count >= MAX_ARCHETYPES {
                return Err(BundleFullError);
            }
            self.count as u16
        };

        // Build the signature mask up-front so it can be written directly
        // into the slot — avoids an &mut self.signature reborrow after
        // in-place construction.
        let mask = ComponentMask::from_components(component_ids);
        let signature = ArchetypeSignature::new(mask);

        // SAFETY (U11 — pointer minting recipe):
        //   Same pattern as `get_archetype_ptr`: mint a `*mut MaybeUninit<Archetype>`
        //   from `self.slots.as_mut_ptr()` and cast to `*mut Archetype` without
        //   creating any & or &mut reference to the slot. `slot_idx < MAX_ARCHETYPES`
        //   is enforced above (capacity check + `free_slots` only ever holds
        //   valid indices that were emitted under the same bound).
        let slab_base: *mut MaybeUninit<Archetype> = self.slots.as_mut_ptr();
        // SAFETY: `slot_idx as usize < MAX_ARCHETYPES` and the slab has
        //   `MAX_ARCHETYPES` elements, so `add(slot_idx)` stays in-bounds.
        let slot_ptr_mu: *mut MaybeUninit<Archetype> = unsafe { slab_base.add(slot_idx as usize) };
        let slot_ptr: *mut Archetype = slot_ptr_mu as *mut Archetype;

        // SAFETY (U13 — in-place archetype construction):
        //   `slot_ptr` points at uninitialised but properly-sized and
        //   -aligned memory for one `Archetype` inside the slab. Each
        //   field of `Archetype` is written exactly once via
        //   `addr_of_mut!.write()` (no intermediate `&mut` reborrow), so
        //   no stack-allocated 8.4 KB `Archetype` temporary is constructed
        //   (Windows main-thread stack is 1 MB by default — W6).
        //
        //   The `columns` array is initialised by zero-filling
        //   `MAX_COMPONENTS * size_of::<Column>()` bytes; the all-zero
        //   bit pattern equals `Column::null()` by const-assert in
        //   `archetype.rs::Column` (Phase 7 U5). Writing zero bytes is
        //   sound because every byte position belongs to the (uninit)
        //   slot allocation.
        unsafe {
            ptr::write_bytes(
                addr_of_mut!((*slot_ptr).columns).cast::<Column>(),
                0u8,
                MAX_COMPONENTS,
            );
            addr_of_mut!((*slot_ptr).id).write(archetype_id);
            addr_of_mut!((*slot_ptr).component_pools).write(ComponentPoolBundle::new());
            addr_of_mut!((*slot_ptr).current_index).write(0usize);
            addr_of_mut!((*slot_ptr).signature).write(signature);
            addr_of_mut!((*slot_ptr).arena).write(arena as *const Arena);
            addr_of_mut!((*slot_ptr).component_ids).write(component_ids.to_vec());
            addr_of_mut!((*slot_ptr).entity_ids).write(Vec::new());
        }

        // All fields are now initialised; promote the raw pointer to a
        // unique reference to register the component pools and refresh
        // the inline column entries (U13 continuation). The reference is
        // scoped strictly to this loop; no aliasing pointer to the slot
        // is live for its duration.
        // SAFETY (U13 continuation): every field of `*slot_ptr` was just
        //   initialised above. No other reference to this slot exists
        //   (private slab + `&mut self` borrow). The `&mut Archetype`
        //   reborrow is sound and stays inside this function.
        let archetype: &mut Archetype = unsafe { &mut *slot_ptr };
        for &cid in component_ids {
            archetype.register_component_inplace(cid, arena);
        }

        // Set the occupancy bit only after full initialisation.
        let word = (slot_idx as usize) / 64;
        let bit = (slot_idx as usize) % 64;
        self.occupied[word] |= 1u64 << bit;

        // Grow `id_to_slot` on demand and record the mapping.
        let raw_id = archetype_id.0;
        if raw_id >= self.id_to_slot.len() {
            self.id_to_slot.resize(raw_id + 1, NO_SLOT);
        }
        self.id_to_slot[raw_id] = slot_idx;

        self.count += 1;
        Ok(slot_idx)
    }

    /// Creates a new archetype from a slice of component IDs and registers it.
    ///
    /// Wraps [`Self::add_archetype_from_components_fallible`] and panics if
    /// the slab is full — Phase-7 callers (`ArchetypeMaster::create_archetype`)
    /// treat overflow as an invariant violation. A future `try_create_archetype`
    /// surface may use the fallible variant directly.
    #[inline]
    pub fn add_archetype_from_components(
        &mut self,
        archetype_id: ArchetypeId,
        component_ids: &[ComponentId],
        arena: &Arena,
    ) -> InlandArchetypeId {
        let slot_idx = self
            .add_archetype_from_components_fallible(archetype_id, component_ids, arena)
            .expect("invariant: archetype bundle below MAX_ARCHETYPES");
        InlandArchetypeId(slot_idx as usize)
    }

    /// Inserts an already-constructed [`Archetype`] into the slab.
    ///
    /// **Legacy path** — prefer [`Self::add_archetype_from_components`] for the
    /// in-place construction recipe (W6). This method is the migration glue
    /// for [`ArchetypeMaster::add_existing_archetype`]; it performs a single
    /// `ptr::write` move from the caller-owned `Archetype` into the slab
    /// slot. The caller pays the 8.4 KB stack frame on this path; this is
    /// acceptable because the call site is not on the hot path.
    ///
    /// If an archetype with the same id already exists, its slot is
    /// overwritten: the previous occupant is `drop_in_place`'d first to
    /// avoid leaking its `Vec` fields.
    pub fn add_archetype(&mut self, archetype: Archetype) -> InlandArchetypeId {
        let archetype_id = archetype.id();
        let raw_id = archetype_id.0;

        // Replace path: same id already registered → drop the old occupant
        // and overwrite the slot in place.
        if raw_id < self.id_to_slot.len() && self.id_to_slot[raw_id] != NO_SLOT {
            let slot_idx = self.id_to_slot[raw_id];
            let slab_base: *mut MaybeUninit<Archetype> = self.slots.as_mut_ptr();
            // SAFETY (U11): mint via raw arithmetic; slot_idx is in-bounds
            //   because it was emitted by an earlier `add_*` under the
            //   `slot_idx < MAX_ARCHETYPES` invariant.
            let slot_ptr: *mut Archetype =
                unsafe { slab_base.add(slot_idx as usize) as *mut Archetype };
            // SAFETY (U12): the occupancy bit confirms a valid Archetype
            //   currently lives at `slot_ptr`. `drop_in_place` runs its
            //   destructor exactly once; we then immediately overwrite the
            //   slot via `ptr::write` so the slab slot is never partially
            //   initialised once this function returns.
            unsafe {
                ptr::drop_in_place(slot_ptr);
                ptr::write(slot_ptr, archetype);
            }
            return InlandArchetypeId(slot_idx as usize);
        }

        // Fresh insert path: allocate a slot index (LIFO recycled if any).
        let slot_idx: u16 = if let Some(idx) = self.free_slots.pop() {
            idx
        } else {
            // Match the panic discipline of
            // `add_archetype_from_components` — the upper layers treat
            // bundle overflow as a hard invariant violation in Phase 7.
            assert!(self.count < MAX_ARCHETYPES, "invariant: archetype bundle below MAX_ARCHETYPES");
            self.count as u16
        };

        let slab_base: *mut MaybeUninit<Archetype> = self.slots.as_mut_ptr();
        // SAFETY (U11): mint via raw arithmetic; slot_idx is in-bounds by
        //   the same invariant as above.
        let slot_ptr: *mut Archetype =
            unsafe { slab_base.add(slot_idx as usize) as *mut Archetype };
        // SAFETY (U13 — move-into-slot variant): `slot_ptr` is
        //   uninitialised slab memory of the correct size/alignment; we
        //   transfer ownership of `archetype` byte-wise into the slot
        //   without invoking its destructor. After this line, the slot is
        //   fully initialised and the local `archetype` binding is logically
        //   moved (no further use is permitted).
        unsafe { ptr::write(slot_ptr, archetype) };

        // Publish: set occupancy bit, record id mapping, bump count.
        let word = (slot_idx as usize) / 64;
        let bit = (slot_idx as usize) % 64;
        self.occupied[word] |= 1u64 << bit;
        if raw_id >= self.id_to_slot.len() {
            self.id_to_slot.resize(raw_id + 1, NO_SLOT);
        }
        self.id_to_slot[raw_id] = slot_idx;
        self.count += 1;

        InlandArchetypeId(slot_idx as usize)
    }

    /// Removes the archetype with the given id and returns whether anything
    /// was removed. Recycles the slot index onto the free list (LIFO).
    pub fn remove_archetype(&mut self, archetype_id: ArchetypeId) -> bool {
        let raw_id = archetype_id.0;
        if raw_id >= self.id_to_slot.len() {
            return false;
        }
        let slot_idx = self.id_to_slot[raw_id];
        if slot_idx == NO_SLOT {
            return false;
        }

        let slab_base: *mut MaybeUninit<Archetype> = self.slots.as_mut_ptr();
        // SAFETY (U11): mint via raw arithmetic; slot index in-bounds.
        let slot_ptr: *mut Archetype =
            unsafe { slab_base.add(slot_idx as usize) as *mut Archetype };
        // SAFETY (U12): the occupancy bit (verified via `id_to_slot != NO_SLOT`)
        //   guarantees the slot is initialised. `drop_in_place` runs the
        //   destructor exactly once; afterwards we clear the bit so the
        //   slot is treated as `MaybeUninit` for all future access.
        unsafe { ptr::drop_in_place(slot_ptr) };

        let word = (slot_idx as usize) / 64;
        let bit = (slot_idx as usize) % 64;
        self.occupied[word] &= !(1u64 << bit);

        self.id_to_slot[raw_id] = NO_SLOT;
        self.free_slots.push(slot_idx);
        self.count -= 1;
        true
    }

    /// Returns the archetype owning `entity_inland`, if any.
    ///
    /// Reads the legacy `archetype_id()` accessor on `EntityInland` and
    /// delegates to [`Self::get_archetype`]. Preserved for callers in
    /// `entity_master.rs` during the Phase 7 shim window; once Step 9 drops
    /// the legacy inland the method goes with it.
    #[inline]
    pub fn get_entity_archetype(&self, entity_inland: &EntityInland) -> Option<&Archetype> {
        self.get_archetype(entity_inland.archetype_id())
    }

    /// Returns a unique reference to the archetype owning `entity_inland`,
    /// if any. Same shim status as [`Self::get_entity_archetype`].
    #[inline]
    pub fn get_entity_archetype_mut(
        &mut self,
        entity_inland: &EntityInland,
    ) -> Option<&mut Archetype> {
        self.get_archetype_mut(entity_inland.archetype_id())
    }

    /// Returns the number of occupied archetype slots.
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Returns `true` when no archetypes are registered.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns the internal slot index for an archetype id, if registered.
    #[inline]
    pub fn get_inland_id(&self, archetype_id: ArchetypeId) -> Option<InlandArchetypeId> {
        let raw_id = archetype_id.0;
        let slot_idx = *self.id_to_slot.get(raw_id)?;
        if slot_idx == NO_SLOT {
            return None;
        }
        Some(InlandArchetypeId(slot_idx as usize))
    }

    /// Drops every occupied archetype and resets the bundle to empty.
    ///
    /// Walks the occupancy bitset via BLSR (`word & (word - 1)`) and runs
    /// each `Archetype`'s destructor exactly once (U12). After the walk the
    /// bitset, `id_to_slot`, `free_slots`, and `count` are reset; the slab
    /// allocation is retained.
    pub fn clear(&mut self) {
        let slab_base: *mut MaybeUninit<Archetype> = self.slots.as_mut_ptr();
        for word_idx in 0..SLAB_WORDS {
            let mut word = self.occupied[word_idx];
            while word != 0 {
                let bit = word.trailing_zeros() as usize;
                let slot_idx = word_idx * 64 + bit;
                // SAFETY (U12): the bit indicates an initialised slot;
                //   `drop_in_place` runs its destructor exactly once. After
                //   the loop we clear the entire occupancy bitset, so the
                //   slot is then treated as `MaybeUninit`.
                unsafe {
                    let slot_ptr: *mut Archetype = slab_base.add(slot_idx) as *mut Archetype;
                    ptr::drop_in_place(slot_ptr);
                }
                word &= word - 1;
            }
            self.occupied[word_idx] = 0;
        }
        self.id_to_slot.clear();
        self.free_slots.clear();
        self.count = 0;
    }

    /// Returns an iterator yielding raw read-only `*const Archetype` pointers
    /// to every occupied slot, in ascending slot-index order.
    ///
    /// Pointers are stable for the bundle's lifetime (U1). Callers may only
    /// read through these pointers — Stacked Borrows tags pointers minted
    /// from `&self` as `SharedReadOnly`, and writes through them would trip
    /// retag UB. For write access use [`Self::iter_occupied_ptrs_mut`].
    ///
    /// Bitset walk via TZCNT (`u64::trailing_zeros`) and BLSR
    /// (`word & word.wrapping_sub(1)`) — `O(popcount(occupied))`.
    #[inline]
    pub fn iter_occupied_ptrs(&self) -> impl Iterator<Item = *const Archetype> + '_ {
        let slab_base: *const MaybeUninit<Archetype> = self.slots.as_ptr();
        let occupied = self.occupied;
        (0..SLAB_WORDS).flat_map(move |word_idx| {
            let mut word = occupied[word_idx];
            core::iter::from_fn(move || {
                if word == 0 {
                    return None;
                }
                let bit = word.trailing_zeros() as usize;
                word &= word.wrapping_sub(1);
                let slot_idx = word_idx * 64 + bit;
                // SAFETY (U8 + U11 + U1): the occupancy bit guarantees an
                //   initialised slot; `slab_base.add(slot_idx)` is in-bounds
                //   because `slot_idx < MAX_ARCHETYPES`; minted via raw
                //   arithmetic (no `&MaybeUninit` reborrow). Slab base is
                //   stable for the bundle's lifetime (U1). Provenance is
                //   read-only (`*const`), matching the `&self` borrow.
                let slot_ptr_mu: *const MaybeUninit<Archetype> =
                    unsafe { slab_base.add(slot_idx) };
                Some(slot_ptr_mu as *const Archetype)
            })
        })
    }

    /// Returns an iterator yielding raw `*mut Archetype` pointers to every
    /// occupied slot, in ascending slot-index order.
    ///
    /// Mirrors [`Self::iter_occupied_ptrs`] but takes `&mut self` so the
    /// returned pointers carry write provenance under Stacked Borrows /
    /// Tree Borrows. Pointers are stable for the bundle's lifetime (U1).
    #[inline]
    pub fn iter_occupied_ptrs_mut(
        &mut self,
    ) -> impl Iterator<Item = *mut Archetype> + '_ {
        let slab_base: *mut MaybeUninit<Archetype> = self.slots.as_mut_ptr();
        let occupied = self.occupied;
        (0..SLAB_WORDS).flat_map(move |word_idx| {
            let mut word = occupied[word_idx];
            core::iter::from_fn(move || {
                if word == 0 {
                    return None;
                }
                let bit = word.trailing_zeros() as usize;
                word &= word.wrapping_sub(1);
                let slot_idx = word_idx * 64 + bit;
                // SAFETY (U8 + U11 + U1): occupancy bit ⇒ slot fully
                //   initialised; `slot_idx < MAX_ARCHETYPES`; raw arithmetic
                //   mint from the `&mut self` slab base, preserving write
                //   provenance. Slab base is stable for the bundle's
                //   lifetime (U1).
                let slot_ptr_mu: *mut MaybeUninit<Archetype> =
                    unsafe { slab_base.add(slot_idx) };
                Some(slot_ptr_mu as *mut Archetype)
            })
        })
    }

    /// Iterator over `&Archetype` references for every occupied slot.
    #[inline]
    pub fn iter(&self) -> ArchetypeBundleIter<'_> {
        ArchetypeBundleIter {
            slab_base: self.slots.as_ptr() as *const Archetype,
            occupied: self.occupied,
            word_idx: 0,
            word: self.occupied[0],
            _bundle: core::marker::PhantomData,
        }
    }

    /// Mutable iterator over `&mut Archetype` references for every occupied slot.
    ///
    /// Returns a hand-written iterator instead of `impl Iterator<...>` so the
    /// borrow checker can verify each yielded `&mut Archetype` is disjoint
    /// (every slot index is yielded at most once because BLSR strictly
    /// shrinks the bitset).
    #[inline]
    pub fn iter_mut(&mut self) -> ArchetypeBundleIterMut<'_> {
        let occupied = self.occupied;
        ArchetypeBundleIterMut {
            slab_base: self.slots.as_mut_ptr() as *mut Archetype,
            occupied,
            word_idx: 0,
            word: occupied[0],
            _bundle: core::marker::PhantomData,
        }
    }

}

impl Drop for ArchetypeBundle {
    /// Phase 7 C7 / U12 — drops every occupied slot exactly once before the
    /// slab allocation is freed by `Box`'s auto-Drop.
    fn drop(&mut self) {
        let slab_base: *mut MaybeUninit<Archetype> = self.slots.as_mut_ptr();
        for word_idx in 0..SLAB_WORDS {
            let mut word = self.occupied[word_idx];
            while word != 0 {
                let bit = word.trailing_zeros() as usize;
                let slot_idx = word_idx * 64 + bit;
                // SAFETY (U12): every set bit corresponds to a slot that
                //   was fully initialised via the in-place construction
                //   recipe in `add_archetype_from_components_fallible` or
                //   `add_archetype`, and has not been dropped since (the
                //   bit is cleared in `remove_archetype` / `clear` before
                //   the next `drop_in_place` would run). We hold `&mut self`,
                //   so no other reference into the slab is live. The bit
                //   is not cleared here because the bitset itself is about
                //   to be freed by Drop; `drop_in_place` runs the
                //   `Archetype` destructor exactly once.
                unsafe {
                    let slot_ptr: *mut Archetype = slab_base.add(slot_idx) as *mut Archetype;
                    ptr::drop_in_place(slot_ptr);
                }
                word &= word.wrapping_sub(1);
            }
        }
        // `Box`'s auto-Drop now frees the slab memory.
    }
}

/// Iterator over `&Archetype` references for every occupied slot in an
/// [`ArchetypeBundle`]. Created by [`ArchetypeBundle::iter`].
pub struct ArchetypeBundleIter<'a> {
    slab_base: *const Archetype,
    occupied: [u64; SLAB_WORDS],
    word_idx: usize,
    word: u64,
    _bundle: core::marker::PhantomData<&'a ArchetypeBundle>,
}

impl<'a> Iterator for ArchetypeBundleIter<'a> {
    type Item = &'a Archetype;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.word != 0 {
                let bit = self.word.trailing_zeros() as usize;
                self.word &= self.word.wrapping_sub(1);
                let slot_idx = self.word_idx * 64 + bit;
                // SAFETY (U8 + U1): the bit guarantees an initialised slot;
                //   slab base is stable; the iterator borrows the bundle
                //   immutably for `'a`, blocking concurrent mutation.
                let ptr = unsafe { self.slab_base.add(slot_idx) };
                return Some(unsafe { &*ptr });
            }
            self.word_idx += 1;
            if self.word_idx >= SLAB_WORDS {
                return None;
            }
            self.word = self.occupied[self.word_idx];
        }
    }
}

/// Mutable iterator over `&mut Archetype` references for every occupied slot
/// in an [`ArchetypeBundle`]. Created by [`ArchetypeBundle::iter_mut`].
pub struct ArchetypeBundleIterMut<'a> {
    slab_base: *mut Archetype,
    occupied: [u64; SLAB_WORDS],
    word_idx: usize,
    word: u64,
    _bundle: core::marker::PhantomData<&'a mut ArchetypeBundle>,
}

impl<'a> Iterator for ArchetypeBundleIterMut<'a> {
    type Item = &'a mut Archetype;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.word != 0 {
                let bit = self.word.trailing_zeros() as usize;
                self.word &= self.word.wrapping_sub(1);
                let slot_idx = self.word_idx * 64 + bit;
                // SAFETY (U8 + U1): every set bit corresponds to a fully
                //   initialised slot (`add_archetype_*` sets the bit only
                //   after full initialisation; `remove_archetype` /
                //   `clear` clear the bit before dropping). The bitset
                //   has each bit visited at most once (BLSR strictly
                //   shrinks `self.word`), so the yielded `&mut Archetype`
                //   pointers are disjoint and respect Rust's mutable
                //   aliasing rule. The iterator borrows the bundle
                //   mutably for `'a`, blocking concurrent access.
                let ptr = unsafe { self.slab_base.add(slot_idx) };
                return Some(unsafe { &mut *ptr });
            }
            self.word_idx += 1;
            if self.word_idx >= SLAB_WORDS {
                return None;
            }
            self.word = self.occupied[self.word_idx];
        }
    }
}

impl Index<ArchetypeId> for ArchetypeBundle {
    type Output = Archetype;

    fn index(&self, index: ArchetypeId) -> &Self::Output {
        self.get_archetype(index).expect("Archetype not found")
    }
}

impl IndexMut<ArchetypeId> for ArchetypeBundle {
    fn index_mut(&mut self, index: ArchetypeId) -> &mut Self::Output {
        self.get_archetype_mut(index).expect("Archetype not found")
    }
}

impl Index<&EntityInland> for ArchetypeBundle {
    type Output = Archetype;

    fn index(&self, entity_inland: &EntityInland) -> &Self::Output {
        self.get_entity_archetype(entity_inland)
            .expect("Entity not registered with any archetype")
    }
}

impl IndexMut<&EntityInland> for ArchetypeBundle {
    fn index_mut(&mut self, entity_inland: &EntityInland) -> &mut Self::Output {
        self.get_entity_archetype_mut(entity_inland)
            .expect("Entity not registered with any archetype")
    }
}

#[cfg(test)]
mod miri_tests {
    //! Miri-targeted tests for Phase 7 Step 4 invariants. Compiled in normal
    //! `cargo test` runs (acting as smoke tests) and exercised under
    //! `cargo +nightly miri test` for UB / retag detection.

    use super::*;
    use crate::ecs::core::component::component_registry;
    use crate::ecs::identifiers::primitives::ArchetypeId;
    use crate::ecs::memory::arena::Arena;

    // ID range 480-489 reserved for archetype_bundle Phase-7 Miri tests
    // (collisions checked against archetype.rs (400-417),
    // archetype_master.rs (300-308), component_pool_bundle.rs (420-429)).
    const COMP_X: ComponentId = ComponentId(480);
    const COMP_Y: ComponentId = ComponentId(481);

    fn register_test_components() {
        #[repr(C)]
        struct BundleCompX(u32);
        #[repr(C)]
        struct BundleCompY(u64);
        component_registry::register_layout::<BundleCompX>(COMP_X.0);
        component_registry::register_layout::<BundleCompY>(COMP_Y.0);
    }

    /// Exercises the W1 fix for [`ArchetypeBundle::get_archetype`] /
    /// [`ArchetypeBundle::get_archetype_mut`] and the Step-5 read/write
    /// pointer-API split (U11 pointer-minting recipe). The test interleaves
    /// four classes of access against a single slab slot:
    ///   1. A read-only `*const Archetype` minted via `get_archetype_ptr`
    ///      (`&self` flavour) for inspection.
    ///   2. A `&mut Archetype` taken via the safe accessor
    ///      `get_archetype_mut`, which after the W1 fix internally goes
    ///      through `get_archetype_ptr_mut` (raw-arithmetic mint from
    ///      `as_mut_ptr()`) instead of `self.slots[..].assume_init_mut()`
    ///      (which would materialise a `&mut MaybeUninit<Archetype>`
    ///      reborrow).
    ///   3. A write through a raw `*mut Archetype` minted **directly** via
    ///      the Step-5 `get_archetype_ptr_mut` accessor (no intermediate
    ///      `&mut Archetype`), followed by a re-mint via `get_archetype_ptr`
    ///      to verify the write survived.
    ///   4. A write through a `*mut Archetype` derived from a `&mut
    ///      Archetype` (reference-to-pointer cast). Mirrors the Step-7
    ///      `EntityInland` flow where a raw `*mut Archetype` is
    ///      dereferenced under `&mut EcsMaster`.
    ///
    /// Under Tree Borrows the legacy `assume_init_mut()` path materialises
    /// a `&mut MaybeUninit<Archetype>` whose borrow-stack frame would,
    /// on pop, retag-conflict with raw pointers held by `EntityInland`
    /// (Step 7). The W1 fix routes both safe accessors through raw
    /// arithmetic so no such reborrow is created.
    ///
    /// The `*const → *mut` cast is deliberately avoided for write
    /// pathways — Tree Borrows tags `as_ptr()` provenance `SharedReadOnly`
    /// and would flag any child-write through it. Step 5 introduces the
    /// dedicated `get_archetype_ptr_mut` accessor so write callers do not
    /// have to launder provenance through `*const`.
    #[test]
    fn phase7_miri_archetype_ptr_no_retag_ub() {
        register_test_components();
        // 16 MB headroom for a single 2-component archetype's pools.
        let arena = Arena::with_capacity(16 * 1024 * 1024);
        let mut bundle = ArchetypeBundle::new();
        let id = ArchetypeId(1);
        let _ = bundle.add_archetype_from_components_fallible(id, &[COMP_X, COMP_Y], &arena);

        // Leg 1 — read via the `&self`-flavoured raw pointer.
        let ptr_read1: *const Archetype = bundle
            .get_archetype_ptr(id)
            .expect("registered above");
        // SAFETY (test): freshly-initialised slot; no aliasing &mut alive
        //   here (the bundle was only mutated via `add_*` above).
        let observed_id = unsafe { (*ptr_read1).id() };
        assert_eq!(observed_id, id);

        // Leg 2 — `&mut Archetype` through the safe accessor; write a
        // probe value through it. After W1 the accessor's internal
        // mint comes from `as_mut_ptr()`, so the resulting `&mut` does
        // not need a `&mut MaybeUninit` reborrow.
        {
            let archetype_mut: &mut Archetype = bundle
                .get_archetype_mut(id)
                .expect("registered above");
            archetype_mut.current_index = 7;
        }

        // Verify the write through a fresh `*const` mint.
        let ptr_read2: *const Archetype = bundle
            .get_archetype_ptr(id)
            .expect("registered above");
        assert_eq!(
            ptr_read1 as usize, ptr_read2 as usize,
            "slab pointers stable across re-mints (U1)",
        );
        // SAFETY (test): `&self`-provenance read; no live & or &mut.
        let observed = unsafe { (*ptr_read2).current_index };
        assert_eq!(observed, 7, "write through &mut Archetype survived");

        // Leg 3 — write through a `*mut Archetype` minted directly via
        // the Step-5 `get_archetype_ptr_mut` accessor. This is the
        // production path for write capability — no `&mut Archetype` is
        // materialised first, the raw pointer carries write provenance
        // by construction (mint from `as_mut_ptr()`).
        let ptr_write_direct: *mut Archetype = bundle
            .get_archetype_ptr_mut(id)
            .expect("registered above");
        // SAFETY (test): pointer was just minted via raw arithmetic from
        //   `&mut self`; no live & or &mut to this slot exists.
        unsafe { addr_of_mut!((*ptr_write_direct).current_index).write(11) };

        // Verify the direct-mint write through a fresh `*const` re-mint.
        let ptr_read_after_direct: *const Archetype = bundle
            .get_archetype_ptr(id)
            .expect("registered above");
        // SAFETY (test): `&self`-provenance read; no live & or &mut.
        let observed_after_direct = unsafe { (*ptr_read_after_direct).current_index };
        assert_eq!(
            observed_after_direct, 11,
            "raw write through get_archetype_ptr_mut survived",
        );

        // Leg 4 — write through a `*mut Archetype` derived from a
        // `&mut Archetype` taken via the safe accessor. The cast
        // `&mut → *mut` preserves write-capable provenance under Tree
        // Borrows. This mirrors the Step-7 `EntityInland` flow where a
        // raw `*mut Archetype` is dereferenced under `&mut EcsMaster`.
        let ptr_write: *mut Archetype = bundle
            .get_archetype_mut(id)
            .expect("registered above") as *mut Archetype;
        // SAFETY (test): the `&mut Archetype` was dropped at the end of
        //   the previous statement (its scope is the expression). The
        //   resulting `*mut Archetype` carries write provenance; no
        //   other & or &mut to this slot is currently alive.
        unsafe { addr_of_mut!((*ptr_write).current_index).write(13) };

        // Verify the raw write through the safe shared accessor.
        let archetype_ref: &Archetype = bundle
            .get_archetype(id)
            .expect("registered above");
        assert_eq!(
            archetype_ref.current_index, 13,
            "raw write observable through safe shared accessor",
        );

        // Final stability check: re-mint via the `&self` API and
        // confirm the slab address is unchanged after all interleaving.
        let ptr_read3: *const Archetype = bundle
            .get_archetype_ptr(id)
            .expect("registered above");
        assert_eq!(
            ptr_read1 as usize, ptr_read3 as usize,
            "slab pointers stay stable after interleaved access (U1)",
        );
    }

    /// Registers three archetypes, removes the middle one (vacating its
    /// slot), then drops the bundle. Under Miri, `drop_in_place` must run
    /// exactly twice (for the two remaining live slots) — never on the
    /// vacated slot, never twice on the same slot.
    ///
    /// We cannot inject a sentinel into `Archetype::Drop` without a wider
    /// refactor; instead, this test confirms the bitset-driven drop walk
    /// is sound under Miri's invalid-deref detection (drop_in_place on a
    /// vacated slot would dereference uninitialised memory and trip Miri).
    #[test]
    fn phase7_miri_bundle_drop_runs_archetype_drop_for_occupied_only() {
        register_test_components();
        // 64 MB matches the precedent in `archetype.rs::create_entity_wide_archetype_8_components`;
        // 3 archetypes × ~1.5 pools each at DEFAULT_CHUNKS_PER_POOL = 128 chunks
        // exceeds the 4 MB used by simpler 1-archetype tests.
        let arena = Arena::with_capacity(64 * 1024 * 1024);
        let mut bundle = ArchetypeBundle::new();

        let id1 = ArchetypeId(10);
        let id2 = ArchetypeId(11);
        let id3 = ArchetypeId(12);
        bundle
            .add_archetype_from_components_fallible(id1, &[COMP_X], &arena)
            .expect("slab has free space");
        bundle
            .add_archetype_from_components_fallible(id2, &[COMP_X, COMP_Y], &arena)
            .expect("slab has free space");
        bundle
            .add_archetype_from_components_fallible(id3, &[COMP_Y], &arena)
            .expect("slab has free space");

        assert_eq!(bundle.len(), 3);
        assert!(bundle.remove_archetype(id2));
        assert_eq!(bundle.len(), 2);

        // Drop the bundle: the bitset walk must hit only id1's and id3's
        // slots. Miri's pointer-validity check trips on `drop_in_place` of
        // an uninitialised slot, so if the walk were to visit id2's freed
        // slot, this test would fail under `cargo +nightly miri test`.
        drop(bundle);
    }
}
