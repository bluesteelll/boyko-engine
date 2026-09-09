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
    /// The column's write-capable base, cached for the view's lifetime.
    ///
    /// Sound to hold across the column's own `&mut` methods: it comes from
    /// `ScratchColumn::solve_base`, which reads the pool's stored `buffer` field
    /// rather than reborrowing `&self`, so its provenance is the reservation's own
    /// and a `&mut ComponentPool` does not touch it. The base is write-once and
    /// address-stable across growth (pages commit in place), so it never needs
    /// re-deriving either.
    base: *mut T,
    /// Cached frontier. THE live length while the view exists; written back into
    /// the column on `Drop`.
    len: usize,
    /// Cached committed-row ceiling — the `push` fast-path comparator. Re-read from
    /// the column after every grow.
    committed: usize,
    /// Pins the view `!Send` / `!Sync` explicitly (a raw pointer is neither),
    /// independent of the inferred auto-trait of `&mut ScratchColumn`.
    _not_send: PhantomData<*mut ()>,
}

impl<T: Copy> Drop for ScratchBuildView<'_, T> {
    /// Publishes the cached frontier back into the column.
    ///
    /// ⚠ `mem::forget`ing the view loses the pushes made through it — the column
    /// keeps the length it had when the view was taken. That is a lost write, not
    /// unsoundness: the rows above the published length are simply not live, which
    /// is exactly the state a `clear` leaves them in.
    #[inline]
    fn drop(&mut self) {
        self.column.set_len(self.len);
    }
}

impl<'a, T: Copy> ScratchBuildView<'a, T> {
    /// Wraps an exclusive borrow of the column. Constructed via
    /// [`ScratchColumn::build_view`](super::scratch_column::ScratchColumn::build_view).
    #[inline]
    pub(crate) fn new(column: &'a mut super::scratch_column::ScratchColumn<T>) -> Self {
        let base = column.solve_base();
        let len = column.len();
        let committed = column.committed_rows();
        Self {
            column,
            base,
            len,
            committed,
            _not_send: PhantomData,
        }
    }

    /// Grows the backing column by at least one row and re-reads the ceiling.
    ///
    /// `#[cold]` + `#[inline(never)]`: it runs once per commit step, never on the
    /// warm push, and keeping it out of line is what leaves `push` as a compare, a
    /// store and an increment.
    ///
    /// # Panics
    /// * the backing column's reserve ceiling is exhausted.
    #[cold]
    #[inline(never)]
    fn grow_for_push(&mut self) {
        // The column's own `len` must be current before it grows: `grow_rows`
        // commits from the frontier it can see.
        self.column.set_len(self.len);
        assert!(
            self.column.grow_to(self.len + 1),
            "invariant: ScratchColumn reserve ceiling exhausted"
        );
        self.committed = self.column.committed_rows();
    }

    /// The whole-buffer mutable slice over the column's `len` live elements.
    /// Single-threaded ONLY — this is the surface the solve view deliberately
    /// lacks (the SP4 fix).
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        // SAFETY: `base` is the column's write-capable, address-stable base, aligned
        // to at least `align_of::<T>()`; rows `[0, len)` are initialised `T` (written
        // by this view or live in the column when it was taken); `&mut self` gives
        // exclusive access for the slice's lifetime, and the view is `!Send` so no
        // other thread holds the column.
        unsafe { core::slice::from_raw_parts_mut(self.base, self.len) }
    }

    /// The whole-buffer read-only slice over the column's `len` live elements.
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        // SAFETY: as `as_mut_slice`, through a shared borrow.
        unsafe { core::slice::from_raw_parts(self.base, self.len) }
    }

    /// Logically empties the column (`len = 0`) WITHOUT freeing the backing
    /// reservation — the committed pages stay resident for the next step's
    /// refill (zero re-commit, the reuse contract). Sound only because
    /// `T: Copy` ⇒ `!needs_drop`, so dropping the old contents is a no-op.
    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
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
        if self.len == self.committed {
            self.grow_for_push();
        }
        let idx = self.len;
        // SAFETY: `idx < committed` (grown above if needed), so the slot lies inside
        // the pool's committed pages; `base` carries the reservation's own write
        // provenance and is aligned for `T`; the view holds the column exclusively
        // and is `!Send`, so nothing else can be writing it.
        unsafe { self.base.add(idx).write(value) };
        self.len = idx + 1;
        idx as u32
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
        for &v in values {
            self.push(v);
        }
    }

    /// Sets the live length to `new_len`, filling any new slots with `value`
    /// (`Vec::resize` semantics).
    ///
    /// The refill idiom for a buffer that is SIZED first and written by index
    /// afterwards — a CSR cursor pass, a parallel emit into pre-reserved slots, a
    /// bitset cleared to a known chunk count. Growing costs one grow of the
    /// backing column plus a constant-stride fill, not a grow check per element.
    ///
    /// # Panics
    /// * the backing column's reserve ceiling is exhausted.
    #[inline]
    pub fn resize(&mut self, new_len: usize, value: T)
    where
        T: 'static,
    {
        if new_len <= self.len {
            self.len = new_len;
            return;
        }
        // Delegate the grow + fill to the column (one grow for the whole span), then
        // re-read what it published.
        self.column.set_len(self.len);
        self.column.resize(new_len, value);
        self.len = self.column.len();
        self.committed = self.column.committed_rows();
    }

    /// Shortens the buffer to `new_len` live elements, keeping the committed
    /// pages. A `new_len` at or above the current length is a no-op.
    ///
    /// The compaction half of the sized-then-written idiom: a pass that reserves
    /// an upper bound, writes `w <= bound` survivors and then cuts the length to
    /// `w`. Without it such a pass cannot be expressed at all — `clear` plus
    /// re-pushing every survivor is a different algorithm, not the same one.
    #[inline]
    pub fn truncate(&mut self, new_len: usize) {
        if new_len < self.len {
            self.len = new_len;
        }
    }

    /// The number of live elements (`len`).
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// `true` iff the column has no live elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
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
