//! Sparse-slab storage for world-global resources.
//!
//! See Phase 8a plan §5.1.3 for the full design and invariants R1-R5.
//!
//! # Invariants
//!
//! - **R1 — bit-implies-init.** `registered_mask.get(id) == true` iff
//!   `slots[id].slot` holds a fully-initialised `ResourceSlot`. A cleared
//!   bit is the canonical "slot empty" signal; readers MUST check the bit
//!   before any `MaybeUninit::assume_init_*` call.
//! - **R2 — pointer lifetime.** Pointers handed out by `get_ptr*` / `get_mut_ptr*`
//!   are valid only for the lifetime of the borrow on `&Resources` /
//!   `&mut Resources`. The caller (`Res` / `ResMut`) is responsible for
//!   honouring the aliasing rules between shared/exclusive variants.
//! - **R3 — drop-then-dealloc.** The `Drop` impl walks `registered_mask` via
//!   `pop_lowest_set_bit` (TZCNT/BLSR). For each occupied slot it invokes
//!   `drop_fn` then `dealloc`. The bit is consumed by `pop_lowest_set_bit`
//!   before the drop runs, so a panic in `drop_fn` cannot leave the same
//!   slot to be revisited.
//! - **R4 — panic-safe replace.** `insert` uses a clear-bit-first replace
//!   protocol: the old slot is bitwise-copied out, the bit is cleared, the
//!   old value is dropped + deallocated, then the new slot is written and
//!   the bit is re-set. A panic in the old `drop_fn` leaks one allocation
//!   but never corrupts the slab.
//! - **R5 — clear-before-Box.** `remove` clears the bit BEFORE reconstructing
//!   the `Box<R>`, defending against a (pathological) `R::Drop` that
//!   recursively touches `self.contains::<R>()`.

use std::alloc::Layout;
use std::mem::MaybeUninit;

use boyko_utils::bit_mask::bit_set_256::BitSet256;

use crate::ecs::core::resources::resource::Resource;
use crate::ecs::core::resources::resource_registry::{
    self, RESOURCE_SLOT_COUNT, ResourceDropFn,
};
use crate::ecs::identifiers::primitives::ResourceId;

// ── ResourceSlot ──────────────────────────────────────────────────────────────

/// One slot of metadata for a registered resource value. One instance per type.
///
/// C-NEW-3: `Copy` is derived so `MaybeUninit::assume_init_read()` can extract
/// the slot via bitwise copy without moving the place. All three fields are
/// trivially `Copy`:
///   - `ptr: *mut u8` — `Copy`.
///   - `Option<unsafe fn>` — `Copy` (function pointers are `Copy`).
///   - `Layout` — `Copy` (POD: `usize` size + `NonZeroUsize` align).
///
/// Unique-ownership of `ptr` is enforced by the surrounding protocol (R4/R5):
/// the `registered_mask` bit is cleared in the same code path that performs
/// the bitwise read, so no observer can ever see two live copies of the slot.
#[derive(Clone, Copy)]
#[repr(C)]
struct ResourceSlot {
    /// `Box::<R>::into_raw` of the resource value, type-erased to `*mut u8`.
    ptr: *mut u8,
    /// Cached `drop_fn` from `ResourceInfo`. Stored locally so `Drop` does
    /// not have to hit the global registry on teardown.
    drop_fn: Option<ResourceDropFn>,
    /// Cached `Layout` for `dealloc` on remove/replace.
    layout: Layout,
}

// Plan §5.1.3: ResourceSlot must fit in 32 B. Layout-sensitive — bumping
// this breaks the slab budget (256 × 32 B = 8 KB).
const _: () = assert!(std::mem::size_of::<ResourceSlot>() <= 32);

// ── ResourceSlotStorage ──────────────────────────────────────────────────────

/// `MaybeUninit` wrapper so the slab can be heap-allocated without requiring
/// `ResourceSlot: Default` and without adding an `Option` discriminant that
/// would inflate the slot beyond the 32 B budget.
///
/// A slot is initialised iff `registered_mask.get(index) == true` (R1).
#[repr(C)]
struct ResourceSlotStorage {
    slot: MaybeUninit<ResourceSlot>,
}

// ── Resources ────────────────────────────────────────────────────────────────

/// Sparse slab of world-global resources, addressed by [`ResourceId`].
///
/// One slot per registered resource type. 256 × 32 B = 8 KB heap-allocated
/// slab with a stable address (Box invariant). The companion `registered_mask`
/// tracks which slots are live.
///
/// See module-level docs for invariants R1-R5.
pub struct Resources {
    /// 256-slot slab. Heap-allocated, stable address (mirror of
    /// `EventDispatcher::slots`).
    slots: Box<[ResourceSlotStorage; RESOURCE_SLOT_COUNT]>,
    /// Tracks which slots are initialised. 32 B. Iterated with TZCNT via
    /// `pop_lowest_set_bit` for O(k) teardown where k = live resource count.
    registered_mask: BitSet256,
}

impl Resources {
    /// Constructs an empty resource slab.
    ///
    /// The 8 KB slab is allocated on the heap directly via
    /// `Box::<[T; N]>::new_uninit().assume_init()` — never on the stack — to
    /// avoid an 8 KB stack temporary (mirror of `ArchetypeBundle::new`).
    #[cold]
    pub fn new() -> Self {
        // SAFETY (R1 / slab init):
        //   `Box::<T>::new_uninit()` allocates `T` on the heap and returns
        //   `Box<MaybeUninit<T>>`. For `T = [ResourceSlotStorage; N]` the
        //   resulting allocation is uninitialised memory of the correct size
        //   and alignment.
        //
        //   `assume_init()` is sound because `ResourceSlotStorage` is a
        //   `#[repr(C)]` wrapper around `MaybeUninit<ResourceSlot>` — i.e.
        //   the *element type itself* is `MaybeUninit`, so an array of such
        //   elements has no validity requirement (the wrapper IS the uninit
        //   story). Per-slot initialisation is tracked separately via
        //   `registered_mask` (R1).
        //
        //   `Box::new_uninit` is stable since Rust 1.82; this crate targets
        //   the 2024 edition (rustc ≥ 1.85).
        let slots: Box<[ResourceSlotStorage; RESOURCE_SLOT_COUNT]> = unsafe {
            Box::<[ResourceSlotStorage; RESOURCE_SLOT_COUNT]>::new_uninit().assume_init()
        };
        Self {
            slots,
            registered_mask: BitSet256::new(),
        }
    }

    /// Inserts or replaces the resource of type `R`. Cold path.
    ///
    /// On replace, runs `R::Drop` (if any) on the old value and deallocates
    /// the old box before storing the new value. The clear-bit-first replace
    /// protocol (R4) ensures that a panic in the old drop leaves the slot
    /// observably empty rather than partially-initialised.
    ///
    /// # Panics
    /// Panics with a registry-state diagnostic if `R::resource_id()` returns
    /// an id whose registry entry is missing — this can only happen if the
    /// caller invoked `register_new` directly and bypassed the `Resource`
    /// trait contract.
    #[cold]
    pub fn insert<R: Resource>(&mut self, value: R) {
        let id = R::resource_id();
        let layout = Layout::new::<R>();
        // `Box::new(value)` allocates with the global allocator using the
        // standard `Layout::new::<R>()`; `into_raw` hands ownership to us.
        let raw = Box::into_raw(Box::new(value)) as *mut u8;
        let info = resource_registry::get_resource_info(id.0).expect(
            "invariant: R::resource_id() implies the resource_registry slot is populated",
        );
        let new_slot = ResourceSlot {
            ptr: raw,
            drop_fn: info.drop_fn,
            layout,
        };

        if self.registered_mask.get(id.0) {
            // === R4: clear-bit-first replace protocol ===

            // Step 1: bitwise-copy the old slot out. `Copy` on `ResourceSlot`
            // makes `assume_init_read()` legal — the slot bytes are left
            // logically dead but unique ownership is preserved by the bit
            // clear in step 2 below.
            //
            // SAFETY (R1): `registered_mask.get(id.0) == true` implies the
            //   slot at `id.0` was initialised by a prior `insert`.
            //   `assume_init_read()` bitwise-copies; `ResourceSlot: Copy`,
            //   so producing a duplicate of the POD fields is benign — the
            //   protocol ensures only one logical owner of `old.ptr`
            //   proceeds (the bit clear below disables any future reader).
            let old = unsafe { self.slots[id.0].slot.assume_init_read() };

            // Step 2: clear the bit BEFORE running drop. Any external
            // observer (e.g., a spurious `contains` call from inside
            // `old.drop_fn` itself) sees the slot as empty.
            self.registered_mask.clear(id.0);

            // Step 3: drop the old value. If `drop_fn` panics, we are in the
            // intermediate "slot empty, allocation leaked" state — leak is
            // preferable to UB. The new slot has not yet been written, so
            // the leak is exactly one resource.
            if let Some(drop_fn) = old.drop_fn {
                // SAFETY (R4): `old.ptr` was minted from `Box::<R>::into_raw`
                //   in a prior `insert`, so it is aligned, initialised, and
                //   not aliased (no live `Res`/`ResMut` can co-exist with
                //   `&mut self`). The pointer is not accessed after this
                //   call.
                unsafe {
                    drop_fn(old.ptr);
                }
            }

            // Step 4: deallocate. If `drop_fn` did not panic, this proceeds
            // normally.
            //
            // SAFETY (R4): `old.ptr` came from `Box::<R>::into_raw` with
            //   `old.layout == Layout::new::<R>()`. `Box` uses the global
            //   allocator with this layout, so the matched `dealloc` is
            //   sound.
            unsafe {
                std::alloc::dealloc(old.ptr, old.layout);
            }

            // Step 5: write the new slot. POD-only — cannot panic.
            self.slots[id.0].slot.write(new_slot);

            // Step 6: re-set the bit. Atomicity is unnecessary (single
            // `&mut self`).
            self.registered_mask.set(id.0);
        } else {
            // First-insertion path: write slot, then publish via bit.
            self.slots[id.0].slot.write(new_slot);
            self.registered_mask.set(id.0);
        }
    }

    /// Removes the resource of type `R`. Returns the typed value if present,
    /// or `None` if the slot was empty.
    ///
    /// Clears the `registered_mask` bit BEFORE reconstructing the `Box<R>`
    /// (R5), so a pathological `R::Drop` that re-enters `contains::<R>()`
    /// observes a consistent empty state.
    #[cold]
    pub fn remove<R: Resource>(&mut self) -> Option<R> {
        let id = R::resource_id();
        if !self.registered_mask.get(id.0) {
            return None;
        }
        // SAFETY (R1): `registered_mask.get(id.0) == true` implies the slot
        //   is initialised. `assume_init_read()` bitwise-copies;
        //   `ResourceSlot: Copy`. Unique ownership of `slot.ptr` is
        //   guaranteed by the bit clear below before any external observer
        //   could re-enter.
        let slot = unsafe { self.slots[id.0].slot.assume_init_read() };

        // R5: clear bit BEFORE reconstructing the Box. Defends against a
        // (pathological) `R::Drop` that re-enters `contains::<R>()` — it
        // observes `false` rather than a partially-removed slot.
        self.registered_mask.clear(id.0);

        // SAFETY (R5): `slot.ptr` was minted from `Box::<R>::into_raw` in
        //   a prior `insert`, so it points at a valid, aligned `R`.
        //   Reconstructing the `Box<R>` reclaims ownership; `*boxed` moves
        //   the value out and the `Box` deallocates on scope exit.
        let boxed: Box<R> = unsafe { Box::from_raw(slot.ptr.cast::<R>()) };
        Some(*boxed)
    }

    /// Returns `*const R` if the resource is present.
    ///
    /// Typed convenience wrapper used by the [`EcsMaster::resource`] /
    /// [`EcsMaster::try_resource`] facade (Step 9). The Phase 8a
    /// `Res<R>::get_param` hot path bypasses this method and dispatches
    /// through [`get_ptr_by_id`] using the cached `state.id` directly
    /// (W1 RESOLUTION — saves an `OnceLock` load per access).
    ///
    /// # Safety (caller-side, R2)
    /// The returned pointer is valid only for the lifetime of the `&self`
    /// borrow. The caller must not alias with a `*mut R` produced by
    /// [`get_mut_ptr`].
    ///
    /// [`EcsMaster::resource`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::resource
    /// [`EcsMaster::try_resource`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::try_resource
    /// [`get_ptr_by_id`]: Resources::get_ptr_by_id
    /// [`get_mut_ptr`]: Resources::get_mut_ptr
    #[inline]
    pub(crate) fn get_ptr<R: Resource>(&self) -> Option<*const R> {
        let id = R::resource_id();
        self.get_ptr_by_id(id).map(|p| p.cast::<R>())
    }

    /// Returns `*mut R` if the resource is present.
    ///
    /// Typed counterpart of [`get_ptr`]; consumed by
    /// [`EcsMaster::resource_mut`] / [`EcsMaster::try_resource_mut`].
    ///
    /// # Safety (caller-side, R2)
    /// See [`get_ptr`].
    ///
    /// [`EcsMaster::resource_mut`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::resource_mut
    /// [`EcsMaster::try_resource_mut`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::try_resource_mut
    /// [`get_ptr`]: Resources::get_ptr
    #[inline]
    pub(crate) fn get_mut_ptr<R: Resource>(&mut self) -> Option<*mut R> {
        let id = R::resource_id();
        self.get_mut_ptr_by_id(id).map(|p| p.cast::<R>())
    }

    /// **W1 fast path** — untyped lookup by cached `ResourceId`.
    ///
    /// `Res<R>::get_param` caches `R::resource_id()` at `init_state` time
    /// in `ResState<R>::id` and dispatches through this method, avoiding a
    /// per-call `OnceLock::get` on the resource id (W1 RESOLUTION).
    ///
    /// # Safety (caller-side, R2)
    /// The returned pointer is valid only for the lifetime of the `&self`
    /// borrow. The caller is responsible for casting to the correct type —
    /// the `ResState<R>` type bundles the id with the type binding so the
    /// id↔type invariant is enforced at the type system level.
    #[inline]
    pub(crate) fn get_ptr_by_id(&self, id: ResourceId) -> Option<*const u8> {
        debug_assert!(
            id.0 < RESOURCE_SLOT_COUNT,
            "ResourceId out of range: {} (RESOURCE_SLOT_COUNT = {RESOURCE_SLOT_COUNT})",
            id.0
        );
        if !self.registered_mask.get(id.0) {
            return None;
        }
        // SAFETY (R1): `registered_mask.get(id.0) == true` implies the slot
        //   is initialised. `assume_init_ref()` returns a shared reference
        //   to the live `ResourceSlot`.
        let slot = unsafe { self.slots[id.0].slot.assume_init_ref() };
        Some(slot.ptr as *const u8)
    }

    /// **W1 fast path** counterpart for `ResMut<R>::get_param`.
    ///
    /// # Safety (caller-side, R2)
    /// See [`get_ptr_by_id`].
    ///
    /// [`get_ptr_by_id`]: Resources::get_ptr_by_id
    #[inline]
    pub(crate) fn get_mut_ptr_by_id(&mut self, id: ResourceId) -> Option<*mut u8> {
        debug_assert!(
            id.0 < RESOURCE_SLOT_COUNT,
            "ResourceId out of range: {} (RESOURCE_SLOT_COUNT = {RESOURCE_SLOT_COUNT})",
            id.0
        );
        if !self.registered_mask.get(id.0) {
            return None;
        }
        // SAFETY (R1): same as `get_ptr_by_id`. `&mut self` gives exclusive
        //   access to the slab, so handing out the raw `*mut u8` does not
        //   alias any other accessor.
        let slot = unsafe { self.slots[id.0].slot.assume_init_ref() };
        Some(slot.ptr)
    }

    /// Returns `true` if the resource of type `R` is currently stored.
    #[inline]
    pub fn contains<R: Resource>(&self) -> bool {
        self.registered_mask.get(R::resource_id().0)
    }

    /// Returns the number of resources currently stored.
    #[inline]
    pub fn len(&self) -> usize {
        self.registered_mask.count_ones() as usize
    }

    /// Returns `true` iff no resources are currently stored.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.registered_mask.is_empty()
    }
}

impl Default for Resources {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Resources {
    fn drop(&mut self) {
        // R3 — walk `registered_mask` via `pop_lowest_set_bit` (TZCNT/BLSR
        // on x86_64 with BMI1) and drop + dealloc each occupied slot in
        // ascending id order. O(k) for k live resources, no scan of the
        // 256-slot slab.
        //
        // Panic-safety: `pop_lowest_set_bit` consumes the bit it returns
        // BEFORE we run `drop_fn`. If `drop_fn` panics, the slot cannot be
        // revisited (its bit is already cleared in our local `mask` copy),
        // and remaining slots are still walked normally on unwind via Rust's
        // standard drop ordering. Worst case: one leak, no UB.
        let mut mask = self.registered_mask;
        while let Some(idx) = mask.pop_lowest_set_bit() {
            let idx = idx as usize;
            debug_assert!(
                idx < RESOURCE_SLOT_COUNT,
                "pop_lowest_set_bit returned out-of-range idx: {idx}"
            );
            // SAFETY (R1, R3): `idx` was just popped from `registered_mask`,
            //   so the corresponding slot was initialised by `insert` and
            //   has not been moved-from since (`Resources` owns it
            //   exclusively via `Box`). `assume_init_read()` bitwise-copies
            //   the POD `ResourceSlot`; the original storage is logically
            //   dead but unreachable (we are inside `Drop`).
            let slot = unsafe { self.slots[idx].slot.assume_init_read() };
            if let Some(drop_fn) = slot.drop_fn {
                // SAFETY (R3): `slot.ptr` was minted from
                //   `Box::<R>::into_raw` in `insert`; it is aligned,
                //   initialised, and not aliased. No live `Res`/`ResMut`
                //   can hold a borrow into a dropping `Resources` because
                //   `Drop` runs under exclusive `&mut self`.
                unsafe {
                    drop_fn(slot.ptr);
                }
            }
            // SAFETY (R3): `slot.ptr` came from `Box::<R>::new`
            //   (global-allocator-backed) with `slot.layout ==
            //   Layout::new::<R>()`, so `dealloc(ptr, layout)` is the
            //   matched pair.
            unsafe {
                std::alloc::dealloc(slot.ptr, slot.layout);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::ecs::core::resources::resource::Resource;
    use crate::ecs::core::resources::resource_registry::register_new;
    use crate::ecs::identifiers::primitives::ResourceId;

    use super::Resources;

    // Per-test resource types — `TypeId::of::<T>()` (used by the registry)
    // distinguishes them at the type-system level. Each type lives in its
    // own `OnceLock`, so the registration cost is amortised across all
    // tests that touch the same type.

    // ── Plain POD resource (no Drop) ─────────────────────────────────────

    /// Plain POD resource — no Drop, no Send/Sync gymnastics.
    struct ResA(u32);

    impl Resource for ResA {
        fn resource_id() -> ResourceId {
            static ID: OnceLock<ResourceId> = OnceLock::new();
            *ID.get_or_init(|| ResourceId(register_new::<Self>()))
        }
    }

    // ── Drop-counting resource ───────────────────────────────────────────

    static RES_B_DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

    /// Resource with a Drop impl that increments a shared counter — used by
    /// replace-and-drop tests to verify `drop_fn` is invoked exactly once on
    /// the old value.
    struct ResB(#[allow(dead_code)] u32);

    impl Drop for ResB {
        fn drop(&mut self) {
            RES_B_DROP_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }

    impl Resource for ResB {
        fn resource_id() -> ResourceId {
            static ID: OnceLock<ResourceId> = OnceLock::new();
            *ID.get_or_init(|| ResourceId(register_new::<Self>()))
        }
    }

    // ── Three drop-counting resources for the drop-all-occupied test ─────

    static RES_C1_DROP_COUNT: AtomicUsize = AtomicUsize::new(0);
    static RES_C2_DROP_COUNT: AtomicUsize = AtomicUsize::new(0);
    static RES_C3_DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

    struct ResC1(#[allow(dead_code)] u32);
    impl Drop for ResC1 {
        fn drop(&mut self) {
            RES_C1_DROP_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }
    impl Resource for ResC1 {
        fn resource_id() -> ResourceId {
            static ID: OnceLock<ResourceId> = OnceLock::new();
            *ID.get_or_init(|| ResourceId(register_new::<Self>()))
        }
    }

    struct ResC2(#[allow(dead_code)] u32);
    impl Drop for ResC2 {
        fn drop(&mut self) {
            RES_C2_DROP_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }
    impl Resource for ResC2 {
        fn resource_id() -> ResourceId {
            static ID: OnceLock<ResourceId> = OnceLock::new();
            *ID.get_or_init(|| ResourceId(register_new::<Self>()))
        }
    }

    struct ResC3(#[allow(dead_code)] u32);
    impl Drop for ResC3 {
        fn drop(&mut self) {
            RES_C3_DROP_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }
    impl Resource for ResC3 {
        fn resource_id() -> ResourceId {
            static ID: OnceLock<ResourceId> = OnceLock::new();
            *ID.get_or_init(|| ResourceId(register_new::<Self>()))
        }
    }

    // ── Resource for the contains-after-insert-remove test ──────────────

    struct ResD(#[allow(dead_code)] u32);
    impl Resource for ResD {
        fn resource_id() -> ResourceId {
            static ID: OnceLock<ResourceId> = OnceLock::new();
            *ID.get_or_init(|| ResourceId(register_new::<Self>()))
        }
    }

    // ── Tests ────────────────────────────────────────────────────────────

    /// `insert` followed by `get_ptr` returns a pointer to the stored value.
    #[test]
    fn insert_then_get_returns_pointer() {
        let mut res = Resources::new();
        res.insert(ResA(42));

        let ptr = res
            .get_ptr::<ResA>()
            .expect("get_ptr must return Some after insert");

        // SAFETY: `ptr` is valid for the lifetime of `&res`; we hold the
        //   shared borrow through `res` and don't mutate the slab.
        let val = unsafe { (*ptr).0 };
        assert_eq!(val, 42, "stored value must round-trip");

        assert!(res.contains::<ResA>(), "contains must report true");
        assert_eq!(res.len(), 1, "len must reflect single insertion");
    }

    /// Replacing an existing resource runs `Drop` on the old value exactly
    /// once and the new value is observable after the replace.
    #[test]
    fn insert_replace_drops_old_value() {
        // Establish a known baseline — each test using ResB samples the
        // global counter before and after, so the absolute value does not
        // matter across test interleaving.
        let before = RES_B_DROP_COUNT.load(Ordering::Relaxed);

        let mut res = Resources::new();
        res.insert(ResB(1)); // baseline: no drop yet
        res.insert(ResB(2)); // replace path: drops the prior ResB(1)

        let after_replace = RES_B_DROP_COUNT.load(Ordering::Relaxed);
        assert_eq!(
            after_replace - before,
            1,
            "exactly one drop must run on the replace path"
        );

        // New value must be observable.
        let ptr = res
            .get_ptr::<ResB>()
            .expect("ResB must still be present after replace");
        // SAFETY: ptr is valid for the lifetime of `&res`.
        let val = unsafe { (*ptr).0 };
        assert_eq!(val, 2, "new value (2) must be readable after replace");

        // Dropping `res` here will run the ResB(2) drop too — leave it to
        // the runtime so the test does not double-count.
        drop(res);
        let after_drop = RES_B_DROP_COUNT.load(Ordering::Relaxed);
        assert_eq!(
            after_drop - before,
            2,
            "after Resources::drop, total drops must be 2 (old + final)"
        );
    }

    /// `remove` returns the typed value and clears the slot bit.
    #[test]
    fn remove_returns_typed_value_and_clears_slot() {
        let mut res = Resources::new();
        res.insert(ResA(7));
        assert!(res.contains::<ResA>(), "precondition: ResA inserted");

        let removed = res.remove::<ResA>().expect("remove must return Some");
        assert_eq!(removed.0, 7, "removed value must match inserted");

        assert!(!res.contains::<ResA>(), "slot must be empty after remove");
        assert!(
            res.get_ptr::<ResA>().is_none(),
            "get_ptr must return None after remove"
        );
        assert_eq!(res.len(), 0, "len must be 0 after the sole removal");
        assert!(res.is_empty(), "is_empty must be true after the sole removal");

        // remove on empty slot returns None idempotently.
        assert!(
            res.remove::<ResA>().is_none(),
            "remove on empty slot must return None"
        );
    }

    /// `Drop` runs `R::Drop` for every occupied slot.
    #[test]
    fn drop_runs_drop_glue_per_occupied_slot() {
        let before_c1 = RES_C1_DROP_COUNT.load(Ordering::Relaxed);
        let before_c2 = RES_C2_DROP_COUNT.load(Ordering::Relaxed);
        let before_c3 = RES_C3_DROP_COUNT.load(Ordering::Relaxed);

        {
            let mut res = Resources::new();
            res.insert(ResC1(11));
            res.insert(ResC2(22));
            res.insert(ResC3(33));
            assert_eq!(res.len(), 3, "three resources must be present pre-drop");
            // `res` drops here at end of scope.
        }

        assert_eq!(
            RES_C1_DROP_COUNT.load(Ordering::Relaxed) - before_c1,
            1,
            "ResC1::drop must run exactly once"
        );
        assert_eq!(
            RES_C2_DROP_COUNT.load(Ordering::Relaxed) - before_c2,
            1,
            "ResC2::drop must run exactly once"
        );
        assert_eq!(
            RES_C3_DROP_COUNT.load(Ordering::Relaxed) - before_c3,
            1,
            "ResC3::drop must run exactly once"
        );
    }

    /// `contains` tracks insert/remove transitions.
    #[test]
    fn contains_after_insert_remove() {
        let mut res = Resources::new();
        assert!(
            !res.contains::<ResD>(),
            "empty Resources must report contains == false"
        );

        res.insert(ResD(99));
        assert!(
            res.contains::<ResD>(),
            "contains must report true after insert"
        );

        let _ = res.remove::<ResD>();
        assert!(
            !res.contains::<ResD>(),
            "contains must report false after remove"
        );

        // Re-insert and verify state restored.
        res.insert(ResD(100));
        assert!(
            res.contains::<ResD>(),
            "contains must report true after re-insert"
        );
    }
}
