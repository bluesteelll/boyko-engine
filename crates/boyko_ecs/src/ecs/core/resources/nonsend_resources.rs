//! Sparse-slab storage for **non-`Send`** world-global resources (Phase 4
//! Seam 2 — D6 + CR-A).
//!
//! Structurally identical to [`Resources`] — a 256-slot type-erased slab of
//! `(*mut u8 data ptr, Option<drop_fn>, Layout, TypeId)` records — but it
//! homes types that carry NO `Send + Sync` bound (RHI handles, FFI pointers,
//! `Rc`-state).
//!
//! # Type-erasure keeps the slab off SEND1's hot path (CR-A)
//!
//! Like [`Resources`], the slab stores only the type-**erased** pointer +
//! drop fn + `TypeId` — never an inline `R` value. The `!Send` payload `R`
//! is reachable ONLY through the `unsafe` `NonSendRes` / `NonSendResMut`
//! accessors, whose SAFETY contract is the apply-window single-thread-touch
//! invariant (a NonSend system declares universal access → resolves
//! `SystemKind::CpuExclusive` → runs solo on the dispatcher when
//! `running == 0`). Adding this slab as an `EcsMaster` field therefore needs
//! NO NEW `unsafe impl` beyond the existing SEND1 one (which already covers
//! the `!Send` raw-pointer interior of the sibling [`Resources`] slab) — see
//! SEND10 in `ecs_master.rs`.
//!
//! # Invariants (mirror of [`Resources`] R1-R5)
//!
//! - **N1 — bit-implies-init.** `registered_mask.get(id) == true` iff
//!   `slots[id]` holds a fully-initialised `NonSendSlot`.
//! - **N2 — single-thread touch.** Every `insert` / read / drop of a NonSend
//!   value happens on the value's owning (dispatcher) thread (CR-A). The
//!   slab API takes `&self` / `&mut self`, and the `!Send` payload is only
//!   dereferenced behind the `unsafe` SystemParam accessors.
//! - **N3 — drop-then-dealloc.** `Drop` walks `registered_mask`, invoking
//!   `drop_fn` then `dealloc` per occupied slot, consuming the bit first.
//! - **N4 — panic-safe replace.** `insert` clears the bit before dropping
//!   the old value (leak-not-UB on a panicking drop).
//! - **N5 — ZST guard.** Manual `dealloc` is skipped for `layout.size() == 0`
//!   (a ZST `Box` pointer the allocator never handed out).
//!
//! [`Resources`]: crate::ecs::core::resources::resources::Resources

use std::alloc::Layout;
use std::any::TypeId;
use std::collections::HashMap;
use std::mem::MaybeUninit;
use std::sync::{Mutex, OnceLock};

use boyko_utils::bit_mask::bit_set_256::BitSet256;

use crate::ecs::core::resources::nonsend_resource_registry::{
    self, NON_SEND_RESOURCE_SLOT_COUNT, NonSendDropFn,
};
use crate::ecs::core::resources::resource::NonSendResource;
use crate::ecs::identifiers::primitives::NonSendResourceId;

// ── NonSendSlot ────────────────────────────────────────────────────────────

/// One slot of metadata for a registered non-send resource value.
///
/// `Copy` is derived so `MaybeUninit::assume_init_read()` can bitwise-extract
/// the slot without moving the place — all four fields are trivially `Copy`
/// (`*mut u8`, `Option<unsafe fn>`, `Layout`, `TypeId`). Unique ownership of
/// `ptr` is enforced by the surrounding clear-bit-first protocol (N4).
#[derive(Clone, Copy)]
#[repr(C)]
struct NonSendSlot {
    /// `Box::<R>::into_raw` of the value, type-erased to `*mut u8`.
    ptr: *mut u8,
    /// Cached `drop_fn`; `None` iff `!needs_drop::<R>()`.
    drop_fn: Option<NonSendDropFn>,
    /// Cached `Layout` for `dealloc` on replace / Drop.
    layout: Layout,
    /// `TypeId::of::<R>()` of the stored value, for the debug type-tag check.
    type_id: TypeId,
}

/// `MaybeUninit` wrapper so the slab can be heap-allocated without requiring
/// `NonSendSlot: Default`. A slot is initialised iff
/// `registered_mask.get(index) == true` (N1).
#[repr(C)]
struct NonSendSlotStorage {
    slot: MaybeUninit<NonSendSlot>,
}

// ── NonSendResources ───────────────────────────────────────────────────────

/// Sparse slab of `!Send` world-global resources, addressed by
/// [`NonSendResourceId`].
///
/// One slot per registered non-send type. 256 × slot bytes heap-allocated
/// with a stable address (Box invariant). The companion `registered_mask`
/// tracks which slots are live. See module docs for invariants N1-N5.
pub struct NonSendResources {
    /// 256-slot slab. Heap-allocated, stable address.
    slots: Box<[NonSendSlotStorage; NON_SEND_RESOURCE_SLOT_COUNT]>,
    /// Tracks which slots are initialised. Iterated with TZCNT via
    /// `pop_lowest_set_bit` for O(k) teardown.
    registered_mask: BitSet256,
    /// Phase 5 Option C (M2) — debug-only tripwire: the thread that first
    /// inserted a `!Send` value (the slab's owning thread). Every projection
    /// (`UnsafeEcsCell::nonsend_resources[_mut]` + `DispatcherToken`) asserts
    /// the current thread against this via [`debug_assert_owning_thread`], so a
    /// projection from the wrong thread fails loud in debug long before it can
    /// be UB in release. `None` until the first insert (an empty slab has no
    /// `!Send` payload to touch). Zero release cost.
    ///
    /// [`debug_assert_owning_thread`]: NonSendResources::debug_assert_owning_thread
    #[cfg(debug_assertions)]
    owning_thread: Option<std::thread::ThreadId>,
}

impl NonSendResources {
    /// Constructs an empty non-send resource slab.
    ///
    /// The slab is heap-allocated via `Box::<[T; N]>::new_uninit()` — never on
    /// the stack — mirroring `Resources::new`.
    #[cold]
    pub fn new() -> Self {
        // SAFETY (N1 / slab init): `Box::<[NonSendSlotStorage; N]>::new_uninit()`
        //   allocates uninitialised heap memory of the correct size/align.
        //   `assume_init()` is sound because the element type is a `#[repr(C)]`
        //   wrapper around `MaybeUninit<NonSendSlot>` — the wrapper IS the
        //   uninit story; per-slot init is tracked by `registered_mask` (N1).
        let slots: Box<[NonSendSlotStorage; NON_SEND_RESOURCE_SLOT_COUNT]> = unsafe {
            Box::<[NonSendSlotStorage; NON_SEND_RESOURCE_SLOT_COUNT]>::new_uninit()
                .assume_init()
        };
        Self {
            slots,
            registered_mask: BitSet256::new(),
            #[cfg(debug_assertions)]
            owning_thread: None,
        }
    }

    /// Inserts or replaces the non-send resource of type `R`. Cold path.
    ///
    /// On replace, runs `R::Drop` (if any) on the old value and deallocates
    /// the old box before storing the new value. The clear-bit-first replace
    /// protocol (N4) ensures a panic in the old drop leaves the slot
    /// observably empty rather than partially-initialised.
    ///
    /// # Single-thread contract (N2)
    /// `R` is `!Send`; this method must run on `R`'s owning thread. The
    /// `EcsMaster` facade calls it under `&mut self` on the dispatcher.
    #[cold]
    pub fn insert<R: NonSendResource>(&mut self, value: R) {
        // M2 (Phase 5 Option C): stamp the owning thread on the FIRST insert —
        // the thread that homes the `!Send` payload. Every subsequent
        // projection asserts against it. Debug-only; zero release cost.
        #[cfg(debug_assertions)]
        {
            let current = std::thread::current().id();
            match self.owning_thread {
                None => self.owning_thread = Some(current),
                Some(owner) => debug_assert_eq!(
                    current, owner,
                    "invariant M2/N2: NonSendResources::insert ran on a thread \
                     other than the slab's owning thread — !Send values must be \
                     inserted only on the owning (dispatcher) thread"
                ),
            }
        }
        let id = nonsend_id::<R>();
        let layout = Layout::new::<R>();
        // ZST contract (N5): for a zero-sized `R`, `Box::new` performs NO heap
        // allocation; `Box::into_raw` returns a dangling-but-aligned pointer.
        // The matching reclaim is `Box::from_raw` (ZST-safe); the ONLY op that
        // must not run for a ZST is the manual `dealloc`, guarded below.
        let raw = Box::into_raw(Box::new(value)) as *mut u8;
        let info = nonsend_resource_registry::get_nonsend_resource_info(id.0).expect(
            "invariant: nonsend_id::<R>() implies the registry slot is populated",
        );
        let new_slot = NonSendSlot {
            ptr: raw,
            drop_fn: info.drop_fn,
            layout,
            type_id: TypeId::of::<R>(),
        };

        if self.registered_mask.get(id.0) {
            // === N4: clear-bit-first replace protocol ===

            // SAFETY (N1): `registered_mask.get(id.0) == true` ⇒ the slot was
            //   initialised by a prior `insert`. `assume_init_read()`
            //   bitwise-copies the POD `NonSendSlot`; the clear below disables
            //   any future reader before the old value is dropped.
            let old = unsafe { self.slots[id.0].slot.assume_init_read() };
            self.registered_mask.clear(id.0);

            if let Some(drop_fn) = old.drop_fn {
                // SAFETY (N2, N4): `old.ptr` was minted from `Box::<R>::into_raw`
                //   in a prior `insert` on this thread; it is aligned,
                //   initialised, not aliased (no live `NonSendRes`/`NonSendResMut`
                //   can co-exist with `&mut self`), and dropped on the owning
                //   thread. The pointer is not accessed after this call.
                unsafe {
                    drop_fn(old.ptr);
                }
            }

            // ZST guard (N5): a zero-sized `R` was never heap-allocated; skip
            // the manual free.
            //
            // SAFETY (N4): when `old.layout.size() != 0`, `old.ptr` came from
            //   `Box::<R>::into_raw` of a sized `R` with `old.layout ==
            //   Layout::new::<R>()`; the matched `dealloc` is sound.
            if old.layout.size() != 0 {
                unsafe {
                    std::alloc::dealloc(old.ptr, old.layout);
                }
            }

            self.slots[id.0].slot.write(new_slot);
            self.registered_mask.set(id.0);
        } else {
            self.slots[id.0].slot.write(new_slot);
            self.registered_mask.set(id.0);
        }
    }

    /// Removes the non-send resource of type `R`, returning it if present.
    ///
    /// Clears the `registered_mask` bit BEFORE reconstructing the `Box<R>`
    /// (N4/N5 mirror), so a pathological `R::Drop` re-entering `contains`
    /// observes a consistent empty state.
    ///
    /// # Single-thread contract (N2)
    /// Must run on `R`'s owning thread (the returned `R` is `!Send`).
    #[cold]
    pub fn remove<R: NonSendResource>(&mut self) -> Option<R> {
        let id = nonsend_id::<R>();
        if !self.registered_mask.get(id.0) {
            return None;
        }
        // SAFETY (N1): `registered_mask.get(id.0) == true` ⇒ the slot is
        //   initialised; `assume_init_read()` bitwise-copies the POD slot.
        let slot = unsafe { self.slots[id.0].slot.assume_init_read() };
        debug_assert_eq!(
            slot.type_id,
            TypeId::of::<R>(),
            "invariant N1: NonSendResourceId is type-bound to R"
        );
        self.registered_mask.clear(id.0);

        // SAFETY (N2): `slot.ptr` was minted from `Box::<R>::into_raw` in a
        //   prior `insert` on this thread; reconstructing the `Box<R>` reclaims
        //   ownership and `*boxed` moves the value out. `Box::from_raw` is
        //   ZST-safe.
        let boxed: Box<R> = unsafe { Box::from_raw(slot.ptr.cast::<R>()) };
        Some(*boxed)
    }

    /// Returns `*const u8` to the stored value if present (untyped).
    ///
    /// Hot accessor for `NonSendRes<R>::get_param`. The caller casts to `*const R`
    /// using its own type binding.
    ///
    /// # Safety (caller-side, N2)
    /// The returned pointer is valid only for the lifetime of the `&self`
    /// borrow, and the value behind it is `!Send` — the caller must touch it
    /// only on the owning (dispatcher) thread.
    #[inline]
    pub(crate) fn get_ptr_by_id(&self, id: NonSendResourceId) -> Option<*const u8> {
        debug_assert!(
            id.0 < NON_SEND_RESOURCE_SLOT_COUNT,
            "NonSendResourceId out of range: {} (slots = {NON_SEND_RESOURCE_SLOT_COUNT})",
            id.0
        );
        if !self.registered_mask.get(id.0) {
            return None;
        }
        // SAFETY (N1): the bit is set ⇒ the slot is initialised;
        //   `assume_init_ref()` returns a shared reference to the live slot.
        let slot = unsafe { self.slots[id.0].slot.assume_init_ref() };
        Some(slot.ptr as *const u8)
    }

    /// Returns `*mut u8` to the stored value if present (untyped).
    ///
    /// Hot accessor for `NonSendResMut<R>::get_param`.
    ///
    /// # Safety (caller-side, N2)
    /// See [`get_ptr_by_id`](Self::get_ptr_by_id).
    #[inline]
    pub(crate) fn get_mut_ptr_by_id(&mut self, id: NonSendResourceId) -> Option<*mut u8> {
        debug_assert!(
            id.0 < NON_SEND_RESOURCE_SLOT_COUNT,
            "NonSendResourceId out of range: {} (slots = {NON_SEND_RESOURCE_SLOT_COUNT})",
            id.0
        );
        if !self.registered_mask.get(id.0) {
            return None;
        }
        // SAFETY (N1, FIX-4 / X3): the bit is set ⇒ the slot is initialised;
        //   `&mut self` gives exclusive access. Project through `assume_init_mut`
        //   (a MUTABLE place) rather than `assume_init_ref`, so the returned
        //   `*mut u8` is derived from a write-capable provenance — deriving a
        //   `*mut` through a shared `assume_init_ref` projection would narrow
        //   provenance to read-only under Tree Borrows, a write-UB hazard for the
        //   `NonSendResMut` caller. Handing out the raw pointer does not alias
        //   any other accessor (exclusive `&mut self`).
        let slot = unsafe { self.slots[id.0].slot.assume_init_mut() };
        Some(slot.ptr)
    }

    /// Returns `*const R` to the stored value if present (typed).
    ///
    /// Used by the [`EcsMaster`] direct-API facade
    /// (`try_non_send_resource`); the `NonSendRes<R>::get_param` hot path
    /// bypasses this and dispatches through [`get_ptr_by_id`](Self::get_ptr_by_id)
    /// using the cached id.
    ///
    /// # Safety (caller-side, N2)
    /// The returned pointer is valid only for the `&self` borrow; `R` is
    /// `!Send`, so the caller must touch it on the owning thread only.
    ///
    /// [`EcsMaster`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster
    #[inline]
    pub(crate) fn get_ptr<R: NonSendResource>(&self) -> Option<*const R> {
        self.get_ptr_by_id(nonsend_id::<R>()).map(|p| p.cast::<R>())
    }

    /// Returns `*mut R` to the stored value if present (typed). Counterpart of
    /// [`get_ptr`](Self::get_ptr) for the `&mut` facade.
    ///
    /// # Safety (caller-side, N2)
    /// See [`get_ptr`](Self::get_ptr).
    #[inline]
    pub(crate) fn get_mut_ptr<R: NonSendResource>(&mut self) -> Option<*mut R> {
        self.get_mut_ptr_by_id(nonsend_id::<R>())
            .map(|p| p.cast::<R>())
    }

    /// Returns `true` if the non-send resource of type `R` is currently
    /// stored.
    #[inline]
    pub fn contains<R: NonSendResource>(&self) -> bool {
        self.registered_mask.get(nonsend_id::<R>().0)
    }

    /// Returns the number of non-send resources currently stored.
    #[inline]
    pub fn len(&self) -> usize {
        self.registered_mask.count_ones() as usize
    }

    /// Returns `true` iff no non-send resources are currently stored.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.registered_mask.is_empty()
    }

    /// Phase 5 Option C (M2) — debug-only tripwire that the current thread is
    /// the slab's owning (dispatcher) thread.
    ///
    /// Called by every `!Send` projection
    /// (`UnsafeEcsCell::nonsend_resources[_mut]`) so a projection from the wrong
    /// thread fails loud in debug. Compiles to NOTHING in release (the whole
    /// body is `#[cfg(debug_assertions)]`), so it costs zero on the hot path.
    /// A slab with no insert yet has no `owning_thread` and no `!Send` payload,
    /// so the check is vacuous there.
    #[inline]
    pub(crate) fn debug_assert_owning_thread(&self) {
        #[cfg(debug_assertions)]
        if let Some(owner) = self.owning_thread {
            debug_assert_eq!(
                std::thread::current().id(),
                owner,
                "invariant M2/N2: a NonSend resource was projected off the slab's \
                 owning (dispatcher) thread — the !Send payload must be touched \
                 only on the thread that inserted it"
            );
        }
    }
}

/// Mints (once) and returns the [`NonSendResourceId`] for `R`.
///
/// `NonSendResource` is a bare marker trait with no `resource_id()` method, so
/// the per-type id cache is interned through a process-global `TypeId`-keyed
/// map of leaked `OnceLock`s ([`type_keyed_slot`]). A generic-fn-local
/// `static` would collapse across monomorphisations (the Phase 12.5 `static
/// SLOT` bug), so the map keyed on `TypeId::of::<R>()` is the correct
/// per-`R` discriminator. The map is touched only on the cold registration
/// path (`NonSendRes::init_state` / `NonSendResources::insert`), never on a
/// hot loop.
#[inline]
pub(crate) fn nonsend_id<R: NonSendResource>() -> NonSendResourceId {
    *type_keyed_slot::<R>()
        .get_or_init(|| NonSendResourceId(nonsend_resource_registry::register_new::<R>()))
}

/// Process-global, `TypeId`-keyed map of `TypeId → &'static OnceLock<id>`.
///
/// One leaked `OnceLock<NonSendResourceId>` per distinct `R`, so two distinct
/// types never share an id slot. The `Mutex` guards only the cold insert into
/// the map; once a slot exists, [`nonsend_id`] reads it through `OnceLock`.
fn type_keyed_slot<R: 'static>() -> &'static OnceLock<NonSendResourceId> {
    static MAP: OnceLock<Mutex<HashMap<TypeId, &'static OnceLock<NonSendResourceId>>>> =
        OnceLock::new();
    let map = MAP.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().expect("invariant: nonsend id map mutex not poisoned");
    guard
        .entry(TypeId::of::<R>())
        .or_insert_with(|| Box::leak(Box::new(OnceLock::new())))
}

impl Default for NonSendResources {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for NonSendResources {
    fn drop(&mut self) {
        // N3 — walk `registered_mask` via `pop_lowest_set_bit` and drop +
        // dealloc each occupied slot. The bit is consumed before `drop_fn`
        // runs, so a panicking drop cannot revisit the slot (leak, not UB).
        //
        // N2: `Drop` runs under `&mut self` on the owning thread (the world is
        // torn down on its owning thread), so dropping the `!Send` values here
        // is sound.
        let mut mask = self.registered_mask;
        while let Some(idx) = mask.pop_lowest_set_bit() {
            let idx = idx as usize;
            debug_assert!(
                idx < NON_SEND_RESOURCE_SLOT_COUNT,
                "pop_lowest_set_bit returned out-of-range idx: {idx}"
            );
            // SAFETY (N1, N3): `idx` was just popped from `registered_mask`, so
            //   the slot was initialised by `insert` and not moved-from since
            //   (`NonSendResources` owns it exclusively). `assume_init_read()`
            //   bitwise-copies the POD slot; the original storage is logically
            //   dead but unreachable (we are inside `Drop`).
            let slot = unsafe { self.slots[idx].slot.assume_init_read() };
            if let Some(drop_fn) = slot.drop_fn {
                // SAFETY (N2, N3): `slot.ptr` was minted from `Box::<R>::into_raw`
                //   in `insert`; aligned, initialised, not aliased, dropped on
                //   the owning thread.
                unsafe {
                    drop_fn(slot.ptr);
                }
            }
            // ZST guard (N5): skip `dealloc` for a dangling ZST pointer.
            //
            // SAFETY (N3): when `slot.layout.size() != 0`, `slot.ptr` came from
            //   `Box::<R>::new` with `slot.layout == Layout::new::<R>()`.
            if slot.layout.size() != 0 {
                unsafe {
                    std::alloc::dealloc(slot.ptr, slot.layout);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `!Send` resource (raw pointer interior). No Drop.
    struct NonSendA(u32, #[allow(dead_code)] *const u8);
    impl NonSendResource for NonSendA {}

    /// `!Send` resource WITH a Drop impl — drives the drop_fn path.
    struct NonSendDrop {
        flag: *mut bool,
    }
    impl Drop for NonSendDrop {
        fn drop(&mut self) {
            // SAFETY: `flag` points at a live `bool` owned by the test stack;
            //   the value is dropped exactly once on the owning thread.
            unsafe {
                *self.flag = true;
            }
        }
    }
    impl NonSendResource for NonSendDrop {}

    #[test]
    fn insert_then_get_round_trips() {
        let mut res = NonSendResources::new();
        res.insert(NonSendA(42, std::ptr::null()));
        assert!(res.contains::<NonSendA>());
        assert_eq!(res.len(), 1);

        let id = nonsend_id::<NonSendA>();
        let ptr = res
            .get_ptr_by_id(id)
            .expect("get_ptr_by_id must return Some after insert");
        // SAFETY: `ptr` is valid for the `&res` borrow and points at a live
        //   `NonSendA`; we read its first field on the owning thread.
        let v = unsafe { (*(ptr as *const NonSendA)).0 };
        assert_eq!(v, 42, "stored value must round-trip");
    }

    /// Phase 5 Option C (M2) — a NonSend resource inserted on thread A and
    /// projected on thread B trips the debug-only owning-thread tripwire.
    ///
    /// `NonSendResources` is `!Send` (raw-pointer interior), so the slab cannot
    /// be moved to thread B directly; we hand the checker a raw `*const` across
    /// the boundary (sound for the read-only `debug_assert_owning_thread` call,
    /// which the test only reaches to PROVE it panics). Debug-only — the M2 stamp
    /// + check compile to nothing in release, so the panic cannot fire there.
    #[cfg(debug_assertions)]
    #[test]
    fn projection_off_owning_thread_trips_m2() {
        // Insert on thread A (this thread) → stamps `owning_thread = A`.
        let res = {
            let mut r = NonSendResources::new();
            r.insert(NonSendA(1, std::ptr::null()));
            r
        };

        /// Carries a `*const NonSendResources` across the thread boundary. The
        /// pointee stays pinned on thread A for the join; thread B only calls the
        /// read-only checker, which we EXPECT to panic.
        struct SendPtr(*const NonSendResources);
        // SAFETY: the test keeps `res` alive on thread A across the join below,
        //   and thread B touches it only through the read-only
        //   `debug_assert_owning_thread` (which panics before any field access).
        unsafe impl Send for SendPtr {}

        let ptr = SendPtr(&res as *const NonSendResources);
        let handle = std::thread::spawn(move || {
            let ptr = ptr; // move the wrapper in
            // SAFETY (test-only): `ptr.0` points at the live `res` on thread A;
            //   `debug_assert_owning_thread` reads only `owning_thread` and panics
            //   because the current thread (B) != the stamped owner (A).
            let slab: &NonSendResources = unsafe { &*ptr.0 };
            slab.debug_assert_owning_thread();
        });
        let joined = handle.join();
        assert!(
            joined.is_err(),
            "M2: projecting a NonSend resource off the owning thread must panic in debug"
        );
        // Keep `res` alive until after the join.
        drop(res);
    }

    #[test]
    fn remove_returns_value_and_clears() {
        let mut res = NonSendResources::new();
        res.insert(NonSendA(7, std::ptr::null()));
        let removed = res.remove::<NonSendA>().expect("remove must return Some");
        assert_eq!(removed.0, 7);
        assert!(!res.contains::<NonSendA>());
        assert!(res.is_empty());
        assert!(res.remove::<NonSendA>().is_none(), "double remove is None");
    }

    #[test]
    fn drop_runs_drop_glue() {
        let mut dropped = false;
        {
            let mut res = NonSendResources::new();
            res.insert(NonSendDrop {
                flag: &mut dropped as *mut bool,
            });
            // `res` drops here, running NonSendDrop::drop.
        }
        assert!(dropped, "NonSendResources::drop must run R::drop");
    }

    #[test]
    fn replace_drops_old_value() {
        let mut dropped_first = false;
        let mut res = NonSendResources::new();
        res.insert(NonSendDrop {
            flag: &mut dropped_first as *mut bool,
        });
        let mut dropped_second = false;
        // Replace: must drop the first value exactly once.
        res.insert(NonSendDrop {
            flag: &mut dropped_second as *mut bool,
        });
        assert!(dropped_first, "replace must drop the prior value");
        assert!(!dropped_second, "the new value is still live after replace");
        drop(res);
        assert!(dropped_second, "teardown drops the surviving value");
    }
}
