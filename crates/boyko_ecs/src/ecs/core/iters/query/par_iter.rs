//! Phase 9 Wave 6 — intra-system parallel query iteration.
//!
//! [`Query::par_iter`] / [`Query::par_iter_mut`] fan out the iteration across
//! the current `ThreadPool`'s workers via [`ThreadPool::scope`]. Within each
//! matched archetype the row range `[0..entity_count)` is split into chunks
//! by [`BatchingStrategy`]; chunks below [`MIN_ARCHETYPE_FOR_PARALLEL`] run
//! inline on the calling thread to avoid the fork/join overhead dominating
//! tiny archetypes (plan PAR9 / Round 2 O2).
//!
//! # Soundness rails
//!
//! - **PAR1**: the closure passed to [`ParQuery::for_each`] /
//!   [`ParQueryMut::for_each`] is `Fn(D::Item<'_>) + Send + Sync`. The `Fn`
//!   (not `FnMut`) bound is the canonical race-free shape — multiple workers
//!   may invoke it simultaneously, on disjoint rows. The `Send + Sync` bound
//!   is load-bearing for the compile-time rejection of `&mut Commands`
//!   capture (`Commands<'s>: !Sync` per CQ-SEND2; see Wave 7 trybuild test
//!   `tests/par_iter_captures_commands_fails.rs`).
//! - **PAR2**: each chunk operates on a disjoint row range. The monotonic
//!   `for chunk in 0..n_chunks { start..end }` walk guarantees disjointness
//!   without explicit synchronisation between workers.
//! - **PAR3**: `D::Item<'_>` is required to be `Send` (encoded indirectly
//!   through the `Send + Sync` bound on the user closure — Rust's auto-trait
//!   inference of the closure captures forces the per-row item to be Send).
//! - **PAR6**: `par_iter` is NOT nested. The Wave 1 scope API (`pool.scope`)
//!   handles re-entry from inside a worker via its work-stealing
//!   `Scope::Drop` (plan §4.5.5).
//! - **PAR7**: when no active pool is attached to the calling thread,
//!   `for_each` falls back to a sequential walk via [`Query::iter`] /
//!   [`Query::iter_mut`]. This matches Bevy's "host-thread fallback" — the
//!   caller is responsible for setting up an `install` frame if parallelism
//!   is required.
//! - **PAR8**: `par_iter` requires `D: ReadOnlyQueryData`; `par_iter_mut`
//!   accepts any `D: QueryData`. The bound replicates the [`Query::iter`] /
//!   [`Query::iter_mut`] split.
//! - **PAR9**: archetypes with fewer than [`MIN_ARCHETYPE_FOR_PARALLEL`]
//!   rows run inline on the calling thread.
//!
//! # SAFETY of per-chunk dispatch (S1 / SEND1 / SEND3)
//!
//! Each spawned chunk closure captures `UnsafeEcsCell<'q>` (Copy + Send +
//! Sync per SEND3), a `*const D::State` and `*const F::State`, and a
//! `*mut Archetype` (the archetype pointer minted from the cell). The cell's
//! lifetime `'q` is bounded by the surrounding `pool.scope` Drop wait: the
//! scope blocks until every chunk completes, so no worker outlives the
//! borrow that produced the cell. The archetype pointer is slab-stable per
//! Phase 7 U1/U2 — pointers minted from `archetype_ptr(_mut)` stay valid
//! for the entire `'q` scope.
//!
//! [`ThreadPool::scope`]: boyko_threadpool::ThreadPool::scope
//! [`Query::par_iter`]: crate::ecs::core::iters::query::Query::par_iter
//! [`Query::par_iter_mut`]: crate::ecs::core::iters::query::Query::par_iter_mut

use std::marker::PhantomData;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::iters::query::data::{QueryData, ReadOnlyQueryData};
use crate::ecs::core::iters::query::filter::QueryFilter;
use crate::ecs::core::iters::query::state::QueryDataState;
use crate::ecs::core::iters::query::tag_terms::{TagTerms, archetype_passes_tag_terms};
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

/// Row count below which an archetype is processed inline on the calling
/// thread rather than dispatched to workers (plan PAR9 / Round 2 O2).
///
/// Rationale: a single `scope.spawn` costs ~120 ns (plan §10.3). A
/// hypothetical chunk of 10 rows that takes ~5 ns/row to process would
/// pay 240% overhead in dispatch alone. 1024 rows × 5 ns ≈ 5 µs of work,
/// against ~120 ns dispatch = under 3% overhead. The cut-off is a
/// pragmatic engineering choice; benches in Wave 7 will refine if needed.
pub const MIN_ARCHETYPE_FOR_PARALLEL: usize = 1024;

/// How [`ParQuery::for_each`] / [`ParQueryMut::for_each`] split each
/// archetype into worker-bound chunks.
///
/// The default strategy is "one chunk per worker, at least
/// [`MIN_ARCHETYPE_FOR_PARALLEL`] rows per chunk, no upper cap". Tuning
/// knobs follow Bevy's `BatchingStrategy` shape so the migration path is
/// familiar to ECS users.
#[derive(Clone, Copy, Debug)]
pub struct BatchingStrategy {
    /// Number of chunks per worker thread.
    ///
    /// Defaults to `1` — each archetype splits into `worker_count × 1`
    /// chunks. Increasing this value yields finer-grained work-stealing
    /// at the cost of more per-chunk dispatch overhead.
    pub batches_per_thread: usize,

    /// Minimum chunk size. Chunks smaller than this run inline on the
    /// calling thread (plan PAR9). Defaults to
    /// [`MIN_ARCHETYPE_FOR_PARALLEL`].
    pub min_batch_size: usize,

    /// Maximum chunk size. Defaults to [`usize::MAX`] (no cap).
    pub max_batch_size: usize,
}

impl Default for BatchingStrategy {
    #[inline]
    fn default() -> Self {
        Self {
            batches_per_thread: 1,
            min_batch_size: MIN_ARCHETYPE_FOR_PARALLEL,
            max_batch_size: usize::MAX,
        }
    }
}

impl BatchingStrategy {
    /// Compute the chunk size for an archetype with `entity_count` rows
    /// fanned across `worker_count` workers.
    ///
    /// Plan §6.3 heuristic: `entity_count / (worker_count × batches_per_thread)`,
    /// clamped to `[min_batch_size, max_batch_size]`. Returns at least 1
    /// even after the clamp so the dispatch loop always makes progress.
    #[inline]
    pub(crate) fn chunk_size(&self, entity_count: usize, worker_count: usize) -> usize {
        let divisor = worker_count
            .saturating_mul(self.batches_per_thread)
            .max(1);
        let raw = entity_count / divisor;
        raw.clamp(self.min_batch_size, self.max_batch_size).max(1)
    }
}

// ── ParQuery (read-only) ────────────────────────────────────────────────────

/// Parallel iteration handle for read-only [`Query`] access. Returned by
/// [`Query::par_iter`].
///
/// Consume via [`Self::for_each`]; the call blocks until every chunk has
/// run (plan §4.5.5 work-stealing Drop).
///
/// [`Query`]: crate::ecs::core::iters::query::Query
/// [`Query::par_iter`]: crate::ecs::core::iters::query::Query::par_iter
pub struct ParQuery<'q, 's, D: QueryData, F: QueryFilter> {
    pub(super) state: &'s QueryDataState<D, F>,
    pub(super) world: UnsafeEcsCell<'q>,
    pub(super) batching: BatchingStrategy,
    /// Phase 10 Round 2 W6: per-system tick snapshot forwarded into every
    /// chunk dispatch path (inline + owned + PAR7 fallback).
    pub(super) meta: &'s SystemMeta,
    /// Phase 22 D4: per-view dynamic-tag terms, applied in the per-archetype
    /// distribution loop (already archetype-granular — workers receive only
    /// term-passing archetypes' chunks).
    pub(super) terms: TagTerms,
}

impl<'q, 's, D, F> ParQuery<'q, 's, D, F>
where
    D: ReadOnlyQueryData,
    F: QueryFilter,
{
    /// Override the default [`BatchingStrategy`].
    #[inline]
    pub fn batching_strategy(mut self, strategy: BatchingStrategy) -> Self {
        self.batching = strategy;
        self
    }

    /// Invoke `body` on every matched row, dispatching chunks across the
    /// current `ThreadPool`'s workers.
    ///
    /// When no pool is attached to the calling thread (no surrounding
    /// `install` / `Schedule::run`), `body` is invoked sequentially on the
    /// calling thread via the read-only [`Query::iter`] path (PAR7
    /// fallback).
    ///
    /// [`Query::iter`]: crate::ecs::core::iters::query::Query::iter
    #[inline]
    pub fn for_each<Body>(self, body: Body)
    where
        Body: Fn(D::Item<'_>) + Send + Sync,
    {
        for_each_impl::<D, F, Body>(
            self.state,
            self.world,
            self.batching,
            self.meta,
            &self.terms,
            body,
            false,
        );
    }
}

// ── ParQueryMut (mutable) ───────────────────────────────────────────────────

/// Parallel iteration handle for mutable [`Query`] access. Returned by
/// [`Query::par_iter_mut`].
///
/// Consume via [`Self::for_each`]; the call blocks until every chunk has
/// run (plan §4.5.5 work-stealing Drop).
///
/// [`Query`]: crate::ecs::core::iters::query::Query
/// [`Query::par_iter_mut`]: crate::ecs::core::iters::query::Query::par_iter_mut
pub struct ParQueryMut<'q, 's, D: QueryData, F: QueryFilter> {
    pub(super) state: &'s QueryDataState<D, F>,
    pub(super) world: UnsafeEcsCell<'q>,
    pub(super) batching: BatchingStrategy,
    /// Phase 10 Round 2 W6: per-system tick snapshot (see [`ParQuery::meta`]).
    pub(super) meta: &'s SystemMeta,
    /// Phase 22 D4: per-view dynamic-tag terms (see [`ParQuery::terms`]).
    pub(super) terms: TagTerms,
    /// `&'q mut` reborrow marker — the [`Query::par_iter_mut`] entry takes
    /// `&mut self` so the type system already enforces cursor uniqueness;
    /// this marker keeps the handle invariant in `'q`.
    pub(super) _mut_marker: PhantomData<&'q mut ()>,
}

impl<'q, 's, D, F> ParQueryMut<'q, 's, D, F>
where
    D: QueryData,
    F: QueryFilter,
{
    /// Override the default [`BatchingStrategy`].
    #[inline]
    pub fn batching_strategy(mut self, strategy: BatchingStrategy) -> Self {
        self.batching = strategy;
        self
    }

    /// Invoke `body` on every matched row, dispatching chunks across the
    /// current `ThreadPool`'s workers.
    ///
    /// When no pool is attached to the calling thread, `body` runs
    /// sequentially via [`Query::iter_mut`] (PAR7 fallback).
    ///
    /// [`Query::iter_mut`]: crate::ecs::core::iters::query::Query::iter_mut
    #[inline]
    pub fn for_each<Body>(self, body: Body)
    where
        Body: Fn(D::Item<'_>) + Send + Sync,
    {
        for_each_impl::<D, F, Body>(
            self.state,
            self.world,
            self.batching,
            self.meta,
            &self.terms,
            body,
            true,
        );
    }
}

// ── for_each driver ─────────────────────────────────────────────────────────

/// Shared dispatch driver for `ParQuery::for_each` (read-only) and
/// `ParQueryMut::for_each` (mutable).
///
/// `mutable = true` mints `*mut Archetype` pointers via
/// [`UnsafeEcsCell::archetype_ptr_mut`] and dispatches every per-chunk
/// table refresh through `set_table_mut`. `mutable = false` mints
/// `*const Archetype` pointers via [`UnsafeEcsCell::archetype_ptr`] and
/// dispatches through `set_table_readonly`.
///
/// # Why a runtime flag instead of two monomorphic drivers
///
/// The hot path inside one driver is identical for the two kinds modulo
/// the pointer-mint method and the `set_table_*` dispatch; the runtime
/// branch is two CMOV / one cold branch — negligible against the ~120 ns
/// per-spawn cost. Two monomorphic drivers would duplicate the (already
/// long) chunk loop without observable speedup; sharing keeps the I-cache
/// footprint smaller.
// Phase 22 D4: `terms` is the per-view dynamic-tag term list; the test runs
// in the per-archetype distribution loop (already archetype-granular), so
// workers receive only term-passing archetypes' chunks. The 7-arg signature
// mirrors `run_chunk_raw`'s rationale: bundling into a struct would add a
// deref layer at both call sites for no codegen win.
#[allow(clippy::too_many_arguments)]
fn for_each_impl<D, F, Body>(
    state: &QueryDataState<D, F>,
    world: UnsafeEcsCell<'_>,
    batching: BatchingStrategy,
    meta: &SystemMeta,
    terms: &TagTerms,
    body: Body,
    mutable: bool,
) where
    D: QueryData,
    F: QueryFilter,
    Body: Fn(D::Item<'_>) + Send + Sync,
{
    // PAR7: fall back to sequential iteration if no pool is attached.
    // `try_with_active_pool` returns `None` for threads that never entered
    // an `install` frame; this is the common case for ad-hoc tests and
    // for the `EcsMaster::run_closure_once` path which bypasses the
    // scheduler entirely.
    let dispatched = boyko_threadpool::try_with_active_pool(|pool| {
        let worker_count = pool.num_threads().max(1);
        let body_ref = &body;
        let strategy = batching;

        // Use `scope` (re-entrant) instead of `install` — `par_iter` may
        // be invoked from inside a worker task body running under
        // `Schedule::run`'s outer `install` (Wave 5). `scope`'s Drop
        // performs work-stealing so nested invocations cannot deadlock
        // (plan §4.5.5 / Round 2 C3).
        pool.scope(|scope| {
            for &arch_id in state.archetype_state.matched_ids_pre_terms() {
                // SAFETY (U_C2 / U_C3): the cell is scoped to `'_` (caller
                //   contract on `for_each`); the mint path matches the
                //   `mutable` flag — the cell carries write-capable
                //   provenance iff `mutable == true` (enforced upstream by
                //   `ParQueryMut`'s `&mut self` borrow gate).
                let arch_ptr: *mut Archetype = unsafe {
                    if mutable {
                        match world.archetype_ptr_mut(arch_id) {
                            Some(p) => p,
                            None => continue,
                        }
                    } else {
                        match world.archetype_ptr(arch_id) {
                            Some(p) => p as *mut Archetype,
                            None => continue,
                        }
                    }
                };

                // Phase 22 D4: archetype-level dynamic-tag term test in the
                // (single-threaded) distribution loop — one predicted
                // not-taken branch per archetype transition when no terms
                // are set; workers never see term-rejected archetypes.
                //
                // SAFETY (U1 / U2): same bounded shared-reborrow discipline
                //   as the `entity_count` probe below; the `&Archetype`
                //   dies inside the call.
                if !archetype_passes_tag_terms(terms, unsafe { &*arch_ptr }) {
                    continue;
                }

                // SAFETY (U1 / U2 — Phase 7 slab stability): `arch_ptr`
                //   is live for the surrounding cell scope; the `&Archetype`
                //   reborrow is bounded to this expression — no
                //   `&mut Archetype` is produced.
                let entity_count = unsafe { (*arch_ptr).entity_count() };
                if entity_count == 0 {
                    continue;
                }

                // PAR9: tiny archetypes process inline on the calling
                // thread. No `scope.spawn` round-trip means no Send/Sync
                // bound on the body for the inline path — but we keep the
                // bound at the public API level so the spawn-path code
                // stays consistent.
                if entity_count < strategy.min_batch_size {
                    // SAFETY (PAR9): inline path; the calling thread is the
                    //   sole accessor, and the cell + body satisfy the
                    //   read/write contract per the outer borrow.
                    run_chunk_inline::<D, F, Body>(
                        state,
                        arch_ptr,
                        0,
                        entity_count,
                        mutable,
                        meta,
                        body_ref,
                    );
                    continue;
                }

                let chunk_size = strategy.chunk_size(entity_count, worker_count);

                let mut start = 0usize;
                while start < entity_count {
                    let end = (start + chunk_size).min(entity_count);

                    let captured = ChunkCaptures::<D, F> {
                        data_state: &state.data_state as *const _,
                        filter_state: &state.filter_state as *const _,
                        archetype: arch_ptr,
                        start,
                        end,
                        mutable,
                        // Phase 10 Round 2 W6: `meta` is captured as a raw
                        // pointer to make `ChunkCaptures` `Send`. The
                        // surrounding `scope.Drop` blocks until every
                        // chunk completes, so the borrow that produced
                        // `meta` outlives the closure body.
                        meta: meta as *const SystemMeta,
                    };

                    // SAFETY (PAR2 / PAR3 / S1 / SEND1 / SEND3):
                    //   - `captured.data_state` / `captured.filter_state`
                    //     are pointers to fields of the `QueryDataState`
                    //     borrowed for `'s`; the surrounding `scope.Drop`
                    //     blocks until every chunk completes, so the
                    //     pointers remain valid for the closure body.
                    //   - `captured.archetype` is a `*mut Archetype` minted
                    //     above; for `mutable == false` it carries
                    //     read-only provenance (cast from `*const` purely
                    //     for storage uniformity — no write methods are
                    //     called).
                    //   - `captured.meta` is a raw pointer to the active
                    //     system's `SystemMeta`; the borrow outlives the
                    //     closure body via the scope.Drop wait.
                    //   - Each chunk's row range `[start..end)` is disjoint
                    //     from every other chunk by construction (monotonic
                    //     walk). The conflict graph / `FilteredAccessSet`
                    //     guarantees no concurrent system aliases this
                    //     archetype's columns for the current dispatch
                    //     round.
                    //   - `body_ref: &Body` is `Send + Sync` (Body bound) so
                    //     the worker can invoke it concurrently with
                    //     sibling chunks.
                    scope.spawn(move || {
                        // SAFETY: forwarded; see outer SAFETY block.
                        unsafe {
                            run_chunk_owned::<D, F, Body>(captured, body_ref);
                        }
                    });

                    start = end;
                }
            }
        });
    });

    if dispatched.is_none() {
        // PAR7 fallback: no active pool — walk sequentially using the
        // same data/filter state and cell. We replicate the inline-chunk
        // walk per archetype (mirrors what `Query::iter` / `iter_mut`
        // would produce; we avoid going through `Query` itself to keep
        // the borrow shape compatible with the by-value `world` cell).
        for &arch_id in state.archetype_state.matched_ids_pre_terms() {
            // SAFETY (U_C2 / U_C3): same as in the dispatched branch.
            let arch_ptr: *mut Archetype = unsafe {
                if mutable {
                    match world.archetype_ptr_mut(arch_id) {
                        Some(p) => p,
                        None => continue,
                    }
                } else {
                    match world.archetype_ptr(arch_id) {
                        Some(p) => p as *mut Archetype,
                        None => continue,
                    }
                }
            };
            // Phase 22 D4: term test on the PAR7 sequential fallback — same
            // contract as the dispatched branch above.
            //
            // SAFETY (U1 / U2): bounded shared reborrow; dies inside the call.
            if !archetype_passes_tag_terms(terms, unsafe { &*arch_ptr }) {
                continue;
            }
            // SAFETY (U1 / U2): slab-stable archetype pointer; the
            //   `&Archetype` reborrow is bounded to this expression.
            let entity_count = unsafe { (*arch_ptr).entity_count() };
            if entity_count == 0 {
                continue;
            }
            // SAFETY (PAR9 fallback): single-threaded execution; same
            //   contract as the inline path on the dispatched branch.
            // Phase 10 Round 2 W6: forward `meta` so Wave C consumers
            // see the active system's tick snapshot even on the no-pool
            // fallback path.
            run_chunk_inline::<D, F, Body>(state, arch_ptr, 0, entity_count, mutable, meta, &body);
        }
    }
}

/// Per-chunk capture used by [`for_each_impl`]'s `scope.spawn` calls.
///
/// We bundle the captures into a `Copy` struct so the closure picks up
/// the whole struct (not individual `*const` fields which would defeat
/// `Send` inference). Same pattern as `Schedule::try_dispatch_ready`'s
/// `SpawnPointers`.
#[derive(Clone, Copy)]
struct ChunkCaptures<D: QueryData, F: QueryFilter> {
    data_state: *const D::State,
    filter_state: *const F::State,
    archetype: *mut Archetype,
    start: usize,
    end: usize,
    mutable: bool,
    /// Phase 10 Round 2 W6: pointer to the active system's `SystemMeta`.
    /// Stored as `*const` (raw) so the struct is `Send` despite the
    /// pointer pointing into the dispatcher's stack-borrowed meta.
    /// Reborrowed as `&SystemMeta` inside the spawned closure body via
    /// `run_chunk_owned`; the surrounding `scope.Drop` keeps the meta
    /// alive for the entire chunk lifetime.
    meta: *const SystemMeta,
}

// SAFETY (PAR2 / SEND1 / SEND3 — Phase 9 §9.2 + Phase 10 Round 2 W6):
//
// `ChunkCaptures` holds four raw pointers; each is `!Send` by default.
// We hand-mark `Send` because the struct travels through `scope.spawn`
// onto a worker. The pointees:
//
//   - `data_state` / `filter_state` reference fields of a
//     `QueryDataState` borrowed for `'s`. The surrounding `scope.Drop`
//     blocks until every chunk completes (work-stealing Drop wait,
//     plan §4.5.5), so the borrow outlives every captured pointer's use.
//     `D::State: Sync` is inherited from the `Send + Sync + 'static`
//     bound on `QueryData::State`.
//   - `archetype` references a slab-stable archetype (Phase 7 U1/U2).
//     The conflict graph guarantees no concurrent worker aliases this
//     archetype's columns for the current dispatch round (SCH3).
//   - `meta` references the active system's `SystemMeta`. `SystemMeta`
//     is `Send + Sync` (Phase 8a / Phase 9 SEND10). The pointer is a
//     shared `*const` — workers only read from it.
//
// `Sync` is unnecessary — the bundle is captured by value into the
// `move` closure, never shared by reference.
unsafe impl<D: QueryData, F: QueryFilter> Send for ChunkCaptures<D, F> {}

/// Run one chunk in the calling worker / dispatcher thread.
///
/// # Safety (PAR2 + S1)
/// * `captured.data_state` / `captured.filter_state` must reference the
///   `QueryDataState` fields whose borrow encloses this call.
/// * `captured.archetype` must reference a live archetype slot under the
///   surrounding cell scope; for `captured.mutable == true` it must
///   carry write-capable provenance.
/// * `[captured.start, captured.end)` must be in-range for the
///   archetype's `entity_count()` at the moment of this call. The
///   monotonic-walk dispatch loop in [`for_each_impl`] guarantees the
///   range is disjoint from every other chunk's range.
/// * Concurrent invocations on the same archetype with disjoint ranges
///   are sound under the conflict graph (SCH3): no other system writes
///   to the same component bytes while this chunk runs.
#[inline]
unsafe fn run_chunk_owned<D: QueryData, F: QueryFilter, Body>(
    captured: ChunkCaptures<D, F>,
    body: &Body,
) where
    Body: Fn(D::Item<'_>) + Send + Sync,
{
    // SAFETY (PAR2 + S1 + Phase 10 Round 2 W6): forwarded from the
    //   outer SAFETY block. The `meta` reborrow rebuilds an `&SystemMeta`
    //   from the `*const SystemMeta` captured in `ChunkCaptures`; the
    //   borrow is scoped to this call, and the surrounding `scope.Drop`
    //   keeps the underlying `SystemMeta` alive for the closure body.
    unsafe {
        run_chunk_raw::<D, F, Body>(
            &*captured.data_state,
            &*captured.filter_state,
            captured.archetype,
            captured.start,
            captured.end,
            captured.mutable,
            &*captured.meta,
            body,
        );
    }
}

/// Per-chunk inline runner shared with the PAR7 sequential fallback. The
/// `state` parameter is the typed `QueryDataState` borrow; we destructure
/// it into raw pointer pieces inside [`run_chunk_raw`] so the worker path
/// (which receives the pointers via `ChunkCaptures`) and the inline path
/// share a single body.
///
/// # Safety
/// Same as [`run_chunk_raw`].
#[inline]
fn run_chunk_inline<D: QueryData, F: QueryFilter, Body>(
    state: &QueryDataState<D, F>,
    archetype: *mut Archetype,
    start: usize,
    end: usize,
    mutable: bool,
    meta: &SystemMeta,
    body: &Body,
) where
    Body: Fn(D::Item<'_>) + Send + Sync,
{
    // SAFETY (PAR2 + S1): the inline path runs on the calling thread;
    //   the `state`/`archetype`/`meta` borrows are scoped to the
    //   surrounding call frame in `for_each_impl`. `[start..end)` is
    //   the full archetype range for the inline call, which is by
    //   construction in-range.
    unsafe {
        run_chunk_raw::<D, F, Body>(
            &state.data_state,
            &state.filter_state,
            archetype,
            start,
            end,
            mutable,
            meta,
            body,
        );
    }
}

/// Core chunk loop. Initialises a per-chunk `D::Fetch` / `F::Fetch` from
/// the archetype pointer, then walks `[start..end)` invoking `body` per
/// matched row.
///
/// # Safety
/// * `data_state` / `filter_state` must outlive the call (the surrounding
///   `QueryDataState` borrow is the upstream root).
/// * `archetype` must reference a live archetype slot. For `mutable ==
///   true` it must carry write-capable provenance; for `mutable == false`
///   it may carry either provenance (`set_table_readonly` accepts
///   `*const`, but for storage uniformity we accept `*mut` and cast).
/// * `start <= end <= archetype.entity_count()` at call time; this is
///   the disjoint-range invariant maintained by the dispatch loop.
// The raw chunk runner is the shared body for the inline / owned / PAR7
// paths; bundling its arguments into a struct would require yet another
// layer of unsafe deref at each call site (and complicate the Send
// inference for the spawned closure capture). The 8-arg signature is
// intentional.
#[allow(clippy::too_many_arguments)]
#[inline]
unsafe fn run_chunk_raw<D: QueryData, F: QueryFilter, Body>(
    data_state: &D::State,
    filter_state: &F::State,
    archetype: *mut Archetype,
    start: usize,
    end: usize,
    mutable: bool,
    meta: &SystemMeta,
    body: &Body,
) where
    Body: Fn(D::Item<'_>) + Send + Sync,
{
    let mut data_fetch = <D as QueryData>::init_fetch(data_state);
    let mut filter_fetch = <F as QueryFilter>::init_fetch(filter_state);

    // Phase 12.5 Track B NCD6: const-fold dispatcher. When neither
    // `D` nor `F` declares `NEEDS_CHANGE_DETECTION = true`, route
    // through the `_no_meta` variants — `meta` is never loaded on
    // this monomorphisation. Same contract as `QueryIter::next` /
    // `QueryIterMut::next`. Mirrors plan §NCD6.
    if mutable {
        // SAFETY (QD3 / QD4 / QF3): `archetype` carries write-capable
        //   provenance (caller contract); `set_table_mut(_no_meta)` is the
        //   kind-correct dispatch for the mutable cursor. Phase 10 Round 2
        //   W6 / W7: meta-bearing branch — `meta` forwarded by reference so
        //   Wave C `Mut<T>` / `Changed<C>` impls can read the per-frame
        //   tick snapshot.
        unsafe {
            if const { D::NEEDS_CHANGE_DETECTION || F::NEEDS_CHANGE_DETECTION } {
                <D as QueryData>::set_table_mut(&mut data_fetch, data_state, archetype, meta);
                <F as QueryFilter>::set_table_mut(
                    &mut filter_fetch,
                    filter_state,
                    archetype,
                    meta,
                );
            } else {
                <D as QueryData>::set_table_mut_no_meta(&mut data_fetch, data_state, archetype);
                <F as QueryFilter>::set_table_mut_no_meta(
                    &mut filter_fetch,
                    filter_state,
                    archetype,
                );
            }
        }
    } else {
        // SAFETY (QD3 / QD4 / QF3): downgrade `*mut → *const` for the
        //   read-only dispatch. The caller's `mutable == false` branch
        //   guarantees no write-capable use downstream; this matches
        //   `QueryIter::next`'s read-only mint path. Phase 10 Round 2
        //   W6 / W7: meta-bearing branch — `meta` forwarded by reference.
        //   Phase 12.5 Track B NCD6: meta-free branch skips the meta load.
        unsafe {
            if const { D::NEEDS_CHANGE_DETECTION || F::NEEDS_CHANGE_DETECTION } {
                <D as QueryData>::set_table_readonly(
                    &mut data_fetch,
                    data_state,
                    archetype as *const Archetype,
                    meta,
                );
                <F as QueryFilter>::set_table_readonly(
                    &mut filter_fetch,
                    filter_state,
                    archetype as *const Archetype,
                    meta,
                );
            } else {
                <D as QueryData>::set_table_readonly_no_meta(
                    &mut data_fetch,
                    data_state,
                    archetype as *const Archetype,
                );
                <F as QueryFilter>::set_table_readonly_no_meta(
                    &mut filter_fetch,
                    filter_state,
                    archetype as *const Archetype,
                );
            }
        }
    }

    // Hot row loop. Const-fold of `F::IS_ARCHETYPAL` mirrors
    // `QueryIter::next` (plan §6.2 / Phase 8b §7.1) — for every Phase 8b
    // filter (`()`, `With<C>`, `Without<C>`, `Or<F>` post-archetype) the
    // per-row `filter_fetch` call vanishes at monomorphisation.
    let mut row = start;
    while row < end {
        if !const { F::IS_ARCHETYPAL } {
            // SAFETY (QF1): `set_table_*` ran above for this archetype;
            //   `row < end <= entity_count()` (caller contract).
            let pass = unsafe { <F as QueryFilter>::filter_fetch(&filter_fetch, row) };
            if !pass {
                row += 1;
                continue;
            }
        }

        // SAFETY (QD2 / QD3): `set_table_*` cached the column base
        //   pointers above; `row < end <= entity_count()`.
        let item = unsafe { <D as QueryData>::fetch(&data_fetch, row) };
        body(item);
        row += 1;
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// `BatchingStrategy::chunk_size` honours the min/max clamp and the
    /// per-worker divisor (plan §6.3).
    #[test]
    fn batching_strategy_clamp() {
        let s = BatchingStrategy::default();
        // 8192 / (8 workers × 1 batch) = 1024 → clamped at min 1024.
        assert_eq!(s.chunk_size(8192, 8), 1024);
        // Smaller than min: clamped up.
        assert_eq!(s.chunk_size(100, 8), 1024);
        // Custom max cap.
        let bounded = BatchingStrategy {
            batches_per_thread: 1,
            min_batch_size: 64,
            max_batch_size: 256,
        };
        // 10_000 / (8 × 1) = 1250 → clamped at max 256.
        assert_eq!(bounded.chunk_size(10_000, 8), 256);
        // Below min — clamped up to 64.
        assert_eq!(bounded.chunk_size(10, 8), 64);
    }

    /// Sanity: default constants match plan values.
    #[test]
    fn default_strategy_constants() {
        let s = BatchingStrategy::default();
        assert_eq!(s.batches_per_thread, 1);
        assert_eq!(s.min_batch_size, MIN_ARCHETYPE_FOR_PARALLEL);
        assert_eq!(s.max_batch_size, usize::MAX);
        assert_eq!(MIN_ARCHETYPE_FOR_PARALLEL, 1024);
    }
}
