//! The [`ScratchBuildView`] / [`ScratchSolveView`] type-split for
//! [`ScratchColumn`](super::scratch_column::ScratchColumn) — the same structural
//! discipline the committed [`DenseStore`](crate::ecs::core::component::dense::DenseStore)
//! uses (Dense plan Decision 8), applied to transient solver scratch.
//!
//! Scratch is a Copy-only, drop-free, refill-every-step buffer (gather scratch,
//! per-element solver state). The SP4 race was an unsound whole-buffer
//! `&mut [T]` reborrow handed to parallel workers. The fix is structural, not a
//! discipline: split the column's access so the whole-buffer mutable reborrow is
//! **un-typeable** on the path workers touch.
//!
//! * [`ScratchBuildView`] (`!Send`) — the single-threaded refill surface: the
//!   ONLY view that exposes a whole-buffer slice (`as_mut_slice` / `as_slice`)
//!   and runs `clear` / `push` / `extend_from_slice`. This is where a step's
//!   scratch is refilled.
//! * [`ScratchSolveView`] (`Copy`, `Send + Sync`) — the parallel solve surface:
//!   per-element `row_ptr(i) -> *mut T` (TYPED, no `u8` cast) and `len` ONLY.
//!   There is NO `as_mut_slice`, NO `Deref`/`DerefMut<[T]>`, and NO slice over
//!   the buffer, so a worker cannot reborrow the whole column.

use std::marker::PhantomData;

/// Single-threaded refill view of a
/// [`ScratchColumn`](super::scratch_column::ScratchColumn) (mirrors
/// [`DenseBuildView`](crate::ecs::core::component::dense::DenseBuildView)).
///
/// `!Send` (it holds `&mut ScratchColumn`, and is further pinned `!Send` by a
/// `PhantomData<*mut ()>` so the negative impl is explicit and robust against
/// future field changes). It is the ONLY surface exposing a whole-buffer slice
/// plus the refill ops (`clear` / `push` / `extend_from_slice`), all of which
/// require single-threaded exclusive access.
pub struct ScratchBuildView<'a, T: Copy> {
    column: &'a mut super::scratch_column::ScratchColumn<T>,
    /// Pins the view `!Send` / `!Sync` explicitly (a raw pointer is neither),
    /// independent of the inferred auto-trait of `&mut ScratchColumn`.
    _not_send: PhantomData<*mut ()>,
}

impl<'a, T: Copy> ScratchBuildView<'a, T> {
    /// Wraps an exclusive borrow of the column. Constructed via
    /// [`ScratchColumn::build_view`](super::scratch_column::ScratchColumn::build_view).
    #[inline]
    pub(crate) fn new(column: &'a mut super::scratch_column::ScratchColumn<T>) -> Self {
        Self {
            column,
            _not_send: PhantomData,
        }
    }

    /// The whole-buffer mutable slice over the column's `len` live elements.
    /// Single-threaded ONLY — this is the surface the solve view deliberately
    /// lacks (the SP4 fix).
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        self.column.as_mut_slice()
    }

    /// The whole-buffer read-only slice over the column's `len` live elements.
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        self.column.as_slice()
    }

    /// Logically empties the column (`len = 0`) WITHOUT freeing the backing
    /// reservation — the committed pages stay resident for the next step's
    /// refill (zero re-commit, the reuse contract). Sound only because
    /// `T: Copy` ⇒ `!needs_drop`, so dropping the old contents is a no-op.
    #[inline]
    pub fn clear(&mut self) {
        self.column.clear();
    }

    /// Appends `value` at the frontier, growing the backing column IN PLACE if
    /// needed (the base never moves — address-stable). Returns the assigned
    /// index.
    ///
    /// # Panics
    /// * the backing column's reserve ceiling is exhausted.
    #[inline]
    pub fn push(&mut self, value: T) -> u32
    where
        T: 'static,
    {
        self.column.push(value)
    }

    /// Appends every element of `values` at the frontier (one in-place grow at
    /// most per crossed commit step; the base never moves).
    ///
    /// # Panics
    /// * the backing column's reserve ceiling is exhausted.
    #[inline]
    pub fn extend_from_slice(&mut self, values: &[T])
    where
        T: 'static,
    {
        self.column.extend_from_slice(values);
    }

    /// The number of live elements (`len`).
    #[inline]
    pub fn len(&self) -> usize {
        self.column.len()
    }

    /// `true` iff the column has no live elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.column.is_empty()
    }
}

/// Parallel solve view of a
/// [`ScratchColumn`](super::scratch_column::ScratchColumn) —
/// `Copy + Send + Sync` (mirrors
/// [`DenseSolveView`](crate::ecs::core::component::dense::DenseSolveView)).
///
/// Exposes per-element `row_ptr(i) -> *mut T` (TYPED — no `u8` cast in the hot
/// loop) and `len()` ONLY. By construction there is no `as_mut_slice`, no
/// `Deref`/`DerefMut<[T]>`, and no method handing back `&mut [T]` over the
/// buffer — so the whole-buffer reborrow that caused the SP4 race is
/// un-typeable from this view (the structural SP4 fix; verified by the trybuild
/// compile-fail).
///
/// The view is `Copy` so the scheduler can hand a copy to each worker; each
/// worker writes only the DISTINCT indices its color owns.
pub struct ScratchSolveView<'a, T: Copy> {
    /// Column data base — address-stable (the backing `ComponentPool`'s VM
    /// reservation never realloc-moves; the base is write-once in
    /// `ComponentPool::new` — component_pool.rs:147-216). Typed `*mut T` so
    /// `row_ptr` needs no cast.
    base: *mut T,
    /// Element count (index ceiling for `row_ptr`).
    len: usize,
    /// Binds the view to the column borrow so it cannot outlive a refill.
    _marker: PhantomData<&'a ()>,
}

impl<T: Copy> Clone for ScratchSolveView<'_, T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Copy> Copy for ScratchSolveView<'_, T> {}

// SAFETY: `ScratchSolveView` is `Send + Sync` because:
//  * `base` is ADDRESS-STABLE — it is the backing `ComponentPool`'s VM-reserved
//    column base, which never realloc-moves (growth commits pages IN PLACE at
//    the frontier of the SAME reservation; the base is write-once in
//    `ComponentPool::new` — see component_pool.rs:147-216, the `buffer` field
//    invariant). So a copy handed to a worker stays valid for the view's `'a`
//    borrow. This is exactly the property `std::Vec` lacks (a `Vec` realloc
//    moves the base) — the SP4 root cause.
//  * each `row_ptr(i)` is ONE `add` to a DISTINCT element (`base + i`); the only
//    mutable access is per-element, never whole-buffer (no `&mut [T]` is
//    producible from this view — the SP4 fix).
//  * the CALLER guarantees distinct-index access across workers — the coloring
//    invariant: two workers never write the same index, so two `&mut T` derived
//    from `row_ptr` never alias. This is the same contract `std::thread::scope`
//    workers rely on for disjoint indexing.
//  * `T: Copy` ⇒ `!needs_drop`, so a raw scratch write never runs drop glue on
//    stale bytes and there is no drop-ordering hazard across workers.
// The raw pointer itself carries no `!Send` payload; its safety is the
// distinct-index + address-stable discipline above.
unsafe impl<T: Copy> Send for ScratchSolveView<'_, T> {}
// SAFETY: see the `Send` impl above — shared access through `&ScratchSolveView`
// only ever yields per-element `*mut T` pointers (never a whole-buffer slice),
// and the distinct-index coloring invariant means concurrent `row_ptr` callers
// target disjoint memory.
unsafe impl<T: Copy> Sync for ScratchSolveView<'_, T> {}

impl<'a, T: Copy> ScratchSolveView<'a, T> {
    /// Builds a solve view from the column's cached base + length. Constructed
    /// via [`ScratchColumn::solve_view`](super::scratch_column::ScratchColumn::solve_view).
    #[inline]
    pub(crate) fn new(base: *mut T, len: usize) -> Self {
        Self {
            base,
            len,
            _marker: PhantomData,
        }
    }

    /// The element count — the exclusive upper bound for `index`.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// `true` iff the column has no live elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Typed raw row pointer for `index` (`base + index`).
    ///
    /// The per-element write surface for the colored solver — workers each call
    /// `row_ptr` on the DISTINCT indices their color owns. There is NO
    /// whole-buffer slice path (the SP4 fix). Returns a typed `*mut T`, not
    /// `*mut u8`, so the hot loop needs no cast.
    ///
    /// # Safety
    /// * `index < len()` — debug-asserted.
    /// * The caller guarantees no other worker writes the same `index`
    ///   concurrently (the coloring distinct-index invariant).
    /// * The returned pointer is valid for one `T` and properly aligned (the
    ///   backing column base is `align_of::<T>()`-aligned at least).
    #[inline]
    pub unsafe fn row_ptr(&self, index: usize) -> *mut T {
        debug_assert!(
            index < self.len,
            "ScratchSolveView::row_ptr: index {index} >= len {}",
            self.len
        );
        // SAFETY: `index < len` (debug-asserted), so `base + index` is the
        // `index`-th element inside the column's committed, address-stable
        // reservation (the base never realloc-moves — component_pool.rs:147-216).
        // Provenance derives from `self.base` via one typed `add`. The
        // distinct-index coloring invariant (caller contract) guarantees no
        // aliasing across workers.
        unsafe { self.base.add(index) }
    }
}
