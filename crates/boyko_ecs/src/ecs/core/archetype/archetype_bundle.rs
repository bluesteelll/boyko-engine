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

use core::cell::UnsafeCell;
use core::ptr::{self, addr_of_mut};
use std::mem::MaybeUninit;
use std::ops::{Index, IndexMut};

use static_assertions::assert_impl_all;

use crate::ecs::core::archetype::archetype::{Archetype, Column};
use crate::ecs::core::archetype::archetype_signature::ArchetypeSignature;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::component::component_pool_bundle::ComponentPoolBundle;
use crate::ecs::core::component::component_registry::MAX_COMPONENTS;
use crate::ecs::core::component::hooks::archetype_flags::ArchetypeFlags;
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

// Phase F4 (I8): wrapping the slab element in `UnsafeCell<MaybeUninit<_>>`
// must not change its size, alignment, or stride versus a bare `Archetype`.
// Both `UnsafeCell<T>` and `MaybeUninit<T>` are `#[repr(transparent)]`, so the
// slab allocation (`size_of::<element>() * MAX_ARCHETYPES`) and the
// `slot_idx * stride` pointer arithmetic stay byte-identical to the pre-F4
// `Box<[MaybeUninit<Archetype>; N]>` layout. If a future libstd ever altered
// this, these asserts trip before any UB.
const _: () = assert!(
    size_of::<UnsafeCell<MaybeUninit<Archetype>>>() == size_of::<Archetype>(),
    "F4: UnsafeCell<MaybeUninit<Archetype>> must be the same size as Archetype",
);
const _: () = assert!(
    align_of::<UnsafeCell<MaybeUninit<Archetype>>>() == align_of::<Archetype>(),
    "F4: UnsafeCell<MaybeUninit<Archetype>> must have the same align as Archetype",
);

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
/// # Layout & invariants (Phase 7 D1 / U1, U2, U8, U11, U12, U13; Phase F4 F1-F3)
///
/// - `slots` is a [`Box`] of
///   `[UnsafeCell<MaybeUninit<Archetype>>; MAX_ARCHETYPES]`. The `Box` is
///   allocated once in [`Self::new`] and **never reassigned**. The slab base
///   address is therefore stable for the bundle's lifetime (U1) and outlives
///   every pointer minted from it (U2).
/// - Phase F4: every slab element is wrapped in an [`UnsafeCell`] (cell
///   OUTERMOST, `MaybeUninit` inside) so that all bytes of every slot —
///   including `Archetype::current_index`, which a sibling spawn mutates —
///   are interior-mutable. Pointers minted via [`UnsafeCell::raw_get`] carry
///   `SharedReadWrite` provenance under Stacked Borrows and survive sibling
///   structural writes under Tree Borrows. This is what lets `EntityInland`
///   cache a `*mut Archetype` that stays legal to reborrow after later spawns
///   into the same archetype write through a sibling pointer (F4 finding,
///   `docs/PHASE-14-F4-FINDING.md`). Mirrors the established
///   `UnsafeCell<Tick>`-slot discipline in the tick sub-regions of
///   `component_pool.rs` (Phase X.I vm-reservation form).
/// - **F4 mint discipline (load-bearing):** the SOLE entry points that mint a
///   slot pointer are [`Self::slot_ptr_mut`] / [`Self::slot_ptr`]. No method
///   ever calls `self.slots.as_mut_ptr()` — that would form a transient
///   `&mut [UnsafeCell<…>; N]` array-level retag whose children re-introduce
///   the sibling relationship one level up, defeating the fix. The `&self`
///   helpers go through `self.slots.as_ptr()` (a shared `&[UnsafeCell; N]`
///   whose elements' contents are interior-mutable) and never form an array
///   `&mut`.
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
    ///
    /// Phase F4: each element is `UnsafeCell<MaybeUninit<Archetype>>` (cell
    /// outermost = interior-mutability/provenance root, `MaybeUninit` inside =
    /// per-slot init tracking via `self.occupied`). `UnsafeCell<T>` and
    /// `MaybeUninit<T>` are both `#[repr(transparent)]`, so the element's
    /// size/align/stride is identical to `Archetype` (const-asserted below);
    /// slab allocation and `slot * stride` arithmetic are unchanged.
    slots: Box<[UnsafeCell<MaybeUninit<Archetype>>; MAX_ARCHETYPES]>,
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

// SAFETY (Phase F4 R1 / SEND10 — mirrors `ComponentPool` `component_pool.rs`):
//
// Wrapping the slab element in `UnsafeCell` makes `ArchetypeBundle` `!Sync`
// (and, transitively, `!Send`) by the auto-trait rules, removing the auto
// impls that the Phase-9 scheduler relies on for sharing `&EcsMaster` across
// worker threads (`Archetype`, `ArchetypeMaster`, and `EcsMaster` all carry
// MANUAL `unsafe impl Send/Sync` and would otherwise compile against a
// `!Send`/`!Sync` field without complaint — hence the `assert_impl_all!`
// gate below makes a regression a build error). The manual impls are sound:
//
//   - All slab MUTATION (`add_archetype*`, `remove_archetype`, `clear`, the
//     in-place construction writes, and the sibling-spawn `current_index += 1`
//     reached through a minted `*mut Archetype`) happens on `&mut self`
//     structural-op paths that the dispatcher serialises inside the apply
//     window (SCH3). No worker thread holds a `&mut`-derived slab pointer
//     concurrently with another thread's structural op.
//   - Worker threads access the bundle only through `&self` read paths
//     (`get_archetype_ptr`, `iter_occupied_ptrs`, `iter`), reading immutable
//     `Archetype`s; the `ConflictGraph` guarantees no overlapping mutable
//     view is live (SCH7).
//   - Each slot is its OWN `UnsafeCell` (per-element, mirroring the per-row
//     `Tick` cells), so interior mutation through one slot's minted pointer
//     is a distinct memory location from every other slot — no shared
//     interior-mutable state is transmitted across threads beyond the
//     dispatcher-governed window.
//
// The `UnsafeCell` therefore changes only the borrow-stack/tree provenance of
// the slab pointers (the F4 fix); it introduces no new cross-thread sharing
// beyond what the pre-F4 `Box<[MaybeUninit<Archetype>; N]>` already exposed
// under the same scheduler contract.
unsafe impl Send for ArchetypeBundle {}
unsafe impl Sync for ArchetypeBundle {}

// Phase F4 R1 gate (P2): the manual impls above are load-bearing for the
// Phase-9 scheduler. A bare `cargo check` is a false-green here because
// `Archetype` / `ArchetypeMaster` / `EcsMaster` have their own manual
// `unsafe impl Send/Sync` and compile regardless of this type. This positive
// assertion (a `const _:` item, fires on every compile) makes the build FAIL
// if the manual impls are ever removed or the field change is reverted.
// Mirrors the `QueryView` / `BundleColumnCache` Send/Sync assertions.
assert_impl_all!(ArchetypeBundle: Send, Sync);

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
        // SAFETY (slab init / C3 + F4):
        //   `Box::<T>::new_uninit()` allocates space for `T` on the heap and
        //   returns `Box<MaybeUninit<T>>`. For
        //   `T = [UnsafeCell<MaybeUninit<Archetype>>; N]` the resulting
        //   allocation is uninitialised memory of the correct size and
        //   alignment, sized via the heap allocator with no stack
        //   construction of the 8.4 MB temporary.
        //
        //   `assume_init()` is sound because the array element type is
        //   `UnsafeCell<MaybeUninit<Archetype>>`, which has NO validity
        //   invariant: `MaybeUninit<Archetype>` is valid for any bit pattern
        //   (including uninitialised), and the `#[repr(transparent)]`
        //   `UnsafeCell` wrapper adds none. An array of such elements is
        //   therefore always "initialised" in the type-system sense.
        //   Per-slot initialisation of the inner `Archetype` is tracked
        //   separately via `self.occupied`.
        //
        //   `Box::new_uninit` is stable since Rust 1.82; boyko-engine targets
        //   Rust 2024 (≥ 1.93).
        let slots = unsafe {
            Box::<[UnsafeCell<MaybeUninit<Archetype>>; MAX_ARCHETYPES]>::new_uninit()
                .assume_init()
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

    /// Mints a write-capable `*mut Archetype` for the slab slot `slot_idx`.
    ///
    /// Phase F4 — **the SOLE mint entry** for slot pointers (read paths go
    /// through [`Self::slot_ptr`], which delegates here and casts to
    /// `*const`). Every structural-op method, iterator, and `Drop` routes its
    /// pointer through this helper; no site calls `self.slots.as_mut_ptr()`.
    ///
    /// The caller is responsible for the slot being initialised (occupancy
    /// bit set) before dereferencing the result for a read, and for upholding
    /// the aliasing contract (`&mut self` / apply-window exclusivity, SCH3).
    /// This helper only mints provenance; it neither reads nor writes the slot.
    #[inline(always)]
    fn slot_ptr_mut(&self, slot_idx: usize) -> *mut Archetype {
        debug_assert!(slot_idx < MAX_ARCHETYPES);
        // SAFETY (F1 / F2 / F3 + U1):
        //   - F1: the pointer is rooted at an `UnsafeCell` element. The ENTIRE
        //     slab element is `UnsafeCell<MaybeUninit<Archetype>>`, so every
        //     byte of the slot — including `current_index`, which a sibling
        //     spawn mutates — is interior-mutable. Pointers derived from the
        //     same cell address do not Disable one another under current Tree
        //     Borrows, and carry `SharedReadWrite` (not the `SharedReadOnly` a
        //     `&` would give) under Stacked Borrows; a sibling's write through
        //     a same-cell-derived pointer does not pop this one. Identical to
        //     the `UnsafeCell<Tick>`-slot precedent in the tick sub-regions
        //     of `component_pool.rs` (SEND10 / write_added_tick).
        //   - F2: `UnsafeCell::raw_get` takes a `*const UnsafeCell<T>` and
        //     returns a `*mut T` WITHOUT forming any reference, preserving U11
        //     (no `&MaybeUninit`/`&UnsafeCell` reborrow is ever materialised).
        //     `self.slots.as_ptr()` under `&self` yields only a shared
        //     `&[UnsafeCell; N]` whose elements' CONTENTS are interior-mutable
        //     — no `&mut [UnsafeCell; N]` array-level retag forms (F4 mint
        //     discipline; calling `as_mut_ptr()` here would defeat the fix).
        //   - F3: `*mut MaybeUninit<Archetype> as *mut Archetype` is a
        //     transparent no-op (`MaybeUninit` is `#[repr(transparent)]`).
        //   - U1: `slot_idx < MAX_ARCHETYPES` keeps `add` in-bounds of the
        //     `MAX_ARCHETYPES`-element slab, whose heap base is stable for the
        //     bundle's lifetime.
        unsafe {
            UnsafeCell::raw_get(self.slots.as_ptr().add(slot_idx)).cast::<Archetype>()
        }
    }

    /// Mints a read-only `*const Archetype` for the slab slot `slot_idx`.
    ///
    /// Phase F4 — delegates to [`Self::slot_ptr_mut`] (same `UnsafeCell`-rooted
    /// provenance) and narrows to `*const`. The read/write split is a CALLER
    /// CONTRACT, not a provenance distinction: post-F4 the underlying pointer
    /// is `SharedReadWrite`, so casting back to `*mut` is no longer UB — but
    /// the read-only API surface is preserved so callers cannot accidentally
    /// write through a `&self`-derived pointer.
    #[inline(always)]
    fn slot_ptr(&self, slot_idx: usize) -> *const Archetype {
        self.slot_ptr_mut(slot_idx) as *const _
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
        // SAFETY (U1, U2, U8, U11, F4):
        //   - U11/F4: `get_archetype_ptr_mut` mints `ptr` through
        //     [`Self::slot_ptr_mut`] (`UnsafeCell::raw_get`); the slab element
        //     is `UnsafeCell`-wrapped, so the pointer addresses an
        //     interior-mutable (`SharedReadWrite`) location and no
        //     `&mut MaybeUninit<Archetype>` (nor `&mut [UnsafeCell; N]`)
        //     reborrow is ever created. Post-F4 the read-only
        //     `get_archetype_ptr` mints the SAME `SharedReadWrite` provenance,
        //     so the read/write split is a CALLER CONTRACT, not a provenance
        //     one — `&mut *ptr` here is the sanctioned write surface and is
        //     legal under both Tree Borrows and Stacked Borrows.
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
    /// provenance, now F4-rooted. The pointer is minted through
    /// [`Self::slot_ptr_mut`] (`UnsafeCell::raw_get`); no
    /// `&mut MaybeUninit<Archetype>` reborrow is created along the way. Post-F4
    /// the read-only [`Self::get_archetype_ptr`] mints the SAME `SharedReadWrite`
    /// provenance, so the read/write distinction between the two methods is a
    /// CALLER CONTRACT, not a provenance one. This method is the write surface
    /// (Step 7's `EntityInland` storage, the `&mut Archetype` rematerialisation
    /// inside `EcsMaster::create_entity`, and the internal safe accessor
    /// [`Self::get_archetype_mut`]).
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
        // F4: mint via the sole `&self` helper (`UnsafeCell::raw_get`). A
        // `&mut self` method calling a `&self` helper is fine and avoids the
        // `&mut [UnsafeCell; N]` array retag that `self.slots.as_mut_ptr()`
        // would form. `slot_idx < MAX_ARCHETYPES` is enforced by the
        // `id_to_slot` invariant (only ever populated with in-bound indices).
        Some(self.slot_ptr_mut(slot_idx as usize))
    }

    /// Returns a read-only raw `*const Archetype` pointer to the slot for
    /// `archetype_id`, or `None` if no slot is registered for that id.
    ///
    /// Phase 7 C4 / U11 — pointer minting recipe, now F4-rooted. The pointer is
    /// minted through [`Self::slot_ptr`] (`UnsafeCell::raw_get`, narrowed to
    /// `*const`); no `&MaybeUninit<Archetype>` / `&UnsafeCell<…>` reborrow is
    /// ever materialised — the raw-arithmetic + `raw_get` recipe never produces
    /// a reference to the slot.
    ///
    /// # Provenance contract (updated for F4)
    /// Callers may **only read** through the returned pointer — but this is a
    /// CALLER CONTRACT, not a provenance distinction. Post-F4 the whole slab
    /// element is `UnsafeCell`-wrapped, so the pointer carries `SharedReadWrite`
    /// provenance (the same root [`Self::get_archetype_ptr_mut`] mints) rather
    /// than the pre-F4 `SharedReadOnly`. Casting it to `*mut` and writing is no
    /// longer provenance-UB; the read-only return type is preserved only so the
    /// read/write surfaces stay distinct at the API level. Write callers should
    /// still obtain a pointer via [`Self::get_archetype_ptr_mut`].
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
        // F4: mint via the sole `&self` read helper (`raw_get` narrowed to
        // `*const`). `slot_idx < MAX_ARCHETYPES` is enforced by the
        // `id_to_slot` invariant.
        Some(self.slot_ptr(slot_idx as usize))
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

        // F4: mint via the sole `&self` helper (`UnsafeCell::raw_get`).
        // `slot_idx < MAX_ARCHETYPES` is enforced above (capacity check +
        // `free_slots` only ever holds valid indices emitted under that bound).
        let slot_ptr: *mut Archetype = self.slot_ptr_mut(slot_idx as usize);

        // SAFETY (U13 — in-place archetype construction; F1-rooted):
        //   `slot_ptr` points at uninitialised but properly-sized and
        //   -aligned interior-mutable memory for one `Archetype` inside the
        //   slab (the slot's `UnsafeCell` cell). Each field of `Archetype` is
        //   written exactly once via `addr_of_mut!.write()` (no intermediate
        //   `&mut` reborrow), so no stack-allocated 8.4 KB `Archetype`
        //   temporary is constructed (Windows main-thread stack is 1 MB by
        //   default — W6).
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
            // Phase 14a: every `Archetype` field must be initialised on the
            // in-place slab path (U13) or the slot is partially uninit (UB).
            // Start empty; the OR-compute over the registered components runs
            // in the `register_component_inplace` loop below (Wave 2).
            addr_of_mut!((*slot_ptr).flags).write(ArchetypeFlags::empty());
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
    /// overwritten under the **AB-R1 clear-bit-first protocol**: the
    /// `occupied` bit and `id_to_slot` mapping are cleared *before*
    /// `drop_in_place` runs, so a panic inside a user component's `Drop`
    /// cannot leave the slab in a state where `ArchetypeBundle::Drop`'s
    /// bitset walk would revisit the half-dropped slot (double-drop UB).
    /// `count` is intentionally left unchanged through the drop window —
    /// the brief inconsistency with `occupied` is observable only via
    /// `len()`, never via `Drop` or any read path.
    pub fn add_archetype(&mut self, archetype: Archetype) -> InlandArchetypeId {
        let archetype_id = archetype.id();
        let raw_id = archetype_id.0;

        // Replace path: same id already registered → clear-bit-first then
        // drop the old occupant and overwrite the slot.
        if raw_id < self.id_to_slot.len() && self.id_to_slot[raw_id] != NO_SLOT {
            let slot_idx = self.id_to_slot[raw_id];
            // F4: mint via the sole `&self` helper; slot_idx is in-bounds
            // because it was emitted by an earlier `add_*` under the
            // `slot_idx < MAX_ARCHETYPES` invariant.
            let slot_ptr: *mut Archetype = self.slot_ptr_mut(slot_idx as usize);

            // === AB-R1: clear-bit-first ===
            // Step 1a: clear the `occupied` bit BEFORE drop_in_place. THIS
            //   is what gates ArchetypeBundle::Drop's bitset walk — even
            //   if drop_in_place panics, Drop will skip this slot.
            let word_idx = (slot_idx as usize) / 64;
            let bit = (slot_idx as usize) % 64;
            self.occupied[word_idx] &= !(1u64 << bit);

            // Step 1b: clear the lookup too. Belt-and-suspenders for
            //   external observers that might call `get_archetype_ptr`
            //   mid-replace.
            self.id_to_slot[raw_id] = NO_SLOT;

            // Step 2: drop the old occupant. If this panics, the slab
            //   cell is unreachable from both `id_to_slot` and Drop's
            //   bitset walk — one `Archetype`'s allocations leak; no UB.
            // SAFETY (U12 + AB-R1): the previous occupancy was confirmed
            //   by the outer `if`; clear-bit-first ensures non-revisitation
            //   on panic.
            unsafe { ptr::drop_in_place(slot_ptr); }

            // Step 3: write the new value. Cannot panic (POD memcpy).
            // SAFETY (U13): the slab cell is logically empty after step 2's
            //   drop; we transfer ownership of `archetype` byte-wise into
            //   the slot without invoking its destructor.
            unsafe { ptr::write(slot_ptr, archetype); }

            // Step 4a: re-set the `occupied` bit. Single &mut, no
            //   atomicity needed.
            self.occupied[word_idx] |= 1u64 << bit;

            // Step 4b: re-set the lookup.
            self.id_to_slot[raw_id] = slot_idx;

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

        // F4: mint via the sole `&self` helper; slot_idx is in-bounds by the
        // same invariant as above.
        let slot_ptr: *mut Archetype = self.slot_ptr_mut(slot_idx as usize);
        // SAFETY (U13 — move-into-slot variant; F1-rooted): `slot_ptr` is
        //   uninitialised interior-mutable slab memory of the correct
        //   size/alignment; we transfer ownership of `archetype` byte-wise
        //   into the slot without invoking its destructor. After this line,
        //   the slot is fully initialised and the local `archetype` binding
        //   is logically moved (no further use is permitted).
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

        // F4: mint via the sole `&self` helper; slot index in-bounds.
        let slot_ptr: *mut Archetype = self.slot_ptr_mut(slot_idx as usize);
        // SAFETY (U12 + F1): the occupancy bit (verified via `id_to_slot !=
        //   NO_SLOT`) guarantees the slot is initialised. `&mut self` rules
        //   out any concurrent reader through a stored sibling pointer.
        //   `drop_in_place` runs the destructor exactly once; afterwards we
        //   clear the bit so the slot is treated as `MaybeUninit` for all
        //   future access.
        unsafe { ptr::drop_in_place(slot_ptr) };

        let word = (slot_idx as usize) / 64;
        let bit = (slot_idx as usize) % 64;
        self.occupied[word] &= !(1u64 << bit);

        self.id_to_slot[raw_id] = NO_SLOT;
        self.free_slots.push(slot_idx);
        self.count -= 1;
        true
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
        for word_idx in 0..SLAB_WORDS {
            let mut word = self.occupied[word_idx];
            while word != 0 {
                let bit = word.trailing_zeros() as usize;
                let slot_idx = word_idx * 64 + bit;
                // F4: mint via the sole `&self` helper. `&mut self` rules out
                // any concurrent reader through a stored sibling pointer.
                let slot_ptr: *mut Archetype = self.slot_ptr_mut(slot_idx);
                // SAFETY (U12 + F1): the bit indicates an initialised slot;
                //   `drop_in_place` runs its destructor exactly once. After
                //   the loop we clear the entire occupancy bitset, so the
                //   slot is then treated as `MaybeUninit`.
                unsafe {
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
    /// read through these pointers — this is a CALLER CONTRACT; post-F4 each
    /// pointer is interior-mutable (`SharedReadWrite`, F1-rooted). For write
    /// access use [`Self::iter_occupied_ptrs_mut`].
    ///
    /// Bitset walk via TZCNT (`u64::trailing_zeros`) and BLSR
    /// (`word & word.wrapping_sub(1)`) — `O(popcount(occupied))`.
    #[inline]
    pub fn iter_occupied_ptrs(&self) -> impl Iterator<Item = *const Archetype> + '_ {
        // F4 / P6: cache the CELL-ARRAY base (array provenance over the whole
        // slab), NOT a single per-element `*Archetype` (whose provenance would
        // cover one cell, making `add` into other cells out-of-bounds UB).
        // `self.slots.as_ptr()` under `&self` is a shared `&[UnsafeCell; N]`
        // (no `&mut`-array retag). Each element is minted in the closure via
        // `raw_get` so it carries the per-element interior-mutable provenance.
        let cell_base: *const UnsafeCell<MaybeUninit<Archetype>> = self.slots.as_ptr();
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
                // SAFETY (U8 + F1/F2/F3 + U1): the occupancy bit guarantees an
                //   initialised slot; `cell_base.add(slot_idx)` strides over
                //   the cell array (in-bounds, array provenance) because
                //   `slot_idx < MAX_ARCHETYPES`; `raw_get` then yields the
                //   per-element interior-mutable pointer WITHOUT forming any
                //   reference. Slab base is stable for the bundle's lifetime
                //   (U1). Narrowed to `*const`: read-only by caller contract.
                let slot_ptr: *const Archetype =
                    unsafe { UnsafeCell::raw_get(cell_base.add(slot_idx)).cast::<Archetype>() };
                Some(slot_ptr)
            })
        })
    }

    /// Returns an iterator yielding raw `*mut Archetype` pointers to every
    /// occupied slot, in ascending slot-index order.
    ///
    /// Mirrors [`Self::iter_occupied_ptrs`] but takes `&mut self` (the caller
    /// gets exclusive structural access). Pointers are stable for the bundle's
    /// lifetime (U1) and interior-mutable (`SharedReadWrite`, F1-rooted).
    #[inline]
    pub fn iter_occupied_ptrs_mut(
        &mut self,
    ) -> impl Iterator<Item = *mut Archetype> + '_ {
        // F4 / P4 / P6: cache the CELL-ARRAY base. NOTE `self.slots.as_ptr()`
        // even though this is `&mut self` — `as_mut_ptr()` would form a
        // `&mut [UnsafeCell; N]` array-level retag whose children reintroduce
        // the sibling relationship one level up, defeating the fix. A shared
        // array reborrow (`as_ptr` through `&mut self`) does not; write
        // capability is restored per-element by `raw_get` (interior-mutable
        // root), not by the array provenance.
        let cell_base: *const UnsafeCell<MaybeUninit<Archetype>> = self.slots.as_ptr();
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
                // SAFETY (U8 + F1/F2/F3 + U1): occupancy bit ⇒ slot fully
                //   initialised; `cell_base.add(slot_idx)` strides over the
                //   cell array in-bounds (`slot_idx < MAX_ARCHETYPES`, array
                //   provenance); `raw_get` yields the per-element
                //   interior-mutable `*mut` WITHOUT forming any reference (so
                //   no `&mut [UnsafeCell; N]` retag — P4). Slab base is stable
                //   for the bundle's lifetime (U1). The `&mut self` borrow on
                //   the returned iterator blocks concurrent access.
                let slot_ptr: *mut Archetype =
                    unsafe { UnsafeCell::raw_get(cell_base.add(slot_idx)).cast::<Archetype>() };
                Some(slot_ptr)
            })
        })
    }

    /// Iterator over `&Archetype` references for every occupied slot.
    #[inline]
    pub fn iter(&self) -> ArchetypeBundleIter<'_> {
        // F4 / P6: cache the CELL-ARRAY base (array provenance), not a single
        // `*Archetype` (one-cell provenance). `next()` mints each element via
        // `raw_get`. `self.slots.as_ptr()` is a shared `&[UnsafeCell; N]`.
        ArchetypeBundleIter {
            cell_base: self.slots.as_ptr(),
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
        // F4 / P4 / P6: cache the CELL-ARRAY base via `as_ptr()` (NOT
        // `as_mut_ptr()`, which would form a `&mut [UnsafeCell; N]` array
        // retag). `next()` restores write capability per-element via
        // `raw_get` (interior-mutable root). The `&mut self` borrow on the
        // returned iterator keeps the yielded `&mut Archetype`s exclusive.
        ArchetypeBundleIterMut {
            cell_base: self.slots.as_ptr(),
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
        for word_idx in 0..SLAB_WORDS {
            let mut word = self.occupied[word_idx];
            while word != 0 {
                let bit = word.trailing_zeros() as usize;
                let slot_idx = word_idx * 64 + bit;
                // F4: mint via the sole `&self` helper. We hold `&mut self`,
                // so no other reference or stored sibling pointer into the
                // slab is live.
                let slot_ptr: *mut Archetype = self.slot_ptr_mut(slot_idx);
                // SAFETY (U12 + F1): every set bit corresponds to a slot that
                //   was fully initialised via the in-place construction
                //   recipe in `add_archetype_from_components_fallible` or
                //   `add_archetype`, and has not been dropped since (the
                //   bit is cleared in `remove_archetype` / `clear` before
                //   the next `drop_in_place` would run). The bit is not
                //   cleared here because the bitset itself is about to be
                //   freed by Drop; `drop_in_place` runs the `Archetype`
                //   destructor exactly once.
                unsafe {
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
    /// F4 / P6: cell-array base (array provenance over the whole slab). Each
    /// element is minted in `next()` via `raw_get` so the per-element pointer
    /// carries interior-mutable provenance. Caching a single `*Archetype` base
    /// would give one-cell provenance, making `add` into other cells UB.
    cell_base: *const UnsafeCell<MaybeUninit<Archetype>>,
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
                // SAFETY (U8 + F1/F2/F3 + U1): the bit guarantees an
                //   initialised slot; `cell_base.add(slot_idx)` strides over
                //   the cell array in-bounds (array provenance); `raw_get`
                //   yields the per-element interior-mutable pointer without
                //   forming a reference. Slab base is stable; the iterator
                //   borrows the bundle immutably for `'a`, blocking
                //   concurrent mutation.
                let ptr: *const Archetype = unsafe {
                    UnsafeCell::raw_get(self.cell_base.add(slot_idx)).cast::<Archetype>()
                };
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
    /// F4 / P4 / P6: cell-array base (array provenance). Each `&mut Archetype`
    /// is minted in `next()` via `raw_get` (interior-mutable root) — never via
    /// a strided single-element `*mut Archetype` base (one-cell provenance) and
    /// never via `&mut [UnsafeCell; N]` (array retag that would defeat F4).
    cell_base: *const UnsafeCell<MaybeUninit<Archetype>>,
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
                // SAFETY (U8 + F1/F2/F3 + U1): every set bit corresponds to a
                //   fully initialised slot (`add_archetype_*` sets the bit
                //   only after full initialisation; `remove_archetype` /
                //   `clear` clear the bit before dropping). The bitset has
                //   each bit visited at most once (BLSR strictly shrinks
                //   `self.word`), so the yielded `&mut Archetype` references
                //   target disjoint slots and respect Rust's mutable aliasing
                //   rule. `cell_base.add(slot_idx)` strides over the cell
                //   array in-bounds (array provenance); `raw_get` yields the
                //   per-element interior-mutable `*mut` without forming a
                //   reference. The iterator borrows the bundle mutably for
                //   `'a`, blocking concurrent access.
                let ptr: *mut Archetype = unsafe {
                    UnsafeCell::raw_get(self.cell_base.add(slot_idx)).cast::<Archetype>()
                };
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
    ///      through `get_archetype_ptr_mut` (mint via `slot_ptr_mut`,
    ///      `UnsafeCell::raw_get`) instead of `self.slots[..].assume_init_mut()`
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
        // mint comes from `slot_ptr_mut` (`UnsafeCell::raw_get`), so the
        // resulting `&mut` does not need a `&mut MaybeUninit` reborrow.
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
        // by construction (mint via `slot_ptr_mut`/`UnsafeCell::raw_get`).
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

    /// Phase F4 — the minimal stored-pointer-survives-sibling-write reproducer
    /// (`docs/PHASE-14-F4-FINDING.md`). Models the engine's `EntityInland`
    /// flow at the bundle level:
    ///
    ///   1. Register an archetype and STASH a read-only `*const Archetype`
    ///      (`stored` = the `EntityInland.archetype_ptr` analogue, "T0").
    ///   2. Mint a SEPARATE sibling `*mut Archetype` ("T1") and perform a
    ///      foreign structural write through it (`current_index += 1`,
    ///      exactly the `archetype.rs` write a later spawn into the same
    ///      archetype runs). Repeat (mimics spawns B and C).
    ///   3. READ through the originally-stashed `stored` pointer.
    ///
    /// Pre-F4 (`Box<[MaybeUninit<Archetype>; N]>` minted via `as_mut_ptr`),
    /// step 2's write through the sibling T1 transitioned T0 Reserved →
    /// Disabled under Tree Borrows, so step 3's reborrow was TB-UB. Post-F4
    /// (`UnsafeCell`-rooted slab, `raw_get` mint), T0 and T1 derive from the
    /// same per-slot `UnsafeCell`, so the interior-mutable write through T1
    /// does NOT Disable T0 — the read is legal. This test is TB-clean only
    /// with the F4 fix in place. Uses the reserved id range 483.
    #[test]
    fn f4_stored_ptr_survives_sibling_spawn() {
        register_test_components();
        // 16 MB headroom for a single 2-component archetype.
        let arena = Arena::with_capacity(16 * 1024 * 1024);
        let mut bundle = ArchetypeBundle::new();
        let id = ArchetypeId(3);
        let _ = bundle.add_archetype_from_components_fallible(id, &[COMP_X, COMP_Y], &arena);

        // (1) Stash the read-only pointer — the `EntityInland.archetype_ptr`
        // analogue. It is held UNCHANGED across all subsequent sibling writes.
        let stored: *const Archetype = bundle
            .get_archetype_ptr(id)
            .expect("registered above");
        // SAFETY (test): freshly-initialised slot; no aliasing `&mut` is live.
        let id_before = unsafe { (*stored).id() };
        assert_eq!(id_before, id);

        // (2) Two sibling structural writes through FRESHLY-minted `*mut`
        // pointers, each analogous to a later spawn's `current_index += 1`.
        // Pre-F4 these foreign writes Disabled `stored` under Tree Borrows.
        for _ in 0..2 {
            let sibling: *mut Archetype = bundle
                .get_archetype_ptr_mut(id)
                .expect("registered above");
            // SAFETY (test): `sibling` is a fresh same-cell `*mut`; no live
            //   `&`/`&mut` to the slot exists at this point. Writing
            //   `current_index` mirrors `Archetype::create_entity`'s bump.
            unsafe {
                let cur = (*sibling).current_index;
                addr_of_mut!((*sibling).current_index).write(cur + 1);
            }
        }

        // (3) Read through the ORIGINALLY-stashed pointer. Pre-F4 this reborrow
        // was TB-UB (T0 Disabled by the sibling writes). Post-F4 it is legal.
        // SAFETY (test): `stored` is F4-rooted interior-mutable provenance into
        //   a live slot; the slab base is stable (U1); no `&mut` is live here.
        let observed = unsafe { (*stored).current_index };
        assert_eq!(
            observed, 2,
            "stored pointer observes the sibling writes (interior-mutable, F4-rooted)",
        );
        // The stashed pointer is also still address-stable.
        let re_mint: *const Archetype = bundle
            .get_archetype_ptr(id)
            .expect("registered above");
        assert_eq!(stored as usize, re_mint as usize, "slab address stable (U1)");
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

    /// Phase 8a Step 12 (C-NEW-1 + C-R3-1) — verifies the **AB-R1
    /// clear-bit-first protocol** in [`ArchetypeBundle::add_archetype`]'s
    /// replace path: a panic inside the previous occupant's `Drop` must NOT
    /// cause the bundle's later `Drop` to revisit the half-dropped slot
    /// (double-drop UB).
    ///
    /// Mechanism:
    /// 1. A component type [`PanicDropComp`] whose `Drop` impl bumps a
    ///    global counter and panics whenever `PANIC_DROP_ARMED` is `true`.
    ///    We arm the flag for the replace's `drop_in_place`, then disarm
    ///    it before letting `Drop` of the bundle (which will not revisit
    ///    the slot under AB-R1) finish cleanly.
    /// 2. Insert one entity carrying `PanicDropComp` into the old
    ///    archetype so the panicking `Drop` actually fires during
    ///    `ComponentPool::Drop` inside `Archetype::Drop`.
    /// 3. Catch the panic from `add_archetype(new_archetype_same_id)`.
    /// 4. Inspect the bundle's internal state: `id_to_slot[raw_id]` must
    ///    be `NO_SLOT` and the `occupied` bit must be cleared.
    /// 5. Disarm the panic, drop the bundle, and assert the global drop
    ///    counter reached exactly 1 (never 2). Under the previous
    ///    `drop_in_place; ptr::write` shape, step 5 would observe a count
    ///    of 2 — the bug fixed in Step 12.
    ///
    /// Reuses ID 482 from the archetype_bundle Phase-7 reserved range.
    #[test]
    fn phase7_carry_over_add_archetype_replace_panic_in_drop_no_double_drop() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        // Counts every invocation of `PanicDropComp::drop`. Static so the
        // type-erased `drop_fn` registered via `ComponentLayout` can reach it.
        static PANIC_DROP_COUNT: AtomicUsize = AtomicUsize::new(0);
        // When `true`, `PanicDropComp::drop` panics after incrementing the
        // counter. Used to gate the "first drop panics, no further drops
        // happen" protocol.
        static PANIC_DROP_ARMED: AtomicBool = AtomicBool::new(false);

        #[repr(C)]
        struct PanicDropComp(u32);

        impl Drop for PanicDropComp {
            fn drop(&mut self) {
                PANIC_DROP_COUNT.fetch_add(1, Ordering::Relaxed);
                if PANIC_DROP_ARMED.load(Ordering::Relaxed) {
                    panic!("PanicDropComp::drop intentional panic for AB-R1 test");
                }
            }
        }

        const PANIC_COMP_ID: ComponentId = ComponentId(482);
        component_registry::register_layout::<PanicDropComp>(PANIC_COMP_ID.0);

        // Counter reset in case another test in the same process touched
        // PanicDropComp via the registry (the type is module-local so this
        // should not happen, but be defensive).
        PANIC_DROP_COUNT.store(0, Ordering::Relaxed);
        PANIC_DROP_ARMED.store(false, Ordering::Relaxed);

        // 16 MB headroom for two 1-component archetypes.
        let arena = Arena::with_capacity(16 * 1024 * 1024);
        let mut bundle = ArchetypeBundle::new();
        let arch_id = ArchetypeId(20);

        // Build the OLD archetype and push one entity with a `PanicDropComp`
        // instance so that `Archetype::Drop` will trigger the user-defined
        // panicking drop_fn through `ComponentPool::Drop`.
        let _ = bundle
            .add_archetype_from_components_fallible(arch_id, &[PANIC_COMP_ID], &arena)
            .expect("slab has free space for the old archetype");
        {
            let archetype: &mut Archetype = bundle
                .get_archetype_mut(arch_id)
                .expect("just registered");
            let pool = archetype
                .component_pools
                .get_pool_mut(PANIC_COMP_ID)
                .expect("pool was registered for PANIC_COMP_ID");
            // PanicDropComp does not impl Component (Component trait requires
            // 4 methods including component_id; manually impl'ing it here
            // would shadow the registered_layout assumption). Use the
            // byte-level pool.add API instead — the registered drop_fn from
            // register_layout::<PanicDropComp> still fires on pool drop.
            let value = PanicDropComp(0xDEAD_BEEF);
            // SAFETY: value is a fully-initialised PanicDropComp; reading
            //   size_of::<PanicDropComp>() bytes out of it as &[u8] is sound
            //   while the &-borrow is live.
            let bytes: &[u8] = unsafe {
                std::slice::from_raw_parts(
                    std::ptr::addr_of!(value) as *const u8,
                    std::mem::size_of::<PanicDropComp>(),
                )
            };
            pool.add(bytes).expect("pool has capacity for one entity");
            // The pool now owns the bytes; suppress the local Drop so the
            // counter only ticks once when the pool drops, not twice.
            std::mem::forget(value);
        }

        // Snapshot the slot index BEFORE the replace so we can inspect the
        // bitset after the panic. `get_inland_id` returns the slot the old
        // archetype lives in.
        let slot_idx: usize = bundle
            .get_inland_id(arch_id)
            .expect("old archetype must be registered")
            .0;
        let word_idx: usize = slot_idx / 64;
        let bit_mask: u64 = 1u64 << (slot_idx % 64);
        // Sanity: the bit is set BEFORE the replace.
        assert!(
            bundle.occupied[word_idx] & bit_mask != 0,
            "pre-replace: occupied bit must be set for slot {slot_idx}",
        );

        // The replacement archetype: same id, but no components. Its
        // construction never touches `PANIC_DROP_COUNT`, so its presence in
        // the bundle after the (failed) replace is harmless to the count.
        let new_archetype = Archetype::new(arch_id, &arena);

        // Arm the panic and try the replace. We move `bundle` into the
        // catch_unwind via `AssertUnwindSafe` because the inner closure
        // mutates it; the panic-safety reasoning rests on the AB-R1 protocol
        // itself, which the test is here to verify.
        PANIC_DROP_ARMED.store(true, Ordering::Relaxed);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            bundle.add_archetype(new_archetype);
        }));
        // Disarm before any further drops fire.
        PANIC_DROP_ARMED.store(false, Ordering::Relaxed);

        assert!(
            result.is_err(),
            "add_archetype must propagate the user Drop panic",
        );

        // === AB-R1 post-conditions ===
        // Step 1a's clearing of the bit must have been done BEFORE
        // drop_in_place ran, so even though drop_in_place panicked the bit
        // is now cleared.
        let raw_id = arch_id.0;
        assert!(
            raw_id < bundle.id_to_slot.len(),
            "id_to_slot must still cover raw_id after a panicked replace",
        );
        assert_eq!(
            bundle.id_to_slot[raw_id], NO_SLOT,
            "AB-R1 Step 1b: id_to_slot[{raw_id}] must be NO_SLOT after panicked replace",
        );
        assert_eq!(
            bundle.occupied[word_idx] & bit_mask, 0,
            "AB-R1 Step 1a: occupied bit for slot {slot_idx} must be cleared after panicked replace",
        );

        // The user Drop fired exactly once during the panicked replace.
        // If AB-R1 were absent, the *next* observation (after dropping the
        // bundle below) would see this counter rise to 2.
        let count_after_replace = PANIC_DROP_COUNT.load(Ordering::Relaxed);
        assert_eq!(
            count_after_replace, 1,
            "user Drop must have run exactly once during the replace's drop_in_place",
        );

        // Drop the bundle. With AB-R1 the bundle's Drop walks `occupied`,
        // sees the bit for `slot_idx` cleared, and does NOT revisit the
        // half-dropped slot. The counter must therefore stay at 1.
        drop(bundle);

        let count_after_bundle_drop = PANIC_DROP_COUNT.load(Ordering::Relaxed);
        assert_eq!(
            count_after_bundle_drop, 1,
            "AB-R1 guarantee: ArchetypeBundle::Drop must NOT revisit the panicked slot \
             (observed double-drop count = {count_after_bundle_drop}, expected 1)",
        );
    }
}
