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

    /// The column's address-stable, **write-capable** raw base, typed `*mut T`.
    ///
    /// This is the single derivation of the solve-time base: it is exactly what
    /// [`Self::solve_view`] caches, and the SoA-decomposed callers that build a
    /// bespoke multi-column solve view (e.g. the physics contact columns, which
    /// pack many `ScratchColumn`s into one worker-facing view) use it to obtain a
    /// per-column write-capable base WITHOUT interposing a `&[T]` / `&mut [T]`
    /// reborrow. Going through `ComponentPool::buffer_ptr` (provenance-preserving,
    /// see component_pool.rs:1164-1170) and `cast_mut` from the SAME raw base
    /// yields a pointer that carries write provenance — unlike
    /// `as_read_slice().as_ptr().cast_mut()`, whose tag is Frozen / shared-read
    /// and is therefore Tree-Borrows-UB to write through.
    ///
    /// The base is address-stable across `grow_rows` (the backing
    /// `ComponentPool`'s VM reservation commits pages in place; the base is
    /// write-once in `ComponentPool::new`), so a copy captured here stays valid
    /// for as long as the `&self` borrow keeps the column alive.
    ///
    /// # Safety / provenance contract
    /// The returned pointer is a write-capable base; writing through `base + i`
    /// is sound only if the caller upholds disjointness — no two concurrent
    /// writers (and no concurrent reader-via-`&[T]`) touch the same element `i`.
    /// This is the coloring distinct-index invariant the colored solver relies
    /// on. The pointer is valid for `[0, len())` elements and aligned to at
    /// least `align_of::<T>()`.
    #[inline]
    pub fn solve_base(&self) -> *mut T {
        // Provenance-preserving write-capable base (NOT via `as_read_slice`,
        // whose `&[T]` reborrow would brand the pointer Frozen / SharedReadOnly
        // and make writes through it Tree-Borrows UB).
        self.column.buffer_ptr().cast_mut().cast::<T>()
    }

    /// Borrows the column as a `Copy + Send + Sync` solve view. The
    /// [`ScratchSolveView`] exposes per-element `row_ptr(i) -> *mut T` ONLY — no
    /// whole-buffer `&mut [T]` path exists, so the SP4 reborrow is un-typeable.
    ///
    /// The view caches the column's address-stable base + length; it must not
    /// outlive any refill of the column (enforced by `'a` borrowing `&self`).
    #[inline]
    pub fn solve_view(&self) -> ScratchSolveView<'_, T> {
        ScratchSolveView::new(self.solve_base(), self.column.count())
    }

    /// The whole-buffer READ-ONLY slice over `[0, len)`, borrowed through `&self`.
    ///
    /// This is the SHARED read surface for consumers that hold the column behind a
    /// shared borrow (e.g. a `Res<_>`-resolved resource) and need a `&[T]` with the
    /// same address arithmetic as a `&Vec<T>` deref-to-slice. It is read-only — the
    /// SP4-unsound whole-buffer MUTABLE reborrow stays un-typeable (only the
    /// single-threaded [`ScratchBuildView::as_mut_slice`](super::views::ScratchBuildView::as_mut_slice)
    /// hands out `&mut [T]`).
    ///
    /// `[0, len)` is contiguous and tombstone-free (a `clear` + refill column never
    /// leaves holes), so the returned slice is exactly the live elements in push
    /// order.
    #[inline]
    pub fn as_read_slice(&self) -> &[T] {
        self.as_slice()
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
    /// pool's `drop_fn` is `None`, so the O(1) `clear_no_drop` (which never runs
    /// drop glue) correctly empties the column without leaking or double-freeing.
    #[inline]
    pub(crate) fn clear(&mut self) {
        // O(1) `len = 0`: the build path refills every column from scratch each
        // step, so the per-element no-drop pop loop was pure overhead
        // (~316k decrements/step on the rigid colored-solve hot path).
        self.column.clear_no_drop();
    }

    /// Appends `value` at the frontier, growing the backing column in place if
    /// needed. Returns the assigned index.
    ///
    /// # Panics
    /// * the backing column's reserve ceiling is exhausted.
    #[inline]
    pub(crate) fn push(&mut self, value: T) -> u32
    where
        T: 'static,
    {
        // Inlined typed Copy store (`push_copy`), NOT the type-erased,
        // non-inlined byte `add` — the latter built a `&[u8]` span, made a
        // cross-crate non-inlined call, and did a `copy_nonoverlapping` memcpy
        // per push, all of which dominated the per-step build of 31 columns.
        self.column
            .push_copy(value)
            .expect("invariant: ScratchColumn reserve ceiling exhausted") as u32
    }

    /// Appends every element of `values` at the frontier (in-place grow as
    /// needed; the base never moves).
    ///
    /// # Panics
    /// * the backing column's reserve ceiling is exhausted.
    pub(crate) fn extend_from_slice(&mut self, values: &[T])
    where
        T: 'static,
    {
        for &v in values {
            self.column
                .push_copy(v)
                .expect("invariant: ScratchColumn reserve ceiling exhausted");
        }
    }
}
