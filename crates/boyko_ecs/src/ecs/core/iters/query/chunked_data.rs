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

use super::data::QueryData;
use crate::ecs::core::archetype::archetype::Archetype;

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
