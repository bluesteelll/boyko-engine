//! Phase X.A Wave 6 — parallel `Query::par_for_each_chunk` driver.
//!
//! Sibling of [`super::chunk_iter`] (sequential) and [`super::par_iter`] (per-row
//! parallel). Splits each matched archetype's row range into sub-ranges per
//! [`BatchingStrategy`] and dispatches each sub-range to a worker via
//! [`ThreadPool::scope`]. PAR7 fallback (no active pool → sequential walk on
//! the calling thread) is preserved.
//!
//! # Closure invocation frequency
//!
//! The closure is invoked **once per archetype sub-range, not once per
//! archetype** — see plan §2.4 / §1.2 for the worked example. The sequential
//! [`chunk_iter::for_each_chunk_impl`] driver yields exactly one invocation per
//! non-empty matched archetype with a full row slice; the parallel driver
//! yields `worker_count × batches_per_thread` (medium-large regime) or
//! `entity_count / MIN_ARCHETYPE_FOR_PARALLEL` (small-archetype floor regime)
//! invocations per archetype, each with a sub-range slice.
//!
//! # Soundness rails (inherits from `par_iter`)
//!
//! - **PAR1**: `Func: for<'c> Fn(D::ChunkItem<'c>) + Send + Sync`. The `Fn`
//!   (not `FnMut`) bound is the canonical race-free shape — multiple workers
//!   invoke it concurrently on disjoint row sub-ranges.
//! - **PAR2**: each sub-range is `[start, start + len)` with monotonic
//!   non-overlapping walk; CD3 (chunked-data disjointness invariant) is
//!   discharged structurally by the dispatch loop.
//! - **PAR3**: `D::ChunkItem<'_>` is required to be `Send` indirectly via the
//!   `Send + Sync` bound on the user closure.
//! - **PAR6**: re-entrant via [`ThreadPool::scope`] (work-stealing Drop, plan
//!   §4.5.5); nested `par_for_each_chunk` from inside a worker is safe.
//! - **PAR7**: when no pool is attached to the calling thread, dispatch falls
//!   back to the sequential [`super::chunk_iter::for_each_chunk_impl`] driver,
//!   wrapping the user `Fn` in a `FnMut` adapter.
//! - **PAR9**: archetypes with fewer than [`MIN_ARCHETYPE_FOR_PARALLEL`] rows
//!   run inline on the calling thread (no `scope.spawn` round-trip).
//!
//! # Why no meta plumbing
//!
//! [`ChunkedQueryData`] excludes `Ref<T>` / `Mut<T>` (CD-trait gate);
//! [`ArchetypalQueryFilter`] excludes `Added<C>` / `Changed<C>`. Therefore
//! `NEEDS_CHANGE_DETECTION` const-folds to `false` at this monomorphisation —
//! the meta-bearing branch from [`super::par_iter::for_each_impl`] does not
//! appear here, and the NCD6 dispatcher split (per `par_iter.rs:577-641`) is
//! absent. The driver path is strictly leaner than its per-row sibling.
//!
//! [`ThreadPool::scope`]: boyko_threadpool::ThreadPool::scope
//! [`ChunkedQueryData`]: super::chunked_data::ChunkedQueryData
//! [`ArchetypalQueryFilter`]: super::filter::ArchetypalQueryFilter
//! [`BatchingStrategy`]: super::par_iter::BatchingStrategy
//! [`MIN_ARCHETYPE_FOR_PARALLEL`]: super::par_iter::MIN_ARCHETYPE_FOR_PARALLEL

use std::marker::PhantomData;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::iters::query::chunk_iter;
use crate::ecs::core::iters::query::chunked_data::ChunkedQueryData;
use crate::ecs::core::iters::query::filter::ArchetypalQueryFilter;
use crate::ecs::core::iters::query::par_iter::{BatchingStrategy, MIN_ARCHETYPE_FOR_PARALLEL};
use crate::ecs::core::iters::query::state::QueryDataState;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

/// Parallel chunked-iter driver. Shared between
/// [`Query::par_for_each_chunk`] and [`QueryView::par_for_each_chunk`].
///
/// `mutable = true` mints `*mut Archetype` via
/// [`UnsafeEcsCell::archetype_ptr_mut`] and dispatches `set_chunk_mut`.
/// `mutable = false` mints `*const Archetype` via
/// [`UnsafeEcsCell::archetype_ptr`] and dispatches `set_chunk_readonly`.
///
/// # Inline policy
///
/// Deliberately **not** marked `#[inline]` — mirrors
/// [`super::par_iter::for_each_impl`] (`par_iter.rs:244`). LLVM has full
/// visibility into the single-call-site driver and decides whether to inline
/// based on its cost model. Per CLAUDE.md principle 7, no blind
/// `#[inline(always)]`.
///
/// # Safety
///
/// The caller MUST satisfy two contracts:
///
/// * **Read/write contract of `D`** — when `mutable == false`, `world` must
///   carry read-only mint provenance; when `mutable == true`, `world` must
///   carry write-capable provenance (from [`UnsafeEcsCell::new_mutable`]).
///   The conflict graph (SCH3) upstream guarantees no concurrent system
///   aliases any column touched by `D` for the current dispatch round.
/// * **State-sync** — `state` must already be synced against the live
///   archetype set via [`QueryDataState::update`]. The driver does not call
///   `update` itself; it walks `state.archetype_state.matched_ids()`
///   verbatim. Stale ids (Q5) are skipped transparently via the
///   `archetype_ptr(_mut)` `None` arm.
///
/// [`Query::par_for_each_chunk`]: super::query::Query::par_for_each_chunk
/// [`QueryView::par_for_each_chunk`]: super::query_view::QueryView
/// [`UnsafeEcsCell::new_mutable`]: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::new_mutable
pub(crate) unsafe fn par_for_each_chunk_impl<'q, 's, D, F, Func>(
    state: &'s QueryDataState<D, F>,
    world: UnsafeEcsCell<'q>,
    mutable: bool,
    batching: BatchingStrategy,
    f: Func,
) where
    D: ChunkedQueryData + 's,
    F: ArchetypalQueryFilter + 's,
    Func: for<'c> Fn(D::ChunkItem<'c>) + Send + Sync + 's,
{
    // PAR7 fallback: when no pool is attached to the calling thread, walk the
    //   matched archetypes sequentially via the shared seq driver. We adapt
    //   the user's `Fn` to the seq driver's `FnMut` via a shared reference
    //   wrapper (the underlying closure is `Fn`, so `&f` calls compose).
    let dispatched = boyko_threadpool::try_with_active_pool(|pool| {
        let worker_count = pool.num_threads().max(1);
        let f_ref = &f;

        // Use `scope` (re-entrant) instead of `install` — `par_for_each_chunk`
        // may run inside a worker body that already entered `install` via
        // `Schedule::run` (Wave 5). `scope`'s Drop performs work-stealing so
        // nested invocations cannot deadlock (plan §4.5.5 / Round 2 C3).
        pool.scope(|scope| {
            for &arch_id in state.archetype_state.matched_ids() {
                // SAFETY (U_C2 / U_C3): mirrors the read-only / write-capable
                //   mint split from `chunk_iter::for_each_chunk_impl`
                //   (`chunk_iter.rs:113-125`) and `par_iter::for_each_impl`
                //   (`par_iter.rs:278-290`). The cell is scoped to `'q` per
                //   the caller contract; `archetype_ptr(_mut)` returns `None`
                //   for archetype ids whose slot was removed after `state`
                //   was last synced — Q5 stale-id-skip via `continue`. When
                //   `mutable == false` we take the read-only mint and cast
                //   `*const → *mut` purely so `arch_ptr` has a single type;
                //   only `set_chunk_readonly` is called below.
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

                // SAFETY (U1 / U2 — Phase 7 slab stability): `arch_ptr` is
                //   live for the surrounding cell scope (`'q`); the
                //   `&Archetype` reborrow materialised by the raw deref is
                //   bounded to this single expression. No `&mut Archetype`
                //   is produced.
                let entity_count = unsafe { (*arch_ptr).entity_count() };
                if entity_count == 0 {
                    continue;
                }

                // PAR9: tiny-archetype inline path — process on the calling
                //   thread to avoid the ~120 ns/spawn dispatch overhead
                //   dominating tiny work units. No `scope.spawn` round-trip,
                //   no Send/Sync usage of `f` on this path (but the
                //   public-API bound is kept for the spawn-path code).
                if entity_count < MIN_ARCHETYPE_FOR_PARALLEL {
                    // SAFETY (CD1, CD2, CD4, PAR9): inline path; the calling
                    //   thread is the sole accessor. `mutable` selects the
                    //   correct `set_chunk_*` dispatch (CD4); `fetch_chunk(0,
                    //   entity_count)` covers the full live row range
                    //   `[0, entity_count)` exactly once (CD2; CD3 is vacuous
                    //   on the inline single-call path).
                    let mut chunk_fetch =
                        <D as ChunkedQueryData>::init_chunk_fetch(&state.data_state);
                    unsafe {
                        if mutable {
                            <D as ChunkedQueryData>::set_chunk_mut(
                                &mut chunk_fetch,
                                &state.data_state,
                                arch_ptr,
                            );
                        } else {
                            <D as ChunkedQueryData>::set_chunk_readonly(
                                &mut chunk_fetch,
                                &state.data_state,
                                arch_ptr as *const _,
                            );
                        }
                        let item = <D as ChunkedQueryData>::fetch_chunk(
                            &chunk_fetch,
                            0,
                            entity_count,
                        );
                        f_ref(item);
                    }
                    continue;
                }

                let chunk_size = batching.chunk_size(entity_count, worker_count);

                // Spawn per-subrange tasks. Each `(start, end)` is monotonic
                // non-overlapping by construction; CD3 disjointness for
                // `&mut [T]` slices is therefore satisfied structurally.
                let mut start = 0usize;
                while start < entity_count {
                    let end = (start + chunk_size).min(entity_count);

                    let captured = ChunkChunkCaptures::<'_, D, F, Func> {
                        data_state: &state.data_state as *const _,
                        archetype: arch_ptr,
                        start,
                        len: end - start,
                        mutable,
                        f: f_ref as *const Func,
                        _state_borrow: PhantomData,
                        _data_filter_invariance: PhantomData,
                    };

                    // SAFETY (PAR2 / PAR3 / S1 / SEND1 / SEND3 / CD1-CD4):
                    //   - `captured.data_state` is a pointer to the field of
                    //     `QueryDataState` borrowed for `'s`; the surrounding
                    //     `scope.Drop` blocks until every chunk completes, so
                    //     the pointer remains valid for the closure body.
                    //   - `captured.archetype` is a `*mut Archetype` minted
                    //     above; for `mutable == false` it carries read-only
                    //     provenance (cast from `*const` purely for storage
                    //     uniformity — no write methods are called).
                    //   - Each chunk's row range `[start, start + len)` is
                    //     disjoint from every other chunk by construction
                    //     (monotonic walk; CD3 discharged structurally).
                    //   - `captured.f` is a `*const Func` reborrow of `f_ref`;
                    //     the surrounding scope outlives every worker, so the
                    //     pointer is valid for the closure body.
                    //   - `Func: Send + Sync` (public-API bound) so the worker
                    //     can invoke it concurrently with sibling chunks via
                    //     the shared `&Func` reborrow.
                    //   - The conflict graph / `FilteredAccessSet` guarantees
                    //     no concurrent system aliases this archetype's
                    //     columns for the current dispatch round (SCH3).
                    scope.spawn(move || {
                        // SAFETY: forwarded; see outer SAFETY block.
                        unsafe { run_chunk_owned::<D, F, Func>(captured); }
                    });

                    start = end;
                }
            }
        });
    });

    if dispatched.is_none() {
        // PAR7 fallback: no active pool — walk sequentially. The shared
        //   `chunk_iter::for_each_chunk_impl` driver expects a `FnMut`; the
        //   user's `Fn` composes trivially. No allocation, no Send/Sync use.
        // SAFETY (forwarded): caller upheld the read/write contract for `D`
        //   and the state-sync contract for `state`; `chunk_iter` itself
        //   enforces no further preconditions.
        unsafe {
            chunk_iter::for_each_chunk_impl(state, world, mutable, |item| f(item));
        }
    }
}

/// Per-chunk capture used by [`par_for_each_chunk_impl`]'s `scope.spawn` calls.
///
/// Mirrors the structural template of [`super::par_iter::ChunkCaptures`] from
/// plan §9.4. The chunked variant drops two fields relative to the per-row
/// sibling:
///
/// * `filter_state: *const F::State` — every [`ArchetypalQueryFilter`] has an
///   empty `State = ()` (the archetypal predicate ran at `state.update(master)`
///   time); no per-task pointer needed.
/// * `meta: *const SystemMeta` — `NEEDS_CHANGE_DETECTION` const-folds to
///   `false` at this monomorphisation (see module docs), so the meta-bearing
///   branch is unreachable.
///
/// `(start, len)` replaces `(start, end)` to mirror the
/// [`ChunkedQueryData::fetch_chunk`] signature directly (no per-task
/// `end - start` subtraction inside the worker body).
struct ChunkChunkCaptures<'s, D: ChunkedQueryData, F: ArchetypalQueryFilter, Func> {
    data_state: *const D::State,
    archetype: *mut Archetype,
    start: usize,
    len: usize,
    mutable: bool,
    /// Raw reborrow of `&Func` from the enclosing scope. Stored as `*const`
    /// (not `&'s Func`) to make `ChunkChunkCaptures` `Send` despite the
    /// pointer pointing into the dispatcher's stack-borrowed closure.
    /// Reborrowed as `&Func` inside the spawned closure body via
    /// `run_chunk_owned`; the surrounding `scope.Drop` keeps the closure
    /// alive for the entire chunk lifetime.
    f: *const Func,
    /// Lifetime carrier for the `'s` state borrow.
    _state_borrow: PhantomData<&'s ()>,
    /// Invariance over `(D, F)`. `fn() -> (D, F)` keeps the marker
    /// `Send + Sync` independently of `D`/`F` auto-trait bounds.
    _data_filter_invariance: PhantomData<fn() -> (D, F)>,
}

// Manual `Copy`/`Clone` so the auto-derive does not synthesise a
// `Func: Copy + Clone` bound (the `*const Func` field would be auto-`Copy`
// regardless of `Func`'s own trait surface). Same rationale as
// `par_iter::ChunkCaptures`'s manual derive at `par_iter.rs:422`.
impl<D: ChunkedQueryData, F: ArchetypalQueryFilter, Func> Clone
    for ChunkChunkCaptures<'_, D, F, Func>
{
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<D: ChunkedQueryData, F: ArchetypalQueryFilter, Func> Copy
    for ChunkChunkCaptures<'_, D, F, Func>
{
}

// SAFETY (PAR2 / SEND1 / SEND3 — plan §9.4):
//
// `ChunkChunkCaptures` holds three raw pointers; each is `!Send` by default.
// We hand-mark `Send` because the struct travels through `scope.spawn` onto a
// worker. The pointees:
//
//   - `data_state` references a field of a `QueryDataState` borrowed for
//     `'s`. The surrounding `scope.Drop` blocks until every chunk completes
//     (work-stealing Drop wait, plan §4.5.5), so the borrow outlives every
//     captured pointer's use. `D::State: Sync` is inherited from the
//     `Send + Sync + 'static` bound on `QueryData::State` (data.rs).
//   - `archetype` references a slab-stable archetype slot (Phase 7 U1/U2).
//     The conflict graph guarantees no concurrent worker aliases this
//     archetype's columns for the current dispatch round (SCH3); disjoint
//     row sub-ranges discharge CD3 structurally.
//   - `f` is a raw `*const Func` reborrow of a `&Func` from the dispatcher's
//     stack frame. `Func: Sync` (public-API bound) so the worker can read it
//     concurrently with sibling chunks; the surrounding scope outlives every
//     worker.
//
// `Sync` is unnecessary — the bundle is captured by value into the `move`
// closure, never shared by reference.
unsafe impl<D, F, Func> Send for ChunkChunkCaptures<'_, D, F, Func>
where
    D: ChunkedQueryData,
    F: ArchetypalQueryFilter,
    Func: Send + Sync,
{
}

/// Run one chunk in a spawned worker.
///
/// `#[inline]` mirrors the analogous policy on
/// [`super::par_iter::run_chunk_owned`] (`par_iter.rs:478`) — the body is the
/// per-task entry point, called from a single `scope.spawn` site per chunk;
/// inlining exposes the captured pointer reborrows to LTO without bloating
/// I-cache (the only call site is the spawned closure body).
///
/// # Safety
///
/// * `captured.data_state` must reference the `QueryDataState`'s `data_state`
///   field whose borrow encloses this call (PAR2/SEND1 — anchored by the
///   surrounding `scope.Drop`).
/// * `captured.archetype` must reference a live archetype slot under the
///   surrounding cell scope; for `captured.mutable == true` it must carry
///   write-capable provenance.
/// * `[captured.start, captured.start + captured.len)` must be in-range for
///   the archetype's `entity_count()` at the moment of this call. The
///   monotonic-walk dispatch loop in [`par_for_each_chunk_impl`] guarantees
///   the range is disjoint from every other chunk's range (CD3).
/// * Concurrent invocations on the same archetype with disjoint ranges are
///   sound under the conflict graph (SCH3): no other system writes to the
///   same component bytes while this chunk runs.
/// * `captured.f` is a valid `*const Func` reborrow whose pointee outlives
///   the call (the scope.Drop wait anchors it).
#[inline]
unsafe fn run_chunk_owned<'s, D, F, Func>(captured: ChunkChunkCaptures<'s, D, F, Func>)
where
    D: ChunkedQueryData,
    F: ArchetypalQueryFilter,
    Func: for<'c> Fn(D::ChunkItem<'c>) + Send + Sync + 's,
{
    // SAFETY (CD1, CD2, CD4, PAR2): mirrors the inline path in
    //   `par_for_each_chunk_impl` but writes to a sub-range only. CD3
    //   disjointness is enforced by the outer while-loop emitting
    //   non-overlapping `[start, start + len)` half-open ranges via the
    //   `BatchingStrategy` monotonic walk. The `data_state` deref is bounded
    //   by the surrounding `scope.Drop`; the `f` deref likewise.
    let mut chunk_fetch =
        <D as ChunkedQueryData>::init_chunk_fetch(unsafe { &*captured.data_state });
    unsafe {
        if captured.mutable {
            <D as ChunkedQueryData>::set_chunk_mut(
                &mut chunk_fetch,
                &*captured.data_state,
                captured.archetype,
            );
        } else {
            <D as ChunkedQueryData>::set_chunk_readonly(
                &mut chunk_fetch,
                &*captured.data_state,
                captured.archetype as *const _,
            );
        }
        let item = <D as ChunkedQueryData>::fetch_chunk(
            &chunk_fetch,
            captured.start,
            captured.len,
        );
        (*captured.f)(item);
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::component::component_registry;
    use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
    use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Component id reserved for Phase X.A Wave 6 par_chunk tests. The free
    // slot below was verified at write time against existing crate-wide
    // allocations (extending the Wave 4/5 reservation comment in
    // `chunk_iter.rs:188-197` and `query_view.rs:517-532`):
    //   * 450-461 — component_registry tests + Phase X.A Wave 4 (460-461)
    //   * 462    — component_registry COLLISION_SLOT
    //   * 463    — Phase X.A Wave 5 query_view test
    //   * 465    — component_registry IDEMPOTENT_SLOT (occupied by ColTypeA)
    //   * 480-482 — archetype_bundle miri tests
    //   * 483-485 — query/iter.rs
    //   * 486-488 — query/query.rs
    //   * 490-497 — query_state / component_set
    //   * 503-504 — query/data.rs
    //   * 506-510 — query/state.rs / resource_registry
    // Slot 466 is clear: above the component_registry IDEMPOTENT_SLOT (465)
    // and below the archetype_bundle reservation (480). Wave 6 tests only
    // need a single component (single-archetype scenarios) — slot 467 is
    // left free for future expansion.
    const COMP_A: ComponentId = ComponentId(466);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CompA(u32);

    impl Component for CompA {
        fn component_id() -> ComponentId {
            COMP_A
        }
    }

    /// Idempotent registry priming.
    fn register_test_components() {
        component_registry::register_layout::<CompA>(COMP_A.0);
    }

    /// Spawns a `CompA(value)` entity into `arch_id`.
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

    /// PAR7 sanity: with no active pool attached to the calling thread, the
    /// driver falls back to the sequential walk. Verifies the closure fires
    /// exactly once per non-empty archetype and the total row count matches
    /// the spawn count.
    ///
    /// This test does NOT exercise `scope.spawn` — that path requires
    /// `ThreadPool::install` (Wave 7 covers the multi-thread cases).
    #[test]
    fn par_for_each_chunk_no_pool_fallback() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[COMP_A]);
        for i in 0..50u32 {
            spawn_a(&mut ecs, arch, i + 700);
        }

        let state = QueryDataState::<&CompA, ()>::new(&mut ecs);
        // SAFETY (U_C1): cell is consumed inside this scope; it does not
        //   outlive the `&mut ecs` borrow above.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };

        let invocations = AtomicUsize::new(0);
        let total = AtomicUsize::new(0);

        // SAFETY (Q1, CD1-CD4): direct driver test; `D = &CompA` ⇒
        //   `IS_READ_ONLY = true` ⇒ `mutable = false`. `F = ()` ⇒ archetypal.
        //   No active pool attached on this thread ⇒ PAR7 sequential fallback
        //   is the only path exercised here.
        unsafe {
            par_for_each_chunk_impl::<&CompA, (), _>(
                &state,
                cell,
                false,
                BatchingStrategy::default(),
                |slice: &[CompA]| {
                    invocations.fetch_add(1, Ordering::Relaxed);
                    total.fetch_add(slice.len(), Ordering::Relaxed);
                },
            );
        }

        assert_eq!(
            invocations.load(Ordering::Relaxed),
            1,
            "PAR7 fallback ⇒ sequential walk ⇒ exactly one closure invocation \
             per non-empty archetype",
        );
        assert_eq!(
            total.load(Ordering::Relaxed),
            50,
            "PAR7 fallback ⇒ total rows = spawn count",
        );
    }

    /// PAR9 sanity: a small archetype (`entity_count <
    /// MIN_ARCHETYPE_FOR_PARALLEL = 1024`) inside an active pool dispatches
    /// inline on the calling thread, not via `scope.spawn`. The closure must
    /// still fire exactly once per archetype with the full slice.
    ///
    /// We attach a 2-worker pool but spawn only 100 entities — well below the
    /// inline threshold. The inline path is structurally identical to the
    /// PAR7 fallback (single closure invocation per archetype) but exercises
    /// a different code path (inside the `pool.scope`, before the
    /// `scope.spawn` branch).
    #[test]
    fn par_for_each_chunk_inline_below_min() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[COMP_A]);
        for i in 0..100u32 {
            spawn_a(&mut ecs, arch, i + 800);
        }

        let state = QueryDataState::<&CompA, ()>::new(&mut ecs);
        // SAFETY (U_C1): cell consumed inside the closure passed to
        //   `pool.install` below; does not escape the `&mut ecs` borrow.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };

        let invocations = AtomicUsize::new(0);
        let total = AtomicUsize::new(0);

        let pool = boyko_threadpool::ThreadPoolBuilder::new()
            .num_threads(2)
            .build();

        pool.install(|_scope| {
            // SAFETY (Q1, CD1-CD4, PAR9): inline path inside the active pool;
            //   `entity_count = 100 < MIN_ARCHETYPE_FOR_PARALLEL` triggers
            //   the dispatch loop's inline branch. `D = &CompA` ⇒ read-only;
            //   `F = ()` ⇒ archetypal. No aliasing live.
            unsafe {
                par_for_each_chunk_impl::<&CompA, (), _>(
                    &state,
                    cell,
                    false,
                    BatchingStrategy::default(),
                    |slice: &[CompA]| {
                        invocations.fetch_add(1, Ordering::Relaxed);
                        total.fetch_add(slice.len(), Ordering::Relaxed);
                    },
                );
            }
        });

        assert_eq!(
            invocations.load(Ordering::Relaxed),
            1,
            "PAR9 inline path ⇒ exactly one closure invocation per non-empty \
             archetype (no scope.spawn fan-out)",
        );
        assert_eq!(
            total.load(Ordering::Relaxed),
            100,
            "PAR9 inline path ⇒ total rows = spawn count",
        );
    }
}
