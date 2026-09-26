//! The parallel narrowphase (L5): the candidate pairs cut into contiguous chunks, each chunk
//! collided on a worker of the ambient pool, and the chunks' output joined back in pair order.
//!
//! Requested by [`PhysicsConfig::parallel_narrowphase`](crate::resources::PhysicsConfig) and
//! entered from [`physics_narrowphase`](crate::systems::physics_narrowphase) (and its colored
//! variant) through [`try_parallel_sets`], which returns the number of chunks it ran, or `0`
//! when it ran none and the serial loop must produce the step.
//!
//! # L10's arm (design 08 D1′)
//!
//! [`np_chunk`] and the serial loop share one route predicate, `sleep_sets::np_route`, chosen
//! by a const generic hoisted out of the loop: on the `Off` arm every pair is collided (the
//! route is constant and folds away); on the `Sets` arm a non-sensor pair with a held endpoint
//! writes the held-skip tag and an `AXIS_NONE` commit through the same two per-slot writes, and
//! each chunk stores its skip count in its own slot of a second function-local array (design
//! 06 D-D D3). No new `unsafe` site: the skip reuses the tag and commit writes.
//!
//! # Chunking (D4)
//!
//! [`chunk_count`] gives `C = min(lanes × NP_CHUNKS_PER_LANE, n / NP_MIN_PAIRS_PER_CHUNK)`,
//! clamped to [`NP_MAX_CHUNKS`], and `0` — the serial loop — when there are fewer than two
//! lanes or fewer than two chunks. `lanes` is the pool's `num_threads()`, never `+ 1`: the
//! thread that opens the scope is one of the W workers (KE16 App-1). The lanes term is what
//! keeps a one-worker world on the serial loop: at one lane the other term still yields six
//! chunks on any scene of at least 768 pairs, and each dispatch costs a `pool.scope`. Chunk `i`
//! owns the pairs `[i·n/C, (i+1)·n/C)` — equal pair counts, cut in closed form. Six chunks per
//! lane is the solver's measured setting for work stealing, and it absorbs the imbalance of the
//! SAT's early-outs; 128 pairs at the measured 0.329 µs a pair is about 42 µs a chunk, at
//! Box2D's 40 µs task floor.
//!
//! # Output: dense per-chunk runs in one ECS-owned staging column (D1)
//!
//! Chunk `c`, owning `[lo, hi)`, writes its solver manifolds upward from `stage[lo]` and its
//! sensor overlaps downward from `stage[hi − 1]`. A pair emits at most one manifold, so the two
//! runs cannot meet. After the join the calling thread appends each chunk's solver run to
//! `manifolds` and its sensor run, read in reverse, to `sensor_overlaps`, in ascending `c`
//! ([`compact`]). The staging column is `Manifolds::np_stage`, a kernel `ScratchColumn`
//! (principle 0); its length only grows, and rows outside a chunk's written runs are never
//! read.
//!
//! # Axis writes: serial, in pair order, after the join (D3)
//!
//! A chunk never writes the hysteresis table. It writes, per pair `k`, the axis the pair's
//! `set` would store, or `AXIS_NONE` for a pair that stores nothing — and also `AXIS_NONE` when
//! there was no pre-read and the hint already equals the chosen axis, because that `set` is an
//! identity (`axis_cache.rs`, Lemma 2). `BoxAxisCache::commit_axes` then replays the writes on
//! the calling thread in ascending `k`.
//!
//! # Why the streams equal the serial loop's (Lemma 3)
//!
//! The cuts partition `[0, n)` into ascending contiguous ranges. Within a chunk the solver run
//! is in ascending `k` by address, and the sensor run is in ascending `k` by descending
//! address, which the reverse read restores. The runs are joined in ascending chunk index. The
//! routing predicate is the serial loop's, `is_sensor(a) || is_sensor(b)`, and every manifold
//! is `collide_pair`'s output for the same pair and — by the axis cache's Lemma 1 — the same
//! hint. So both streams equal the serial streams, element for element and byte for byte. With
//! Lemma 2 the table state and `occupied` are equal too, so everything downstream (the graph,
//! the solve, the sleep keys) sees the same inputs: for any worker count, any partition and any
//! steal order the poses are bit-identical. The two paths are the same binary and use IEEE
//! operations only; no crate here changes MXCSR, and the pool's workers share one
//! (`boyko_threadpool/tests/mxcsr_uniform.rs`).
//!
//! # The pair carry (L9 D9)
//!
//! Each chunk joins its pairs to the previous step's tags with its own cursor
//! (`narrowphase/carry.rs`), which starts at the lower bound of the chunk's first merged key, and
//! writes each pair's tag into row `k` of this step's tag column — and, with contact reuse on
//! (L9b), a slow touching box pair's reuse record into row `k` of this step's record column. The
//! join is a function of the pair alone (lemma L9-J), so every pair reads the tag and the record
//! the serial loop reads and writes the ones the serial loop writes: the tag and record columns
//! equal the serial loop's for any partition.
//!
//! # Threads, borrows and allocation
//!
//! * **Shared and read-only while the chunks run:** the bodies, the pairs, the axis slots, the
//!   carried-axis column, the per-row orientation frames (L9 D2), and the pair carry's previous
//!   pairs, previous tags, previous records, jumper bitset and row map (L9 D9). Their writers
//!   (`begin_frame_synced`, [`prepare`]'s frame fill and carry opening, `commit_axes`) run
//!   serially on the calling thread, before and after the scope; the frames and the carry reach
//!   the chunks as shared slices taken after the writers' views are dropped.
//! * **Exclusive per chunk:** its stage rows `[lo, hi)`, its commit rows `[lo, hi)`, its tag rows
//!   `[lo, hi)`, its record rows `[lo, hi)` and its `meta` slot. The cuts are a partition, so no two
//!   tasks write one element.
//! * **Synchronisation:** `spawn` publishes the captures and the scope's join acquires every
//!   task's effects, both under the pool's own loom models. `meta` is stored and loaded
//!   `Relaxed`: its ordering comes from that join, not from the atomic. No new protocol, so no
//!   new loom model.
//! * **Tree Borrows:** a chunk writes only through `ScratchSolveView::row_ptr`, whose provenance
//!   is the column's own write base (`ScratchColumn::solve_base`). No `&[T]` or `&mut [T]` over
//!   the stage, the commit, the tags or the records exists until after the join, which is the Grid
//!   emit's rule
//!   (`BroadphaseGrid::emit_passes`). A base laundered through `as_read_slice().as_ptr()` would
//!   carry a shared-read tag and make every write UB — the shape that root-caused SP4.
//! * **Allocation:** a dispatched step opens one `pool.scope`: one boxed shared frame plus its
//!   task blocks. That is the solver's per-dispatch cost class, and one more per step is a ruled
//!   exception (lever rulings, L5 open question 1) until the allocator campaign removes scope
//!   allocation for every caller. The chunk metadata is a function-local array; the stage and
//!   the commit are grown only when the pair count exceeds every earlier dispatched step's.
//! * **Instruments:** the three zones (`phys_np_dispatch`, `phys_np_compact`,
//!   `phys_np_axis_commit`) open on the calling thread only, and the dispatch zone only after
//!   the chunk count said the step dispatches. No zone runs inside a chunk task.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use boyko_diag::zone;
use boyko_ecs::ecs::core::component::scratch::{ScratchColumn, ScratchSolveView};
use boyko_threadpool::try_with_active_pool;

use crate::manifold::{BodyIndex, Manifold};
use crate::narrowphase::axis_cache::{AXIS_NONE, AxisHints, SAT_AXIS_COUNT};
use crate::narrowphase::carry::{CarryIn, PairJoin, PairTag};
use crate::narrowphase::reuse::{ReuseRecord, RowFrame, fill_row_frames};
use crate::profiling::{PHYS_NP_AXIS_COMMIT, PHYS_NP_COMPACT, PHYS_NP_DISPATCH};
use crate::components::ColliderShape;
use crate::resources::{BodyState, Manifolds};
use crate::sleep_sets::{NpCounts, NpSets, Route, RowCls, np_route, sensor_pair};
use crate::systems::{PairOut, collide_pair};

/// Chunks per pool lane: the solver's measured work-stealing oversubscription
/// (`solver/colored.rs`'s `CHUNKS_PER_WORKER`), which also absorbs the SAT early-out imbalance.
pub const NP_CHUNKS_PER_LANE: usize = 6;

/// The fewest pairs a chunk may own: 128 pairs at the measured 0.329 µs a pair is about 42 µs,
/// Box2D's 40 µs task floor.
pub const NP_MIN_PAIRS_PER_CHUNK: usize = 128;

/// The most chunks one step dispatches; also the length of the chunk metadata array.
pub const NP_MAX_CHUNKS: usize = 256;

/// The bit offset of a chunk's solver-run length in its packed `meta` word; the sensor-run
/// length takes the low half.
const RUN_SHIFT: u32 = 32;

/// The mask of a chunk's sensor-run length in its packed `meta` word.
const RUN_MASK: u64 = (1 << RUN_SHIFT) - 1;

/// The chunk count for `n_pairs` candidate pairs on a pool of `lanes` workers, or `0` when the
/// step must run the serial loop (D4, with the `lanes < 2` term of lever ruling W1).
///
/// A benchmark that checks a run's structure re-implements this from the exported constants;
/// the two must agree on every term, the lanes term included.
#[inline]
pub(crate) const fn chunk_count(n_pairs: usize, lanes: usize) -> usize {
    if lanes < 2 {
        return 0;
    }
    let by_lanes = lanes * NP_CHUNKS_PER_LANE;
    let by_work = n_pairs / NP_MIN_PAIRS_PER_CHUNK;
    let chunks = if by_lanes < by_work { by_lanes } else { by_work };
    let chunks = if chunks > NP_MAX_CHUNKS { NP_MAX_CHUNKS } else { chunks };
    if chunks < 2 { 0 } else { chunks }
}

/// The pair range chunk `chunk` of `chunks` owns over `n_pairs` pairs: `[i·n/C, (i+1)·n/C)`.
#[inline]
pub(crate) const fn chunk_bounds(chunk: usize, chunks: usize, n_pairs: usize) -> (usize, usize) {
    (chunk * n_pairs / chunks, (chunk + 1) * n_pairs / chunks)
}

/// Packs a chunk's two run lengths into its `meta` word.
#[inline]
const fn pack_runs(n_out: usize, n_sensor: usize) -> u64 {
    ((n_out as u64) << RUN_SHIFT) | (n_sensor as u64)
}

/// Unpacks a chunk's `meta` word into `(solver run, sensor run)` lengths.
#[inline]
const fn unpack_runs(packed: u64) -> (usize, usize) {
    ((packed >> RUN_SHIFT) as usize, (packed & RUN_MASK) as usize)
}

/// What every chunk task reads and writes: `Copy`, so each task owns its copy and borrows
/// nothing from another task's frame.
///
/// `Send + Sync` without an `unsafe impl`: shared slices of POD, the pair carry's join (shared
/// slices too), the four solve views (whose own impls state the per-row discipline) and a
/// slice of atomics.
#[derive(Clone, Copy)]
pub(crate) struct NpChunkCtx<'a> {
    /// The gathered bodies, read-only.
    bodies: &'a [BodyState],
    /// The candidate pairs, read-only, in `(min, max)` order.
    pairs: &'a [(BodyIndex, BodyIndex)],
    /// The step's per-row orientation frames, read-only, or `None` when the fill declined and
    /// each box pair builds its own (L9 D2).
    frames: Option<&'a [RowFrame]>,
    /// The hysteresis hints, read-only.
    hints: AxisHints<'a>,
    /// The pair carry's join (L9 D9), read-only.
    join: PairJoin<'a>,
    /// The staging column's per-row write view.
    stage: ScratchSolveView<'a, Manifold>,
    /// The per-pair axis commit's per-row write view.
    commit: ScratchSolveView<'a, u8>,
    /// This step's pair tag column's per-row write view (L9 D9).
    tags: ScratchSolveView<'a, PairTag>,
    /// This step's reuse record column's per-row write view (L9b); written only on a step that
    /// reuses, which `PairCarry::open` grew the column for.
    records: ScratchSolveView<'a, ReuseRecord>,
    /// One packed `(solver run, sensor run)` word per chunk.
    meta: &'a [AtomicU64],
    /// L10: this step's per-row sleep-skip classification, read by the `Sets` arm's route
    /// only (design 08 D1′); empty on the `Off` arm.
    cls: &'a [RowCls],
    /// L10: one held-skip count per chunk (design 06 D-D D3), stored once by its chunk; empty
    /// on the `Off` arm, which skips nothing.
    skips: &'a [AtomicU32],
    /// L10: the restore pair source's join (design 06 B4, ruling W2) — L9's join over the kept
    /// pairs of the records restored this step, read-only; the carry's own join on the `Off`
    /// arm, which routes no pair to it.
    restore: PairJoin<'a>,
    /// L10: one `(restored pairs << 32) | restore hits` word per chunk, stored once by its chunk;
    /// empty on the `Off` arm.
    restores: &'a [AtomicU64],
}

impl<'a> NpChunkCtx<'a> {
    /// This context with L10's `Sets`-arm inputs: the step's per-row classification, one
    /// held-skip count per chunk, the restore join and one restore count word per chunk.
    #[inline]
    pub(crate) fn with_sleep(
        self,
        cls: &'a [RowCls],
        skips: &'a [AtomicU32],
        restore: PairJoin<'a>,
        restores: &'a [AtomicU64],
    ) -> Self {
        Self { cls, skips, restore, restores, ..self }
    }

    /// The step's pair carry join, which the restore join is built from.
    #[inline]
    pub(crate) fn join(&self) -> PairJoin<'a> {
        self.join
    }
}

/// Grows the staging column and the per-pair commit to at least the pair count, fills the step's
/// per-row orientation frames (L9 D2), opens the pair carry (L9 D9: the tag and record columns
/// swap, this step's tags — and its records, on a step that reuses — grow to the pair count, the
/// jumper bitset is built on a Rows step) and builds the context every chunk shares.
///
/// The columns' lengths only grow, and their fill runs only on growth, so a warm step writes
/// only the frames here (and the jumper bitset when the rows moved). The views are taken after
/// the growth: a view caches its column's length. `carry` is what `PairCarry::source` returned
/// for this step; the caller has checked that the pairs fit the tag columns' reserve. `cls` is
/// L10's per-row classification on its `Sets` arm (the frame fill's D-F mask), empty otherwise.
pub(crate) fn prepare<'a>(
    manifolds: &'a mut Manifolds,
    bodies: &'a [BodyState],
    pairs: &'a [(BodyIndex, BodyIndex)],
    prefetched: bool,
    carry: CarryIn<'a>,
    meta: &'a [AtomicU64],
    cls: &'a [RowCls],
) -> NpChunkCtx<'a> {
    let n = pairs.len();
    let Manifolds { np_stage, box_axis_cache, row_frames, pair_carry, .. } = manifolds;
    if np_stage.len() < n {
        grow_stage(np_stage, n);
    }
    let frames = fill_row_frames(row_frames, bodies, n, carry.reuse().on, cls);
    let (join, tag_column, record_column) = pair_carry.open(carry, n);
    let tag_column: &'a ScratchColumn<PairTag> = tag_column;
    let record_column: &'a ScratchColumn<ReuseRecord> = record_column;
    let np_stage: &'a ScratchColumn<Manifold> = np_stage;
    let (hints, commit) = box_axis_cache.dispatch_views(prefetched, n);
    let stage = np_stage.solve_view();
    let tags = tag_column.solve_view();
    let records = record_column.solve_view();
    debug_assert!(
        stage.len() >= n
            && commit.len() >= n
            && tags.len() >= n
            && (!join.reuse().on || records.len() >= n),
        "invariant: the stage, the commit, the tags and (on a step that reuses) the records hold \
         a row for every candidate pair"
    );
    NpChunkCtx {
        bodies,
        pairs,
        frames,
        hints,
        join,
        stage,
        commit,
        tags,
        records,
        meta,
        cls: &[],
        skips: &[],
        restore: join,
        restores: &[],
    }
}

/// Grows the staging column to `n` rows; the fill covers only the new rows.
#[cold]
#[inline(never)]
fn grow_stage(stage: &mut ScratchColumn<Manifold>, n: usize) {
    stage.build_view().resize(n, Manifold::new(BodyIndex(0), BodyIndex(0)));
}

/// Collides chunk `chunk`'s pairs `[lo, hi)`: each manifold into the chunk's staging rows, each
/// pair's axis into its commit row, its tag into its tag row and its reuse record, if it wrote one,
/// into its record row (L9 D9), and the two run lengths into `meta[chunk]`.
///
/// `SETS` is L10's arm (design 08 D1′): on the `Sets` arm a non-sensor pair with a held endpoint
/// is not collided — its slot takes the held-skip tag and commits no axis, through the same two
/// writes a collided pair makes, and it stages no manifold — and the chunk stores its skip count
/// into `skips[chunk]`. On the `Off` arm every pair is collided, as before L10.
///
/// # Safety
/// * `ctx` came from [`prepare`] for these pairs, so `hi <= ctx.pairs.len()` is within the stage,
///   the commit and the tag views — and within the record view on a step that reuses, the only
///   step that writes a record — and `lo <= hi`.
/// * While this call runs, no other thread writes or reads rows `[lo, hi)` of the stage, the
///   commit, the tags or the records, or `meta[chunk]`: the concurrent chunks' ranges are pairwise
///   disjoint and their indices distinct, which a partition of `[0, n)` into ascending cuts
///   guarantees.
/// * Nothing holds a slice over the stage, the commit, the tags or the records until every chunk
///   of the step has returned (the scope's join, or `std::thread::scope`'s).
pub(crate) unsafe fn np_chunk<const SETS: bool>(
    ctx: NpChunkCtx<'_>,
    chunk: usize,
    lo: usize,
    hi: usize,
) {
    debug_assert!(
        lo <= hi && hi <= ctx.pairs.len(),
        "invariant: a chunk owns a sub-range of the candidate pairs"
    );
    let prefetched = ctx.hints.prefetched();
    let reuse = ctx.join.reuse();
    // L9 D9: this chunk's own join cursor; it finds its start by one binary search. L10's
    // restore cursor (design 06 B4, ruling W2) is the same join over the restore source, with a
    // cursor of its own that positions itself at the chunk's first restored key.
    let mut join = ctx.join.cursor();
    let mut restore = ctx.restore.cursor();
    let mut wo = lo;
    let mut ws = hi;
    let mut held_skips = 0u32;
    let (mut restored, mut restore_hits) = (0u32, 0u32);
    for k in lo..hi {
        let (a, b) = ctx.pairs[k];
        let ba = &ctx.bodies[a.0 as usize];
        let bb = &ctx.bodies[b.0 as usize];
        let (PairOut { manifold, tag, record, .. }, commit) = match np_route::<SETS>(ctx.cls, a, b) {
            // L10 (design 08 D1′): a held endpoint. The axis commit is AXIS_NONE (the mirror
            // keys the held pair serially before the scope, design 06 D-C) and the tag is the
            // held-skip tag.
            Route::Skip => {
                held_skips += 1;
                (PairOut::held_skip(), AXIS_NONE)
            }
            route @ (Route::Stream | Route::Restore) => {
                if SETS {
                    debug_assert_computed(ctx.cls, ctx.frames.is_some(), a, b, ba, bb);
                }
                let from_restore = route == Route::Restore;
                let mut hint = None;
                let mut hit = false;
                let out = collide_pair(
                    a,
                    b,
                    ba,
                    bb,
                    ctx.frames,
                    reuse,
                    || {
                        if from_restore {
                            let prev = restore.prev(a, b);
                            hit = prev.tag != PairTag::NONE;
                            prev
                        } else {
                            join.prev(a, b)
                        }
                    },
                    || {
                        hint = ctx.hints.read(k, a, b);
                        hint
                    },
                );
                restored += u32::from(from_restore);
                restore_hits += u32::from(hit);
                // The skip rule (D3): without a pre-read, a pair whose hint already names the
                // axis it chose would `set` the value its slot holds — an identity (Lemma 2).
                let commit = match out.axis {
                    Some(chosen) if prefetched || hint != Some(chosen) => {
                        debug_assert!(
                            chosen < usize::from(SAT_AXIS_COUNT),
                            "invariant: a chosen SAT axis is 0..15"
                        );
                        chosen as u8
                    }
                    _ => AXIS_NONE,
                };
                (out, commit)
            }
        };
        // SAFETY: `lo <= k < hi <= commit.len()` (the caller's contract and `prepare`'s
        //   growth), so row `k` is in bounds; this chunk owns `[lo, hi)` and no other thread
        //   touches that row while it runs (the caller's contract). The base carries the
        //   column's own write provenance (`solve_base`), never a slice reborrow. `u8` has no
        //   drop glue, so overwriting last step's byte is a plain store. A pair L10 skipped
        //   writes `AXIS_NONE` through this same store, under the same ownership.
        unsafe { ctx.commit.row_ptr(k).write(commit) };
        // SAFETY: `lo <= k < hi <= tags.len()` (the caller's contract and `prepare`'s growth of
        //   the tag column to the pair count), so row `k` is in bounds; this chunk owns `[lo, hi)`
        //   and no other thread touches that row while it runs (the caller's contract). The base
        //   is the column's own write base (`solve_base`), never a slice reborrow, and no slice
        //   over the tag column exists until the scope has joined. `PairTag` is a `u16` with no
        //   drop glue, so overwriting last step's value is a plain store. A pair L10 skipped
        //   writes `PairTag::HELD_SKIP` through this same store, under the same ownership.
        unsafe { ctx.tags.row_ptr(k).write(tag) };
        if let Some(record) = record {
            // SAFETY: a record exists only on a step that reuses, and `prepare` then asserted,
            //   after `PairCarry::open` grew the record column to the pair count, that
            //   `lo <= k < hi <= records.len()`, so row `k` is in bounds; this chunk owns
            //   `[lo, hi)` and no other thread touches that row while it runs (the caller's
            //   contract). The base is the column's own write base (`solve_base`), never a slice
            //   reborrow, and no slice over the record column exists until the scope has joined.
            //   `ReuseRecord` is `Copy` (no drop glue), so overwriting a stale row is a plain
            //   store.
            unsafe { ctx.records.row_ptr(k).write(record) };
        }

        if let Some(manifold) = manifold {
            debug_assert!(
                (manifold.count as usize) <= crate::math::MAX_CONTACT_POINTS,
                "invariant: manifold.count must not exceed MAX_CONTACT_POINTS"
            );
            if manifold.count > 0 {
                // The serial loop's routing predicate, evaluated per pair.
                if ba.is_sensor || bb.is_sensor {
                    ws -= 1;
                    // SAFETY: at most one manifold per pair, so the pairs before `k` in
                    //   this chunk wrote `(wo - lo) + (hi - ws_before) <= k - lo` rows and
                    //   `wo <= ws < hi` holds after the decrement: row `ws` lies in this
                    //   chunk's own `[lo, hi)`, below `stage.len()`, touched by no other
                    //   thread (the caller's contract), through the column's write base.
                    //   `Manifold` is `Copy`, so the stale row needs no drop.
                    unsafe { ctx.stage.row_ptr(ws).write(manifold) };
                } else {
                    // SAFETY: as for the sensor write: `lo <= wo < ws <= hi` before this
                    //   write, so row `wo` lies in this chunk's own range and below the
                    //   sensor run; exclusive to this thread, through the write base.
                    unsafe { ctx.stage.row_ptr(wo).write(manifold) };
                    wo += 1;
                }
            }
        }
    }
    debug_assert!(wo <= ws, "invariant: a chunk's solver and sensor runs never meet");
    // Relaxed: the reader is the calling thread after the scope's join, which orders this
    // store (and every staging write above) before its load.
    ctx.meta[chunk].store(pack_runs(wo - lo, hi - ws), Ordering::Relaxed);
    if SETS {
        // Relaxed, as `meta`: the scope's join orders it before the caller's load.
        ctx.skips[chunk].store(held_skips, Ordering::Relaxed);
        // Relaxed, as `meta`.
        ctx.restores[chunk]
            .store((u64::from(restored) << 32) | u64::from(restore_hits), Ordering::Relaxed);
    } else {
        debug_assert!(restored == 0, "invariant: the Off arm routes no pair to the kept source");
    }
}

/// The two debug checks of a computed pair on L10's `Sets` arm: D-F (design 08) — a box pair
/// reading the step's frames reads two rows the fill wrote (`RowCls::fills`) — and the D-G tie of
/// the `SENSOR` flag to the bodies' own sensor bit (O-3, through `sensor_pair`).
#[inline]
pub(crate) fn debug_assert_computed(
    cls: &[RowCls],
    frames: bool,
    a: BodyIndex,
    b: BodyIndex,
    ba: &BodyState,
    bb: &BodyState,
) {
    let (ca, cb) = (cls[a.0 as usize], cls[b.0 as usize]);
    debug_assert!(
        sensor_pair(ca.flags, 0) == ba.is_sensor && sensor_pair(0, cb.flags) == bb.is_sensor,
        "invariant: the SENSOR flag is the body's sensor bit (design 08 D-G)"
    );
    let boxes = matches!(ba.shape, ColliderShape::Box { .. }) && matches!(bb.shape, ColliderShape::Box { .. });
    debug_assert!(
        !frames || !boxes || (ca.fills() && cb.fills()),
        "invariant: a computed box pair reads frames the fill wrote (design 08 D-F): {ca:?} {cb:?}"
    );
    let _ = (ca, cb, boxes, frames);
}

/// Joins the chunks' runs into the two streams, in ascending chunk order: each solver run
/// appended as written, each sensor run read from its top row down (Lemma 3).
///
/// `bounds(c)` must return the pair range chunk `c` owned. Runs on the calling thread after
/// every chunk has returned.
pub(crate) fn compact(
    manifolds: &mut Manifolds,
    chunks: usize,
    bounds: impl Fn(usize) -> (usize, usize),
    meta: &[AtomicU64],
) {
    let Manifolds { manifolds: out, sensor_overlaps, np_stage, .. } = manifolds;
    let stage = np_stage.as_read_slice();
    let mut out = out.build_view();
    out.clear();
    let mut sensor = sensor_overlaps.build_view();
    sensor.clear();
    let mut emitted = 0usize;
    for (chunk, packed) in meta.iter().enumerate().take(chunks) {
        let (lo, hi) = bounds(chunk);
        // Relaxed: ordered after the chunk's store by the join (see `np_chunk`).
        let (n_out, n_sensor) = unpack_runs(packed.load(Ordering::Relaxed));
        debug_assert!(
            n_out + n_sensor <= hi - lo,
            "invariant: a chunk's two runs fit its own rows"
        );
        out.extend_from_slice(&stage[lo..lo + n_out]);
        for row in (hi - n_sensor..hi).rev() {
            sensor.push(stage[row]);
        }
        emitted += n_out + n_sensor;
    }
    debug_assert_eq!(
        out.len() + sensor.len(),
        emitted,
        "invariant: the streams hold exactly the manifolds the chunks reported"
    );
}

/// [`try_parallel_sets`] on the `Off` arm, returning the chunk count: for the tests' direct
/// callers.
#[cfg(test)]
pub(crate) fn try_parallel(
    manifolds: &mut Manifolds,
    bodies: &[BodyState],
    pairs: &[(BodyIndex, BodyIndex)],
    prefetched: bool,
    carry: CarryIn<'_>,
) -> usize {
    try_parallel_sets::<false>(manifolds, bodies, pairs, prefetched, carry, NpSets::OFF).0
}

/// The parallel narrowphase for one step. Returns the number of chunks it ran — `0` when it ran
/// none: no pool, fewer than two lanes, fewer than two chunks, or more pairs than the stage can
/// hold, and the caller must run the serial loop — and what L10's `Sets` arm counted (all zero
/// on the `Off` arm).
///
/// `pairs` is the stream, so the chunk count is the stream's (design 06 D-D D2). `prefetched`
/// is what `BoxAxisCache::begin_frame_synced` returned for this frame; it must have run first.
/// `carry` is what `PairCarry::source` returned for it (L9 D9). `SETS` is L10's arm, and `np`
/// its inputs — the step's per-row classification and the restore pair source — read on the
/// `Sets` arm only.
pub(crate) fn try_parallel_sets<const SETS: bool>(
    manifolds: &mut Manifolds,
    bodies: &[BodyState],
    pairs: &[(BodyIndex, BodyIndex)],
    prefetched: bool,
    carry: CarryIn<'_>,
    np: NpSets<'_>,
) -> (usize, NpCounts) {
    let n = pairs.len();
    try_with_active_pool(|pool| {
        let chunks = chunk_count(n, pool.num_threads());
        if chunks == 0 {
            return (0, NpCounts::default());
        }
        if n > manifolds.np_stage.capacity()
            || n > manifolds.box_axis_cache.commit_capacity()
            || n > manifolds.pair_carry.capacity()
        {
            return (beyond_stage_ceiling(), NpCounts::default());
        }
        debug_assert!(chunks <= NP_MAX_CHUNKS, "invariant: chunk_count clamps to NP_MAX_CHUNKS");
        let meta: [AtomicU64; NP_MAX_CHUNKS] = [const { AtomicU64::new(0) }; NP_MAX_CHUNKS];
        // L10 (design 06 D-D D3): the `Sets` arm's per-chunk skip and restore counts,
        // function-local arrays like `meta`, summed after the join. The `Off` arm keeps none.
        let skip_counts: [AtomicU32; NP_MAX_CHUNKS];
        let restore_counts: [AtomicU64; NP_MAX_CHUNKS];
        let (skips, restores): (&[AtomicU32], &[AtomicU64]) = if SETS {
            skip_counts = [const { AtomicU32::new(0) }; NP_MAX_CHUNKS];
            restore_counts = [const { AtomicU64::new(0) }; NP_MAX_CHUNKS];
            (&skip_counts[..chunks], &restore_counts[..chunks])
        } else {
            (&[], &[])
        };
        {
            let _zone = zone!(PHYS_NP_DISPATCH);
            // `ctx.meta` is cut to the step's chunk count, so a task reads `chunks` and `n`
            // from the context and captures two words: `&ctx` (it is `Sync` and outlives the
            // scope) and its chunk index. A task cell is then 16 bytes of header plus 16 of
            // closure, and 128 of them fit the scope's first 4 KiB block — above the 96
            // chunks of a lane-bound W = 16 step. Capturing the 128-byte context by value
            // with the cuts would have been 168 bytes a cell: 24 a block, so W = 8 (48
            // chunks) took two blocks and W = 16 three. C4's census pins the block count.
            let ctx = prepare(manifolds, bodies, pairs, prefetched, carry, &meta[..chunks], np.cls);
            // L10 (ruling W2): the restore route is L9's join over the restore pair source.
            let restore = ctx.join().restored(np.keys, np.tags, np.reuse);
            let ctx = ctx.with_sleep(np.cls, skips, restore, restores);
            let ctx = &ctx;
            pool.scope(|scope| {
                for chunk in 0..chunks {
                    scope.spawn(move || {
                        let chunks = ctx.meta.len();
                        let n = ctx.pairs.len();
                        let (lo, hi) = chunk_bounds(chunk, chunks, n);
                        debug_assert!(
                            (chunk == 0 || lo == chunk_bounds(chunk - 1, chunks, n).1)
                                && (chunk + 1 < chunks || hi == n),
                            "invariant: the cuts partition [0, n)"
                        );
                        // SAFETY: `ctx` came from `prepare` for these pairs; the closed-form
                        //   cuts partition `[0, n)` into ascending, disjoint ranges with
                        //   distinct chunk indices, so no two tasks share a stage row, a
                        //   commit row, a tag row, a record row or a `meta` slot; nothing takes
                        //   a slice over the stage, the commit, the tags or the records until
                        //   `pool.scope` has joined every task. On the `Sets` arm `ctx.skips`
                        //   and `ctx.restores` hold one slot per chunk, and a chunk stores only
                        //   its own; the restore join is shared read-only, like the carry's.
                        unsafe { np_chunk::<SETS>(*ctx, chunk, lo, hi) }
                    });
                }
            });
        }
        {
            let _zone = zone!(PHYS_NP_COMPACT);
            compact(manifolds, chunks, |c| chunk_bounds(c, chunks, n), &meta);
        }
        {
            let _zone = zone!(PHYS_NP_AXIS_COMMIT);
            manifolds.box_axis_cache.commit_axes(pairs);
        }
        manifolds.note_np_dispatch();
        // Relaxed: the scope's join orders every chunk's store before these loads.
        let mut counts = NpCounts {
            skips: skips.iter().map(|s| s.load(Ordering::Relaxed)).sum(),
            ..NpCounts::default()
        };
        for word in restores {
            let w = word.load(Ordering::Relaxed);
            counts.restored += (w >> 32) as u32;
            counts.restore_hits += w as u32;
        }
        (chunks, counts)
    })
    .unwrap_or((0, NpCounts::default()))
}

/// The step has more candidate pairs than the staging column (7.06M rows natively), the
/// per-pair commit or the pair tag columns can hold: run the serial loop, which tags the pairs
/// that fit and leaves the carry's stamp invalid.
///
/// The serial loop's `manifolds` has the SAME reserve as the stage (`Manifolds::with_capacity`),
/// so the ceiling is shared; what differs is how much of it each path needs. [`prepare`] grows
/// the stage to one row per candidate pair before any pair is collided — past the reserve that
/// resize is a "reserve ceiling exhausted" panic — while `manifolds` grows only by what the
/// step emits: at most one row per pair, and about half of the pairs on the measured piles
/// (4,515 manifolds from 9,559 pairs on the Jolt pyramid). So a step the stage cannot hold is
/// still one the serial loop can finish, and the fallback turns a panic into a serial step.
/// It cannot save a step whose EMITTED count crosses the ceiling; no dispatch decision can.
#[cold]
#[inline(never)]
fn beyond_stage_ceiling() -> usize {
    0
}

#[cfg(test)]
mod tests {
    #[cfg(not(miri))]
    use std::cell::Cell;

    #[cfg(not(miri))]
    use proptest::prelude::*;

    use super::*;
    use crate::components::{Collider, ColliderShape, RigidBody, RigidBodyMass};
    use crate::math::{Mat3, Quat, Vec3};
    #[cfg(not(miri))]
    use crate::narrowphase::carry::jumper_words;
    #[cfg(not(miri))]
    use crate::narrowphase::reuse::Prev;
    use crate::narrowphase::reuse::ReuseStep;
    use crate::row_identity::RowRemap;
    #[cfg(not(miri))]
    use crate::row_identity::NO_ROW;
    use crate::systems::{narrowphase_serial_sets, narrowphase_serial_with};

    /// D4 and ruling W1: the lanes term, the work term, the cap and the "two chunks or none"
    /// floor, on the P0 pyramid's pair count.
    #[test]
    fn chunk_count_follows_the_dispatch_conditions() {
        const JOLT_PAIRS: usize = 9_561;
        assert_eq!(chunk_count(JOLT_PAIRS, 1), 0, "one lane runs the serial loop");
        assert_eq!(chunk_count(JOLT_PAIRS, 0), 0, "no lane runs the serial loop");
        assert_eq!(chunk_count(JOLT_PAIRS, 2), 12, "lane-bound: 2 x 6");
        assert_eq!(chunk_count(JOLT_PAIRS, 8), 48, "lane-bound: 8 x 6");
        assert_eq!(chunk_count(JOLT_PAIRS, 16), 74, "work-bound: 9561 / 128");
        assert_eq!(chunk_count(255, 8), 0, "one chunk's worth of pairs runs the serial loop");
        assert_eq!(chunk_count(256, 8), 2, "two chunks' worth dispatches");
        assert_eq!(chunk_count(1 << 24, 64), NP_MAX_CHUNKS, "the cap");
        for chunks in [2usize, 3, 7, 48] {
            let n = 1_001;
            assert_eq!(chunk_bounds(0, chunks, n).0, 0);
            assert_eq!(chunk_bounds(chunks - 1, chunks, n).1, n);
            for c in 1..chunks {
                assert_eq!(chunk_bounds(c, chunks, n).0, chunk_bounds(c - 1, chunks, n).1);
            }
        }
        assert_eq!(unpack_runs(pack_runs(7, 3)), (7, 3));
    }

    /// No pool is attached to a test thread, so the dispatch declines and leaves the counter
    /// flat.
    #[test]
    fn without_a_pool_the_dispatch_declines() {
        let mut rng = Rng::new(3);
        let scene = Scene::random(&mut rng, 40);
        assert!(
            chunk_count(scene.pairs.len(), 8) >= 2,
            "construction: enough pairs to dispatch on a pool"
        );
        let mut m = Manifolds::with_capacity(scene.pairs.len());
        m.box_axis_cache.begin_frame(scene.pairs.len());
        assert_eq!(try_parallel(&mut m, &scene.bodies, &scene.pairs, false, CarryIn::NONE), 0);
        assert_eq!(m.narrowphase_dispatches(), 0);
    }

    // ── Scene and frame generators ───────────────────────────────────────────────

    /// xorshift64*: a seeded, dependency-free generator for the scene draws.
    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Self {
            Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
        }

        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }

        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            let unit = (self.next() >> 40) as f32 / (1u64 << 24) as f32;
            lo + (hi - lo) * unit
        }
    }

    /// Bodies packed into a small cube so most pairs touch, and a strictly ascending subset of
    /// their `(min, max)` pairs.
    struct Scene {
        bodies: Vec<BodyState>,
        pairs: Vec<(BodyIndex, BodyIndex)>,
    }

    impl Scene {
        fn random(rng: &mut Rng, n_bodies: usize) -> Self {
            let bodies: Vec<BodyState> = (0..n_bodies)
                .map(|_| {
                    let shape = if rng.below(3) == 0 {
                        ColliderShape::Sphere { radius: rng.range(0.3, 0.7) }
                    } else {
                        ColliderShape::Box {
                            half_extents: Vec3::new(
                                rng.range(0.3, 0.7),
                                rng.range(0.3, 0.7),
                                rng.range(0.3, 0.7),
                            ),
                        }
                    };
                    let rotation = Quat::new(
                        rng.range(-1.0, 1.0),
                        rng.range(-1.0, 1.0),
                        rng.range(-1.0, 1.0),
                        rng.range(0.2, 1.0),
                    )
                    .normalize();
                    let body = RigidBody {
                        position: Vec3::new(
                            rng.range(-1.2, 1.2),
                            rng.range(-1.2, 1.2),
                            rng.range(-1.2, 1.2),
                        ),
                        linear_velocity: Vec3::ZERO,
                        rotation,
                        angular_velocity: Vec3::ZERO,
                    };
                    let mass = RigidBodyMass {
                        inv_inertia: Mat3::IDENTITY,
                        inv_mass: 1.0,
                        restitution: 0.0,
                        friction: 0.5,
                    };
                    let collider = Collider { shape, layer: 1, mask: 1 };
                    let sensor = rng.below(6) == 0;
                    BodyState::from_columns(&body, &mass, &collider, sensor, true, false)
                })
                .collect();
            let mut pairs = Vec::new();
            for i in 0..n_bodies as u32 {
                for j in i + 1..n_bodies as u32 {
                    if rng.below(8) != 0 {
                        pairs.push((BodyIndex(i), BodyIndex(j)));
                    }
                }
            }
            Self { bodies, pairs }
        }

        #[cfg(not(miri))]
        fn is_box(&self, row: BodyIndex) -> bool {
            matches!(self.bodies[row.0 as usize].shape, ColliderShape::Box { .. })
        }
    }

    /// One frame's table inputs: extra `set`s applied before the frame (T₀ beyond what earlier
    /// frames left), whether it is a pre-read frame, and the carried axes if so.
    struct Frame {
        prefill: Vec<(u32, u32, usize)>,
        prefetched: bool,
        remapped: Vec<u8>,
    }

    impl Frame {
        /// At most `max_fill` extra `set`s: the caller leaves room for the frame's own inserts
        /// (one per pair), so every insert still finds an `EMPTY` slot, as it does in the
        /// system, where `begin_frame` bounds the load.
        fn random(rng: &mut Rng, scene: &Scene, max_fill: usize) -> Self {
            let n_bodies = scene.bodies.len() as u64;
            let fill = rng.below(max_fill as u64 + 1) as usize;
            let prefill = (0..fill)
                .map(|_| {
                    let axis = rng.below(u64::from(SAT_AXIS_COUNT)) as usize;
                    if !scene.pairs.is_empty() && rng.below(2) == 0 {
                        let (a, b) = scene.pairs[rng.below(scene.pairs.len() as u64) as usize];
                        (a.0, b.0, axis)
                    } else {
                        let a = rng.below(n_bodies + 8) as u32;
                        let b = a + 1 + rng.below(8) as u32;
                        (a, b, axis)
                    }
                })
                .collect();
            let prefetched = rng.below(2) == 0;
            let remapped = (0..scene.pairs.len())
                .map(|_| {
                    if rng.below(3) == 0 {
                        AXIS_NONE
                    } else {
                        rng.below(u64::from(SAT_AXIS_COUNT)) as u8
                    }
                })
                .collect();
            Self { prefill, prefetched, remapped }
        }

        /// Brings `m`'s table, already through `begin_frame`, to this frame's T₀: the extra
        /// `set`s and the carried axes.
        fn apply(&self, m: &mut Manifolds) {
            for &(a, b, axis) in &self.prefill {
                m.box_axis_cache.set(BodyIndex(a), BodyIndex(b), axis);
            }
            if self.prefetched {
                m.box_axis_cache.seed_remapped(&self.remapped);
            }
        }
    }

    /// A manifold as words, every field and every point slot: `Manifold` has no `PartialEq`,
    /// and the gate compares bits.
    fn manifold_words(m: &Manifold) -> Vec<u32> {
        let mut w = vec![m.body_a.0, m.body_b.0, u32::from(m.count)];
        w.extend([m.normal.x, m.normal.y, m.normal.z].map(f32::to_bits));
        for p in &m.points {
            w.extend(
                [p.anchor_a.x, p.anchor_a.y, p.anchor_a.z, p.anchor_b.x, p.anchor_b.y, p.anchor_b.z]
                    .map(f32::to_bits),
            );
            w.extend([p.separation.to_bits(), p.feature_id]);
        }
        w
    }

    /// Everything the theorem says equal: both streams, the table state, `occupied`, the
    /// fingerprint, the pair tags (L9 D9) and every record a tag says was written (L9b).
    #[derive(Debug, PartialEq)]
    struct Outcome {
        out: Vec<Vec<u32>>,
        sensor: Vec<Vec<u32>>,
        table: (Vec<(u64, u32)>, usize),
        fingerprint: u64,
        tags: Vec<u16>,
        records: Vec<(usize, [u32; 32])>,
    }

    fn outcome(m: &Manifolds) -> Outcome {
        let tags = m.pair_carry.tags();
        let records = m.pair_carry.records();
        Outcome {
            out: m.solver_manifolds().iter().map(manifold_words).collect(),
            sensor: m.sensor_overlaps().iter().map(manifold_words).collect(),
            table: m.box_axis_cache.table_state(),
            fingerprint: m.box_axis_cache.fingerprint(),
            tags: tags.iter().map(|t| t.bits()).collect(),
            records: tags
                .iter()
                .enumerate()
                .filter(|(_, t)| t.has(PairTag::REC))
                .map(|(k, _)| (k, records[k].words()))
                .collect(),
        }
    }

    /// The step of the frames that reuse contacts (L9b), and their reuse distance.
    const DT: f32 = 1.0 / 60.0;
    /// See [`DT`].
    const TAU: f32 = 1.0e-3;

    /// The frame's contact reuse: on or off, with the table's key-set change `serial`'s
    /// `begin_frame` just settled (both tables are equal at T₀).
    fn reuse_step(on: bool, serial: &Manifolds) -> ReuseStep {
        ReuseStep::new(on, TAU, DT, serial.box_axis_cache.keys_changed())
    }

    /// The parallel path over an explicit partition (`cuts` holds every boundary, `0` and `n`
    /// included), its chunks run on this thread in `order`.
    #[cfg(not(miri))]
    fn run_partition(
        m: &mut Manifolds,
        scene: &Scene,
        prefetched: bool,
        carry: CarryIn<'_>,
        cuts: &[usize],
        order: &[usize],
    ) {
        let chunks = cuts.len() - 1;
        let meta: Vec<AtomicU64> = (0..chunks).map(|_| AtomicU64::new(0)).collect();
        let ctx = prepare(m, &scene.bodies, &scene.pairs, prefetched, carry, &meta, &[]);
        for &chunk in order {
            // SAFETY: `ctx` came from `prepare` for these pairs; `cuts` is ascending from 0 to
            //   n, so the chunks' ranges are disjoint, and they run one after another on this
            //   thread; no slice over the stage, the commit, the tags or the records is taken
            //   until `compact`.
            unsafe { np_chunk::<false>(ctx, chunk, cuts[chunk], cuts[chunk + 1]) };
        }
        compact(m, chunks, |c| (cuts[c], cuts[c + 1]), &meta);
        m.box_axis_cache.commit_axes(&scene.pairs);
    }

    /// Runs `begin_frame(n)` on both tables, as the system would (grow or clear), and returns
    /// how many extra `set`s the frame may add: a quarter of the table at most (a small table,
    /// so home slots collide), and never so many that the frame's own inserts could fill it.
    fn begin_frames(serial: &mut Manifolds, parallel: &mut Manifolds, n: usize) -> usize {
        serial.box_axis_cache.begin_frame(n);
        parallel.box_axis_cache.begin_frame(n);
        let (slots, occupied) = serial.box_axis_cache.table_state();
        slots.len().saturating_sub(occupied + n + 1).min(slots.len() / 4)
    }

    /// A random contiguous partition of `[0, n)`: 1 to 8 chunks, empty chunks allowed.
    #[cfg(not(miri))]
    fn random_cuts(rng: &mut Rng, n: usize) -> Vec<usize> {
        let chunks = 1 + rng.below(8) as usize;
        let mut inner: Vec<usize> = (1..chunks).map(|_| rng.below(n as u64 + 1) as usize).collect();
        inner.sort_unstable();
        let mut cuts = Vec::with_capacity(chunks + 1);
        cuts.push(0);
        cuts.extend(inner);
        cuts.push(n);
        cuts
    }

    /// A random run order of `chunks` chunks (a Fisher-Yates shuffle).
    #[cfg(not(miri))]
    fn random_order(rng: &mut Rng, chunks: usize) -> Vec<usize> {
        let mut order: Vec<usize> = (0..chunks).collect();
        for i in (1..chunks).rev() {
            order.swap(i, rng.below(i as u64 + 1) as usize);
        }
        order
    }

    /// What the proptest's cases exercised, summed, so the gate can refuse a vacuous run.
    #[cfg(not(miri))]
    #[derive(Clone, Copy, Default, Debug)]
    struct Coverage {
        skipped_identity_sets: u64,
        prefetched_box_contacts: u64,
        sensor_manifolds: u64,
        box_sphere_flips: u64,
        multi_chunk_frames: u64,
        stale_stage_frames: u64,
        /// Pairs the carried separating axis rejected (L9a (ii)), in the serial loop.
        carried_sep_hits: u64,
        /// Of those, pairs in a chunk that starts past pair 0 whose joined slot lies below the
        /// chunk's first pair: the M-c4 witness (a cursor started at slot `lo` misses them).
        sep_hits_behind_chunk_start: u64,
        /// Pairs joined across a row-order flip (a Rows frame).
        flipped_joins: u64,
        /// Pairs joined by the jumper search (a Rows frame).
        jumper_joins: u64,
        /// Pairs whose output came from their reuse record (L9b), in the serial loop.
        reuse_hits: u64,
        /// Of those, pairs joined across a row-order flip: the record read flipped (ruling W2).
        flipped_reuse_hits: u64,
        /// Of those, pairs in a chunk that starts past pair 0 whose joined slot lies below the
        /// chunk's first pair (M-c4 on the records).
        reuse_hits_behind_chunk_start: u64,
        /// Pairs that built a record from their full collision (L9b misses).
        records_built: u64,
    }

    /// How a random frame's pairs join the previous frame's tags (G-C-2).
    #[cfg(not(miri))]
    enum CarryKind {
        /// The previous frame's rows are this frame's.
        Identity,
        /// The rows moved: `prev_row`, the rows stage 2 resolved and their jumper bitset.
        Rows { prev_row: Vec<u32>, stage2: Vec<u32>, jumpers: Vec<u64> },
        /// No carry.
        Reset,
    }

    #[cfg(not(miri))]
    impl CarryKind {
        /// A random kind for a frame of `n` rows after one of `m` rows: Identity half the time,
        /// Rows three times in eight, Reset otherwise.
        fn random(rng: &mut Rng, n: usize, m: usize) -> Self {
            match rng.below(8) {
                0..=3 => Self::Identity,
                4..=6 => {
                    let (prev_row, stage2) = random_rows_map(rng, n, m);
                    let jumpers = jumper_words(n, &stage2);
                    Self::Rows { prev_row, stage2, jumpers }
                }
                _ => Self::Reset,
            }
        }

        fn carry<'a>(&'a self, pairs_prev: &'a [(BodyIndex, BodyIndex)]) -> CarryIn<'a> {
            match self {
                Self::Identity => CarryIn::new(RowRemap::Identity, pairs_prev, &[]),
                Self::Rows { prev_row, jumpers, .. } => {
                    CarryIn::new(RowRemap::Rows(prev_row), pairs_prev, jumpers)
                }
                Self::Reset => CarryIn::NONE,
            }
        }

        /// The join by its definition (lemma L9-J), with no cursor: the slot of `pairs_prev` equal
        /// to the pair's previous rows in either order, and whether the order flipped.
        fn expected_join(
            &self,
            pairs_prev: &[(BodyIndex, BodyIndex)],
            a: BodyIndex,
            b: BodyIndex,
        ) -> Option<(usize, bool)> {
            let (pa, pb) = match self {
                Self::Identity => (a.0, b.0),
                Self::Rows { prev_row, .. } => (prev_row[a.0 as usize], prev_row[b.0 as usize]),
                Self::Reset => return None,
            };
            if pa == NO_ROW || pb == NO_ROW {
                return None;
            }
            let key = (BodyIndex(pa.min(pb)), BodyIndex(pa.max(pb)));
            pairs_prev.binary_search(&key).ok().map(|j| (j, pa > pb))
        }

        fn is_jumper_pair(&self, a: BodyIndex, b: BodyIndex) -> bool {
            match self {
                Self::Rows { stage2, .. } => stage2.contains(&a.0) || stage2.contains(&b.0),
                Self::Identity | Self::Reset => false,
            }
        }
    }

    /// A random previous-row map of `n` current rows over `m` previous rows, with the rows it
    /// calls stage-2 resolved: the rows outside the list that held a row carry strictly increasing
    /// previous rows (what `RowIdentity::stage2_rows` guarantees), the listed rows take unused
    /// previous rows in any order, or none. Injective.
    #[cfg(not(miri))]
    fn random_rows_map(rng: &mut Rng, n: usize, m: usize) -> (Vec<u32>, Vec<u32>) {
        let mut prev_row = vec![NO_ROW; n];
        let mut used = vec![false; m];
        let mut stage2 = Vec::new();
        let mut next = 0usize;
        for (r, slot) in prev_row.iter_mut().enumerate() {
            match rng.below(8) {
                0 => {}
                1 | 2 => stage2.push(r as u32),
                _ => {
                    next += rng.below(2) as usize;
                    if next < m {
                        *slot = next as u32;
                        used[next] = true;
                        next += 1;
                    }
                }
            }
        }
        let mut free: Vec<u32> = (0..m as u32).filter(|&p| !used[p as usize]).collect();
        for i in (1..free.len()).rev() {
            free.swap(i, rng.below(i as u64 + 1) as usize);
        }
        for &r in &stage2 {
            if rng.below(4) != 0
                && let Some(p) = free.pop()
            {
                prev_row[r as usize] = p;
            }
        }
        (prev_row, stage2)
    }

    /// Recomputes, from T₀, which pairs of this frame the skip rule will leave out and which
    /// pre-read pairs will commit, and counts the frame's sensor and flip manifolds.
    #[cfg(not(miri))]
    fn cover(cov: &mut Coverage, m: &Manifolds, scene: &Scene, frame: &Frame, cuts: &[usize]) {
        for (k, &(a, b)) in scene.pairs.iter().enumerate() {
            let ba = &scene.bodies[a.0 as usize];
            let bb = &scene.bodies[b.0 as usize];
            let hint = m.box_axis_cache.read_hint(frame.prefetched, k, a, b);
            let PairOut { manifold, axis, .. } =
                collide_pair(a, b, ba, bb, None, ReuseStep::OFF, || Prev::NONE, || hint);
            if let Some(axis) = axis {
                if frame.prefetched {
                    cov.prefetched_box_contacts += 1;
                } else if hint == Some(axis) {
                    cov.skipped_identity_sets += 1;
                }
            }
            if let Some(m) = manifold.filter(|m| m.count > 0) {
                if ba.is_sensor || bb.is_sensor {
                    cov.sensor_manifolds += 1;
                }
                if scene.is_box(a) && !scene.is_box(b) && m.body_a == a {
                    cov.box_sphere_flips += 1;
                }
            }
        }
        if cuts.windows(2).filter(|w| w[1] > w[0]).count() >= 2 {
            cov.multi_chunk_frames += 1;
        }
    }

    /// Counts, from the serial loop's tags, what the frame's carry exercised.
    #[cfg(not(miri))]
    fn cover_carry(
        cov: &mut Coverage,
        serial: &Manifolds,
        scene: &Scene,
        kind: &CarryKind,
        pairs_prev: &[(BodyIndex, BodyIndex)],
        cuts: &[usize],
    ) {
        let tags = serial.pair_carry.tags();
        for (c, w) in cuts.windows(2).enumerate() {
            let (lo, hi) = (w[0], w[1]);
            for (&(a, b), tag) in scene.pairs[lo..hi].iter().zip(&tags[lo..hi]) {
                let join = kind.expected_join(pairs_prev, a, b);
                let sep_hit = tag.has(PairTag::SEPHIT);
                let reuse_hit = tag.has(PairTag::HIT);
                cov.carried_sep_hits += u64::from(sep_hit);
                cov.reuse_hits += u64::from(reuse_hit);
                cov.records_built += u64::from(tag.has(PairTag::REC) && !reuse_hit);
                if let Some((j, flipped)) = join {
                    cov.flipped_joins += u64::from(flipped);
                    cov.flipped_reuse_hits += u64::from(flipped && reuse_hit);
                    cov.jumper_joins += u64::from(kind.is_jumper_pair(a, b));
                    if sep_hit && c > 0 && lo > 0 && j < lo {
                        cov.sep_hits_behind_chunk_start += 1;
                    }
                    if reuse_hit && c > 0 && lo > 0 && j < lo {
                        cov.reuse_hits_behind_chunk_start += 1;
                    }
                }
            }
        }
    }

    /// G-L5-1, extended by L9's G-C-2: over random box/sphere/sensor scenes, a random table state
    /// (a small table, so home slots collide), random pre-read frames with random carried axes,
    /// and random contiguous partitions run in random order, the parallel path's solver stream,
    /// sensor stream, table state, `occupied`, fingerprint and pair tags equal the serial loop's
    /// — on a first frame with no pair carry, and on a second frame that runs over the first
    /// frame's stale stage and commit rows and its table, with its pairs joined to the first
    /// frame's tags by a random carry: the same rows, random moved rows with jumpers and flips,
    /// or a Reset.
    ///
    /// Three cases in four run both frames with contact reuse forced on (L9b's G-L9b-3 at the unit
    /// level): the first frame builds records, and when the second frame keeps the first's bodies —
    /// moved to their new rows on a Rows carry, so a flipped join finds the same physical pair —
    /// its pairs reuse them. The records a tag says were written are compared too.
    ///
    /// Mutations this must turn red (the design's M1-M6): join the chunk runs in descending
    /// chunk order; drop `prefetched ||` from the skip rule; commit in descending `k`; read the
    /// sensor run forward; skip `commit_axes`; write the solver run at `k` instead of `wo`. And
    /// L9's M-c4: a chunk's join cursor starting at slot `lo` instead of its first key's lower
    /// bound misses the joins behind it (the `sep_hits_behind_chunk_start` and
    /// `reuse_hits_behind_chunk_start` witnesses); and M-b6: a chunk reading the record at the
    /// joined slot of this step's record column instead of the previous step's.
    #[test]
    #[cfg(not(miri))]
    fn parallel_chunks_equal_the_serial_loop_on_random_frames() {
        let totals = Cell::new(Coverage::default());
        // 256 cases: the rarest witness, a record reused behind a chunk's first slot, comes about
        // once in twenty cases.
        let config = ProptestConfig {
            cases: 256,
            failure_persistence: None,
            ..ProptestConfig::default()
        };
        proptest!(config, |(seed in any::<u64>())| {
            let mut rng = Rng::new(seed);
            let n_bodies = 4 + rng.below(12) as usize;
            let first = Scene::random(&mut rng, n_bodies);
            let mut second = Scene::random(&mut rng, n_bodies);
            let second_kind = CarryKind::random(&mut rng, n_bodies, n_bodies);
            // The second frame keeps the first frame's bodies half the time, so the table's
            // entries from frame one are live hints, not only stale keys, and the records from
            // frame one describe the same pairs: on a Rows carry each body moves to the row the
            // map gives it.
            if rng.below(2) == 0 {
                second.bodies.clone_from(&first.bodies);
                if let CarryKind::Rows { prev_row, .. } = &second_kind {
                    for (r, &p) in prev_row.iter().enumerate() {
                        if p != NO_ROW {
                            second.bodies[r] = first.bodies[p as usize];
                        }
                    }
                }
            }
            let reuse_on = rng.below(4) != 0;
            let capacity = first.pairs.len().max(second.pairs.len());
            let mut serial = Manifolds::with_capacity(capacity);
            let mut parallel = Manifolds::with_capacity(capacity);
            let mut cov = totals.get();
            for (index, scene) in [&first, &second].into_iter().enumerate() {
                let n = scene.pairs.len();
                let max_fill = begin_frames(&mut serial, &mut parallel, n);
                let frame = Frame::random(&mut rng, scene, max_fill);
                frame.apply(&mut serial);
                frame.apply(&mut parallel);
                prop_assert_eq!(outcome(&serial).table, outcome(&parallel).table, "seed {:#x}: T0 differs", seed);
                let cuts = random_cuts(&mut rng, n);
                let order = random_order(&mut rng, cuts.len() - 1);
                cover(&mut cov, &parallel, scene, &frame, &cuts);
                if index == 1 {
                    cov.stale_stage_frames += 1;
                }
                let kind = if index == 0 { &CarryKind::Reset } else { &second_kind };
                let carry = kind.carry(&first.pairs).with_reuse(reuse_step(reuse_on, &serial));
                narrowphase_serial_with(&mut serial, &scene.bodies, &scene.pairs, frame.prefetched, carry);
                run_partition(&mut parallel, scene, frame.prefetched, carry, &cuts, &order);
                if index == 1 {
                    cover_carry(&mut cov, &serial, scene, kind, &first.pairs, &cuts);
                }
                prop_assert_eq!(
                    outcome(&parallel),
                    outcome(&serial),
                    "seed {:#x}, frame {}, cuts {:?}, order {:?}, prefetched {}",
                    seed, index, cuts, order, frame.prefetched
                );
            }
            totals.set(cov);
        });
        let cov = totals.get();
        println!("G-L5-1 coverage: {cov:?}");
        assert!(cov.skipped_identity_sets > 0, "no case left an identity set out: {cov:?}");
        assert!(cov.prefetched_box_contacts > 0, "no pre-read frame committed an axis: {cov:?}");
        assert!(cov.sensor_manifolds > 0, "no case routed a sensor manifold: {cov:?}");
        assert!(cov.box_sphere_flips > 0, "no case collided a box-sphere pair: {cov:?}");
        assert!(cov.multi_chunk_frames > 0, "no case ran two non-empty chunks: {cov:?}");
        assert!(cov.stale_stage_frames > 0, "no case ran over a stale stage: {cov:?}");
        assert!(cov.carried_sep_hits > 0, "no carried separating axis held: {cov:?}");
        assert!(
            cov.sep_hits_behind_chunk_start > 0,
            "no chunk joined a pair behind its first pair's slot (the M-c4 witness): {cov:?}"
        );
        assert!(cov.flipped_joins > 0, "no pair joined across a row-order flip: {cov:?}");
        assert!(cov.jumper_joins > 0, "no pair joined by the jumper search: {cov:?}");
        assert!(cov.records_built > 0, "no pair built a reuse record: {cov:?}");
        assert!(cov.reuse_hits > 0, "no pair reused its record: {cov:?}");
        assert!(cov.flipped_reuse_hits > 0, "no pair reused its record across a flip: {cov:?}");
        assert!(
            cov.reuse_hits_behind_chunk_start > 0,
            "no chunk reused a record behind its first pair's slot: {cov:?}"
        );
    }

    /// G-L5-2 (Miri, Tree Borrows): three chunks of a frame run on three `std::thread::scope`
    /// threads, then the compaction and the axis commit, equal the serial loop — on a first
    /// frame with no pair carry, and on a second frame of the same scene whose pairs join the
    /// first frame's tags. Both frames run with contact reuse forced on (L9 G-L9b-5): the first
    /// frame's chunks write the records of the pairs they build, and the second frame's chunks
    /// read the previous records beside the previous tags and write the records their hits copy.
    ///
    /// Under Miri this is the provenance check on the chunks' writes: each goes through the
    /// solve view's `row_ptr`, whose base is the column's own write base. Deriving the stage's
    /// (or the tags') base from `as_read_slice().as_ptr().cast_mut()` instead — the SP4 shape —
    /// must make Miri report a Tree Borrows violation here.
    #[test]
    fn chunks_on_threads_equal_the_serial_loop() {
        let mut rng = Rng::new(0x15_5EED);
        let scene = Scene::random(&mut rng, 9);
        let n = scene.pairs.len();
        assert!(n >= 20, "construction: about thirty pairs, got {n}");
        let mut serial = Manifolds::with_capacity(n);
        let mut parallel = Manifolds::with_capacity(n);
        let mut second_frame_sep_hits = 0;
        let mut second_frame_reuse_hits = 0;
        for index in 0..2 {
            let carry = if index == 0 {
                CarryIn::NONE
            } else {
                CarryIn::new(RowRemap::Identity, &scene.pairs, &[])
            };
            let max_fill = begin_frames(&mut serial, &mut parallel, n);
            let carry = carry.with_reuse(reuse_step(true, &serial));
            let frame = Frame::random(&mut rng, &scene, max_fill);
            frame.apply(&mut serial);
            frame.apply(&mut parallel);

            narrowphase_serial_with(&mut serial, &scene.bodies, &scene.pairs, frame.prefetched, carry);

            let chunks = 3;
            let meta: Vec<AtomicU64> = (0..chunks).map(|_| AtomicU64::new(0)).collect();
            let ctx =
                prepare(&mut parallel, &scene.bodies, &scene.pairs, frame.prefetched, carry, &meta, &[]);
            std::thread::scope(|s| {
                for chunk in 0..chunks {
                    let (lo, hi) = chunk_bounds(chunk, chunks, n);
                    s.spawn(move || {
                        // SAFETY: `ctx` came from `prepare` for these pairs; the closed-form cuts
                        //   partition `[0, n)`, so the three threads write disjoint rows and
                        //   distinct `meta` slots; nothing reads the stage, the commit, the tags
                        //   or the records until `std::thread::scope` has joined all three.
                        unsafe { np_chunk::<false>(ctx, chunk, lo, hi) }
                    });
                }
            });
            compact(&mut parallel, chunks, |c| chunk_bounds(c, chunks, n), &meta);
            parallel.box_axis_cache.commit_axes(&scene.pairs);

            let (got, want) = (outcome(&parallel), outcome(&serial));
            assert!(!want.out.is_empty(), "construction: the frame has solver manifolds");
            assert_eq!(got, want, "frame {index}: three threaded chunks must reproduce the serial loop");
            if index == 1 {
                second_frame_sep_hits = serial.separated_axis_hits();
                second_frame_reuse_hits = serial.pair_classes().reused;
            }
        }
        assert!(
            second_frame_sep_hits > 0,
            "construction: the second frame must hold a pair its carried separating axis rejects"
        );
        assert!(
            second_frame_reuse_hits > 0,
            "construction: the second frame must hold a pair that reuses its record"
        );
    }

    // ── L10 C3b: the `Sets` arm (design 06 D-D, 08 D1′; ruling W2) ──────────────────────────

    /// The `Sets` arm's inputs for a frame: a classification — `SENSOR` from the bodies, a
    /// random quarter of the other rows `HELD`, another quarter `RESTORED`, and `SENSOR_NBR` on a
    /// held row with a sensor pair (D-F) — and a restore source: a random subset of the previous
    /// frame's slots, keyed by their previous rows, with the previous frame's tags and records.
    struct SetsInput {
        cls: Vec<RowCls>,
        keys: Vec<(BodyIndex, BodyIndex)>,
        tags: Vec<PairTag>,
        reuse: Vec<ReuseRecord>,
    }

    impl SetsInput {
        fn random(rng: &mut Rng, scene: &Scene, prev: &Manifolds, pairs_prev: &[(BodyIndex, BodyIndex)]) -> Self {
            let mut cls: Vec<RowCls> = scene
                .bodies
                .iter()
                .map(|b| {
                    let flags = if b.is_sensor {
                        RowCls::SENSOR
                    } else {
                        match rng.below(4) {
                            0 => RowCls::HELD,
                            1 => RowCls::RESTORED,
                            _ => 0,
                        }
                    };
                    RowCls { island: RowCls::NO_ISLAND, flags }
                })
                .collect();
            for &(a, b) in &scene.pairs {
                if sensor_pair(cls[a.0 as usize].flags, cls[b.0 as usize].flags) {
                    for r in [a, b] {
                        if cls[r.0 as usize].flags & RowCls::HELD != 0 {
                            cls[r.0 as usize].flags |= RowCls::SENSOR_NBR;
                        }
                    }
                }
            }
            let (tags_prev, records_prev) = (prev.pair_carry.tags(), prev.pair_carry.records());
            let mut input = Self { cls, keys: Vec::new(), tags: Vec::new(), reuse: Vec::new() };
            for (j, &key) in pairs_prev.iter().enumerate() {
                if rng.below(2) == 0 {
                    let tag = tags_prev[j];
                    input.keys.push(key);
                    input.tags.push(tag);
                    input.reuse.push(if tag.has(PairTag::REC) { records_prev[j] } else { ReuseRecord::UNWRITTEN });
                }
            }
            input
        }

        /// A fixed input: rows 0 and 1 `HELD`, rows 2 and 3 `RESTORED` (sensors keep only
        /// `SENSOR`), `SENSOR_NBR` as the D-F rule sets it, and every previous slot restorable.
        fn fixed(scene: &Scene, prev: &Manifolds, pairs_prev: &[(BodyIndex, BodyIndex)]) -> Self {
            let mut cls: Vec<RowCls> = scene
                .bodies
                .iter()
                .enumerate()
                .map(|(r, b)| {
                    let flags = match r {
                        _ if b.is_sensor => RowCls::SENSOR,
                        0 | 1 => RowCls::HELD,
                        2 | 3 => RowCls::RESTORED,
                        _ => 0,
                    };
                    RowCls { island: RowCls::NO_ISLAND, flags }
                })
                .collect();
            for &(a, b) in &scene.pairs {
                if sensor_pair(cls[a.0 as usize].flags, cls[b.0 as usize].flags) {
                    for r in [a, b] {
                        if cls[r.0 as usize].flags & RowCls::HELD != 0 {
                            cls[r.0 as usize].flags |= RowCls::SENSOR_NBR;
                        }
                    }
                }
            }
            let (tags_prev, records_prev) = (prev.pair_carry.tags(), prev.pair_carry.records());
            let reuse = pairs_prev
                .iter()
                .enumerate()
                .map(|(j, _)| if tags_prev[j].has(PairTag::REC) { records_prev[j] } else { ReuseRecord::UNWRITTEN })
                .collect();
            Self { cls, keys: pairs_prev.to_vec(), tags: tags_prev[..pairs_prev.len()].to_vec(), reuse }
        }

        fn np(&self) -> NpSets<'_> {
            NpSets { cls: &self.cls, keys: &self.keys, tags: &self.tags, reuse: &self.reuse }
        }
    }

    /// The `Sets` arm over an explicit partition, its chunks run on this thread in `order`.
    /// Returns what the arm counted: `(skips, restored, restore hits)`.
    #[cfg(not(miri))]
    fn run_partition_sets(
        m: &mut Manifolds,
        scene: &Scene,
        prefetched: bool,
        carry: CarryIn<'_>,
        input: &SetsInput,
        cuts: &[usize],
        order: &[usize],
    ) -> (u32, u32, u32) {
        let chunks = cuts.len() - 1;
        let meta: Vec<AtomicU64> = (0..chunks).map(|_| AtomicU64::new(0)).collect();
        let skips: Vec<AtomicU32> = (0..chunks).map(|_| AtomicU32::new(0)).collect();
        let restores: Vec<AtomicU64> = (0..chunks).map(|_| AtomicU64::new(0)).collect();
        let ctx = prepare(m, &scene.bodies, &scene.pairs, prefetched, carry, &meta, &input.cls);
        let restore = ctx.join().restored(&input.keys, &input.tags, &input.reuse);
        let ctx = ctx.with_sleep(&input.cls, &skips, restore, &restores);
        for &chunk in order {
            // SAFETY: `ctx` came from `prepare` for these pairs; `cuts` is ascending from 0 to
            //   n, so the chunks' ranges are disjoint, and they run one after another on this
            //   thread; each writes only its own `meta`, `skips` and `restores` slot, and no
            //   slice over the stage, the commit, the tags or the records is taken until
            //   `compact`.
            unsafe { np_chunk::<true>(ctx, chunk, cuts[chunk], cuts[chunk + 1]) };
        }
        compact(m, chunks, |c| (cuts[c], cuts[c + 1]), &meta);
        m.box_axis_cache.commit_axes(&scene.pairs);
        let skipped = skips.iter().map(|s| s.load(Ordering::Relaxed)).sum();
        let (mut restored, mut hits) = (0, 0);
        for w in &restores {
            let w = w.load(Ordering::Relaxed);
            restored += (w >> 32) as u32;
            hits += w as u32;
        }
        (skipped, restored, hits)
    }

    /// L10's partition gate (design 06 §7: G-C-2 extended to restored runs; N12, N14, W2-M,
    /// N27): on the `Sets` arm, a random partition of a random frame's pairs into chunks, run in
    /// a random order, then the compaction and the axis commit, equals the serial loop — the
    /// streams, the table, the tags and the records — and counts the same skips, restored pairs
    /// and restore hits. The frame's pairs join the previous frame's tags (Identity, Rows with
    /// jumpers, or Reset), and its restored pairs join a random subset of the previous frame's
    /// slots through the same join (ruling W2). Mutations that go red here: a held skip only on
    /// the serial path (N12), the restore cursor started at the chunk's first index instead of
    /// its lower bound (N14), a jumper pair merged on the restore cursor (W2-M), and a held
    /// skip that leaves its commit unwritten (N27).
    #[test]
    #[cfg(not(miri))]
    fn sets_chunks_equal_the_serial_loop_with_held_and_restored_rows() {
        #[derive(Clone, Copy, Default, Debug)]
        struct SetsCoverage {
            skips: u64,
            restored: u64,
            restore_hits: u64,
            /// Restore hits whose source slot lies below the chunk's first pair index.
            hits_behind_chunk_start: u64,
            /// Restore hits on a pair with a jumper endpoint (a Rows frame).
            jumper_hits: u64,
            multi_chunk: u64,
        }
        let totals = Cell::new(SetsCoverage::default());
        let config = ProptestConfig { cases: 256, failure_persistence: None, ..ProptestConfig::default() };
        proptest!(config, |(seed in any::<u64>())| {
            let mut rng = Rng::new(seed);
            let n_bodies = 4 + rng.below(12) as usize;
            let first = Scene::random(&mut rng, n_bodies);
            let second_kind = CarryKind::random(&mut rng, n_bodies, n_bodies);
            let mut second = Scene::random(&mut rng, n_bodies);
            if rng.below(2) == 0 {
                second.bodies.clone_from(&first.bodies);
                if let CarryKind::Rows { prev_row, .. } = &second_kind {
                    for (r, &p) in prev_row.iter().enumerate() {
                        if p != NO_ROW {
                            second.bodies[r] = first.bodies[p as usize];
                        }
                    }
                }
            }
            let reuse_on = rng.below(4) != 0;
            let capacity = first.pairs.len().max(second.pairs.len());
            let mut serial = Manifolds::with_capacity(capacity);
            let mut parallel = Manifolds::with_capacity(capacity);
            // Frame one: the Off arm on both, to leave tags and records behind.
            let n = first.pairs.len();
            let max_fill = begin_frames(&mut serial, &mut parallel, n);
            let frame = Frame::random(&mut rng, &first, max_fill);
            frame.apply(&mut serial);
            frame.apply(&mut parallel);
            let carry = CarryIn::NONE.with_reuse(reuse_step(reuse_on, &serial));
            narrowphase_serial_with(&mut serial, &first.bodies, &first.pairs, frame.prefetched, carry);
            let cuts = random_cuts(&mut rng, n);
            let order = random_order(&mut rng, cuts.len() - 1);
            run_partition(&mut parallel, &first, frame.prefetched, carry, &cuts, &order);
            prop_assert_eq!(outcome(&parallel), outcome(&serial), "seed {:#x}: frame one", seed);
            // Frame two: the Sets arm.
            let input = SetsInput::random(&mut rng, &second, &serial, &first.pairs);
            let n = second.pairs.len();
            let max_fill = begin_frames(&mut serial, &mut parallel, n);
            let frame = Frame::random(&mut rng, &second, max_fill);
            frame.apply(&mut serial);
            frame.apply(&mut parallel);
            let carry = second_kind.carry(&first.pairs).with_reuse(reuse_step(reuse_on, &serial));
            let counts = narrowphase_serial_sets::<true>(&mut serial, &second.bodies, &second.pairs, frame.prefetched, carry, input.np());
            let cuts = random_cuts(&mut rng, n);
            let order = random_order(&mut rng, cuts.len() - 1);
            let got = run_partition_sets(&mut parallel, &second, frame.prefetched, carry, &input, &cuts, &order);
            prop_assert_eq!(got, (counts.skips, counts.restored, counts.restore_hits), "seed {:#x}: counts", seed);
            prop_assert_eq!(outcome(&parallel), outcome(&serial), "seed {:#x}, cuts {:?}, order {:?}", seed, cuts, order);
            // Coverage, from the definition of the restore join (lemma L9-J over the source).
            let mut cov = totals.get();
            cov.skips += u64::from(counts.skips);
            cov.restored += u64::from(counts.restored);
            cov.restore_hits += u64::from(counts.restore_hits);
            cov.multi_chunk += u64::from(cuts.windows(2).filter(|w| w[1] > w[0]).count() >= 2);
            for c in 0..cuts.len() - 1 {
                for k in cuts[c]..cuts[c + 1] {
                    let (a, b) = second.pairs[k];
                    if np_route::<true>(&input.cls, a, b) != Route::Restore {
                        continue;
                    }
                    let Some((j, _)) = second_kind.expected_join(&input.keys, a, b) else { continue };
                    cov.hits_behind_chunk_start += u64::from(cuts[c] > 0 && j < cuts[c]);
                    cov.jumper_hits += u64::from(second_kind.is_jumper_pair(a, b));
                }
            }
            totals.set(cov);
        });
        let cov = totals.get();
        println!("L10 Sets partition coverage: {cov:?}");
        assert!(cov.skips > 0, "no pair was skipped for a held endpoint: {cov:?}");
        assert!(cov.restored > 0 && cov.restore_hits > 0, "no restored pair found its source: {cov:?}");
        assert!(cov.hits_behind_chunk_start > 0, "no restore hit behind a chunk's first pair (the N14 witness): {cov:?}");
        assert!(cov.jumper_hits > 0, "no restore hit on a jumper pair (the W2-M witness): {cov:?}");
        assert!(cov.multi_chunk > 0, "no case ran two non-empty chunks: {cov:?}");
    }

    /// L10 C3b under Miri (Tree Borrows): the `Sets` arm's chunks — held skips writing the
    /// held-skip tag and an `AXIS_NONE` commit, restored pairs joining the restore source —
    /// run on three `std::thread::scope` threads equal the serial loop, on a second frame whose
    /// pairs join the first frame's tags. The provenance check on the chunks' `Sets` writes.
    #[test]
    fn sets_chunks_on_threads_equal_the_serial_loop() {
        let mut rng = Rng::new(0x10_C3B);
        let scene = Scene::random(&mut rng, 9);
        let n = scene.pairs.len();
        let mut serial = Manifolds::with_capacity(n);
        let mut parallel = Manifolds::with_capacity(n);
        // Frame one: the serial loop on both.
        for m in [&mut serial, &mut parallel] {
            m.box_axis_cache.begin_frame(n);
            let carry = CarryIn::NONE.with_reuse(ReuseStep::new(true, TAU, DT, m.box_axis_cache.keys_changed()));
            narrowphase_serial_with(m, &scene.bodies, &scene.pairs, false, carry);
        }
        let input = SetsInput::fixed(&scene, &serial, &scene.pairs);
        let max_fill = begin_frames(&mut serial, &mut parallel, n);
        let frame = Frame::random(&mut rng, &scene, max_fill);
        frame.apply(&mut serial);
        frame.apply(&mut parallel);
        let carry = CarryIn::new(RowRemap::Identity, &scene.pairs, &[]).with_reuse(reuse_step(true, &serial));
        let counts = narrowphase_serial_sets::<true>(&mut serial, &scene.bodies, &scene.pairs, frame.prefetched, carry, input.np());

        let chunks = 3;
        let meta: Vec<AtomicU64> = (0..chunks).map(|_| AtomicU64::new(0)).collect();
        let skips: Vec<AtomicU32> = (0..chunks).map(|_| AtomicU32::new(0)).collect();
        let restores: Vec<AtomicU64> = (0..chunks).map(|_| AtomicU64::new(0)).collect();
        let ctx = prepare(&mut parallel, &scene.bodies, &scene.pairs, frame.prefetched, carry, &meta, &input.cls);
        let restore = ctx.join().restored(&input.keys, &input.tags, &input.reuse);
        let ctx = ctx.with_sleep(&input.cls, &skips, restore, &restores);
        std::thread::scope(|s| {
            for chunk in 0..chunks {
                let (lo, hi) = chunk_bounds(chunk, chunks, n);
                s.spawn(move || {
                    // SAFETY: `ctx` came from `prepare` for these pairs; the closed-form cuts
                    //   partition `[0, n)`, so the three threads write disjoint rows and
                    //   distinct `meta`, `skips` and `restores` slots; nothing reads the stage,
                    //   the commit, the tags or the records until `std::thread::scope` has joined
                    //   all three.
                    unsafe { np_chunk::<true>(ctx, chunk, lo, hi) }
                });
            }
        });
        compact(&mut parallel, chunks, |c| chunk_bounds(c, chunks, n), &meta);
        parallel.box_axis_cache.commit_axes(&scene.pairs);
        let skipped: u32 = skips.iter().map(|s| s.load(Ordering::Relaxed)).sum();
        let restored: u32 = restores.iter().map(|w| (w.load(Ordering::Relaxed) >> 32) as u32).sum();
        assert_eq!((skipped, restored), (counts.skips, counts.restored), "the chunks count the serial loop's");
        assert!(
            counts.skips > 0 && counts.restored > 0 && counts.restore_hits > 0,
            "construction: the frame skips pairs and restores pairs that find their source: {counts:?}"
        );
        assert_eq!(outcome(&parallel), outcome(&serial), "three threaded Sets chunks must reproduce the serial loop");
    }
}
