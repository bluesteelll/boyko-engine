//! Phase X.A — Chunked query data trait.
//!
//! Sibling trait to [`QueryData`] that adds a per-archetype-slice fetch path.
//! Used by [`Query::for_each_chunk`] / [`Query::par_for_each_chunk`].
//!
//! See `docs/PHASE-X.A-PLAN.md` §2.2 / §4 for the design rationale.
//!
//! # Current status: Step 1C (skeleton)
//!
//! Only the trait declaration exists. Leaf impls (`&T`, `&mut T`, `()`) land
//! in Wave 2 (Steps 2A / 2B / 2C); tuple variadics land in Wave 3 (Step 3A).
//!
//! [`Query::for_each_chunk`]: super::query::Query::for_each_chunk
//! [`Query::par_for_each_chunk`]: super::query::Query::par_for_each_chunk

use std::marker::PhantomData;

use super::data::QueryData;
use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::component::component::Component;

/// Per-row [`QueryData`] augmented with a per-archetype-slice fetch path.
///
/// `Query<D, F>::for_each_chunk` requires `D: ChunkedQueryData` so the
/// closure can receive `D::ChunkItem<'c>` (typically `&'c [T]`, `&'c mut [T]`,
/// or a tuple of element chunk items).
///
/// # Why a sibling trait instead of extending `QueryData`
///
/// Extending `QueryData` with `type ChunkItem<'c>` + `unsafe fn fetch_chunk`
/// would touch every existing impl (78 impls per Phase 10 expansion: 8
/// leaves × {readonly, mut, no-meta} variants + 12 arity tuples + 12 too-large
/// stubs). A sibling trait keeps every existing impl untouched and lets
/// custom `QueryData` types opt in to the chunked API by adding a single
/// extra impl block when (and only when) chunked iteration makes sense.
///
/// # Members (Phase X.A)
///
/// * `&T` for `T: Component` — `ChunkItem<'c> = &'c [T]`.
/// * `&mut T` for `T: Component` — `ChunkItem<'c> = &'c mut [T]`.
/// * `()` — `ChunkItem<'c> = ()` (no payload; useful for entity-only chunks).
/// * Tuples 1..=12 — `ChunkItem<'c> = (D0::ChunkItem<'c>, ...)`.
///
/// NON-members (deliberate):
/// * `Ref<'_, T>`, `Mut<'_, T>` — these expose per-row tick state; a slice
///   can't carry one tick per row without doubling the GAT surface. Filed
///   Phase 13.X for a `ChunkedTickedQueryData` variant if needed.
///
/// # Safety
///
/// Implementations MUST uphold:
///
/// 1. **CD1** — `set_chunk_readonly` / `set_chunk_mut` initialize a
///    `ChunkFetch<'c>` whose cached column bases are valid for the
///    full row range `[0, archetype.entity_count())`. Same contract as
///    `QueryData::set_table_*` but yielding columnar bases instead of
///    per-row scalar bases.
/// 2. **CD2** — `fetch_chunk(fetch, start, len)` returns a `ChunkItem<'c>`
///    whose constituent slices have length `len` and span rows
///    `[start, start + len)`. Caller is responsible for `start + len ≤
///    archetype.entity_count()`.
/// 3. **CD3** — for `ChunkItem<'c> = &'c mut [T]`, distinct invocations of
///    `fetch_chunk` with disjoint `[start, start + len)` ranges produce
///    non-aliasing slices. Caller enforces disjointness.
/// 4. **CD4** — split readonly / mut mirrors `QueryData::set_table_*`. For
///    types containing `&mut T`, `set_chunk_readonly` is forbidden.
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not implement `ChunkedQueryData` — cannot be used in `Query::for_each_chunk`",
    label = "not chunked-iterable",
    note = "members: `&T`, `&mut T`, `()`, tuples of those. `Ref<T>` / `Mut<T>` are NOT members; use `Query::iter()` instead for per-row tick state."
)]
pub unsafe trait ChunkedQueryData: QueryData {
    /// Per-chunk fetch scratch — typically the column bases. Distinct from
    /// `QueryData::Fetch<'w>` only in lifetime parameter (lifetime is `'c`,
    /// the closure-body scope). For most impls this is a `Copy` struct with
    /// the same fields as `QueryData::Fetch`.
    type ChunkFetch<'c>: Copy;

    /// Per-chunk yielded item. For `&T`: `&'c [T]`. For `&mut T`: `&'c mut [T]`.
    /// For tuples: a tuple of element chunk items.
    type ChunkItem<'c>;

    /// Build a `ChunkFetch` with all-NULL column bases. Paired with
    /// `set_chunk_readonly` / `set_chunk_mut`. Same shape as `QueryData::init_fetch`.
    fn init_chunk_fetch<'c>(state: &Self::State) -> Self::ChunkFetch<'c>;

    /// Refresh the chunk fetch from a read-only archetype pointer.
    ///
    /// # Safety
    ///
    /// Upholds CD1 + CD4:
    ///
    /// * `archetype` must be a live `*const Archetype` for `'c` with read-only
    ///   provenance (from `UnsafeEcsCell::archetype_ptr`).
    /// * `archetype` must contain every `ComponentId` in `state`.
    /// * For `D` containing `&mut T`, MUST NOT be called; impls panic as
    ///   per QD4 backstop.
    unsafe fn set_chunk_readonly<'c>(
        fetch: &mut Self::ChunkFetch<'c>,
        state: &Self::State,
        archetype: *const Archetype,
    );

    /// Refresh the chunk fetch from a write-capable archetype pointer.
    ///
    /// # Safety
    ///
    /// Upholds CD1 + CD4:
    ///
    /// * `archetype` must be a live `*mut Archetype` for `'c` with
    ///   write-capable provenance (from `UnsafeEcsCell::archetype_ptr_mut`).
    /// * `archetype` must contain every `ComponentId` in `state`.
    unsafe fn set_chunk_mut<'c>(
        fetch: &mut Self::ChunkFetch<'c>,
        state: &Self::State,
        archetype: *mut Archetype,
    );

    /// Materialize the chunk item for rows `[start, start + len)`.
    ///
    /// # Safety
    ///
    /// Upholds CD2 + CD3:
    ///
    /// * `set_chunk_*` must have been called for the current archetype.
    /// * `start + len ≤ archetype.entity_count()` at call time.
    /// * For `ChunkItem` containing `&'c mut [T]`, the caller must ensure no
    ///   sibling invocation references an overlapping row range.
    unsafe fn fetch_chunk<'c>(
        fetch: &Self::ChunkFetch<'c>,
        start: usize,
        len: usize,
    ) -> Self::ChunkItem<'c>;
}

// ── `&T: ChunkedQueryData` impl (Wave 2 Step 2A) ───────────────────────────

/// Per-archetype read-only fetch scratch for `&T: ChunkedQueryData`.
///
/// Mirrors [`super::data::ReadFetch`] but caches a single column base pointer
/// for use across the full archetype slice instead of per-row indexing.
///
/// `Copy` / `Clone` are implemented manually so the auto-derive does not
/// synthesise an unwanted `T: Copy` blanket bound (same rationale as
/// `ReadFetch`).
pub struct ReadChunkFetch<'c, T> {
    /// Base pointer to the active archetype's column for `T`. NULL until
    /// `set_chunk_readonly` / `set_chunk_mut` runs (CD1).
    base: *const T,
    /// Type binding tying the fetch lifetime to `'c`.
    _marker: PhantomData<&'c [T]>,
}

impl<T> Clone for ReadChunkFetch<'_, T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for ReadChunkFetch<'_, T> {}

// SAFETY (CD1-CD4):
//   - CD1: `set_chunk_readonly` / `set_chunk_mut` overwrite `base` with a
//     column-base pointer valid for the full row range; for `&T` both methods
//     share the same body (read-only data does not need write provenance).
//   - CD2: `fetch_chunk(start, len)` returns a `&'c [T]` spanning rows
//     `[start, start + len)`; caller guarantees `start + len ≤ entity_count`.
//   - CD3: vacuous — `&T` slices are shared, never `&mut`.
//   - CD4: the read-only path is the canonical path for `&T`; `set_chunk_mut`
//     just degrades to it (mirrors `QueryData::set_table_mut` for `&T` in
//     `data.rs:373-386`).
unsafe impl<T: Component> ChunkedQueryData for &T {
    type ChunkFetch<'c> = ReadChunkFetch<'c, T>;
    type ChunkItem<'c> = &'c [T];

    #[inline]
    fn init_chunk_fetch<'c>(_state: &Self::State) -> Self::ChunkFetch<'c> {
        ReadChunkFetch {
            base: std::ptr::null(),
            _marker: PhantomData,
        }
    }

    #[inline]
    unsafe fn set_chunk_readonly<'c>(
        fetch: &mut Self::ChunkFetch<'c>,
        state: &Self::State,
        archetype: *const Archetype,
    ) {
        // SAFETY (CD1): mirror `QueryData::set_table_readonly` for `&T`
        //   (`data.rs:357-370`).
        //   - `archetype` is a live `*const Archetype` for `'c` per caller
        //     contract (Phase 7 U1/U2 slab stability).
        //   - `columns` is at offset 0 (Phase 7 D4 / `archetype.rs:170`
        //     `offset_of!` assert).
        //   - `state.id.0 < MAX_COMPONENTS` by construction of the cached id.
        //   - The matched-archetype invariant from `QueryState` guarantees
        //     the column is present (non-null `column.ptr`).
        //   - The column base is `ComponentPool::buffer_ptr()`, guaranteed
        //     `SIMD_BUFFER_ALIGN`-aligned by Phase X.A SIMD-A1 (Wave 1).
        // O2 (defense-in-depth): a dense `T` has NO archetype column — reading
        // `columns[id]` would yield a NULL/stale `ptr` and `fetch_chunk` would
        // build a slice over a NULL base. Sound usage never reaches here (the
        // `for_each_chunk` call site compile-rejects a dense `D` via
        // `const { assert!(!D::HAS_DENSE) }` in `chunk_iter.rs`), but make the
        // reject local to the impl too.
        debug_assert!(
            !T::STORAGE_IS_DENSE,
            "ChunkedQueryData::set_chunk_readonly on a dense T ({}); \
             dense terms are not supported on for_each_chunk (use Query::iter / dense_iter)",
            std::any::type_name::<T>(),
        );
        let column = unsafe { (*archetype).columns.get_unchecked(state.id.0) };
        debug_assert!(!column.ptr.is_null(), "CD1: column was unexpectedly null");
        fetch.base = column.ptr as *const T;
    }

    #[inline]
    unsafe fn set_chunk_mut<'c>(
        fetch: &mut Self::ChunkFetch<'c>,
        state: &Self::State,
        archetype: *mut Archetype,
    ) {
        // For `&T`, the mutable variant degrades to the same read. Re-borrow
        // as `*const` internally; no write-capable provenance is consumed.
        // SAFETY (CD1, CD4): same conditions as `set_chunk_readonly` with the
        //   additional caller guarantee that `archetype` carries fresh
        //   `archetype_ptr_mut` provenance — strictly stronger than what the
        //   read-only path requires. Mirrors `QueryData::set_table_mut` for
        //   `&T` in `data.rs:373-386`.
        unsafe { Self::set_chunk_readonly(fetch, state, archetype as *const _) }
    }

    #[inline]
    unsafe fn fetch_chunk<'c>(
        fetch: &Self::ChunkFetch<'c>,
        start: usize,
        len: usize,
    ) -> Self::ChunkItem<'c> {
        // SAFETY (CD1, CD2, plan §6 SIMD-A1):
        //   - `set_chunk_readonly` / `set_chunk_mut` was called before this
        //     `fetch_chunk` (caller contract); `fetch.base` is non-null and
        //     points at the active archetype's column for `T`.
        //   - `start + len ≤ archetype.entity_count()` (caller contract;
        //     `chunk_iter` enforces `0..entity_count` for sequential dispatch
        //     and the `BatchingStrategy` partitions rows into non-overlapping
        //     sub-ranges for parallel dispatch).
        //   - `column.ptr` is `SIMD_BUFFER_ALIGN`-aligned per Phase X.A
        //     SIMD-A1 (Wave 1); since `align_of::<T>() ≤ SIMD_BUFFER_ALIGN`
        //     for every `T: Component`, `base.add(start)` is at least
        //     `align_of::<T>()`-aligned (sufficient for `&[T]`).
        //   - The slice lifetime `'c` is the closure-body scope; `'c` is
        //     bounded by the archetype-pointer scope (Phase 7 U1/U2 slab
        //     stability) so the slice does not outlive the column.
        //   - `len ≤ isize::MAX / size_of::<T>()` (slice invariant — checked
        //     by `debug_assert!` below).
        debug_assert!(
            len <= isize::MAX as usize / std::mem::size_of::<T>().max(1),
            "fetch_chunk: len {} exceeds isize::MAX bounds for T = {}",
            len,
            std::any::type_name::<T>(),
        );
        unsafe { std::slice::from_raw_parts(fetch.base.add(start), len) }
    }
}

// ── `&mut T: ChunkedQueryData` impl (Wave 2 Step 2B) ───────────────────────

/// Per-archetype write-capable fetch scratch for `&mut T: ChunkedQueryData`.
///
/// Mirrors [`super::data::WriteFetch`] but caches a single column base pointer
/// for the full archetype slice. The provenance is write-capable because
/// `set_chunk_mut` receives a `*mut Archetype` minted by
/// `UnsafeEcsCell::archetype_ptr_mut` (Phase 7 U7).
///
/// `Copy` / `Clone` are implemented manually so the auto-derive does not
/// synthesise an unwanted `T: Copy` blanket bound (same rationale as
/// `WriteFetch`).
pub struct WriteChunkFetch<'c, T> {
    /// Base pointer to the active archetype's column for `T`. NULL until
    /// `set_chunk_mut` runs (CD1).
    base: *mut T,
    /// Type binding tying the fetch lifetime to `'c`.
    _marker: PhantomData<&'c mut [T]>,
}

impl<T> Clone for WriteChunkFetch<'_, T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for WriteChunkFetch<'_, T> {}

// SAFETY (CD1-CD4):
//   - CD1: `set_chunk_mut` overwrites `base` with a write-capable column-base
//     pointer valid for the full row range; `set_chunk_readonly` is forbidden
//     (CD4 runtime backstop).
//   - CD2: `fetch_chunk(start, len)` returns a `&'c mut [T]` spanning rows
//     `[start, start + len)`.
//   - CD3: distinct invocations of `fetch_chunk` with disjoint ranges produce
//     non-aliasing slices; the `chunk_iter` / `par_chunk` dispatcher enforces
//     disjointness (sequential: single call with the whole archetype;
//     parallel: `BatchingStrategy` partitions rows into non-overlapping
//     sub-ranges before spawning workers).
//   - CD4: `set_chunk_readonly` is forbidden — the type system prevents the
//     call (the chunk dispatcher only invokes `set_chunk_mut` for
//     write-capable `D`). The runtime panic backstop catches contract
//     violations in custom impls. Mirrors `QueryData::set_table_readonly` for
//     `&mut T` in `data.rs:530-547`.
unsafe impl<T: Component> ChunkedQueryData for &mut T {
    type ChunkFetch<'c> = WriteChunkFetch<'c, T>;
    type ChunkItem<'c> = &'c mut [T];

    #[inline]
    fn init_chunk_fetch<'c>(_state: &Self::State) -> Self::ChunkFetch<'c> {
        WriteChunkFetch {
            base: std::ptr::null_mut(),
            _marker: PhantomData,
        }
    }

    #[cold]
    #[inline(never)]
    unsafe fn set_chunk_readonly<'c>(
        _fetch: &mut Self::ChunkFetch<'c>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {
        // CD4 backstop: the chunk dispatcher should never reach this path for
        // `&mut T`. The type-level discipline of `QueryState` plus the
        // dispatcher's mutable-vs-read split makes this `unreachable!` under
        // sound usage. Panic loudly on misuse rather than silently mis-typing
        // the access — mirrors the equivalent `panic!` in
        // `QueryData::set_table_readonly` for `&mut T` (`data.rs:530-547`).
        panic!(
            "CD4 violation: ChunkedQueryData::set_chunk_readonly called on &mut T (T = {}); \
             the dispatcher must use set_chunk_mut for write-capable D",
            std::any::type_name::<T>(),
        );
    }

    #[inline]
    unsafe fn set_chunk_mut<'c>(
        fetch: &mut Self::ChunkFetch<'c>,
        state: &Self::State,
        archetype: *mut Archetype,
    ) {
        // SAFETY (CD1): mirror `QueryData::set_table_mut` for `&mut T`
        //   (`data.rs:549-565`).
        //   - `archetype` carries write-capable provenance (caller obtained
        //     it via `UnsafeEcsCell::archetype_ptr_mut`; Phase 7 U7).
        //   - `columns` is at offset 0 (Phase 7 D4).
        //   - `state.id.0 < MAX_COMPONENTS` by construction of the cached id.
        //   - The matched-archetype invariant guarantees the column is
        //     present (non-null `column.ptr`).
        //   - `column.ptr` is `*mut u8` with write-capable provenance
        //     preserved from `refresh_column` at pool-add time (Phase 7 U7);
        //     the cast to `*mut T` preserves the Unique tag.
        //   - The column base is `SIMD_BUFFER_ALIGN`-aligned per Phase X.A
        //     SIMD-A1 (Wave 1).
        // O2 (defense-in-depth): same dense reject as the `&T` impl —
        // `set_chunk_mut` is the canonical chunk-init path for `&mut T`, so the
        // guard lives here (the `set_chunk_readonly` CD4 backstop above already
        // panics unconditionally). Sound usage is compile-rejected at the
        // `for_each_chunk` call site (`chunk_iter.rs`).
        debug_assert!(
            !T::STORAGE_IS_DENSE,
            "ChunkedQueryData::set_chunk_mut on a dense T ({}); \
             dense terms are not supported on for_each_chunk (use Query::iter / dense_iter)",
            std::any::type_name::<T>(),
        );
        let column = unsafe { (*archetype).columns.get_unchecked(state.id.0) };
        debug_assert!(!column.ptr.is_null(), "CD1: column was unexpectedly null");
        fetch.base = column.ptr as *mut T;
    }

    #[inline]
    unsafe fn fetch_chunk<'c>(
        fetch: &Self::ChunkFetch<'c>,
        start: usize,
        len: usize,
    ) -> Self::ChunkItem<'c> {
        // SAFETY (CD1, CD2, CD3, plan §6 SIMD-A1):
        //   - `set_chunk_mut` was called before this `fetch_chunk` (caller
        //     contract); `fetch.base` is non-null and points at the active
        //     archetype's column for `T` with write-capable provenance.
        //   - `start + len ≤ archetype.entity_count()` (caller contract).
        //   - CD3 disjointness: the caller (`chunk_iter::for_each_chunk_impl`
        //     / `par_chunk::par_for_each_chunk_impl`) ensures distinct
        //     invocations reference non-overlapping row ranges. For
        //     sequential dispatch the whole archetype is one call; for
        //     parallel dispatch the `BatchingStrategy` partitions rows into
        //     non-overlapping sub-ranges before spawning workers.
        //   - `column.ptr` is `SIMD_BUFFER_ALIGN`-aligned per Phase X.A
        //     SIMD-A1 (Wave 1); since `align_of::<T>() ≤ SIMD_BUFFER_ALIGN`,
        //     `base.add(start)` is at least `align_of::<T>()`-aligned.
        //   - The slice lifetime `'c` is the closure-body scope; bounded by
        //     the archetype-pointer scope.
        //   - `len ≤ isize::MAX / size_of::<T>()` (slice invariant — checked
        //     by `debug_assert!` below).
        debug_assert!(
            len <= isize::MAX as usize / std::mem::size_of::<T>().max(1),
            "fetch_chunk: len {} exceeds isize::MAX bounds for T = {}",
            len,
            std::any::type_name::<T>(),
        );
        unsafe { std::slice::from_raw_parts_mut(fetch.base.add(start), len) }
    }
}

// ── `(): ChunkedQueryData` impl (Wave 2 Step 2C) ───────────────────────────

// SAFETY (CD1-CD4):
//   - `()` carries no payload and touches no columns; every method body is a
//     no-op.
//   - CD1/CD4: vacuous — no fetch state to initialise.
//   - CD2: vacuous — empty chunk item.
//   - CD3: vacuous — no shared/mutable slices.
//   Mirrors `QueryData for ()` in `data.rs:1383-1401`.
unsafe impl ChunkedQueryData for () {
    type ChunkFetch<'c> = ();
    type ChunkItem<'c> = ();

    #[inline]
    fn init_chunk_fetch<'c>(_state: &Self::State) -> Self::ChunkFetch<'c> {}

    #[inline]
    unsafe fn set_chunk_readonly<'c>(
        _fetch: &mut Self::ChunkFetch<'c>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {
    }

    #[inline]
    unsafe fn set_chunk_mut<'c>(
        _fetch: &mut Self::ChunkFetch<'c>,
        _state: &Self::State,
        _archetype: *mut Archetype,
    ) {
    }

    #[inline]
    unsafe fn fetch_chunk<'c>(
        _fetch: &Self::ChunkFetch<'c>,
        _start: usize,
        _len: usize,
    ) -> Self::ChunkItem<'c> {
    }
}

// ── Variadic tuple impls (Wave 3 Step 3A, plan §5.1) ───────────────────────
//
// One `macro_rules!` site emits `ChunkedQueryData` impls for tuple arities
// `1..=MAX_QUERY_DATA_ARITY` (= 12). Mirrors the paired-ident invocation syntax
// of `impl_query_data_tuple!` in `data.rs:1230-1363` so the per-element idents
// carry three distinct roles:
//
// * `$D` — type-ident used in trait bounds (`D0: ChunkedQueryData`).
// * `$s` — value-ident bound to the per-element `State` inside
//   `let ($($s,)*) = state` destructures.
// * `$f` — value-ident bound to the per-element `ChunkFetch<'c>` inside
//   `let ($($f,)*) = fetch` destructures.
//
// The tuple impl composes via trait-method calls only — it does NOT reach into
// `ReadChunkFetch::base` / `WriteChunkFetch::base` (those fields are private to
// the leaf impls; encapsulation preserved per Wave 2 code-review).

/// Emits a `ChunkedQueryData` impl for a tuple of the given paired idents (one
/// `(TypeIdent, state_value_ident, fetch_value_ident)` triple per element).
/// Invoked for arity `1..=MAX_QUERY_DATA_ARITY`.
macro_rules! impl_chunked_query_data_tuple {
    ( $( ($D:ident, $s:ident, $f:ident) ),* ) => {
        // SAFETY (CD1-CD4): forwarded per-element; the tuple impl is sound iff
        //   each element's chunked impl is sound. The archetype pointer is
        //   identical for every element (one archetype per `set_chunk_*` call),
        //   and the row range `[start, start + len)` is identical for every
        //   per-element `fetch_chunk` call — guaranteeing same-length slices.
        //   Intra-tuple aliasing among `$D`s is detected at `init_access` via
        //   `FilteredAccessSet` (the `QueryData` supertrait already covers it).
        #[allow(non_snake_case)]
        unsafe impl< $($D: ChunkedQueryData),* > ChunkedQueryData for ( $($D,)* ) {
            type ChunkFetch<'c> = ( $($D::ChunkFetch<'c>,)* );
            type ChunkItem<'c>  = ( $($D::ChunkItem<'c>,)* );

            #[inline]
            fn init_chunk_fetch<'c>(state: &Self::State) -> Self::ChunkFetch<'c> {
                let ( $($s,)* ) = state;
                ( $( <$D as ChunkedQueryData>::init_chunk_fetch($s), )* )
            }

            #[inline]
            unsafe fn set_chunk_readonly<'c>(
                fetch: &mut Self::ChunkFetch<'c>,
                state: &Self::State,
                archetype: *const Archetype,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (CD1, CD4): forwarded per-element; `archetype`
                    //   carries read-only provenance and is identical for
                    //   every element. The caller of the tuple impl upheld
                    //   CD1/CD4 for every `$D`.
                    unsafe { <$D as ChunkedQueryData>::set_chunk_readonly($f, $s, archetype); }
                )*
            }

            #[inline]
            unsafe fn set_chunk_mut<'c>(
                fetch: &mut Self::ChunkFetch<'c>,
                state: &Self::State,
                archetype: *mut Archetype,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (CD1, CD4): write-capable `archetype` is forwarded
                    //   to every element; per-element CD4 enforces wrong-kind
                    //   dispatch prevention. Caller upheld CD1/CD4.
                    unsafe { <$D as ChunkedQueryData>::set_chunk_mut($f, $s, archetype); }
                )*
            }

            #[inline]
            unsafe fn fetch_chunk<'c>(
                fetch: &Self::ChunkFetch<'c>,
                start: usize,
                len: usize,
            ) -> Self::ChunkItem<'c> {
                let ( $($f,)* ) = fetch;
                (
                    $(
                        // SAFETY (CD2, CD3): per-element `fetch_chunk` contract
                        //   held by caller; `start..start+len` is identical
                        //   across elements. Distinct slices into distinct
                        //   columns are non-aliasing by archetype invariant
                        //   (different components → different memory regions);
                        //   intra-tuple `&mut [T]`/`&[T]` over the same column
                        //   is rejected by `FilteredAccessSet::init_access`.
                        unsafe { <$D as ChunkedQueryData>::fetch_chunk($f, start, len) },
                    )*
                )
            }
        }
    };
}

impl_chunked_query_data_tuple!((D0, s0, f0));
impl_chunked_query_data_tuple!((D0, s0, f0), (D1, s1, f1));
impl_chunked_query_data_tuple!((D0, s0, f0), (D1, s1, f1), (D2, s2, f2));
impl_chunked_query_data_tuple!((D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3));
impl_chunked_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4)
);
impl_chunked_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5)
);
impl_chunked_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6)
);
impl_chunked_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7)
);
impl_chunked_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8)
);
impl_chunked_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9)
);
impl_chunked_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10)
);
impl_chunked_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11)
);

// No `impl_chunked_query_data_tuple_too_large!` mirror needed (plan §5.4):
//   tuples of arity > MAX_QUERY_DATA_ARITY (12) fail in `QueryData::init_state`
//   before any `ChunkedQueryData` method is reachable. The supertrait bound
//   `ChunkedQueryData: QueryData` forces `QueryDataState::new` to invoke
//   `<(D0,..,D12) as QueryData>::init_state` (see `data.rs:1493-1501` for the
//   monomorphisation-time `panic!`), which fires before any
//   `ChunkedQueryData::init_chunk_fetch` / `set_chunk_*` call site is reached.

#[cfg(test)]
mod chunked_tuple_tests {
    use super::ChunkedQueryData;
    use crate::ecs::core::component::component::Component;

    fn assert_chunked<D: ChunkedQueryData>() {}

    // Local test components — `Component` impls are compile-only shims
    // (`component_id` panics if ever invoked, but `assert_chunked` is a pure
    // type-system check that never executes the body). Mirrors the
    // `archetypal_marker_tests` pattern in `filter.rs:1753-1809`.
    struct A;
    struct B;
    struct C;
    impl Component for A {
        fn component_id() -> crate::ecs::identifiers::primitives::ComponentId {
            unimplemented!("compile-only test component")
        }
    }
    impl Component for B {
        fn component_id() -> crate::ecs::identifiers::primitives::ComponentId {
            unimplemented!("compile-only test component")
        }
    }
    impl Component for C {
        fn component_id() -> crate::ecs::identifiers::primitives::ComponentId {
            unimplemented!("compile-only test component")
        }
    }

    #[test]
    fn empty_tuple_compiles() {
        assert_chunked::<()>();
    }

    #[test]
    fn arity_one_compiles() {
        assert_chunked::<(&A,)>();
    }

    #[test]
    fn arity_three_mixed_tuple_is_chunked() {
        assert_chunked::<(&A, &mut B, &C)>();
    }

    #[test]
    fn arity_twelve_compiles() {
        struct D;
        struct E;
        struct F;
        struct G;
        struct H;
        struct I;
        struct J;
        struct K;
        struct L;
        macro_rules! impl_c {
            ($($t:ident),*) => { $(
                impl Component for $t {
                    fn component_id() -> crate::ecs::identifiers::primitives::ComponentId {
                        unimplemented!("compile-only test component")
                    }
                }
            )* };
        }
        impl_c!(D, E, F, G, H, I, J, K, L);
        assert_chunked::<(&A, &B, &C, &D, &E, &F, &G, &H, &I, &J, &K, &L)>();
    }
}
