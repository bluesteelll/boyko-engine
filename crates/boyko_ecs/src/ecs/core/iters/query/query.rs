//! `Query<'w, 's, D, F>` — typed component query SystemParam.
//!
//! Phase 8b Step 8 lands the full struct + inherent `iter`/`iter_mut` methods,
//! the [`IntoIterator`] impls for `&Query` / `&mut Query` (C1), and the
//! [`SystemParam`] impl with the C3 two-named-lifetimes binder.
//!
//! # Lifetimes
//!
//! * `'w` — world borrow lifetime (the [`UnsafeEcsCell`] reference scope).
//! * `'s` — per-system state borrow lifetime (the cached [`QueryDataState`]
//!   slot inside the containing system).
//! * `D` — [`QueryData`] (e.g. `&T`, `&mut T`, or a tuple thereof).
//! * `F` — [`QueryFilter`] (defaults to `()`, the no-op filter).
//!
//! See §3 / §14.3 of `docs/PHASE-8B-QUERY-DSL-PLAN.md` for the design and
//! the M7 SAFETY note on the cell-by-value flow through `get_param`.

use std::marker::PhantomData;

use crate::ecs::core::component::component_registry::{EnableTagId, TagId};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::iters::query::chunk_iter;
use crate::ecs::core::iters::query::chunked_data::ChunkedQueryData;
use crate::ecs::core::iters::query::data::{QueryData, ReadOnlyQueryData};
use crate::ecs::core::iters::query::enable_terms::EnableTerms;
use crate::ecs::core::iters::query::filter::{ArchetypalQueryFilter, QueryFilter};
use crate::ecs::core::iters::query::iter::{QueryIter, QueryIterMut};
use crate::ecs::core::iters::query::par_chunk;
use crate::ecs::core::iters::query::par_iter::{BatchingStrategy, ParQuery, ParQueryMut};
use crate::ecs::core::iters::query::state::QueryDataState;
use crate::ecs::core::iters::query::tag_terms::{TagTerms, any_term_matched, count_term_matched};
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::system_param::SystemParam;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
use crate::ecs::identifiers::primitives::ArchetypeId;

/// Typed component query — the canonical Phase 8b iteration handle.
///
/// Produced as a [`SystemParam`] inside a system body. Walking `Query` via
/// [`Self::iter`] (read-only) or [`Self::iter_mut`] (mutable) visits every
/// row of every archetype matched by `D` + `F`.
///
/// The `for x in &q` / `for x in &mut q` sugar is supported via the
/// [`IntoIterator`] impls on shared / exclusive references to `Query`.
///
/// # Field layout
///
/// * `state` — borrow of the per-system [`QueryDataState`], owned by the
///   containing system's state slot. `'s` is the slot's lifetime.
/// * `world` — copy of the world-access cell. By-value pass; not retagged
///   (Phase 8a C1 fix — see [`UnsafeEcsCell`]).
/// * `meta` — borrow of the active system's [`SystemMeta`] for diagnostic
///   hooks (e.g. Phase 9's archetype-refresh callbacks).
/// * `_marker` — `fn() -> (D, F)` keeps `D` / `F` invariant and the marker
///   `Send + Sync` regardless of `D`/`F` bounds.
pub struct Query<'w, 's, D: QueryData, F: QueryFilter = ()> {
    /// Borrow of the per-system state — holds the cached
    /// [`QueryDataState`], `D::State`, `F::State`.
    state: &'s QueryDataState<D, F>,

    /// Copy of the world-access cell. By-value pass; not retagged.
    world: UnsafeEcsCell<'w>,

    /// Per-system tick snapshot + diagnostic handle. Forwarded into the
    /// `QueryIter` / `QueryIterMut` / `ParQuery` / `ParQueryMut`
    /// constructors so non-archetypal filters (Wave C `Added<C>` /
    /// `Changed<C>`) and `Ref<T>` / `Mut<T>` data impls can capture
    /// `last_run` / `this_run` (Phase 10 Round 2 C2).
    meta: &'s SystemMeta,

    /// Phase 22 D4: per-view dynamic-tag terms. Stack-only `Copy` payload —
    /// EMPTY for every `get_param`-minted query; populated by
    /// [`Self::with_tag`] / [`Self::without_tag`]. NEVER written into the
    /// shared per-system `QueryDataState` (QS1 stays term-agnostic).
    terms: TagTerms,

    /// EnableTag Step 9: per-view dynamic **per-row** enable terms (the dynamic
    /// twin of typed `Enabled<T>` / `Disabled<T>`). Stack-only `Copy` payload —
    /// EMPTY for every `get_param`-minted query; populated by
    /// [`Self::with_enabled`] / [`Self::without_enabled`]. NEVER written into
    /// the shared per-system `QueryDataState`. A no-enable-term query takes the
    /// byte-identical cursor fast path (0%-gate).
    enable_terms: EnableTerms,

    /// Invariance over `D` and `F`. `fn() -> (D, F)` keeps the marker
    /// `Send + Sync` regardless of `D`/`F` bounds.
    _marker: PhantomData<fn() -> (D, F)>,
}

impl<'w, 's, D: QueryData, F: QueryFilter> Query<'w, 's, D, F> {
    /// Adds a dynamic-tag presence term (Phase 22 D4): only archetypes
    /// carrying `tag` participate in every driver of this query view
    /// (`iter`/`iter_mut`, `par_iter`/`par_iter_mut`, `for_each_chunk`,
    /// `par_for_each_chunk`, `archetype_count`/`is_empty`).
    ///
    /// Archetype-level filtering only — zero per-row cost; the no-terms fast
    /// path stays byte-identical (one predicted branch per archetype
    /// transition).
    ///
    /// # Panics
    /// Loud release panic past
    /// [`MAX_DYN_TAG_TERMS`](crate::ecs::core::iters::query::MAX_DYN_TAG_TERMS)
    /// combined `with_tag`/`without_tag` terms (setup-time, cold).
    #[must_use]
    #[inline]
    pub fn with_tag(mut self, tag: TagId) -> Self {
        self.terms.push_with(tag);
        self
    }

    /// Adds a dynamic-tag absence term (Phase 22 D4): archetypes carrying
    /// `tag` are excluded. Same cost model and panic contract as
    /// [`Self::with_tag`].
    #[must_use]
    #[inline]
    pub fn without_tag(mut self, tag: TagId) -> Self {
        self.terms.push_without(tag);
        self
    }

    /// Adds a dynamic **per-row** enable term (EnableTag Decision D2 / Step 9):
    /// only rows whose `tag` enable bit is SET participate in every iteration
    /// driver of this query view (`iter`/`iter_mut`, `par_iter`/`par_iter_mut`).
    /// The dynamic twin of the typed [`Enabled<T>`](super::filter_enable::Enabled).
    ///
    /// Per-row filtering: a `tag` whose archetype has no allocated enable column
    /// reads as all-disabled, so no row in it survives a `with_enabled` term —
    /// identical polarity to `Enabled<T>`. The no-enable-term fast path stays
    /// byte-identical (one predicted `is_empty()` branch per row — 0%-gate).
    ///
    /// # Panics
    /// Loud release panic past
    /// [`MAX_ENABLE_TERMS`](crate::ecs::constants::MAX_ENABLE_TERMS) combined
    /// `with_enabled`/`without_enabled` terms (setup-time, cold — C2 dynamic
    /// enforcement).
    #[must_use]
    #[inline]
    pub fn with_enabled(mut self, tag: EnableTagId) -> Self {
        self.enable_terms.push_with(tag);
        self
    }

    /// Adds a dynamic **per-row** enable term (EnableTag Decision D2 / Step 9):
    /// only rows whose `tag` enable bit is CLEAR participate. The dynamic twin
    /// of the typed [`Disabled<T>`](super::filter_enable::Disabled). Same
    /// per-row cost model and panic contract as [`Self::with_enabled`].
    ///
    /// A `tag` whose archetype has no allocated enable column reads as
    /// all-disabled, so every row in it survives a `without_enabled` term
    /// (clear bit ⇒ matches — identical polarity to `Disabled<T>`).
    #[must_use]
    #[inline]
    pub fn without_enabled(mut self, tag: EnableTagId) -> Self {
        self.enable_terms.push_without(tag);
        self
    }

    /// Returns the number of currently-matched archetypes.
    ///
    /// No-terms path: O(1) — reads the length of the cached `matched_ids`
    /// slice. With dynamic-tag terms: O(matched) signature-filtered walk
    /// (archetype-level membership only — `entity_count` is never consulted,
    /// matching the no-terms semantics; stale removed ids do not count on
    /// the term path, since the term test needs the live signature).
    #[inline]
    pub fn archetype_count(&self) -> usize {
        if self.terms.is_empty() {
            // EnableTag Decision 6: route the no-terms arm through the cull set
            // so count stays consistent with `iter` (const-folds to
            // `matched_ids_pre_terms()` for non-enable `F`).
            return self.state.enable_driver_ids().len();
        }
        let ids = self.state.archetype_state.matched_ids_pre_terms();
        // SAFETY (U_C2): shared read mint — `world()` yields `&EcsMaster`
        //   scoped to this statement; no `&mut` access occurs through it
        //   (same pattern as `get_param`'s master reborrow).
        let master = unsafe { self.world.world().archetype_master() };
        count_term_matched(&self.terms, master, ids)
    }

    /// Returns `true` if no archetypes are currently matched.
    ///
    /// Note that an archetype-count of zero does not imply a zero-row
    /// iteration (a matched archetype with no live entities still counts);
    /// conversely, `is_empty() == false` does not guarantee a non-zero
    /// iteration length. Use the iterator for an exact row count.
    ///
    /// Term semantics mirror [`Self::archetype_count`].
    #[inline]
    pub fn is_empty(&self) -> bool {
        if self.terms.is_empty() {
            // EnableTag Decision 6: consistent with `iter` / `archetype_count`.
            return self.state.enable_driver_ids().is_empty();
        }
        let ids = self.state.archetype_state.matched_ids_pre_terms();
        // SAFETY (U_C2): shared read mint scoped to this statement — see
        //   `archetype_count`.
        let master = unsafe { self.world.world().archetype_master() };
        !any_term_matched(&self.terms, master, ids)
    }

    /// Resolves the id slice every iteration-style driver walks (Phase 22.1
    /// Area A, D-C funnel).
    ///
    /// No terms: returns the shared `matched_ids_pre_terms()` slice exactly
    /// as pre-Phase-22 — one predicted branch, no master mint, the term
    /// scratch is never loaded. Terms: routes to the cold
    /// [`Self::driver_ids_term_slow`] which mints the master and resolves the
    /// per-epoch memoised filtered slice. Both arms return a `&'s` slice (the
    /// pre-terms slice lives in the cached `QueryState`; the filtered slice in
    /// the published `TermList` inside the `'s` state slot), decoupled from
    /// `&self` so `iter_mut`'s `&mut self` reborrow does not conflict.
    #[inline]
    fn driver_ids(&self) -> &'s [ArchetypeId] {
        if self.terms.is_empty() {
            // EnableTag Decision 3: the no-terms fast path. For a positive-term
            // enable `F` this is `culled_ids`; for non-enable `F` it const-folds
            // to `matched_ids_pre_terms()` (the 0%-gate — identical load).
            return self.state.enable_driver_ids();
        }
        self.driver_ids_term_slow()
    }

    /// Cold term-bearing arm of [`Self::driver_ids`] — mints the master and
    /// resolves the memoised term-filtered slice (P1–P4).
    #[cold]
    #[inline(never)]
    fn driver_ids_term_slow(&self) -> &'s [ArchetypeId] {
        // SAFETY (U_C2): shared read mint — `world()` yields `&EcsMaster`
        //   scoped to this statement; no `&mut` access occurs through it
        //   (same pattern as `get_param`'s master reborrow). The borrow ends
        //   at the `archetype_master()` deref; the resolved slice borrows the
        //   `'s` state's term scratch, not the cell.
        let master = unsafe { self.world.world().archetype_master() };
        self.state.term_scratch.resolve_term_filtered(
            &self.terms,
            master,
            &self.state.archetype_state,
        )
    }

    /// Returns a read-only iterator over `D::Item<'_>` for every entity in
    /// every matched archetype.
    ///
    /// `D` must be **read-only** at the type level — see
    /// [`QueryData::IS_READ_ONLY`]. For mutable iteration use
    /// [`Self::iter_mut`].
    pub fn iter(&self) -> QueryIter<'_, 's, D, F>
    where
        D: ReadOnlyQueryData,
    {
        // SAFETY (Q1, QD4): `D: ReadOnlyQueryData` ⇒ no `&mut T` in `D`; the
        //   QueryIter constructor will call `cell.archetype_ptr(_)` (read-only
        //   mint) and `D::set_table_readonly(_: *const Archetype)` only.
        //   The cell `self.world` is `Copy`; passing it by value preserves
        //   the raw-pointer provenance through the call (Phase 8a C1).
        //   Phase 10 Round 2 C2: `self.meta` is forwarded so non-archetypal
        //   filters and `Ref<T>` / `Mut<T>` impls can read the tick snapshot.
        //   Phase 22.1 Area A: the id slice is resolved ONCE here (no terms →
        //   pre-terms slice; terms → memoised filtered slice); the cursor
        //   walks it term-free.
        let ids = self.driver_ids();
        unsafe { QueryIter::new(self.state, ids, self.world, self.meta, self.enable_terms) }
    }

    /// Returns a mutable iterator over `D::Item<'_>` for every entity in
    /// every matched archetype.
    ///
    /// `iter_mut` is the only iter method that works for `D` containing
    /// `&mut T`. The `&mut self` borrow guarantees no other live cursor
    /// exists (Q3).
    ///
    /// # Q1 enforcement
    ///
    /// No field-level debug-assert is needed here. Q1 is upheld by:
    /// * The type system — [`Self::iter`] is gated by
    ///   `D: ReadOnlyQueryData`; if the user calls `iter()` on a `D`
    ///   containing `&mut T`, the bound fails to resolve.
    /// * Phase 8a's existing [`UnsafeEcsCell::archetype_ptr_mut`] carries a
    ///   `debug_assert!(self.allows_mutable_access)` inside the cell. Any
    ///   path that calls `archetype_ptr_mut` on a read-only cell trips
    ///   that debug-assert at the cell level.
    pub fn iter_mut(&mut self) -> QueryIterMut<'_, 's, D, F> {
        // SAFETY (Q1, Q3, QD4): `&mut self` enforces cursor uniqueness;
        //   `QueryIterMut::new` will call `cell.archetype_ptr_mut(_)` per
        //   archetype boundary. If `world` carries a read-only mint and `D`
        //   were not gated, the cell's own debug-assert fires. Phase 10
        //   Round 2 C2: `self.meta` is forwarded. Phase 22.1 Area A: the id
        //   slice (term-free for the cursor) is resolved once here; it lives
        //   at `'s` so it does not conflict with the `&mut self` reborrow.
        let ids = self.driver_ids();
        unsafe { QueryIterMut::new(self.state, ids, self.world, self.meta, self.enable_terms) }
    }

    /// Returns a parallel read-only iteration handle.
    ///
    /// Use [`ParQuery::for_each`] to run a closure on every matched row,
    /// fanning the work across the current [`ThreadPool`]'s workers via
    /// [`ThreadPool::scope`]. Archetypes with fewer than
    /// [`MIN_ARCHETYPE_FOR_PARALLEL`] rows process inline on the calling
    /// thread (plan PAR9 / Round 2 O2).
    ///
    /// When no pool is attached to the calling thread, `for_each`
    /// degrades to a sequential walk on the calling thread (PAR7).
    ///
    /// `D` must be [`ReadOnlyQueryData`] — `&mut T` queries must use
    /// [`Self::par_iter_mut`].
    ///
    /// [`ThreadPool`]: boyko_threadpool::ThreadPool
    /// [`ThreadPool::scope`]: boyko_threadpool::ThreadPool::scope
    /// [`MIN_ARCHETYPE_FOR_PARALLEL`]: crate::ecs::core::iters::query::par_iter::MIN_ARCHETYPE_FOR_PARALLEL
    #[inline]
    pub fn par_iter<'q>(&'q self) -> ParQuery<'q, 's, D, F>
    where
        D: ReadOnlyQueryData,
    {
        // Phase 22.1 Area A: resolve the id slice once at handle mint (one
        // load per for_each entry per D-B's cost model); `for_each` forwards
        // it to both the scoped loop and the PAR7 fallback unchanged.
        let ids = self.driver_ids();
        ParQuery {
            state: self.state,
            ids,
            world: self.world,
            batching: BatchingStrategy::default(),
            meta: self.meta,
            enable_terms: self.enable_terms,
        }
    }

    /// Returns a parallel mutable iteration handle.
    ///
    /// Same semantics as [`Self::par_iter`] but accepts any `D: QueryData`
    /// (including `&mut T`). The `&mut self` borrow gates cursor
    /// uniqueness; concurrent chunks within one `for_each` call write to
    /// disjoint row ranges by construction (PAR2).
    #[inline]
    pub fn par_iter_mut<'q>(&'q mut self) -> ParQueryMut<'q, 's, D, F> {
        // Phase 22.1 Area A: resolve once before handing out the mutable
        // handle. The slice lives at `'s` so it does not conflict with the
        // `&mut self` reborrow gate.
        let ids = self.driver_ids();
        ParQueryMut {
            state: self.state,
            ids,
            world: self.world,
            batching: BatchingStrategy::default(),
            meta: self.meta,
            enable_terms: self.enable_terms,
            _mut_marker: PhantomData,
        }
    }

    /// Invoke `f` once per matched archetype, passing a slice (or tuple of
    /// slices) covering every row in that archetype.
    ///
    /// `D` must satisfy [`ChunkedQueryData`]; `F` must satisfy
    /// [`ArchetypalQueryFilter`]. Both bounds are compile-time —
    /// `Query<&T, Changed<U>>::for_each_chunk` is a type error,
    /// redirecting the user to [`Self::iter`] for per-row tick filtering.
    ///
    /// # Performance
    ///
    /// Outer-loop overhead per archetype: target ≤ 15 ns + 1 ns/row
    /// user-closure body (plan §1.2). Allocations: 0 in steady state.
    /// Empty matched archetypes are skipped at the `entity_count == 0`
    /// guard; stale-id entries (Q5) are skipped transparently via the
    /// driver's `archetype_ptr_mut` `None` arm.
    ///
    /// # See also
    ///
    /// * [`Self::par_for_each_chunk`] — parallel variant (Phase X.A Wave 6).
    ///
    /// [`ChunkedQueryData`]: super::chunked_data::ChunkedQueryData
    /// [`ArchetypalQueryFilter`]: super::filter::ArchetypalQueryFilter
    #[inline]
    pub fn for_each_chunk<Func>(&mut self, f: Func)
    where
        D: ChunkedQueryData,
        F: ArchetypalQueryFilter,
        Func: for<'c> FnMut(D::ChunkItem<'c>),
    {
        // SAFETY (Q1, Q3, CD1-CD4): `&mut self` enforces cursor
        //   uniqueness (Q3); `D::IS_READ_ONLY` selects the readonly /
        //   mut chunk-dispatch arm inside the driver (mirrors the
        //   `iter` / `iter_mut` split as a runtime flag — no separate
        //   driver is needed because `D: ChunkedQueryData` excludes
        //   `Ref` / `Mut`, forcing NCD const-fold to `false` at this
        //   monomorphisation, eliminating the meta-bearing branch from
        //   `iter.rs`). The cell `self.world` is `Copy`; passing by
        //   value preserves the raw-pointer provenance through the call
        //   (Phase 8a C1 fix).
        // Phase 22.1 Area A: resolve the id slice once before dispatch; the
        // driver walks it term-free.
        let ids = self.driver_ids();
        let mutable = !D::IS_READ_ONLY;
        unsafe {
            chunk_iter::for_each_chunk_impl(self.state, ids, self.world, mutable, f);
        }
    }

    /// Parallel mirror of [`Self::for_each_chunk`].
    ///
    /// Splits each matched archetype's row range into sub-ranges per
    /// [`BatchingStrategy`] and dispatches each sub-range to a
    /// [`boyko_threadpool::ThreadPool`] worker via
    /// [`boyko_threadpool::ThreadPool::scope`]. Archetypes with fewer than
    /// [`MIN_ARCHETYPE_FOR_PARALLEL`][min] rows run inline on the calling
    /// thread (plan PAR9). When no pool is attached to the calling thread,
    /// dispatch falls back to a sequential walk on the calling thread (PAR7).
    ///
    /// # Closure invocation frequency (IMPORTANT — differs from sequential)
    ///
    /// The closure is invoked **once per archetype sub-range, not once per
    /// archetype** — see plan §2.4 / §1.2. Examples:
    ///
    /// * 100k-row archetype on an 8-worker pool with default
    ///   `batches_per_thread = 1` → `batch_size = 12500`, 8 invocations.
    /// * 4096-row archetype on the same pool → raw `4096 / 8 = 512`
    ///   clamps to 1024, 4 invocations.
    ///
    /// For reductions, use a thread-safe accumulator
    /// (`[AtomicU64; worker_count]`, sharded thread-local, etc.) instead of
    /// the `FnMut(&mut acc, &[T])` capture pattern. Sequential
    /// [`Self::for_each_chunk`] remains the right shape for single-threaded
    /// fold reductions.
    ///
    /// # Compile-time bounds
    ///
    /// `D` must satisfy [`ChunkedQueryData`]; `F` must satisfy
    /// [`ArchetypalQueryFilter`]; `Func` must be `Fn + Send + Sync`.
    /// `Query<&T, Changed<U>>::par_for_each_chunk` is a type error
    /// (`Changed` is not archetypal). Use [`Self::iter`] / [`Self::iter_mut`]
    /// for per-row tick filtering.
    ///
    /// # See also
    ///
    /// * [`Self::for_each_chunk`] — sequential variant.
    /// * Plan §9 for the granularity rationale and per-worker math.
    ///
    /// [`ChunkedQueryData`]: super::chunked_data::ChunkedQueryData
    /// [`ArchetypalQueryFilter`]: super::filter::ArchetypalQueryFilter
    /// [`BatchingStrategy`]: super::par_iter::BatchingStrategy
    /// [min]: super::par_iter::MIN_ARCHETYPE_FOR_PARALLEL
    #[inline]
    pub fn par_for_each_chunk<Func>(&mut self, f: Func, batching: BatchingStrategy)
    where
        D: ChunkedQueryData,
        F: ArchetypalQueryFilter,
        Func: for<'c> Fn(D::ChunkItem<'c>) + Send + Sync,
    {
        // SAFETY (Q1, Q3, CD1-CD4, §9): `&mut self` enforces cursor
        //   uniqueness (Q3); `D::IS_READ_ONLY` selects the readonly / mut
        //   chunk-dispatch arm inside the driver. `D: ChunkedQueryData`
        //   excludes `Ref<T>` / `Mut<T>` and `F: ArchetypalQueryFilter`
        //   excludes `Added<C>` / `Changed<C>`, so `NEEDS_CHANGE_DETECTION`
        //   const-folds to `false` and the meta-bearing branch from
        //   `par_iter.rs` does not appear in this driver. The cell
        //   `self.world` is `Copy`; passing by value preserves the
        //   raw-pointer provenance through the call (Phase 8a C1 fix).
        // Phase 22.1 Area A: resolve the id slice once on the calling thread
        // before `pool.scope`; workers receive only term-passing archetypes'
        // chunks.
        let ids = self.driver_ids();
        let mutable = !D::IS_READ_ONLY;
        unsafe {
            par_chunk::par_for_each_chunk_impl(
                self.state,
                ids,
                self.world,
                mutable,
                batching,
                f,
            );
        }
    }
}

// ── IntoIterator impls (C1) ─────────────────────────────────────────────────

/// [`IntoIterator`] for a shared reference to a `Query` — desugars
/// `for x in &q` into `(&q).into_iter()`. Gated by `D: ReadOnlyQueryData`
/// so that `&q` over a `Query<&mut T, _>` is a type error (forcing the
/// user to `&mut q`).
impl<'a, 'w, 's, D, F> IntoIterator for &'a Query<'w, 's, D, F>
where
    D: ReadOnlyQueryData,
    F: QueryFilter,
{
    type Item = D::Item<'a>;
    type IntoIter = QueryIter<'a, 's, D, F>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        // Delegates to the inherent `iter` method. The lifetime narrows to
        // `'a` (the shared reborrow scope), strictly inside `'w` (the world
        // access scope). `D::Item<'a>` is a sub-lifetime of `D::Item<'w>`.
        self.iter()
    }
}

/// [`IntoIterator`] for an exclusive reference to a `Query` — desugars
/// `for x in &mut q` into `(&mut q).into_iter()`. Accepts any `D`/`F`
/// because the `&mut self` borrow already enforces cursor uniqueness (Q3).
impl<'a, 'w, 's, D, F> IntoIterator for &'a mut Query<'w, 's, D, F>
where
    D: QueryData,
    F: QueryFilter,
{
    type Item = D::Item<'a>;
    type IntoIter = QueryIterMut<'a, 's, D, F>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

// ── SystemParam impl (§14.3, C3 fixed binder) ───────────────────────────────

// SAFETY (SP1, SP2, SP4): the impl is parameterised over the outer pair of
//   lifetimes `'a` / `'b`, matching the C3 RESOLUTION: a single binder
//   declares both, where Round 1 used `'_` in impl-head position (malformed).
//   The generic blanket satisfies the trait's
//   `Item<'w, 's>: SystemParam<State = Self::State>` bound for all `'w` / `'s`.
//   - SP1: `init_access` delegates to `QueryDataState::init_access`, which
//     forwards to `D::init_access` and `F::init_access`. The cumulative set
//     of declared reads/writes covers every component touched by
//     `D::fetch(row)`.
//   - SP2: `get_param` returns a `Query` bound to the cell's `'w`. The
//     caller asserts no aliasing access through any sibling cell copy.
//   - SP4: `init_state` calls `QueryDataState::new`, which is a pure read
//     against the archetype master (no archetype/resource registrations).
//     Debug-asserted by `FunctionSystem::initialize` via the
//     `archetype_generation()` comparison.
unsafe impl<'a, 'b, D, F> SystemParam for Query<'a, 'b, D, F>
where
    D: QueryData + 'static,
    F: QueryFilter + 'static,
{
    type State = QueryDataState<D, F>;
    type Item<'w, 's> = Query<'w, 's, D, F>;

    fn init_state(world: &mut EcsMaster, _system_meta: &mut SystemMeta) -> Self::State {
        // M7 SAFETY: the `&ArchetypeMaster` borrow inside `QueryDataState::new`
        //   is taken-and-released entirely inside the `new` call; on return,
        //   only `Self::State` is held. No aliasing concern with the cell
        //   that `get_param` will later mint.
        QueryDataState::<D, F>::new(world)
    }

    fn init_access(
        state: &Self::State,
        _system_meta: &mut SystemMeta,
        access_set: &mut FilteredAccessSet,
        _world: &mut EcsMaster,
    ) {
        state.init_access(access_set);
    }

    #[inline]
    unsafe fn get_param<'w, 's>(
        state: &'s mut Self::State,
        system_meta: &SystemMeta,
        world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's> {
        // M7 SAFETY: the `master` binding below is a SHARED borrow
        //   `&'tmp ArchetypeMaster` whose lifetime `'tmp` is contained
        //   strictly within the `state.update(master)` statement. The borrow
        //   is dropped at the semicolon before the `Query { ... }` literal
        //   is constructed. No aliasing with the by-value `world` cell
        //   passed below: the cell is a `Copy<'w>` chain that does NOT
        //   retain any reborrow of `master`. The `Query` holds `world` (by
        //   value), `meta` (a `&SystemMeta` that came from the function
        //   argument), and `state` (a `&'s mut QueryDataState`); the
        //   `master` reborrow is freed first. No cross-borrow conflict.
        //
        // SAFETY (U_C2): `world.world()` returns `&'w EcsMaster` — shared
        //   read access. `archetype_master()` returns `&'w ArchetypeMaster`.
        //   `state.update(master)` consumes the borrow before the `Query`
        //   literal runs. The by-value `world` receiver preserves the raw
        //   pointer's provenance (no `&self` retag on `Copy` cell).
        let master = unsafe { world.world().archetype_master() };

        // The `system_meta: &SystemMeta` parameter carries an anonymous
        // lifetime that the compiler does NOT unify with `'s` (the trait
        // signature does not name the meta's lifetime). The Phase 8b
        // `Query` struct (per §3.1 of the plan) stores its meta at `'s` —
        // the per-system state slot's lifetime — because the meta and the
        // state are both owned by the same long-lived system struct. The
        // explicit reborrow below upgrades the anonymous lifetime to `'s`.
        //
        // SAFETY: The `SystemParam` protocol guarantees that when
        //   `get_param` is invoked, the caller's `SystemMeta` slot lives at
        //   least as long as the `State` slot (`&'s mut Self::State` arg).
        //   Both slots are members of the same system struct (Phase 8c
        //   `FunctionSystem`); the meta slot is
        //   held by the same `&mut SystemBox` that minted `state: &'s mut
        //   ...`. The reborrow is therefore sound: the upgraded lifetime
        //   `'s` does not outlive the actual borrow scope of the
        //   `SystemMeta` slot. The pointer round-trip preserves Rust's
        //   borrow stack (we do not produce a separate `&mut SystemMeta`
        //   alias).
        let meta_s: &'s SystemMeta = unsafe { &*(system_meta as *const SystemMeta) };

        // Phase 22.1 Area A (P2): this is the slot-exclusive mint funnel
        // (`&'s mut Self::State`). Free the retired term list (if any) here,
        // where no resolve on this slot can be in flight — fast path is one
        // Relaxed null-load + a predicted-not-taken branch (off the row loop;
        // covered by the 50-systems schedule bench gate).
        state.term_scratch.reclaim_retired();

        state.update(master);

        Query {
            state,
            world,
            meta: meta_s,
            // Phase 22 D4: a freshly minted SystemParam query carries no
            // dynamic-tag terms; `with_tag` / `without_tag` populate them.
            terms: TagTerms::EMPTY,
            // EnableTag Step 9: no dynamic enable terms until `with_enabled` /
            // `without_enabled` populate them.
            enable_terms: EnableTerms::EMPTY,
            _marker: PhantomData,
        }
    }

    #[inline]
    fn new_archetype(
        _state: &mut Self::State,
        _system_meta: &mut SystemMeta,
        _archetype: &crate::ecs::core::archetype::archetype::Archetype,
    ) {
        // Phase 8b: defer to the next `iter()`'s `state.update(master)` to
        // refresh the cache. The hook exists for Phase 9's scheduler, which
        // will use it to avoid redundant `update_archetypes` work when the
        // scheduler already knows about the new archetype.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::component::component_registry;
    use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

    // Component ids reserved for the Step 8 query-level tests. The free
    // range below was verified at write time against the existing
    // crate-wide allocations:
    //   * 400-417 — archetype.rs
    //   * 200-203 — legacy_query.rs
    //   * 480-482 — archetype_bundle miri tests
    //   * 483-485 — iter.rs (Step 7)
    //   * 490-493 — query_state.rs
    //   * 495-497 — component_set.rs
    //   * 503-504 — query/data.rs
    //   * 506-509 — query/state.rs
    //   * 510      — resource_registry CompThenRes
    // MAX_COMPONENTS = 512 caps valid ids at 511; 486-488 is the
    // orchestrator-suggested free triplet.
    const COMP_A: ComponentId = ComponentId(486);
    const COMP_B: ComponentId = ComponentId(487);
    const COMP_C: ComponentId = ComponentId(488);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CompA(u32);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CompB(u32);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CompC(u32);

    impl Component for CompA {
        fn component_id() -> ComponentId {
            COMP_A
        }
    }
    impl Component for CompB {
        fn component_id() -> ComponentId {
            COMP_B
        }
    }
    impl Component for CompC {
        fn component_id() -> ComponentId {
            COMP_C
        }
    }

    /// Idempotent registry priming.
    fn register_test_components() {
        component_registry::register_layout::<CompA>(COMP_A.0);
        component_registry::register_layout::<CompB>(COMP_B.0);
        component_registry::register_layout::<CompC>(COMP_C.0);
    }

    /// Spawn a single `CompA(value)` entity into `arch_id`.
    fn spawn_a(ecs: &mut EcsMaster, arch_id: ArchetypeId, value: u32) {
        let comp = CompA(value);
        // SAFETY: `CompA` is `#[repr(C)]` POD; reading its bytes produces a
        //   valid byte slice for the duration of this call.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                &comp as *const CompA as *const u8,
                std::mem::size_of::<CompA>(),
            )
        };
        ecs.create_entity(arch_id, &[(COMP_A, bytes)])
            .expect("spawn_a: create_entity must succeed");
    }

    // ── Compile-only tests (C1, C3) ─────────────────────────────────────────

    /// Compile-only shim: instantiating this proves `T: SystemParam`.
    fn assert_impl<T: SystemParam>() {}

    /// Compile-only: `Query<'_, '_, &CompA>` satisfies the generic
    /// SystemParam blanket (C3 — verifies the two-named-lifetime binder).
    #[test]
    fn query_systemparam_impl() {
        assert_impl::<Query<'static, 'static, &CompA>>();
    }

    /// Compile-only: `for x in &q` over a read-only `Query` resolves via the
    /// `IntoIterator for &Query` impl (C1).
    #[allow(dead_code, reason = "Compile-only check; not called at runtime.")]
    fn _check_into_iter_ref(q: &Query<'_, '_, &CompA>) {
        for _ in q {}
    }

    /// Compile-only: `for x in &mut q` over a mutable `Query` resolves via
    /// the `IntoIterator for &mut Query` impl (C1).
    #[allow(dead_code, reason = "Compile-only check; not called at runtime.")]
    fn _check_into_iter_mut(q: &mut Query<'_, '_, &mut CompA>) {
        for _ in q {}
    }

    // ── Runtime tests ───────────────────────────────────────────────────────

    /// Constructing a `Query` by hand and reading `archetype_count` returns
    /// the matched-ids length cached in the underlying state.
    #[test]
    fn archetype_count_reflects_matched() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        // Three archetypes — two contain CompA, one does not.
        let arch_a = ecs.create_archetype(&[COMP_A]);
        let arch_ab = ecs.create_archetype(&[COMP_A, COMP_B]);
        let _arch_c = ecs.create_archetype(&[COMP_C]);

        let state = QueryDataState::<&CompA, ()>::new(&mut ecs);
        let meta = SystemMeta::for_testing("test");

        // SAFETY (U_C1): `cell` is consumed below within this scope; it
        //   does not escape the `&mut ecs` borrow.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };
        let q = Query::<&CompA, ()> {
            state: &state,
            world: cell,
            meta: &meta,
            terms: TagTerms::EMPTY,
            enable_terms: EnableTerms::EMPTY,
            _marker: PhantomData,
        };

        assert_eq!(q.archetype_count(), 2, "both CompA archetypes must be matched");
        assert!(!q.is_empty(), "two archetypes matched ⇒ not empty");
        // Sanity: matched_ids contains exactly the two CompA archetypes.
        let ids = state.archetype_state.matched_ids_pre_terms();
        assert!(ids.contains(&arch_a), "arch_a must be in matched_ids");
        assert!(ids.contains(&arch_ab), "arch_ab must be in matched_ids");
    }

    /// End-to-end smoke through the full system pipeline: spawn entities
    /// into two CompA archetypes and use `run_closure_once` to read back
    /// the archetype count via a Query SystemParam.
    #[test]
    fn iter_yields_components_via_run_closure_once() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let arch_a = ecs.create_archetype(&[COMP_A]);
        let arch_ab = ecs.create_archetype(&[COMP_A, COMP_B]);
        spawn_a(&mut ecs, arch_a, 10);
        spawn_a(&mut ecs, arch_a, 20);
        // Spawn a CompA-only row into the (CompA, CompB) archetype using
        // raw byte slices for both components.
        {
            let ca = CompA(30);
            let cb = CompB(0);
            // SAFETY: `#[repr(C)]` PODs; the byte slices are valid for the
            //   `create_entity` call's duration.
            let a_bytes = unsafe {
                std::slice::from_raw_parts(
                    &ca as *const CompA as *const u8,
                    std::mem::size_of::<CompA>(),
                )
            };
            let b_bytes = unsafe {
                std::slice::from_raw_parts(
                    &cb as *const CompB as *const u8,
                    std::mem::size_of::<CompB>(),
                )
            };
            ecs.create_entity(arch_ab, &[(COMP_A, a_bytes), (COMP_B, b_bytes)])
                .expect("create_entity must succeed");
        }

        // Driver: build a Query<&CompA>, count archetypes, and sum
        // component values. Both are returned through the closure output.
        let (arch_n, value_sum) = ecs
            .run_closure_once(|q: Query<'_, '_, &CompA>| {
                let mut sum = 0u32;
                for a in &q {
                    sum += a.0;
                }
                (q.archetype_count(), sum)
            });

        assert_eq!(arch_n, 2, "Query<&CompA> must match both CompA archetypes");
        assert_eq!(value_sum, 60, "iter must yield 10 + 20 + 30 = 60");
    }
}
