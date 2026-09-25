//! [`VmColumn<T>`] — a typed, address-stable, growable column on ONE
//! [`VmReservation`] (kernel-memory audit F1 / F3).
//!
//! This is the generic sibling of `InlandStore` (`boyko_ecs::ecs::core::entity::inland_store`)
//! and the per-row backing story of `ComponentPool` (`boyko_ecs::ecs::memory::component_pool`):
//! a single contiguous virtual-address reservation, committed lazily in
//! geometric frontier slabs, whose base **never moves**. It replaces the two
//! remaining per-row hot columns still living on a realloc-able `std::Vec`
//! whose raw `as_ptr()` a hot query fetch caches per pass:
//!
//! * `Archetype.entity_ids` (F1) — the entity-id column joined per row by every
//!   query fetch (`entity_ids_slice().as_ptr()` cached at the archetype
//!   boundary, consumed `*base.add(row)` per row);
//! * `DenseStore.s2e` (F3) — the slot→entity table the dense query iterator
//!   caches.
//!
//! A `Vec` doubling on a batch spawn reallocates and memcpys the whole prefix —
//! the realloc-memcpy spike class Phase X.G deleted for `entities_inland`.
//! `VmColumn` grows by committing fresh pages at the frontier: **O(1) in live
//! elements, zero bytes copied**, and the cached `*const T` in the query fetch
//! now points into an address-stable reservation (the cache becomes strictly
//! safer — the base can never be invalidated mid-pass).
//!
//! # `T: Copy` — no element drop
//!
//! Every consumer stores plain-old-data — ids, bare integers, and two 16-byte
//! `#[repr(C)]` records; the full list is under *Supported element domain*
//! below. The bound is `T: Copy`, so `swap_remove` and `clear` never run
//! a destructor: a removed element is simply overwritten and the length rolled
//! back. This is the whole reason the primitive is small and unsafe-light — it
//! is deliberately NOT a general `Vec` replacement for droppable `T`.
//!
//! # Supported element domain (review #4)
//!
//! `size_of::<T>()` must be non-zero AND divide [`COMMIT_PAGE`] — both
//! asserted in [`VmColumn::new`]. The divisibility pin is what keeps every
//! commit frontier page-aligned: `committed_elems * SIZE` is then exact
//! (page-aligned, no flooring slack), so the NEXT `grow_to`'s
//! `commit(old, new)` never hands the OS an unaligned range — without the pin,
//! a `T` whose size does not divide the page (e.g. 12 or 24 bytes) would
//! floor `committed_elems` below the byte frontier and the unix arm's
//! `mprotect` would reject the recommit of a LEGAL growth with a release
//! assert-panic.
//!
//! The consumers, every instantiation in the workspace at rung C1 (the type is
//! public since KC-01 moved it here; `new`'s assert, not visibility, refuses a size outside it):
//!
//! | column | `T` | `size_of::<T>()` |
//! |---|---|---|
//! | `Archetype.entity_ids` | `EntityId` (`#[repr(transparent)]` over `usize`) | 8 |
//! | `DenseStore.s2e` | `EntityId` | 8 |
//! | `EntityReservoir.free` | `Entity` (const-asserted at its site) | 16 |
//! | `Assets.slot_word`, `Assets.refcount` | `u32` | 4 |
//! | `PathIndex.entries` | `PathEntry` (`#[repr(C)]` `u64` + `u32` + `u32`) | 16 |
//! | `LogRing.lines` | `LogLine` (const-asserted at its site) | 16 |
//! | `LogRing.arena` | `u8` | 1 |
//! | `EntityMaster::check_invariants` scratch, in-crate unit tests | `EntityId` | 8 |
//!
//! Each of 1, 4, 8 and 16 divides 4 096, and so also divides the 64 KiB
//! commit page of the non-`x86_64` arm.
//!
//! The divisor is the commit quantum, which the packing plan (D1) lowered
//! from the 64 KiB granule to the page, so the domain tightened from
//! "divides 65 536" to "divides 4 096": the only sizes it newly refuses are
//! 8, 16, 32 and 64 KiB. (Same pin as
//! `InlandStore`'s `COMMIT_GRANULE.is_multiple_of(SLOT_SIZE)` const assert,
//! whose slabs are still granule-stepped, made a constructor assert here
//! because `T` is generic.)
//!
//! # Zero-fill contract
//!
//! Newly committed pages read zero on first access on every arm (the
//! [`vm`](crate::vm) module's zero-fill contract). `VmColumn`
//! never *reads* an element it did not `push`/`set` first (`len` is the bounds
//! oracle for `as_slice`/`get`), so — unlike `InlandStore`, which reads
//! never-written slots as `NULL` — it does not depend on the zero-read property
//! for correctness. It is only relied on for the Miri/wasm fallback arm's eager
//! `alloc_zeroed` to hand back valid-for-`T` bytes (any bit pattern is a valid
//! `EntityId`).

use std::marker::PhantomData;
use std::ptr::NonNull;

use crate::constants::{COMMIT_PAGE, POOL_MAX_SLAB, POOL_MIN_SLAB};
use crate::owner::{ColumnOwner, CommitOwner};
use crate::vm::{VmReservation, commit_at};

/// Typed, address-stable, growable column of `T` on one [`VmReservation`].
///
/// `#[repr(C)]` pins the hot pair (`base`, `len`) at the front so a `get`/slice
/// lowers to the same load-base / load-len / bounds-check / indexed-load
/// sequence a `Vec` would (mirrors the `InlandStore` field-order rationale).
///
/// NOT `Send`/`Sync` (the `NonNull` inside `VmReservation` and `base`): owners
/// that cross threads carry their own exclusivity argument in their manual
/// `unsafe impl Send/Sync` (SEND10 on `Archetype`; the `EcsMaster` blanket impl
/// for `DenseStore` via `DenseRegistry`). The invariants those impls rely on:
/// `base` is write-once (set at lazy materialization inside the `&mut
/// self`-only `grow_to`, stable thereafter), every mutation (`push` /
/// `swap_remove` / `set` / `truncate` / `clear` / `extend_exact`) requires
/// `&mut self`, and cross-thread `&self` reads (`as_slice` / `get`) touch only
/// committed plain-old-data memory below `len` with no interior mutability.
///
/// `O` is the commit owner the column's commits are counted under (UG-04):
/// [`ColumnOwner`] by default; a KC-18 kernel table is `VmColumn<T, TableOwner>`,
/// built with [`with_owner`](Self::with_owner). It is a type, so the choice
/// costs no instruction and no byte.
#[repr(C)]
pub struct VmColumn<T: Copy, O: CommitOwner = ColumnOwner> {
    /// Cached base of the reservation, hot-path twin of `vm`'s base.
    /// **Dangling until the first `grow_to`** — sound because the hot
    /// `as_slice` is `from_raw_parts(base, len)` and `len == 0` until the
    /// first `push`: a dangling-but-aligned `NonNull<T>` with length 0 is
    /// explicitly valid for `from_raw_parts`, and the `len` bounds check
    /// fails before any byte behind `base` could be touched. Write-once at
    /// materialization — every element address is stable thereafter.
    base: NonNull<T>,
    /// Live element count — the bounds oracle for `as_slice`/`get`/`set` and
    /// the frontier for `push`/`swap_remove`. Mutated only under `&mut self`.
    len: usize,
    /// Commit frontier in ELEMENTS (== committed_bytes / size_of::<T>()).
    /// Warm-path comparator in `push`; the `n * size` overflow class is
    /// confined to the cold `grow_to` path's checked math.
    committed_elems: usize,
    /// Hard element ceiling (== reservation / size_of::<T>(), the value the
    /// constructor was sized for). `push` past it panics loudly — the caller's
    /// row-index type (`u32` `unit_index`) cannot represent a larger column.
    reserve_elems: usize,
    /// Reservation size to materialize lazily (bytes, pre-granule-rounding).
    reserve_request: usize,
    /// The reservation itself; `None` until the first `grow_to` (NOT read on
    /// any hot path — `base`/`len` above are the hot pair).
    vm: Option<VmReservation>,
    /// Static owner label naming WHICH column this is (review #11:
    /// `"Archetype.entity_ids"` / `"DenseStore.s2e"`), so the exhaustion and
    /// bounds panics identify the failing column — both consumers share the
    /// `VmColumn<EntityId>` type, so `type_name` alone cannot. Cold-only
    /// diagnostic metadata; never read on a warm path.
    label: &'static str,
    /// Ties the erased byte reservation to `T` for the typed pointer API, and
    /// names the commit owner `O`. A ZST and the last field of the `#[repr(C)]`
    /// struct, so the owner moves no displacement and no size.
    _marker: PhantomData<(T, O)>,
}

impl<T: Copy> VmColumn<T> {
    /// Creates a LAZY column reserving room for `reserve_elems` elements
    /// (materialized by the first `push` — construction pays no reservation
    /// syscall, mirroring `InlandStore::new`), commit 0, len 0.
    ///
    /// `label` is a static owner tag naming the column in panic diagnostics
    /// (review #11). `reserve_elems` is the hard ceiling: a `push` past it
    /// panics. Callers pass the bound their row-index type can represent (F1:
    /// the archetype row ceiling `POOL_MAX_ROWS`; F3: the dense store's
    /// `reserve_rows`).
    ///
    /// # Panics
    /// * `size_of::<T>() == 0` — a ZST column has no element-count math.
    /// * `COMMIT_PAGE % size_of::<T>() != 0` — the supported-domain pin
    ///   (review #4, module doc): a non-dividing size would misalign the
    ///   commit frontier and panic a later LEGAL growth on the unix arm.
    /// * `reserve_elems == 0` — the ceiling must be non-zero.
    /// * `reserve_elems * size_of::<T>()` overflows `usize`.
    pub fn new(label: &'static str, reserve_elems: usize) -> Self {
        assert!(Self::SIZE > 0, "VmColumn[{label}]: element type must not be a ZST");
        // Review #4 — the page-divisibility domain pin (see the module doc's
        // "Supported element domain"): keeps `committed_elems * SIZE` exact, so
        // every `commit(old, new)` range stays page-aligned. The condition is
        // a per-`T` constant, so it folds away for every supported `T`.
        assert!(
            COMMIT_PAGE.is_multiple_of(Self::SIZE),
            "VmColumn[{label}]: size_of::<T>() = {} must divide COMMIT_PAGE ({})",
            Self::SIZE,
            COMMIT_PAGE
        );
        assert!(reserve_elems > 0, "VmColumn[{label}]: reserve_elems must be non-zero");
        let bytes = reserve_elems
            .checked_mul(Self::SIZE)
            .expect("VmColumn::new: reserve_elems * size_of::<T>() overflows usize");
        Self {
            base: NonNull::dangling(),
            len: 0,
            committed_elems: 0,
            reserve_elems,
            reserve_request: bytes,
            vm: None,
            label,
            _marker: PhantomData,
        }
    }
}

impl<T: Copy, O: CommitOwner> VmColumn<T, O> {
    /// Element stride in bytes. A zero-size `T` is rejected at construction
    /// (`new`'s assert) — the id columns this primitive serves are all 8-byte
    /// `EntityId`, and the element-count math (`committed_bytes / SIZE`) would
    /// divide by zero for a ZST.
    const SIZE: usize = size_of::<T>();

    /// Creates a lazy column exactly like [`VmColumn::new`], whose commits are
    /// counted under owner `O` instead of the default [`ColumnOwner`] (UG-04):
    /// a KC-18 kernel table is `VmColumn::<T, TableOwner>::with_owner(..)`.
    ///
    /// # Panics
    /// As [`VmColumn::new`].
    pub fn with_owner(label: &'static str, reserve_elems: usize) -> Self {
        let lazy = VmColumn::<T>::new(label, reserve_elems);
        // Re-tagging moves plain fields only: a lazy column holds no
        // reservation yet (`vm` is `None`), and the owner is read at commit
        // time alone.
        Self {
            base: lazy.base,
            len: lazy.len,
            committed_elems: lazy.committed_elems,
            reserve_elems: lazy.reserve_elems,
            reserve_request: lazy.reserve_request,
            vm: lazy.vm,
            label: lazy.label,
            _marker: PhantomData,
        }
    }

    /// Appends `value` at the frontier, growing the commit frontier (cold,
    /// rare) when the live range reaches the committed frontier.
    ///
    /// O(1) amortized: the warm path is one compare (`len < committed_elems`),
    /// one indexed `ptr::write`, one increment — no realloc, no copy, the base
    /// never moves.
    #[inline]
    pub fn push(&mut self, value: T) {
        if self.len == self.committed_elems {
            self.grow_to(self.len + 1);
        }
        debug_assert!(self.len < self.committed_elems);
        // SAFETY: `len < committed_elems` (the branch above grew the frontier
        //   when they were equal), so the slot at `len` lies in the committed
        //   read/write prefix of the reservation and is 8-aligned (`base` is
        //   granule-aligned on the syscall arms, ≥ 4096-aligned on the
        //   fallback, and `T`'s align divides that). `T: Copy` ⇒ the slot needs
        //   no prior drop. `&mut self` ⇒ exclusive access. After the write the
        //   slot is owned by the column (len is bumped), so no reader observes
        //   uninitialized bytes at or above `len`.
        unsafe {
            self.base.as_ptr().add(self.len).write(value);
        }
        self.len += 1;
    }

    /// Bulk-appends every element of `iter` at the frontier, growing the commit
    /// frontier ONCE to cover all of them (the batch-spawn append path).
    ///
    /// This is the drop-in replacement for `Vec::extend` on the two batch
    /// spawn/load funnels (`spawn_batch_command` / `load_writer`). Unlike
    /// `Vec::extend`'s realloc-doubling chain (the memcpy-spike class this audit
    /// removes), it commits the whole `additional` span at the frontier in one
    /// cold event, then streams the elements into address-stable slots — zero
    /// bytes copied out of a moved buffer, the base never moves.
    ///
    /// `ExactSizeIterator` so the grow is sized exactly once up front. The
    /// reported `len()` is NOT trusted for the write bound (review #1): an
    /// over-yielding iterator (a lying `len()` is safe code) panics on a
    /// release bounds check before it could write past the committed frontier;
    /// an under-yielding iterator leaves `len` grown by the count actually
    /// yielded (safe — never-written slots stay above `len`).
    ///
    /// # Panics
    /// * the iterator yields MORE items than its `len()` claimed (release
    ///   check — panic, never UB).
    /// * `len + iter.len()` overflows or exceeds the reservation ceiling.
    #[inline]
    pub fn extend_exact(&mut self, iter: impl ExactSizeIterator<Item = T>) {
        let additional = iter.len();
        if additional == 0 {
            return;
        }
        let target = self
            .len
            .checked_add(additional)
            .expect("VmColumn::extend_exact: len + additional overflows usize");
        if target > self.committed_elems {
            self.grow_to(target);
        }
        debug_assert!(target <= self.committed_elems);
        // SAFETY: `grow_to(target)` (or the pre-existing frontier) guarantees
        //   `target = len + additional <= committed_elems`, so every slot in
        //   `[len, len + additional)` lies in the committed read/write prefix
        //   and is aligned. The write index is `len + count` with `count <
        //   additional` enforced by the RELEASE assert INSIDE the loop — the
        //   bound is structural, NOT trusted from `ExactSizeIterator::len()`
        //   (review #1: a lying over-yielding iterator is safe code and panics
        //   here instead of writing past the frontier). `T: Copy` ⇒ no slot
        //   needs a prior drop. `&mut self` ⇒ exclusive access. `len` grows by
        //   the count actually written, so an under-yield never exposes an
        //   unwritten slot.
        let mut count = 0usize;
        unsafe {
            let base = self.base.as_ptr();
            for value in iter {
                // Review #1 (BLOCKER fix) — RELEASE bounds check: caps writes at
                // the `additional` slots the grow proved committed. One
                // predictable never-taken compare per element on the batch path.
                assert!(
                    count < additional,
                    "VmColumn[{}]::extend_exact: iterator yielded more items than its \
                     len() ({additional})",
                    self.label
                );
                base.add(self.len + count).write(value);
                count += 1;
            }
        }
        debug_assert_eq!(
            count, additional,
            "VmColumn::extend_exact: iterator under-yielded vs its len()"
        );
        self.len += count;
    }

    /// Removes the element at `index`, moving the last element into its place
    /// and returning the removed value (`Vec::swap_remove` semantics).
    ///
    /// O(1): no shift. `T: Copy`, so the removed value is returned by copy and
    /// the vacated tail slot needs no drop.
    ///
    /// # Panics
    /// * `index >= len` — a RELEASE bounds check (review #2), restoring
    ///   `Vec::swap_remove`'s panic-on-out-of-bounds: an empty-column call
    ///   would otherwise wrap `len - 1` and read out of bounds (reachable from
    ///   `remove_entity`'s `saturating_sub(1)` on a corrupt-empty archetype
    ///   whose zero-pool `pop_entity()` vacuously succeeds). This sits on
    ///   structural-change paths only — one predictable compare, exactly the
    ///   check the former `Vec` paid.
    #[inline]
    pub fn swap_remove(&mut self, index: usize) -> T {
        // Review #2 — release bounds check (the panic arm lowers to a cold
        // call; the taken path is one predictable compare).
        assert!(
            index < self.len,
            "VmColumn[{}]::swap_remove: index {index} out of bounds (len {})",
            self.label,
            self.len
        );
        let last = self.len - 1;
        // SAFETY: `index < len` (release assert above) and `last = len - 1 <
        //   len <= committed_elems` (`len >= 1`, so the sub cannot wrap), so
        //   both slots lie in the committed read/write prefix, are aligned, and
        //   were initialized by `push`/`set`/`extend_exact`. `T: Copy` ⇒ `read`
        //   copies the value out without a move-out drop. The move is BRANCHED
        //   (review #9): when `index != last`, `last`'s value is copied into
        //   `index`, overwriting a `Copy` value (no old-value drop needed);
        //   when `index == last` no write happens at all — the slot's bytes are
        //   simply abandoned above the new `len` and can never be read again.
        //   `&mut self` ⇒ exclusive access.
        unsafe {
            let removed = self.base.as_ptr().add(index).read();
            if index != last {
                let last_value = self.base.as_ptr().add(last).read();
                self.base.as_ptr().add(index).write(last_value);
            }
            self.len = last;
            removed
        }
    }

    /// The number of live elements.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// `true` iff no element is live.
    ///
    /// Part of the primitive's specified API (audit F1/F3): no F1/F3 call site
    /// needs it today (the archetype/dense stores query `len`/`column.count`
    /// directly), but it is a core `Vec`-parity accessor exercised by the unit
    /// tests and kept for future world-reset / diagnostic callers.
    #[allow(dead_code)]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Raw base pointer to element 0. Valid for `len` reads (dangling but
    /// well-aligned when `len == 0`), and — because the base is write-once —
    /// for the reservation's whole lifetime once materialized. The
    /// `EntityReservoir` claim path reads through it from worker threads
    /// without forming a `&[T]` over a length another thread may later change.
    #[inline]
    pub fn as_ptr(&self) -> *const T {
        self.base.as_ptr()
    }

    /// The live elements as a contiguous slice, in index order. This is the
    /// drop-in replacement for the former `&Vec<T>` deref — callers reach it
    /// through the same `entity_ids_slice()` / `s2e()` accessors and take
    /// `.as_ptr()` off it.
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        // SAFETY: before materialization `base` is `NonNull::dangling()` AND
        //   `len == 0` — `from_raw_parts(dangling, 0)` is explicitly valid.
        //   After materialization `base` is non-null, `T`-aligned (page-aligned
        //   on the syscall arms, ≥ 4096-aligned on the fallback, both ⊇
        //   `align_of::<T>()`), its provenance spans the whole single-object
        //   reservation, and `len * SIZE ≤ committed_bytes ≤ os_len ≤
        //   isize::MAX`; every element in `[0, len)` was initialized by
        //   `push`/`set` (nothing is read at or above `len`). No reference
        //   escapes the borrow.
        unsafe { std::slice::from_raw_parts(self.base.as_ptr(), self.len) }
    }

    /// The live elements as a contiguous **mutable** slice, in index order.
    ///
    /// The `&mut self` twin of [`as_slice`](Self::as_slice), for the callers
    /// that overwrite a RUN of elements rather than one. `set` in a loop pays a
    /// release bounds check per element, which is the right price for a
    /// structural-change path and the wrong one for a `copy_from_slice` of a
    /// formatted log line: the bound is the same for every byte and proving it
    /// once is exactly what a slice does.
    ///
    /// It cannot widen what a caller may reach — the slice is `[0, len)`, the
    /// same span `as_slice` exposes — and it cannot move the base, because it
    /// neither grows nor commits.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        // SAFETY: identical to `as_slice`'s, with exclusivity strengthened from
        //   "no other reference escapes" to `&mut self`. Before materialization
        //   `base` is `NonNull::dangling()` AND `len == 0` —
        //   `from_raw_parts_mut(dangling, 0)` is explicitly valid. After it,
        //   `base` is non-null, `T`-aligned, its provenance spans the whole
        //   single-object reservation, and `len * SIZE ≤ committed_bytes ≤
        //   os_len ≤ isize::MAX`; every element in `[0, len)` was initialized by
        //   `push`/`set`/`extend_exact`, so no uninitialized `T` is exposed.
        unsafe { std::slice::from_raw_parts_mut(self.base.as_ptr(), self.len) }
    }

    /// Returns the element at `index`, or `None` if `index >= len`.
    #[inline]
    pub fn get(&self, index: usize) -> Option<T> {
        if index < self.len {
            // SAFETY: `index < len <= committed_elems`, so the slot is in the
            //   committed prefix, aligned, and was initialized by `push`/`set`.
            //   `T: Copy` ⇒ the read copies it out without disturbing the slot.
            Some(unsafe { self.base.as_ptr().add(index).read() })
        } else {
            None
        }
    }

    /// Overwrites the element at `index` with `value`.
    ///
    /// # Panics
    /// * `index >= len` — a RELEASE bounds check (review #2), restoring the
    ///   former `Vec` `IndexMut` panic. Structural-change paths only.
    #[inline]
    pub fn set(&mut self, index: usize, value: T) {
        // Review #2 — release bounds check (Vec `IndexMut` parity).
        assert!(
            index < self.len,
            "VmColumn[{}]::set: index {index} out of bounds (len {})",
            self.label,
            self.len
        );
        // SAFETY: `index < len <= committed_elems` (release assert above), so
        //   the slot is in the committed read/write prefix and aligned. `T:
        //   Copy` ⇒ the old value needs no drop; the write overwrites it in
        //   place. `&mut self` ⇒ exclusive access.
        unsafe {
            self.base.as_ptr().add(index).write(value);
        }
    }

    /// Shortens the column to at most `new_len` elements (`Vec::truncate`
    /// semantics). Growing is a no-op. `T: Copy` ⇒ the dropped tail needs no
    /// destructor; the committed frontier is kept for reuse.
    #[inline]
    pub fn truncate(&mut self, new_len: usize) {
        if new_len < self.len {
            self.len = new_len;
        }
    }

    /// Resets the live range to empty (`len = 0`) WITHOUT decommitting: the
    /// reservation and its committed frontier are kept for reuse (the
    /// world-reset API; mirrors `InlandStore::clear` minus the memset).
    ///
    /// No memset is needed (unlike `InlandStore`): `VmColumn` never reads an
    /// element it did not first `push`/`set` (`len` gates every read), so a
    /// post-clear regrowth re-`push`es into slots that are written before they
    /// are read — a stale byte in `[0, old_len)` can never be observed. `T:
    /// Copy` ⇒ no dropped elements to account for either.
    ///
    /// The world reset of `EntityReservoir` (the recycled-entity stack) is the
    /// in-place-reuse caller; the archetype / dense id columns are dropped
    /// whole instead.
    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Commit frontier in elements (diagnostics — mirror of
    /// `InlandStore::committed_slots` / `ComponentPool::committed_rows`).
    #[inline]
    pub fn committed_elems(&self) -> usize {
        self.committed_elems
    }

    /// Grows the commit frontier to cover AT LEAST `n` elements without
    /// changing `len` — the `Vec::with_capacity` analog for a column whose
    /// caller asked for committed room up front (`EntityMaster::with_capacity`).
    /// A no-op when the frontier already covers `n`.
    ///
    /// # Panics
    /// * `n` exceeds the reservation ceiling (the `grow_to` exhaustion assert).
    #[cold]
    pub fn precommit(&mut self, n: usize) {
        if n > self.committed_elems {
            self.grow_to(n);
        }
    }

    /// Cold frontier growth: commit enough slabs to cover `n` elements.
    ///
    /// Materializes the reservation lazily on the first call (the syscall is
    /// deferred off construction, exactly as `InlandStore::grow_to`). Growth
    /// policy: byte doubling clamped to `[POOL_MIN_SLAB, POOL_MAX_SLAB]`,
    /// request-dominant, capped at the reservation ceiling — the same shape as
    /// `ComponentPool::grow_rows` and `InlandStore::grow_to`, reusing the pool
    /// slab constants.
    #[cold]
    #[inline(never)]
    fn grow_to(&mut self, n: usize) {
        // Lazy materialization: the reservation syscall is deferred from
        // construction to the first growth event (this fn is already #[cold]).
        let vm = match &self.vm {
            Some(vm) => vm,
            None => {
                let vm = VmReservation::reserve(self.reserve_request);
                self.base = vm.base().cast();
                self.vm.insert(vm)
            }
        };

        assert!(
            n <= self.reserve_elems,
            "VmColumn[{}]<{}> exhausted: {n} elements requested, reservation ceiling is {} \
             (grow the reserve at construction)",
            self.label,
            core::any::type_name::<T>(),
            self.reserve_elems
        );

        // Page chain (review #4, module doc): `old_bytes = committed_elems ×
        // SIZE` is page-aligned — either `committed_elems` came from an
        // unclamped `new_bytes / SIZE` (exact because SIZE | page | new_bytes)
        // or the `min(reserve_elems)` clamp bound, in which case `committed_elems
        // == reserve_elems` and THIS call cannot exist (the exhaustion assert
        // above fires first: `n > committed_elems == reserve_elems`). The mul
        // cannot overflow: `committed_elems <= reserve_elems`, overflow-checked
        // in `new`.
        let old_bytes = self.committed_elems * Self::SIZE;
        let needed = checked_slab_round(n * Self::SIZE);
        // Geometric doubling clamped to [MIN, MAX], request-dominant (a single
        // huge request is a single event), never past the reservation ceiling.
        // Every term is a page multiple (old_bytes above; the slab constants;
        // `needed - old_bytes` as a difference of page multiples; `os_len`, a
        // granule multiple), so `new_bytes` stays page-aligned and `vm.commit`
        // receives an aligned range on every arm (the unix `mprotect`
        // alignment requirement).
        let step = old_bytes.clamp(POOL_MIN_SLAB, POOL_MAX_SLAB).max(needed - old_bytes);
        let new_bytes = (old_bytes + step).min(vm.os_len());
        debug_assert!(new_bytes >= needed, "VmColumn::grow_to post-condition (proof) violated");

        // SAFETY: `off + len = new_bytes`, which the `min` above caps at
        // `vm.os_len()`, and `new_bytes > old_bytes`, so `new_bytes - old_bytes`
        // cannot wrap: every caller grows past the frontier (`n >
        // committed_elems`), so `needed >= n * SIZE > old_bytes`; the step is at
        // least `needed - old_bytes`; and `os_len >= needed`, since `os_len` is
        // the granule-rounded `reserve_elems * SIZE` and `n <= reserve_elems`
        // (the exhaustion assert above).
        unsafe { commit_at::<O>(vm, old_bytes, new_bytes - old_bytes) };
        // Exact division (review #4): `SIZE | COMMIT_PAGE | new_bytes`, so no
        // flooring slack exists. The `min(reserve_elems)` clamp guards the case
        // where page or granule padding rounds the byte frontier above the
        // element ceiling the reservation was sized for — a TERMINAL state (see
        // the page-chain note above: no further grow can reach the commit path).
        self.committed_elems = (new_bytes / Self::SIZE).min(self.reserve_elems);
        debug_assert!(
            self.committed_elems >= n,
            "VmColumn::grow_to: committed frontier must cover the request"
        );
    }
}

/// Cold-path page rounding with overflow check. Private twin of
/// `vm::checked_align_up` specialized to the commit page (packing plan D1;
/// `inland_store::checked_slab_round` is the granule-stepped sibling).
fn checked_slab_round(bytes: usize) -> usize {
    bytes
        .checked_add(COMMIT_PAGE - 1)
        .expect("VmColumn: slab rounding overflow")
        & !(COMMIT_PAGE - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An 8-byte id standing in for `boyko_ecs`'s `EntityId` (`#[repr(transparent)]` over
    /// `usize`), the element type of the columns these tests model.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    #[repr(transparent)]
    struct EntityId(usize);

    const SIZE: usize = size_of::<EntityId>(); // 8 on the 64-bit target
    const MIN_ELEMS: usize = POOL_MIN_SLAB / SIZE;

    fn eid(v: usize) -> EntityId {
        EntityId(v)
    }

    fn col(reserve: usize) -> VmColumn<EntityId> {
        VmColumn::new("test", reserve)
    }

    /// An `ExactSizeIterator` whose `len()` LIES — safe code, the review-#1
    /// threat model. `claimed` is reported regardless of how many items the
    /// inner iterator actually yields.
    struct Lying<I> {
        inner: I,
        claimed: usize,
    }

    impl<I: Iterator> Iterator for Lying<I> {
        type Item = I::Item;
        fn next(&mut self) -> Option<I::Item> {
            self.inner.next()
        }
        fn size_hint(&self) -> (usize, Option<usize>) {
            (self.claimed, Some(self.claimed))
        }
    }

    impl<I: Iterator> ExactSizeIterator for Lying<I> {}

    /// Push/read round trip: `as_slice` and `get` agree with a model `Vec`.
    #[test]
    fn push_get_slice_agree_with_vec() {
        let mut c = col(1024);
        let mut model = Vec::new();
        for i in 0..500 {
            c.push(eid(i));
            model.push(eid(i));
        }
        assert_eq!(c.len(), model.len());
        assert_eq!(c.as_slice(), model.as_slice());
        for (i, &expected) in model.iter().enumerate() {
            assert_eq!(c.get(i), Some(expected));
        }
        assert_eq!(c.get(500), None);
    }

    /// `swap_remove` returns the removed value and moves the last into place,
    /// matching `Vec::swap_remove` byte-for-byte across a random sequence.
    #[test]
    fn swap_remove_matches_vec() {
        let mut c = col(1024);
        let mut model: Vec<EntityId> = Vec::new();
        for i in 0..64 {
            c.push(eid(i));
            model.push(eid(i));
        }
        // Deterministic pseudo-random removal indices.
        let mut idx = 7usize;
        while !model.is_empty() {
            let i = idx % model.len();
            assert_eq!(c.swap_remove(i), model.swap_remove(i), "removed value mismatch");
            assert_eq!(c.as_slice(), model.as_slice(), "post-remove slice mismatch");
            idx = idx.wrapping_mul(31).wrapping_add(17);
        }
        assert!(c.is_empty());
    }

    /// `swap_remove` of the LAST element is the degenerate `index == last`
    /// path: it must not corrupt and must shrink by one.
    #[test]
    fn swap_remove_last_element() {
        let mut c = col(16);
        c.push(eid(1));
        c.push(eid(2));
        assert_eq!(c.swap_remove(1), eid(2));
        assert_eq!(c.as_slice(), &[eid(1)]);
        assert_eq!(c.swap_remove(0), eid(1));
        assert!(c.is_empty());
    }

    /// Review #2 — `swap_remove` on an EMPTY column must PANIC (Vec parity),
    /// not wrap `len - 1` into an out-of-bounds read. The bounds check is a
    /// RELEASE assert, so this test proves panic-not-UB in both profiles.
    #[test]
    fn swap_remove_empty_panics_not_ub() {
        let mut c = col(16);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.swap_remove(0)));
        assert!(r.is_err(), "swap_remove on an empty column must panic");
        assert_eq!(c.len(), 0, "failed swap_remove must not change len");
    }

    /// Review #2 — `set` past `len` must PANIC in release (Vec IndexMut parity).
    #[test]
    fn set_out_of_bounds_panics_not_ub() {
        let mut c = col(16);
        c.push(eid(1));
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.set(1, eid(9))));
        assert!(r.is_err(), "set past len must panic");
        assert_eq!(c.get(0), Some(eid(1)), "failed set must not corrupt");
    }

    /// Addresses are stable across multi-slab growth and written values
    /// survive (the whole point vs `Vec`): the cached base pointer never moves.
    #[test]
    fn addresses_stable_across_multi_slab_growth() {
        let mut c = col(8 * MIN_ELEMS);
        c.push(eid(42));
        let base0 = c.as_ptr();
        // SAFETY: `add(0)` offsets by zero bytes, which is in bounds for any pointer.
        let addr0 = unsafe { c.as_ptr().add(0) };

        // Grow across several slab boundaries.
        for i in 1..(4 * MIN_ELEMS) {
            c.push(eid(i));
        }
        assert!(c.committed_elems() >= 4 * MIN_ELEMS);
        assert_eq!(c.as_ptr(), base0, "base pointer moved across growth");
        // SAFETY: as above, a zero offset.
        assert_eq!(unsafe { c.as_ptr().add(0) }, addr0, "element 0 address moved");
        assert_eq!(c.get(0), Some(eid(42)), "written value lost across growth");
    }

    /// `set` overwrites in place; `get` observes it.
    #[test]
    fn set_overwrites_in_place() {
        let mut c = col(16);
        c.push(eid(1));
        c.push(eid(2));
        c.set(0, eid(99));
        assert_eq!(c.get(0), Some(eid(99)));
        assert_eq!(c.get(1), Some(eid(2)));
    }

    /// `clear` resets len, keeps the commit frontier, and a regrowth reads only
    /// freshly written values (no stale bytes surface — the write-before-read
    /// property).
    #[test]
    fn clear_keeps_commit_and_no_stale_bytes() {
        let mut c = col(4 * MIN_ELEMS);
        for i in 0..(MIN_ELEMS + 100) {
            c.push(eid(0xDEAD_0000 + i));
        }
        let committed = c.committed_elems();
        c.clear();
        assert_eq!(c.len(), 0);
        assert_eq!(c.committed_elems(), committed, "clear must not decommit");

        // Regrow: every read is a freshly pushed value, never a stale one.
        for i in 0..(MIN_ELEMS + 200) {
            c.push(eid(i));
        }
        for i in 0..(MIN_ELEMS + 200) {
            assert_eq!(c.get(i), Some(eid(i)), "stale byte surfaced at {i}");
        }
    }

    /// `truncate` shrinks the live range (keeping the prefix intact and the
    /// commit frontier untouched); truncating LARGER is a no-op (Vec parity).
    #[test]
    fn truncate_shrinks_and_larger_is_noop() {
        let mut c = col(64);
        for i in 0..10 {
            c.push(eid(i));
        }
        let committed = c.committed_elems();

        c.truncate(4);
        assert_eq!(c.len(), 4);
        assert_eq!(c.as_slice(), &[eid(0), eid(1), eid(2), eid(3)]);
        assert_eq!(c.get(4), None, "truncated tail must be unreadable");
        assert_eq!(c.committed_elems(), committed, "truncate must not decommit");

        c.truncate(100);
        assert_eq!(c.len(), 4, "truncate to a larger len must be a no-op");

        c.truncate(0);
        assert_eq!(c.len(), 0);
        assert!(c.is_empty());
    }

    /// Grow policy: first event is request-dominant vs MIN_SLAB, then doubling,
    /// every frontier page-aligned.
    ///
    /// Packing plan S2 re-derivation: the alignment pin read "a multiple of
    /// `COMMIT_GRANULE`", which the first frontier no longer is — it is one
    /// `COMMIT_PAGE` (`MIN_ELEMS` = 4096 / 8 = 512 `EntityId`s, was 8192).
    #[test]
    fn grow_policy() {
        let mut c = col(64 * MIN_ELEMS);
        c.push(eid(0)); // first event: MIN_SLAB (request 1 elem < MIN_SLAB)
        assert_eq!(c.committed_elems(), MIN_ELEMS, "first event commits MIN_SLAB");
        assert_eq!(c.committed_elems() * SIZE, COMMIT_PAGE, "the first frontier is one page");
        assert!((c.committed_elems() * SIZE).is_multiple_of(COMMIT_PAGE), "frontier page-aligned");

        // Fill to the frontier, one more push doubles.
        while c.len() < MIN_ELEMS {
            c.push(eid(c.len()));
        }
        c.push(eid(c.len()));
        assert_eq!(c.committed_elems(), 2 * MIN_ELEMS, "second event doubles");
    }

    /// Exhaustion: a push past the element ceiling panics loudly, naming the
    /// column label (review #11).
    #[test]
    fn exhaustion_panics_naming_the_column() {
        let mut c = col(4);
        for i in 0..4 {
            c.push(eid(i));
        }
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.push(eid(4))));
        let err = r.expect_err("push past the ceiling must panic");
        let msg = err
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default();
        assert!(msg.contains("test"), "exhaustion panic must name the column label: {msg}");
    }

    /// A never-materialized column (no push) has a valid empty slice and a
    /// dangling base — `as_slice` must not touch memory.
    #[test]
    fn empty_column_is_valid() {
        let c = col(16);
        assert_eq!(c.len(), 0);
        assert!(c.is_empty());
        assert_eq!(c.as_slice(), &[] as &[EntityId]);
        assert_eq!(c.get(0), None);
    }

    // ── extend_exact (review #6) ───────────────────────────────────────────

    /// An empty iterator is a no-op: no grow, no len change, no materialization.
    #[test]
    fn extend_exact_empty_iter_is_noop() {
        let mut c = col(16);
        c.extend_exact(std::iter::empty());
        assert_eq!(c.len(), 0);
        assert_eq!(c.committed_elems(), 0, "empty extend must not materialize");

        c.push(eid(1));
        c.extend_exact(std::iter::empty());
        assert_eq!(c.as_slice(), &[eid(1)]);
    }

    /// A batch crossing multiple slab boundaries lands in ONE grow event, the
    /// base stays put, and the contents match a model `Vec`.
    #[test]
    fn extend_exact_multi_slab_batch() {
        let mut c = col(16 * MIN_ELEMS);
        c.push(eid(0xAA));
        let base0 = c.as_ptr();

        let n = 5 * MIN_ELEMS; // crosses ≥ 2 slab boundaries past MIN_SLAB
        c.extend_exact((0..n).map(eid));
        assert_eq!(c.len(), 1 + n);
        assert!(c.committed_elems() > n, "one event must cover the batch");
        assert_eq!(c.as_ptr(), base0, "base moved across a batch extend");

        assert_eq!(c.get(0), Some(eid(0xAA)));
        for probe in [0, 1, MIN_ELEMS, 2 * MIN_ELEMS, n - 1] {
            assert_eq!(c.get(1 + probe), Some(eid(probe)), "batch element {probe} mismatch");
        }
    }

    /// An exact fill to the reservation ceiling succeeds; the NEXT push panics.
    #[test]
    fn extend_exact_exact_fit_at_ceiling() {
        let cap = 100;
        let mut c = col(cap);
        c.extend_exact((0..cap).map(eid));
        assert_eq!(c.len(), cap, "exact-fit extend must reach the ceiling");
        assert_eq!(c.get(cap - 1), Some(eid(cap - 1)));

        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.push(eid(cap))));
        assert!(r.is_err(), "push past the exactly-filled ceiling must panic");
    }

    /// Review #1 — a LYING `ExactSizeIterator` (claims 2, yields 100) must
    /// PANIC on the release bounds check, never write past the committed
    /// frontier. This test is profile-independent (the check is a release
    /// `assert!`), proving panic-not-UB under `--release` too.
    #[test]
    fn extend_exact_lying_over_yield_panics_not_ub() {
        let mut c = col(1024);
        c.push(eid(7));
        let lying = Lying { inner: (0..100).map(eid), claimed: 2 };
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            c.extend_exact(lying);
        }));
        assert!(r.is_err(), "an over-yielding lying iterator must panic, not write past bounds");
        // The prefix written before the panic is at most `claimed` items; the
        // pre-existing element is untouched.
        assert_eq!(c.get(0), Some(eid(7)), "pre-existing element corrupted");
    }

    /// Review #1 complement — an UNDER-yielding lying iterator (claims 10,
    /// yields 3) is SAFE: `len` grows by the yielded count only (release); the
    /// debug profile additionally trips the diagnostic `debug_assert_eq!`.
    #[test]
    fn extend_exact_lying_under_yield_is_safe() {
        let mut c = col(1024);
        let lying = Lying { inner: (0..3).map(eid), claimed: 10 };
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            c.extend_exact(lying);
        }));
        if cfg!(debug_assertions) {
            assert!(r.is_err(), "debug profile: the under-yield diagnostic must trip");
        } else {
            assert!(r.is_ok(), "release profile: under-yield is a safe short append");
            assert_eq!(c.len(), 3, "len must grow by the yielded count only");
            assert_eq!(c.as_slice(), &[eid(0), eid(1), eid(2)]);
        }
    }
}
