//! The parallel narrowphase (L5): the candidate pairs cut into contiguous chunks, each chunk
//! collided on a worker of the ambient pool, and the chunks' output joined back in pair order.
//!
//! Requested by [`PhysicsConfig::parallel_narrowphase`](crate::resources::PhysicsConfig) and
//! entered from [`physics_narrowphase`](crate::systems::physics_narrowphase) through
//! [`try_parallel`], which returns the number of chunks it ran, or `0` when it ran none and the
//! serial loop must produce the step.
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
//! # Threads, borrows and allocation
//!
//! * **Shared and read-only while the chunks run:** the bodies, the pairs, the axis slots, the
//!   carried-axis column and the per-row orientation frames (L9 D2). Their writers
//!   (`begin_frame_synced`, [`prepare`]'s frame fill, `commit_axes`) run serially on the
//!   calling thread, before and after the scope; the frames reach the chunks as a shared slice
//!   taken after the fill's view is dropped.
//! * **Exclusive per chunk:** its stage rows `[lo, hi)`, its commit rows `[lo, hi)` and its
//!   `meta` slot. The cuts are a partition, so no two tasks write one element.
//! * **Synchronisation:** `spawn` publishes the captures and the scope's join acquires every
//!   task's effects, both under the pool's own loom models. `meta` is stored and loaded
//!   `Relaxed`: its ordering comes from that join, not from the atomic. No new protocol, so no
//!   new loom model.
//! * **Tree Borrows:** a chunk writes only through `ScratchSolveView::row_ptr`, whose provenance
//!   is the column's own write base (`ScratchColumn::solve_base`). No `&[T]` or `&mut [T]` over
//!   the stage or the commit exists until after the join, which is the Grid emit's rule
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

use std::sync::atomic::{AtomicU64, Ordering};

use boyko_diag::zone;
use boyko_ecs::ecs::core::component::scratch::{ScratchColumn, ScratchSolveView};
use boyko_threadpool::try_with_active_pool;

use crate::manifold::{BodyIndex, Manifold};
use crate::narrowphase::axis_cache::{AXIS_NONE, AxisHints, SAT_AXIS_COUNT};
use crate::narrowphase::reuse::{RowFrame, fill_row_frames};
use crate::profiling::{PHYS_NP_AXIS_COMMIT, PHYS_NP_COMPACT, PHYS_NP_DISPATCH};
use crate::resources::{BodyState, Manifolds};
use crate::systems::collide_pair;

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
/// `Send + Sync` without an `unsafe impl`: shared slices of POD, the two solve views (whose
/// own impls state the per-row discipline) and a slice of atomics.
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
    /// The staging column's per-row write view.
    stage: ScratchSolveView<'a, Manifold>,
    /// The per-pair axis commit's per-row write view.
    commit: ScratchSolveView<'a, u8>,
    /// One packed `(solver run, sensor run)` word per chunk.
    meta: &'a [AtomicU64],
}

/// Grows the staging column and the per-pair commit to at least the pair count, fills the step's
/// per-row orientation frames (L9 D2) and builds the context every chunk shares.
///
/// Both columns' lengths only grow, and their fill runs only on growth, so a warm step writes
/// only the frames here. The views are taken after the growth: a view caches its column's length.
pub(crate) fn prepare<'a>(
    manifolds: &'a mut Manifolds,
    bodies: &'a [BodyState],
    pairs: &'a [(BodyIndex, BodyIndex)],
    prefetched: bool,
    meta: &'a [AtomicU64],
) -> NpChunkCtx<'a> {
    let n = pairs.len();
    let Manifolds { np_stage, box_axis_cache, row_frames, .. } = manifolds;
    if np_stage.len() < n {
        grow_stage(np_stage, n);
    }
    let frames = fill_row_frames(row_frames, bodies, n);
    let np_stage: &'a ScratchColumn<Manifold> = np_stage;
    let (hints, commit) = box_axis_cache.dispatch_views(prefetched, n);
    let stage = np_stage.solve_view();
    debug_assert!(
        stage.len() >= n && commit.len() >= n,
        "invariant: the stage and the commit hold a row for every candidate pair"
    );
    NpChunkCtx { bodies, pairs, frames, hints, stage, commit, meta }
}

/// Grows the staging column to `n` rows; the fill covers only the new rows.
#[cold]
#[inline(never)]
fn grow_stage(stage: &mut ScratchColumn<Manifold>, n: usize) {
    stage.build_view().resize(n, Manifold::new(BodyIndex(0), BodyIndex(0)));
}

/// Collides chunk `chunk`'s pairs `[lo, hi)`: each manifold into the chunk's staging rows, each
/// pair's axis into its commit row, and the two run lengths into `meta[chunk]`.
///
/// # Safety
/// * `ctx` came from [`prepare`] for these pairs, so `hi <= ctx.pairs.len()` is within both
///   views, and `lo <= hi`.
/// * While this call runs, no other thread writes or reads rows `[lo, hi)` of the stage or the
///   commit, or `meta[chunk]`: the concurrent chunks' ranges are pairwise disjoint and their
///   indices distinct, which a partition of `[0, n)` into ascending cuts guarantees.
/// * Nothing holds a slice over the stage or the commit until every chunk of the step has
///   returned (the scope's join, or `std::thread::scope`'s).
pub(crate) unsafe fn np_chunk(ctx: NpChunkCtx<'_>, chunk: usize, lo: usize, hi: usize) {
    debug_assert!(
        lo <= hi && hi <= ctx.pairs.len(),
        "invariant: a chunk owns a sub-range of the candidate pairs"
    );
    let prefetched = ctx.hints.prefetched();
    let mut wo = lo;
    let mut ws = hi;
    for k in lo..hi {
        let (a, b) = ctx.pairs[k];
        let ba = &ctx.bodies[a.0 as usize];
        let bb = &ctx.bodies[b.0 as usize];
        let mut hint = None;
        let (manifold, axis) = collide_pair(a, b, ba, bb, ctx.frames, || {
            hint = ctx.hints.read(k, a, b);
            hint
        });
        // The skip rule (D3): without a pre-read, a pair whose hint already names the axis
        // it chose would `set` the value its slot holds — an identity (Lemma 2).
        let commit = match axis {
            Some(chosen) if prefetched || hint != Some(chosen) => {
                debug_assert!(
                    chosen < usize::from(SAT_AXIS_COUNT),
                    "invariant: a chosen SAT axis is 0..15"
                );
                chosen as u8
            }
            _ => AXIS_NONE,
        };
        // SAFETY: `lo <= k < hi <= commit.len()` (the caller's contract and `prepare`'s
        //   growth), so row `k` is in bounds; this chunk owns `[lo, hi)` and no other thread
        //   touches that row while it runs (the caller's contract). The base carries the
        //   column's own write provenance (`solve_base`), never a slice reborrow. `u8` has no
        //   drop glue, so overwriting last step's byte is a plain store.
        unsafe { ctx.commit.row_ptr(k).write(commit) };

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

/// The parallel narrowphase for one step. Returns the number of chunks it ran, or `0` when it
/// ran none — no pool, fewer than two lanes, fewer than two chunks, or more pairs than the
/// stage can hold — and the caller must run the serial loop.
///
/// `prefetched` is what `BoxAxisCache::begin_frame_synced` returned for this frame; it must
/// have run first.
pub(crate) fn try_parallel(
    manifolds: &mut Manifolds,
    bodies: &[BodyState],
    pairs: &[(BodyIndex, BodyIndex)],
    prefetched: bool,
) -> usize {
    let n = pairs.len();
    try_with_active_pool(|pool| {
        let chunks = chunk_count(n, pool.num_threads());
        if chunks == 0 {
            return 0;
        }
        if n > manifolds.np_stage.capacity() || n > manifolds.box_axis_cache.commit_capacity() {
            return beyond_stage_ceiling();
        }
        debug_assert!(chunks <= NP_MAX_CHUNKS, "invariant: chunk_count clamps to NP_MAX_CHUNKS");
        let meta: [AtomicU64; NP_MAX_CHUNKS] = [const { AtomicU64::new(0) }; NP_MAX_CHUNKS];
        {
            let _zone = zone!(PHYS_NP_DISPATCH);
            // `ctx.meta` is cut to the step's chunk count, so a task reads `chunks` and `n`
            // from the context and captures two words: `&ctx` (it is `Sync` and outlives the
            // scope) and its chunk index. A task cell is then 16 bytes of header plus 16 of
            // closure, and 128 of them fit the scope's first 4 KiB block — above the 96
            // chunks of a lane-bound W = 16 step. Capturing the 128-byte context by value
            // with the cuts would have been 168 bytes a cell: 24 a block, so W = 8 (48
            // chunks) took two blocks and W = 16 three. C4's census pins the block count.
            let ctx = prepare(manifolds, bodies, pairs, prefetched, &meta[..chunks]);
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
                        //   commit row or a `meta` slot; nothing takes a slice over the stage
                        //   or the commit until `pool.scope` has joined every task.
                        unsafe { np_chunk(*ctx, chunk, lo, hi) }
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
        chunks
    })
    .unwrap_or(0)
}

/// The step has more candidate pairs than the staging column (7.06M rows natively) or the
/// per-pair commit can hold: run the serial loop.
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
    use crate::systems::narrowphase_serial;

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
        assert_eq!(try_parallel(&mut m, &scene.bodies, &scene.pairs, false), 0);
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

    /// Everything the theorem says equal: both streams, the table state, `occupied` and the
    /// fingerprint.
    #[derive(Debug, PartialEq)]
    struct Outcome {
        out: Vec<Vec<u32>>,
        sensor: Vec<Vec<u32>>,
        table: (Vec<(u64, u32)>, usize),
        fingerprint: u64,
    }

    fn outcome(m: &Manifolds) -> Outcome {
        Outcome {
            out: m.manifolds().iter().map(manifold_words).collect(),
            sensor: m.sensor_overlaps().iter().map(manifold_words).collect(),
            table: m.box_axis_cache.table_state(),
            fingerprint: m.box_axis_cache.fingerprint(),
        }
    }

    /// The parallel path over an explicit partition (`cuts` holds every boundary, `0` and `n`
    /// included), its chunks run on this thread in `order`.
    #[cfg(not(miri))]
    fn run_partition(
        m: &mut Manifolds,
        scene: &Scene,
        prefetched: bool,
        cuts: &[usize],
        order: &[usize],
    ) {
        let chunks = cuts.len() - 1;
        let meta: Vec<AtomicU64> = (0..chunks).map(|_| AtomicU64::new(0)).collect();
        let ctx = prepare(m, &scene.bodies, &scene.pairs, prefetched, &meta);
        for &chunk in order {
            // SAFETY: `ctx` came from `prepare` for these pairs; `cuts` is ascending from 0 to
            //   n, so the chunks' ranges are disjoint, and they run one after another on this
            //   thread; no slice over the stage or the commit is taken until `compact`.
            unsafe { np_chunk(ctx, chunk, cuts[chunk], cuts[chunk + 1]) };
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
    }

    /// Recomputes, from T₀, which pairs of this frame the skip rule will leave out and which
    /// pre-read pairs will commit, and counts the frame's sensor and flip manifolds.
    #[cfg(not(miri))]
    fn cover(cov: &mut Coverage, m: &Manifolds, scene: &Scene, frame: &Frame, cuts: &[usize]) {
        for (k, &(a, b)) in scene.pairs.iter().enumerate() {
            let ba = &scene.bodies[a.0 as usize];
            let bb = &scene.bodies[b.0 as usize];
            let hint = m.box_axis_cache.read_hint(frame.prefetched, k, a, b);
            let (manifold, axis) = collide_pair(a, b, ba, bb, None, || hint);
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

    /// G-L5-1: over random box/sphere/sensor scenes, a random table state (a small table, so
    /// home slots collide), random pre-read frames with random carried axes, and random
    /// contiguous partitions run in random order, the parallel path's solver stream, sensor
    /// stream, table state, `occupied` and fingerprint equal the serial loop's — on a first
    /// frame and on a second frame that runs over the first frame's stale stage and commit
    /// rows and its table.
    ///
    /// Mutations this must turn red (the design's M1-M6): join the chunk runs in descending
    /// chunk order; drop `prefetched ||` from the skip rule; commit in descending `k`; read the
    /// sensor run forward; skip `commit_axes`; write the solver run at `k` instead of `wo`.
    #[test]
    #[cfg(not(miri))]
    fn parallel_chunks_equal_the_serial_loop_on_random_frames() {
        let totals = Cell::new(Coverage::default());
        let config = ProptestConfig {
            cases: 128,
            failure_persistence: None,
            ..ProptestConfig::default()
        };
        proptest!(config, |(seed in any::<u64>())| {
            let mut rng = Rng::new(seed);
            let n_bodies = 4 + rng.below(12) as usize;
            let first = Scene::random(&mut rng, n_bodies);
            let mut second = Scene::random(&mut rng, n_bodies);
            // The second frame keeps the first frame's bodies half the time, so the table's
            // entries from frame one are live hints, not only stale keys.
            if rng.below(2) == 0 {
                second.bodies.clone_from(&first.bodies);
            }
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
                narrowphase_serial(&mut serial, &scene.bodies, &scene.pairs, frame.prefetched);
                run_partition(&mut parallel, scene, frame.prefetched, &cuts, &order);
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
    }

    /// G-L5-2 (Miri, Tree Borrows): three chunks of one frame run on three `std::thread::scope`
    /// threads, then the compaction and the axis commit, equal the serial loop.
    ///
    /// Under Miri this is the provenance check on the chunks' writes: each goes through the
    /// solve view's `row_ptr`, whose base is the column's own write base. Deriving the stage's
    /// base from `as_read_slice().as_ptr().cast_mut()` instead — the SP4 shape — must make Miri
    /// report a Tree Borrows violation here.
    #[test]
    fn chunks_on_threads_equal_the_serial_loop() {
        let mut rng = Rng::new(0x15_5EED);
        let scene = Scene::random(&mut rng, 9);
        let n = scene.pairs.len();
        assert!(n >= 20, "construction: about thirty pairs, got {n}");
        let mut serial = Manifolds::with_capacity(n);
        let mut parallel = Manifolds::with_capacity(n);
        let max_fill = begin_frames(&mut serial, &mut parallel, n);
        let frame = Frame::random(&mut rng, &scene, max_fill);
        frame.apply(&mut serial);
        frame.apply(&mut parallel);

        narrowphase_serial(&mut serial, &scene.bodies, &scene.pairs, frame.prefetched);

        let chunks = 3;
        let meta: Vec<AtomicU64> = (0..chunks).map(|_| AtomicU64::new(0)).collect();
        let ctx = prepare(&mut parallel, &scene.bodies, &scene.pairs, frame.prefetched, &meta);
        std::thread::scope(|s| {
            for chunk in 0..chunks {
                let (lo, hi) = chunk_bounds(chunk, chunks, n);
                s.spawn(move || {
                    // SAFETY: `ctx` came from `prepare` for these pairs; the closed-form cuts
                    //   partition `[0, n)`, so the three threads write disjoint rows and
                    //   distinct `meta` slots; nothing reads the stage or the commit until
                    //   `std::thread::scope` has joined all three.
                    unsafe { np_chunk(ctx, chunk, lo, hi) }
                });
            }
        });
        compact(&mut parallel, chunks, |c| chunk_bounds(c, chunks, n), &meta);
        parallel.box_axis_cache.commit_axes(&scene.pairs);

        let (got, want) = (outcome(&parallel), outcome(&serial));
        assert!(!want.out.is_empty(), "construction: the frame has solver manifolds");
        assert_eq!(got, want, "three threaded chunks must reproduce the serial loop");
    }
}
