//! `DenseQueryIter` / `DenseQueryIterMut` — the pure-dense fast-path cursors
//! (Dense plan D3, FORK 2).
//!
//! `Query::iter` / `iter_mut` are kept **byte-identical** for the 0%-gate (no
//! enum cursor, no runtime variant-match in the archetypal `next()`). This
//! module is the SEPARATE, opt-in entry point a pure-dense consumer (and the
//! physics solver in Stage P) uses to stride ONE contiguous column directly:
//!
//! ```text
//! for slot in 0..len {
//!     if !live(slot) { continue }      // one predictable compare/slot
//!     yield (s2e[slot], row_ptr(slot)) // insertion order
//! }
//! ```
//!
//! No archetype walk, no per-row `entity_ids → e2s` gather (the mixed path).
//! The contiguous stride is what makes this the solver path.
//!
//! # The "all terms dense" precondition (FORK 2)
//!
//! `Query::dense_iter` is gated on `D: DenseQueryData` (a sealed marker for
//! `&T` / `&mut T`) PLUS a `const { D::HAS_DENSE }` assert at the call site, so
//! a `dense_iter` over a TABLE `D` is a compile error. A single global column
//! has no archetype-membership question, so the contiguous stride is sound by
//! construction.

use std::marker::PhantomData;

use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::dense::DenseStore;
use crate::ecs::core::iters::query::data::{QueryData, ReadOnlyQueryData};
use crate::ecs::identifiers::primitives::{ComponentId, EntityId};

/// Sealed marker for the query-data shapes admissible on the pure-dense fast
/// path ([`Query::dense_iter`](super::query::Query::dense_iter)).
///
/// Implemented for `&T` / `&mut T` (any `T: Component`); `Query::dense_iter`
/// additionally `const`-asserts `D::HAS_DENSE`, so a non-dense `T` is rejected
/// at monomorphisation. A tuple / `Option` / `AnyOf` is NOT a member, so a
/// multi-term `dense_iter` is a compile error (FORK 2's precondition).
///
/// # Safety
///
/// A declarative marker: an implementor's `Item<'w>` MUST be constructible from
/// one `*mut u8` row pointer into the dense column (via [`Self::dense_item`])
/// and [`Self::dense_component_id`] MUST return the dense `ComponentId` whose
/// `DenseStore` backs the column. `&T` / `&mut T` satisfy both by construction.
pub unsafe trait DenseQueryData: QueryData {
    /// Returns the dense `ComponentId` whose [`DenseStore`] this query strides.
    fn dense_component_id(state: &Self::State) -> ComponentId;

    /// Builds the per-slot item from a row pointer into the dense column.
    ///
    /// # Safety
    /// * `ptr` is `DenseSolveView::row_ptr(slot)` for a LIVE slot — valid for
    ///   `stride` bytes, correctly aligned for `T`.
    /// * The returned item's lifetime `'w` ties to the world borrow; the caller
    ///   ([`DenseQueryIter`]) holds the borrow that keeps the column alive.
    /// * For a `&mut T` impl the caller guarantees no other live reference to
    ///   the same slot (the `&mut self` cursor + distinct slots).
    unsafe fn dense_item<'w>(ptr: *mut u8) -> Self::Item<'w>;
}

// SAFETY: `&T`'s `Item<'w> = &'w T` is built from one `*mut u8` cast to
//   `*const T`; the dense id is `T::component_id()`. `Query::dense_iter`
//   const-asserts `T::STORAGE_IS_DENSE`, so the store always exists when
//   reached.
unsafe impl<T: Component> DenseQueryData for &T {
    #[inline]
    fn dense_component_id(_state: &Self::State) -> ComponentId {
        T::component_id()
    }

    #[inline]
    unsafe fn dense_item<'w>(ptr: *mut u8) -> Self::Item<'w> {
        // SAFETY (D3): `ptr` is a live row pointer for `T` (caller contract);
        //   the cast matches the registered type; `'w` ties to the world borrow.
        unsafe { &*(ptr as *const T) }
    }
}

// SAFETY: `&mut T`'s `Item<'w> = &'w mut T` is built from one write-capable
//   `*mut u8` cast to `*mut T`; distinct slots ⇒ no aliasing across rows. The
//   `&mut self` cursor guarantees a single live `DenseQueryIterMut`.
unsafe impl<T: Component> DenseQueryData for &mut T {
    #[inline]
    fn dense_component_id(_state: &Self::State) -> ComponentId {
        T::component_id()
    }

    #[inline]
    unsafe fn dense_item<'w>(ptr: *mut u8) -> Self::Item<'w> {
        // SAFETY (D3): `ptr` is a live, write-capable row pointer for `T`
        //   (caller contract); the cast matches the registered type; distinct
        //   slots ⇒ no two yielded `&mut` alias; `'w` ties to the world borrow.
        unsafe { &mut *(ptr as *mut T) }
    }
}

/// The geometry a pure-dense cursor strides — cached once from the resolved
/// [`DenseStore`] so the per-slot loop pays no indirection.
///
/// A NULL `store` means the dense store was never created (no entity ever
/// inserted): `len == 0`, so the cursor yields nothing.
struct DenseCursor<'w> {
    /// Column data base (`DenseSolveView::row_ptr` strides from here).
    base: *mut u8,
    /// Component stride in bytes.
    stride: usize,
    /// Column high-water mark (the slot ceiling).
    len: usize,
    /// `live` words base (read-only liveness oracle).
    live: *const u64,
    /// Number of `live` words (bounds the liveness read).
    live_words: usize,
    /// `slot -> EntityId` base (yielded alongside the item).
    s2e: *const EntityId,
    /// Next slot to test.
    next_slot: usize,
    /// Binds the cursor to the world borrow.
    _marker: PhantomData<&'w ()>,
}

impl<'w> DenseCursor<'w> {
    /// Resolves the geometry from `store` (or an empty cursor when NULL).
    ///
    /// # Safety
    /// * `store` is NULL or a live `*const DenseStore` valid for `'w`.
    #[inline]
    unsafe fn new(store: *const DenseStore) -> Self {
        if store.is_null() {
            return Self {
                base: std::ptr::null_mut(),
                stride: 0,
                len: 0,
                live: std::ptr::null(),
                live_words: 0,
                s2e: std::ptr::null(),
                next_slot: 0,
                _marker: PhantomData,
            };
        }
        // SAFETY (caller contract): `store` is a live `DenseStore` for `'w`.
        let store_ref = unsafe { &*store };
        let view = store_ref.solve_view();
        let s2e = store_ref.s2e();
        Self {
            base: view.base_ptr(),
            stride: view.stride(),
            len: view.len(),
            live: view.live_words_ptr(),
            live_words: view.live_word_count(),
            s2e: s2e.as_ptr(),
            next_slot: 0,
            _marker: PhantomData,
        }
    }

    /// Advances to the next live slot, returning `(EntityId, row_ptr)` or
    /// `None` at the column tail. Insertion order (slot order).
    ///
    /// # Safety
    /// * The cursor was built from a live store valid for `'w`.
    #[inline]
    unsafe fn next_live(&mut self) -> Option<(EntityId, *mut u8)> {
        while self.next_slot < self.len {
            let slot = self.next_slot;
            self.next_slot += 1;
            // Liveness: a dead (tombstoned) slot is skipped — one predictable
            // compare/slot (const-folds to taken in the zero-tombstone state).
            let word = slot >> 6;
            let live = word < self.live_words && {
                // SAFETY: `word < live_words` (checked); `self.live` is the
                //   store's `LiveBitmap` words base, valid for `'w`.
                let w = unsafe { *self.live.add(word) };
                (w >> (slot & 63)) & 1 == 1
            };
            if !live {
                continue;
            }
            // SAFETY: `slot < len` and is live, so `base + slot*stride` is a
            //   live, stride-aligned pointer into the address-stable column;
            //   `s2e[slot]` is the owning entity (≠ TOMBSTONE for a live slot).
            let entity = unsafe { *self.s2e.add(slot) };
            let ptr = unsafe { self.base.add(slot * self.stride) };
            return Some((entity, ptr));
        }
        None
    }
}

/// Read-only pure-dense cursor — yields `(EntityId, D::Item<'q>)` per LIVE slot
/// in insertion order, striding ONE contiguous column (Dense plan D3, FORK 2).
///
/// Constructed via [`Query::dense_iter`](super::query::Query::dense_iter) /
/// [`QueryView::dense_iter`](super::query_view::QueryView::dense_iter). `D` must
/// be a read-only dense leaf (`&T`, `T::STORAGE_IS_DENSE`).
pub struct DenseQueryIter<'q, D: DenseQueryData> {
    cursor: DenseCursor<'q>,
    _marker: PhantomData<fn() -> D>,
}

impl<'q, D: DenseQueryData> DenseQueryIter<'q, D> {
    /// Builds a pure-dense read cursor over `store`.
    ///
    /// # Safety
    /// * `store` is NULL or a live `*const DenseStore` for `'q` whose registered
    ///   type matches `D`'s component.
    /// * No aliasing writer of the column is live for `'q` (read-only cursor).
    #[inline]
    pub(crate) unsafe fn new(store: *const DenseStore) -> Self {
        Self {
            // SAFETY (caller contract): `store` is NULL or a live store for `'q`.
            cursor: unsafe { DenseCursor::new(store) },
            _marker: PhantomData,
        }
    }
}

impl<'q, D> Iterator for DenseQueryIter<'q, D>
where
    D: DenseQueryData + ReadOnlyQueryData,
{
    type Item = (EntityId, D::Item<'q>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        // SAFETY (D3): the cursor was built from a live store for `'q`;
        //   `next_live` yields a live row pointer; `D::dense_item` builds the
        //   read-only item from it. Read-only cursor ⇒ no aliasing writer.
        let (entity, ptr) = unsafe { self.cursor.next_live()? };
        let item = unsafe { D::dense_item(ptr) };
        Some((entity, item))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        // Upper bound is the remaining slot span (live + tombstoned); the lower
        // bound is conservative (we do not popcount the live mask here).
        (0, Some(self.cursor.len - self.cursor.next_slot.min(self.cursor.len)))
    }
}

/// Mutable pure-dense cursor — yields `(EntityId, D::Item<'q>)` (with `&mut T`)
/// per LIVE slot in insertion order, striding ONE contiguous column. Writes
/// land in the column (round-trip). Constructed via
/// [`Query::dense_iter_mut`](super::query::Query::dense_iter_mut).
///
/// Each yielded `&mut` targets a DISTINCT slot, and the `&mut self` borrow at
/// the `dense_iter_mut` call gates cursor uniqueness, so no two yielded `&mut`
/// alias.
pub struct DenseQueryIterMut<'q, D: DenseQueryData> {
    cursor: DenseCursor<'q>,
    _marker: PhantomData<fn() -> D>,
}

impl<'q, D: DenseQueryData> DenseQueryIterMut<'q, D> {
    /// Builds a pure-dense mutable cursor over `store`.
    ///
    /// # Safety
    /// * `store` is NULL or a live `*const DenseStore` for `'q` whose registered
    ///   type matches `D`'s component.
    /// * The caller holds the `&mut`-borrow gating cursor uniqueness; no other
    ///   reference into the column is live for `'q`.
    #[inline]
    pub(crate) unsafe fn new(store: *const DenseStore) -> Self {
        Self {
            // SAFETY (caller contract): `store` is NULL or a live store for `'q`.
            cursor: unsafe { DenseCursor::new(store) },
            _marker: PhantomData,
        }
    }
}

impl<'q, D> Iterator for DenseQueryIterMut<'q, D>
where
    D: DenseQueryData,
{
    type Item = (EntityId, D::Item<'q>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        // SAFETY (D3): the cursor was built from a live store for `'q`; each
        //   `next_live` yields a DISTINCT live slot's write-capable row pointer;
        //   `D::dense_item` (for `&mut T`) builds `&mut T` from it. Distinct
        //   slots ⇒ no two yielded `&mut` alias; the `&mut self` borrow at the
        //   call site gates cursor uniqueness.
        let (entity, ptr) = unsafe { self.cursor.next_live()? };
        let item = unsafe { D::dense_item(ptr) };
        Some((entity, item))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(self.cursor.len - self.cursor.next_slot.min(self.cursor.len)))
    }
}
