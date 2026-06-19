//! [`ScratchColumn<T>`] — a `ComponentPool`-backed, Copy-only, transient SoA
//! scratch column with the same BuildView/SolveView type-split as the committed
//! [`DenseStore`](crate::ecs::core::component::dense::DenseStore).
//!
//! This is the Stage-0 enabler for moving solver scratch (gather buffers,
//! per-element solver state) off `std::Vec` onto the engine's OWN storage. The
//! point is the parallel-access discipline: the [`ScratchSolveView`] hands out a
//! per-element `row_ptr` ONLY (no whole-buffer reborrow), and the backing
//! `ComponentPool`'s base is ADDRESS-STABLE across growth (in-place commit) —
//! the property `std::Vec` lacks that caused the SP4 colored-solve data race.
//!
//! Scope: `T: Copy` (POD — `f32` / `u32` / the solver's body-state structs).
//! `clear` is `len = 0` with NO free (the committed pages stay resident for the
//! next step's refill). There is NO change-detection tick use — this is raw
//! scratch, refilled every step.

use std::marker::PhantomData;

use crate::ecs::identifiers::primitives::ComponentId;
use crate::ecs::memory::component_pool::ComponentPool;

use super::views::{ScratchBuildView, ScratchSolveView};

/// A transient, Copy-only scratch column backed by one [`ComponentPool`].
///
/// Backed by `ComponentPool::new(component_id, reserve_rows)` directly — the
/// same backing the committed dense kernel uses, no bespoke `VmReservation`
/// primitive. The pool gives the two load-bearing properties:
/// * **address-stable base** — `ComponentPool` grows IN PLACE (commits fresh
///   pages at the frontier of the SAME reservation; the base is write-once in
///   `ComponentPool::new`), so a [`ScratchSolveView`] copy handed to a worker
///   stays valid across a refill that grows the column. `std::Vec` reallocates
///   and moves its base — the SP4 root cause.
/// * **per-element `row_ptr`** — the SoA write surface for the colored solver.
///
/// `T: Copy` ⇒ `!needs_drop::<T>()` (asserted in [`Self::new`]); the pool's
/// `drop_fn` is `None` for such a type, so `clear` / drop never run drop glue
/// on scratch bytes.
pub struct ScratchColumn<T: Copy> {
    /// The one contiguous data column. Address-stable across `grow_rows`.
    column: ComponentPool,
    /// Ties the column to its element type for the typed view surface.
    _marker: PhantomData<T>,
}

impl<T: Copy> ScratchColumn<T> {
    /// Creates an empty scratch column for `component_id`, backing the data with
    /// `ComponentPool::new(component_id, reserve_rows)` directly.
    ///
    /// `component_id`'s layout MUST already be registered in the
    /// `ComponentRegistry` and MUST match `T` (the `ComponentPool::new`
    /// contract). The caller owns the id assignment — `ScratchColumn` is a
    /// generic kernel primitive, not bound to any one component.
    ///
    /// # Panics
    /// * `T` needs drop — scratch is POD-only (asserted at construction: a
    ///   `clear`/refill that skips drop would leak or double-free a non-`Copy`
    ///   `T`; `T: Copy` already forbids `Drop`, this asserts the corollary
    ///   loudly).
    /// * the `ComponentPool::new` panics (unregistered id, `reserve_rows == 0`,
    ///   alignment over a page, etc. — see its contract).
    pub fn new(component_id: ComponentId, reserve_rows: usize) -> Self {
        assert!(
            !core::mem::needs_drop::<T>(),
            "ScratchColumn requires a POD (Copy, no Drop) element type; \
             {} needs drop",
            core::any::type_name::<T>()
        );
        let column = ComponentPool::new(component_id.get(), reserve_rows);
        debug_assert_eq!(
            column.component_layout().size(),
            core::mem::size_of::<T>(),
            "ScratchColumn: registered layout size != size_of::<T>()"
        );
        debug_assert!(
            column.component_layout().align() >= core::mem::align_of::<T>(),
            "ScratchColumn: registered layout align < align_of::<T>()"
        );
        Self {
            column,
            _marker: PhantomData,
        }
    }

    /// The number of live elements (`len`). The exclusive upper bound for
    /// `row_ptr`.
    #[inline]
    pub fn len(&self) -> usize {
        self.column.count()
    }

    /// `true` iff no element is live.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.column.count() == 0
    }

    /// The reserve ceiling (max `len` before the backing column rejects a
    /// `push`).
    #[inline]
    pub fn capacity(&self) -> usize {
        self.column.capacity()
    }

    /// Borrows the column as a single-threaded refill view. The
    /// [`ScratchBuildView`] is `!Send` and is the ONLY surface exposing
    /// whole-buffer slices + the refill ops (`clear` / `push` /
    /// `extend_from_slice`).
    #[inline]
    pub fn build_view(&mut self) -> ScratchBuildView<'_, T> {
        ScratchBuildView::new(self)
    }

    /// Borrows the column as a `Copy + Send + Sync` solve view. The
    /// [`ScratchSolveView`] exposes per-element `row_ptr(i) -> *mut T` ONLY — no
    /// whole-buffer `&mut [T]` path exists, so the SP4 reborrow is un-typeable.
    ///
    /// The view caches the column's address-stable base + length; it must not
    /// outlive any refill of the column (enforced by `'a` borrowing `&self`).
    #[inline]
    pub fn solve_view(&self) -> ScratchSolveView<'_, T> {
        ScratchSolveView::new(self.column.buffer_ptr().cast_mut().cast::<T>(), self.column.count())
    }

    // ── pub(crate) refill surface (driven by ScratchBuildView) ──────────────

    /// The whole-buffer read-only slice over `[0, len)`.
    #[inline]
    pub(crate) fn as_slice(&self) -> &[T] {
        let len = self.column.count();
        // SAFETY: `buffer_ptr()` is the column's address-stable base, aligned to
        // at least `align_of::<T>()` (the registered layout's align). Elements
        // `[0, len)` were all written by `push` / `extend_from_slice` from a
        // valid `T`, so each is an initialised, valid `T` (a Copy bit-pattern).
        // The `&self` borrow keeps the column alive for the slice; no `&mut`
        // path is live. `len * size_of::<T>()` fits the committed reservation
        // (`len <= committed_rows`).
        unsafe { core::slice::from_raw_parts(self.column.buffer_ptr().cast::<T>(), len) }
    }

    /// The whole-buffer mutable slice over `[0, len)`. Single-threaded ONLY.
    #[inline]
    pub(crate) fn as_mut_slice(&mut self) -> &mut [T] {
        let len = self.column.count();
        // SAFETY: `buffer_ptr_mut()` is the column's address-stable, write-capable
        // base, aligned to at least `align_of::<T>()`. Elements `[0, len)` are
        // initialised valid `T` (written by `push` / `extend_from_slice`). The
        // `&mut self` borrow gives exclusive access for the slice's lifetime, so
        // no other path (no solve view, no other slice) aliases it. `len *
        // size_of::<T>()` lies inside the committed reservation.
        unsafe { core::slice::from_raw_parts_mut(self.column.buffer_ptr_mut().cast::<T>(), len) }
    }

    /// Logically empties the column (`len = 0`) WITHOUT releasing the backing
    /// reservation — the committed pages stay resident for the next refill.
    ///
    /// Sound because `T: Copy` ⇒ `!needs_drop` (asserted in [`Self::new`]): the
    /// pool's `drop_fn` is `None`, so `pop_entity_no_drop` (which never runs
    /// drop glue) correctly empties the column without leaking or double-freeing.
    pub(crate) fn clear(&mut self) {
        // Walk the high-water mark down to 0 without dropping. The backing
        // pool exposes no `set_len`, but `pop_entity_no_drop` is exactly the
        // per-element no-drop decrement, and for a `!needs_drop` `T` the loop is
        // semantically `len = 0`. The compiler lowers it to a tight counter
        // decrement (no per-element drop glue is emitted).
        while self.column.count() != 0 {
            self.column.pop_entity_no_drop();
        }
    }

    /// Appends `value` at the frontier, growing the backing column in place if
    /// needed. Returns the assigned index.
    ///
    /// # Panics
    /// * the backing column's reserve ceiling is exhausted.
    pub(crate) fn push(&mut self, value: T) -> u32 {
        // `value` is `Copy`, so reading its bytes does not move it and the local
        // drops trivially (no-op for a Copy type).
        let bytes = value_bytes(&value);
        self.column
            .add(bytes)
            .expect("invariant: ScratchColumn reserve ceiling exhausted") as u32
    }

    /// Appends every element of `values` at the frontier (in-place grow as
    /// needed; the base never moves).
    ///
    /// # Panics
    /// * the backing column's reserve ceiling is exhausted.
    pub(crate) fn extend_from_slice(&mut self, values: &[T]) {
        for v in values {
            let bytes = value_bytes(v);
            self.column
                .add(bytes)
                .expect("invariant: ScratchColumn reserve ceiling exhausted");
        }
    }
}

/// Views one `T: Copy` as its own byte span for the `ComponentPool::add` raw
/// API. Sound for any `Copy` `T`: reading its bytes is a non-moving read of an
/// initialised value, and the bytes ARE a valid representation of the registered
/// type (the layout match is debug-asserted in [`ScratchColumn::new`]).
#[inline]
fn value_bytes<T: Copy>(value: &T) -> &[u8] {
    // SAFETY: `value` is an initialised `T` (a live reference); `T: Copy` so its
    // bytes are a plain-old-data representation with no interior pointers/padding
    // invariants to violate. The span is exactly `size_of::<T>()` bytes within
    // the single `T` object, read-only, tied to `value`'s borrow.
    unsafe { core::slice::from_raw_parts((value as *const T).cast::<u8>(), core::mem::size_of::<T>()) }
}
