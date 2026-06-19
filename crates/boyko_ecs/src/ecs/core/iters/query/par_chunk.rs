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
//! `chunk_iter::for_each_chunk_impl` driver yields exactly one invocation per
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
//!   back to the sequential `super::chunk_iter::for_each_chunk_impl` driver,
//!   wrapping the user `Fn` in a `FnMut` adapter.
//! - **PAR9**: archetypes with fewer than [`MIN_ARCHETYPE_FOR_PARALLEL`] rows
//!   run inline on the calling thread (no `scope.spawn` round-trip).
//!
//! # Why no meta plumbing
//!
//! [`ChunkedQueryData`] excludes `Ref<T>` / `Mut<T>` (CD-trait gate);
//! [`ArchetypalQueryFilter`] excludes `Added<C>` / `Changed<C>`. Therefore
//! `NEEDS_CHANGE_DETECTION` const-folds to `false` at this monomorphisation —
//! the meta-bearing branch from `super::par_iter::for_each_impl` does not
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
use crate::ecs::identifiers::primitives::ArchetypeId;

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
///   `update` itself; it walks the caller-supplied `ids` slice verbatim.
///   Stale ids (Q5) are skipped transparently via the `archetype_ptr(_mut)`
///   `None` arm.
///
/// Phase 22.1 Area A: `ids` is resolved ONCE on the calling thread at the
/// driver entry (no terms → `matched_ids_pre_terms()`; terms → the per-epoch
/// memoised filtered slice) BEFORE `pool.scope`, so workers receive only
/// term-passing archetypes' chunks and this driver carries no term code. The
/// PAR7 fallback forwards the SAME slice (no re-resolve).
///
/// [`Query::par_for_each_chunk`]: super::query::Query::par_for_each_chunk
/// [`QueryView::par_for_each_chunk`]: super::query_view::QueryView
/// [`UnsafeEcsCell::new_mutable`]: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::new_mutable
pub(crate) unsafe fn par_for_each_chunk_impl<'q, 's, D, F, Func>(
    state: &'s QueryDataState<D, F>,
    ids: &[ArchetypeId],
    world: UnsafeEcsCell<'q>,
    mutable: bool,
    batching: BatchingStrategy,
    f: Func,
) where
    D: ChunkedQueryData + 's,
    F: ArchetypalQueryFilter + 's,
    Func: for<'c> Fn(D::ChunkItem<'c>) + Send + Sync + 's,
{
    // Dense plan D3: same compile-reject as the sequential chunk path — a dense
    // term has no archetype-aligned chunk. Use `Query::dense_iter` (pure dense)
    // or `Query::iter` (mixed). Const-folds out for a no-dense query (0%-gate).
    const {
        assert!(
            !D::HAS_DENSE && !F::HAS_DENSE,
            "a dense (storage = \"dense\") term is not supported on `par_for_each_chunk` — \
             use `Query::iter` / `iter_mut` (mixed) or `Query::dense_iter` (pure dense)"
        )
    };
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
            for &arch_id in ids {
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
        //   enforces no further preconditions. Phase 22.1 Area A: the SAME
        //   pre-resolved `ids` slice is forwarded (no re-resolve) — the
        //   sequential driver walks it identically.
        unsafe {
            chunk_iter::for_each_chunk_impl(state, ids, world, mutable, |item| f(item));
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
            let ids = state.archetype_state.matched_ids_pre_terms();
            par_for_each_chunk_impl::<&CompA, (), _>(
                &state,
                ids,
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
                let ids = state.archetype_state.matched_ids_pre_terms();
                par_for_each_chunk_impl::<&CompA, (), _>(
                    &state,
                    ids,
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

    // ── Phase X.A Wave 7 Step 7A — §11.4 parallel multi-thread tests ────────
    //
    // Component-id slot reservations (extending Wave 6's slot 466):
    //   * 470 — `CompW7a` (used by the multi-archetype dispatch test as the
    //                       first archetype's distinguishing component)
    //   * 471 — `CompW7b` (used by the multi-archetype dispatch test as the
    //                       second archetype's distinguishing component)
    //   * 472 — `CompW7Pos` (mutable payload for the parallel-write doubling
    //                         test)
    //
    // The 467-469 slots are claimed by `chunk_iter.rs::tests` (Wave 7 7A);
    // slots 473-479 remain free for future expansion. The slot map is the
    // single source of truth for the chunked-iter test suite — collisions
    // here propagate as panics inside `register_layout`.
    //
    // # Miri limitation (deferred to Phase 9.1)
    //
    // The three tests below (`parallel_disjoint_subrange_full_coverage_*`,
    // `parallel_multi_archetype_dispatch`, `parallel_mut_write_doubles`)
    // attach an active `ThreadPool` via `pool.install` and exercise the
    // `scope.spawn` fan-out path. Per Phase 9 closeout memory and
    // `miri_phase9.rs:14-26`, multi-thread `Schedule::run` (and by extension
    // any `pool.install` / `scope.spawn` invocation) triggers a Tree Borrows
    // `protected-tag` conflict inside `boyko_threadpool::Scope::spawn` —
    // sound by design but flagged by TB until Phase 9.1 revisits the
    // `ScopeShared` raw-pointer protocol. The Miri runs documented in plan
    // §11.5 therefore exclude these three tests, as called out explicitly
    // by `Specifically run: ... The single-threaded
    // par_for_each_chunk_no_pool_fallback (PAR7 path bypasses scope.spawn)`.
    // Plan §11.5 already documents this gap; the tests still run cleanly
    // under regular `cargo test` (the Tree Borrows model only fires under
    // Miri).

    /// Distinguishing component for the first archetype of the multi-archetype
    /// dispatch test.
    const COMP_W7A: ComponentId = ComponentId(470);
    /// Distinguishing component for the second archetype of the multi-archetype
    /// dispatch test.
    const COMP_W7B: ComponentId = ComponentId(471);
    /// Payload component for the parallel mutable-write doubling test.
    const COMP_W7POS: ComponentId = ComponentId(472);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CompW7a(u32);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CompW7b(u32);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CompW7Pos(u32);

    impl Component for CompW7a {
        fn component_id() -> ComponentId {
            COMP_W7A
        }
    }
    impl Component for CompW7b {
        fn component_id() -> ComponentId {
            COMP_W7B
        }
    }
    impl Component for CompW7Pos {
        fn component_id() -> ComponentId {
            COMP_W7POS
        }
    }

    fn register_wave7_components() {
        component_registry::register_layout::<CompW7a>(COMP_W7A.0);
        component_registry::register_layout::<CompW7b>(COMP_W7B.0);
        component_registry::register_layout::<CompW7Pos>(COMP_W7POS.0);
    }

    /// Spawn helper for `CompW7a`-bearing archetypes.
    fn spawn_w7a(ecs: &mut EcsMaster, arch_id: ArchetypeId, value: u32) {
        let comp = CompW7a(value);
        // SAFETY: `CompW7a` is `#[repr(C)]` POD; byte slice valid for the call.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                &comp as *const CompW7a as *const u8,
                std::mem::size_of::<CompW7a>(),
            )
        };
        ecs.create_entity(arch_id, &[(COMP_W7A, bytes)])
            .expect("spawn_w7a: create_entity must succeed");
    }

    /// Spawn helper for `CompW7b`-bearing archetypes.
    fn spawn_w7b(ecs: &mut EcsMaster, arch_id: ArchetypeId, value: u32) {
        let comp = CompW7b(value);
        // SAFETY: `CompW7b` is `#[repr(C)]` POD; byte slice valid for the call.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                &comp as *const CompW7b as *const u8,
                std::mem::size_of::<CompW7b>(),
            )
        };
        ecs.create_entity(arch_id, &[(COMP_W7B, bytes)])
            .expect("spawn_w7b: create_entity must succeed");
    }

    /// Spawn helper for `CompW7Pos`-bearing archetypes.
    fn spawn_w7pos(ecs: &mut EcsMaster, arch_id: ArchetypeId, value: u32) {
        let comp = CompW7Pos(value);
        // SAFETY: `CompW7Pos` is `#[repr(C)]` POD; byte slice valid for the
        //   call.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                &comp as *const CompW7Pos as *const u8,
                std::mem::size_of::<CompW7Pos>(),
            )
        };
        ecs.create_entity(arch_id, &[(COMP_W7POS, bytes)])
            .expect("spawn_w7pos: create_entity must succeed");
    }

    /// PAR2 / CD3 disjointness, large-archetype path: 10k entities in a single
    /// archetype dispatched across a 4-worker pool. The atomic counter sums the
    /// per-chunk slice lengths — full coverage (counter == 10000) verifies that
    /// every row is processed exactly once, no row is dropped, and no overlap
    /// double-counts.
    ///
    /// The driver walks `[start, start + len)` half-open ranges via the
    /// `BatchingStrategy` monotonic walk; CD3 is therefore structural — this
    /// test pins the structural property under a live multi-worker pool.
    #[test]
    fn parallel_disjoint_subrange_full_coverage_via_atomic_counter() {
        register_test_components();
        register_wave7_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[COMP_W7A]);
        for i in 0..10_000u32 {
            spawn_w7a(&mut ecs, arch, i);
        }

        let state = QueryDataState::<&CompW7a, ()>::new(&mut ecs);
        // SAFETY (U_C1): cell consumed inside the `pool.install` closure
        //   below; does not outlive the `&mut ecs` borrow.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };

        let counter = AtomicUsize::new(0);
        let invocations = AtomicUsize::new(0);

        let pool = boyko_threadpool::ThreadPoolBuilder::new()
            .num_threads(4)
            .build();

        pool.install(|_scope| {
            // SAFETY (Q1, CD1-CD4, PAR2 / CD3 / PAR9): inside the active 4-
            //   worker pool with `entity_count = 10000 >=
            //   MIN_ARCHETYPE_FOR_PARALLEL` ⇒ scope.spawn fan-out path. D =
            //   &CompW7a read-only; F = () archetypal; no aliasing live; the
            //   atomic counter is `Sync`.
            unsafe {
                let ids = state.archetype_state.matched_ids_pre_terms();
                par_for_each_chunk_impl::<&CompW7a, (), _>(
                    &state,
                    ids,
                    cell,
                    false,
                    BatchingStrategy::default(),
                    |slice: &[CompW7a]| {
                        invocations.fetch_add(1, Ordering::Relaxed);
                        counter.fetch_add(slice.len(), Ordering::Relaxed);
                    },
                );
            }
        });

        assert_eq!(
            counter.load(Ordering::Relaxed),
            10_000,
            "PAR2/CD3 full coverage: every row processed exactly once \
             (counter == 10000, no overlap, no drop)",
        );
        // At least 2 invocations (large-archetype split fan-out); upper bound
        // is the worker count × chunks-per-worker, but we only pin the
        // counter-sum invariant strictly. Lower bound > 1 confirms the
        // scope.spawn path actually fanned out instead of running inline.
        let inv = invocations.load(Ordering::Relaxed);
        assert!(
            inv >= 2,
            "scope.spawn fan-out expected ⇒ ≥ 2 invocations; got {inv}",
        );
    }

    /// Multi-archetype dispatch — 2 disjoint archetypes (5000 + 7000 rows) on a
    /// 4-worker pool. Each closure invocation increments a thread-shared atomic
    /// counter by its slice length; the final sum must equal the total spawned
    /// row count (12000).
    ///
    /// The two archetypes are matched by a disjunctive `Or<(With<a>,
    /// With<b>)>` filter — only `a OR b` archetypes participate. Verifies the
    /// per-archetype dispatch path across the matched set, with each archetype
    /// independently fanning out across the worker pool.
    #[test]
    fn parallel_multi_archetype_dispatch() {
        register_test_components();
        register_wave7_components();
        let mut ecs = EcsMaster::new();
        // Two distinct archetypes — one carries `(CompA, CompW7a)`, the other
        // `(CompA, CompW7b)`. The `Query<&CompA, ...>` shape matches both via
        // the shared CompA column; the `Or<(With<CompW7a>, With<CompW7b>)>`
        // filter selects only these two.
        let arch_a = ecs.create_archetype(&[COMP_A, COMP_W7A]);
        let arch_b = ecs.create_archetype(&[COMP_A, COMP_W7B]);

        // 5000 entities into arch_a.
        for i in 0..5000u32 {
            let ca = CompA(i);
            let cw = CompW7a(i);
            // SAFETY: both `#[repr(C)]` POD; byte slices valid for the call.
            let a_bytes = unsafe {
                std::slice::from_raw_parts(
                    &ca as *const CompA as *const u8,
                    std::mem::size_of::<CompA>(),
                )
            };
            let w_bytes = unsafe {
                std::slice::from_raw_parts(
                    &cw as *const CompW7a as *const u8,
                    std::mem::size_of::<CompW7a>(),
                )
            };
            ecs.create_entity(arch_a, &[(COMP_A, a_bytes), (COMP_W7A, w_bytes)])
                .expect("multi-archetype spawn arch_a must succeed");
        }
        // 7000 entities into arch_b.
        for i in 0..7000u32 {
            let ca = CompA(i + 100_000);
            let cw = CompW7b(i);
            // SAFETY: both `#[repr(C)]` POD; byte slices valid for the call.
            let a_bytes = unsafe {
                std::slice::from_raw_parts(
                    &ca as *const CompA as *const u8,
                    std::mem::size_of::<CompA>(),
                )
            };
            let w_bytes = unsafe {
                std::slice::from_raw_parts(
                    &cw as *const CompW7b as *const u8,
                    std::mem::size_of::<CompW7b>(),
                )
            };
            ecs.create_entity(arch_b, &[(COMP_A, a_bytes), (COMP_W7B, w_bytes)])
                .expect("multi-archetype spawn arch_b must succeed");
        }
        let _ = spawn_w7a; // suppress unused-helper warning when the helper is
        let _ = spawn_w7b; // not consumed inline above.

        // Use `Or<(With<CompW7a>, With<CompW7b>)>` to pin both archetypes.
        let state = QueryDataState::<
            &CompA,
            crate::ecs::core::iters::query::filter::Or<(
                crate::ecs::core::iters::query::With<CompW7a>,
                crate::ecs::core::iters::query::With<CompW7b>,
            )>,
        >::new(&mut ecs);
        assert_eq!(
            state.archetype_state.matched_ids_pre_terms().len(),
            2,
            "Or<(With<W7a>, With<W7b>)> must match exactly the two W7-bearing archetypes",
        );

        // SAFETY (U_C1): cell consumed inside the pool.install closure.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };

        let total = AtomicUsize::new(0);

        let pool = boyko_threadpool::ThreadPoolBuilder::new()
            .num_threads(4)
            .build();

        pool.install(|_scope| {
            // SAFETY (Q1, CD1-CD4, PAR2): D = &CompA read-only across both
            //   matched archetypes; F = Or<(With<a>, With<b>)> archetypal
            //   (Or-propagation in filter.rs:1718-1722); no aliasing live; the
            //   atomic counter is `Sync`.
            unsafe {
                let ids = state.archetype_state.matched_ids_pre_terms();
                par_for_each_chunk_impl::<
                    &CompA,
                    crate::ecs::core::iters::query::filter::Or<(
                        crate::ecs::core::iters::query::With<CompW7a>,
                        crate::ecs::core::iters::query::With<CompW7b>,
                    )>,
                    _,
                >(
                    &state,
                    ids,
                    cell,
                    false,
                    BatchingStrategy::default(),
                    |slice: &[CompA]| {
                        total.fetch_add(slice.len(), Ordering::Relaxed);
                    },
                );
            }
        });

        assert_eq!(
            total.load(Ordering::Relaxed),
            12_000,
            "multi-archetype dispatch sum: 5000 + 7000 = 12000 (every row across both archetypes processed exactly once)",
        );
    }

    /// Parallel mutable write: 4k entities in a single archetype with
    /// `Query<&mut CompW7Pos>::par_for_each_chunk(|s| s.iter_mut().for_each(|p|
    /// p.0 *= 2))`. Reread via the sequential driver confirms every value
    /// doubled. Disjointness is verified implicitly — if any two workers
    /// overlap on a row, the row would be doubled twice (× 4) and the
    /// equality check fails.
    #[test]
    fn parallel_mut_write_doubles() {
        register_test_components();
        register_wave7_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[COMP_W7POS]);
        for i in 0..4000u32 {
            spawn_w7pos(&mut ecs, arch, i);
        }

        // Phase 1 — parallel mutate via &mut driver.
        {
            let state = QueryDataState::<&mut CompW7Pos, ()>::new(&mut ecs);
            // SAFETY (U_C1): cell consumed inside the pool.install closure.
            let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };
            let pool = boyko_threadpool::ThreadPoolBuilder::new()
                .num_threads(4)
                .build();
            pool.install(|_scope| {
                // SAFETY (Q1, CD1-CD4, PAR2 / CD3): mutable = true; D = &mut
                //   CompW7Pos; F = () archetypal; the conflict-graph guarantee
                //   is vacuous here (single direct driver call, no concurrent
                //   sibling system); CD3 is structurally enforced by the
                //   monotonic walk on disjoint `[start, start + len)` ranges.
                unsafe {
                    let ids = state.archetype_state.matched_ids_pre_terms();
                    par_for_each_chunk_impl::<&mut CompW7Pos, (), _>(
                        &state,
                        ids,
                        cell,
                        true,
                        BatchingStrategy::default(),
                        |slice: &mut [CompW7Pos]| {
                            for p in slice.iter_mut() {
                                p.0 = p.0.wrapping_mul(2);
                            }
                        },
                    );
                }
            });
        }

        // Phase 2 — sequential read-back; every row must equal `original * 2`.
        // Using the sequential driver here keeps the read-back independent of
        // the par driver (so a bug in the par read-only path could not mask a
        // bug in the par mut-write path).
        let state = QueryDataState::<&CompW7Pos, ()>::new(&mut ecs);
        // SAFETY (U_C1): cell consumed within this scope.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };
        let mut collected: Vec<u32> = Vec::with_capacity(4000);
        // SAFETY (Q1, CD1-CD4): read-only re-iteration; no aliasing live.
        unsafe {
            let ids = state.archetype_state.matched_ids_pre_terms();
            chunk_iter::for_each_chunk_impl::<&CompW7Pos, (), _>(
                &state,
                ids,
                cell,
                false,
                |slice: &[CompW7Pos]| {
                    for p in slice {
                        collected.push(p.0);
                    }
                },
            );
        }

        assert_eq!(collected.len(), 4000, "every row must reappear after mutation");
        collected.sort_unstable();
        let expected: Vec<u32> = (0..4000u32).map(|i| i.wrapping_mul(2)).collect();
        assert_eq!(
            collected, expected,
            "every CompW7Pos(i) must now read back as CompW7Pos(i*2) — \
             implicit disjointness check: any overlap would × 4 instead of × 2",
        );
    }
}
