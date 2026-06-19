//! The [`DenseBuildView`] / [`DenseSolveView`] type-split — the structural fix
//! for the SP4 race (Dense plan, Decision 8).
//!
//! The SP4 race was an unsound whole-buffer `&mut [T]` reborrow handed to
//! parallel workers. The fix is structural, not a discipline: split the dense
//! column's access into two view types so the whole-buffer mutable reborrow is
//! **un-typeable** on the path workers ever touch.
//!
//! * [`DenseBuildView`] (`!Send`) — the single-threaded structural surface:
//!   the ONLY view that can expose a whole-buffer mutable slice and run
//!   push / tombstone / compact. This is where refill happens.
//! * [`DenseSolveView`] (`Copy`, `Send + Sync`, ~32 B) — the parallel solve
//!   surface: per-slot `row_ptr` / `len` / `is_live` ONLY. There is NO
//!   `as_mut_slice`, NO `Deref`/`DerefMut<[T]>`, and NO method returning
//!   `&mut [T]` over the buffer, so a worker cannot reborrow the whole column.

use std::marker::PhantomData;

use super::dense_store::DenseStore;
use super::live_bitmap::LiveBitmap;

/// Single-threaded structural view of a [`DenseStore`] (Dense plan Decision 8).
///
/// `!Send` (it holds `&mut DenseStore`, and is further pinned `!Send` by a
/// `PhantomData<*mut ()>` so the negative impl is explicit and robust against
/// future field changes). It is the ONLY surface exposing a whole-buffer
/// mutable slice plus the structural ops (insert / remove / compact), all of
/// which require single-threaded exclusive access.
pub struct DenseBuildView<'a> {
    store: &'a mut DenseStore,
    /// Pins the view `!Send` / `!Sync` explicitly (a raw pointer is neither),
    /// independent of the inferred auto-trait of `&mut DenseStore`.
    _not_send: PhantomData<*mut ()>,
}

impl<'a> DenseBuildView<'a> {
    /// Wraps an exclusive borrow of the store. Constructed via
    /// [`DenseStore::build_view`].
    #[inline]
    pub(crate) fn new(store: &'a mut DenseStore) -> Self {
        Self {
            store,
            _not_send: PhantomData,
        }
    }

    /// The whole-buffer mutable byte slice over the column's high-water-mark
    /// rows (`0..len * stride`). Single-threaded ONLY — this is the surface the
    /// solve view deliberately lacks.
    ///
    /// Includes tombstoned slots' bytes (they are logically uninitialised); the
    /// caller is the structural code that knows which slots are live.
    #[inline]
    pub fn as_mut_bytes(&mut self) -> &mut [u8] {
        let len = self.store.len();
        let stride = self.store.stride();
        let base = self.store.column_base_mut();
        // SAFETY: `base` is the column's address-stable write-capable base;
        // rows `[0, len)` are committed read/write within the column's single
        // reservation, so `len * stride` bytes from `base` lie inside one
        // allocated object. `&mut self` borrows the view (hence the store)
        // exclusively for the slice's lifetime, so no other access path is live
        // — this is the single-threaded structural surface, never shared with a
        // worker.
        unsafe { core::slice::from_raw_parts_mut(base, len * stride) }
    }

    /// Inserts `value_bytes` for `entity`, returning the assigned slot
    /// (forwards to [`DenseStore::insert`]).
    #[inline]
    pub fn push(&mut self, entity: crate::ecs::identifiers::primitives::EntityId, value_bytes: &[u8]) -> u32 {
        self.store.insert(entity, value_bytes)
    }

    /// Tombstones `entity`, returning `true` if it was present (forwards to
    /// [`DenseStore::remove`]).
    #[inline]
    pub fn tombstone(&mut self, entity: crate::ecs::identifiers::primitives::EntityId) -> bool {
        self.store.remove(entity)
    }

    /// Compacts the column (forwards to [`DenseStore::compact`]). COLD,
    /// between-steps only — never reachable from a worker (this view is
    /// `!Send`).
    #[inline]
    pub fn compact(&mut self) {
        self.store.compact();
    }
}

/// Parallel solve view of a [`DenseStore`] (Dense plan Decision 8) —
/// `Copy + Send + Sync`, ~32 B.
///
/// Exposes per-slot `row_ptr(slot)` / `len()` / `is_live(slot)` ONLY. By
/// construction there is no `as_mut_slice`, no `Deref`/`DerefMut<[T]>`, and no
/// method handing back `&mut [T]` over the buffer — so the whole-buffer
/// reborrow that caused the SP4 race is un-typeable from this view (the
/// structural SP4 fix; verified by the trybuild compile-fail).
///
/// The view is `Copy` so the scheduler can hand a copy to each worker; each
/// worker writes only the DISTINCT slots its color owns.
#[derive(Clone, Copy)]
pub struct DenseSolveView<'a> {
    /// Column data base — address-stable (the column's VM reservation never
    /// realloc-moves; Dense plan, `component_pool.rs:184-189`).
    base: *mut u8,
    /// Component stride in bytes.
    stride: usize,
    /// Column high-water mark (slot ceiling for `row_ptr`).
    len: usize,
    /// `live` words base (read-only liveness oracle).
    live: *const u64,
    /// Number of `live` words (bounds the liveness read).
    live_words: usize,
    /// Binds the view to the store borrow so it cannot outlive a structural
    /// mutation.
    _marker: PhantomData<&'a ()>,
}

// SAFETY: `DenseSolveView` is `Send + Sync` because:
//  * `base` is ADDRESS-STABLE — it is the `ComponentPool`'s VM-reserved column
//    base, which never realloc-moves (growth commits pages in place; the base
//    is write-once in `ComponentPool::new` — see component_pool.rs:184-189 and
//    the `buffer` field invariant). So a copy handed to a worker stays valid.
//  * each `row_ptr(slot)` is ONE `add` to a DISTINCT element (`base + slot *
//    stride`); the only mutable access is per-element, never whole-buffer (no
//    `&mut [T]` is producible from this view — the SP4 fix).
//  * the CALLER guarantees distinct-slot access across workers — the coloring
//    invariant, enforced by the (future) scheduler: two workers never write the
//    same slot, so two `&mut T` derived from `row_ptr` never alias. This is the
//    same contract `std::thread::scope` workers rely on for disjoint indexing.
//  * sentinel / dead slots are never handed out: `row_ptr`'s `debug_assert!`
//    (W3, liveness-checked) trips in debug if a tombstoned slot is passed, and
//    the bounds debug_assert guards `slot < len`.
// The raw pointers themselves carry no `!Send` payload; their safety is the
// distinct-slot + address-stable + liveness-guard discipline above.
unsafe impl Send for DenseSolveView<'_> {}
// SAFETY: see the `Send` impl above — shared access through `&DenseSolveView`
// only ever yields per-element `*mut u8` pointers (never a whole-buffer slice),
// and the distinct-slot coloring invariant means concurrent `row_ptr` callers
// target disjoint memory.
unsafe impl Sync for DenseSolveView<'_> {}

impl<'a> DenseSolveView<'a> {
    /// Builds a solve view from the store's cached column geometry + liveness
    /// words. Constructed via [`DenseStore::solve_view`].
    #[inline]
    pub(crate) fn new(
        base: *mut u8,
        stride: usize,
        len: usize,
        live: *const u64,
        live_words: usize,
    ) -> Self {
        Self {
            base,
            stride,
            len,
            live,
            live_words,
            _marker: PhantomData,
        }
    }

    /// The column high-water mark — the exclusive upper bound for `slot`.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// The column data base — the stride origin for `row_ptr` (Dense plan D3
    /// pure-dense cursor). Address-stable for the view's `'a` borrow.
    #[inline]
    pub(crate) fn base_ptr(&self) -> *mut u8 {
        self.base
    }

    /// The component stride in bytes (Dense plan D3 pure-dense cursor).
    #[inline]
    pub(crate) fn stride(&self) -> usize {
        self.stride
    }

    /// The `live` words base — the read-only liveness oracle the pure-dense
    /// cursor strides (Dense plan D3).
    #[inline]
    pub(crate) fn live_words_ptr(&self) -> *const u64 {
        self.live
    }

    /// Number of `live` words (bounds the cursor's liveness read).
    #[inline]
    pub(crate) fn live_word_count(&self) -> usize {
        self.live_words
    }

    /// `true` iff the column has no appended slots.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Reads the liveness of `slot` through the cached `live` words pointer.
    ///
    /// Slots beyond the live word range read as dead.
    #[inline]
    pub fn is_live(&self, slot: usize) -> bool {
        if (slot >> 6) >= self.live_words {
            return false;
        }
        // SAFETY: `slot >> 6 < live_words` (checked above) and `self.live` is
        // the store's `LiveBitmap` words base, kept alive by this view's `'a`
        // borrow of the store, so the word load is in-bounds and initialised.
        unsafe { LiveBitmap::test_raw(self.live, slot) }
    }

    /// Raw row pointer for `slot` (`base + slot * stride`).
    ///
    /// The per-element write surface for the colored solver — workers each call
    /// `row_ptr` on the DISTINCT slots their color owns. There is NO
    /// whole-buffer slice path (the SP4 fix).
    ///
    /// # Safety
    /// * `slot < len()` — debug-asserted.
    /// * `slot` is LIVE — debug-asserted (W3 liveness-checked accessor; a
    ///   tombstoned / sentinel slot trips the assert in debug).
    /// * The caller guarantees no other worker writes the same `slot`
    ///   concurrently (the coloring distinct-slot invariant).
    /// * The returned pointer is valid for `stride` bytes; the type cast from
    ///   it must match the store's registered component type.
    #[inline]
    pub unsafe fn row_ptr(&self, slot: usize) -> *mut u8 {
        debug_assert!(slot < self.len, "DenseSolveView::row_ptr: slot {slot} >= len {}", self.len);
        debug_assert!(
            self.is_live(slot),
            "DenseSolveView::row_ptr: slot {slot} is not live (W3 liveness guard)"
        );
        // SAFETY: `slot < len` (debug-asserted), so `slot * stride + stride <=
        // len * stride`, which lies inside the column's committed, address-stable
        // reservation (the base never realloc-moves — Dense plan,
        // component_pool.rs:184-189). Provenance derives from `self.base` via
        // one `add`. The distinct-slot coloring invariant (caller contract)
        // guarantees no aliasing across workers.
        unsafe { self.base.add(slot * self.stride) }
    }
}
