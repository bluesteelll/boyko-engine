//! S5, the parallel tree query (`docs/physics/perf-campaign/levers/scaling/01-DESIGN.md` §6.5): the
//! leaf-list query of a Tree step cut into chunks of Morton-adjacent active leaf nodes and answered
//! on the ambient pool.
//!
//! # When a step dispatches
//!
//! All of: the tree's switch ([`BroadphaseTree::parallel_query`], mirrored from
//! `PhysicsConfig::parallel_tree_query`); the [`QueryKernel::LeafList`](super::QueryKernel::LeafList) kernel; at least
//! [`S5_MIN_LEAVES`] active leaf nodes; a history (the last tree-path step queried a row); a pool
//! whose `num_threads()` gives [`s5_chunk_count`] ≥ 2 — zero below two lanes, so a one-worker world
//! runs the serial pass and opens no scope; and a region total within half the stream column's
//! reserve. Otherwise the serial leaf-list pass runs, untouched.
//!
//! # Why the output is the serial pass's
//!
//! Chunk `c` of `C` owns the active leaf nodes `[c·L/C, (c+1)·L/C)`. Every Q row is in exactly one
//! leaf lane, so the chunks own disjoint rows and each row's `(seg, nrev, nfwd)` has one writer. A
//! chunk answers each of its rows as the serial pass does — the same collections, prefilter and
//! emits in the same slot order, the same sort — so each segment holds the serial segment's bytes;
//! only its offset `seg` differs. The assembly reads a segment only through `seg`, `nrev` and
//! `nfwd`, and the verify zeroes all three every step, so `ContactPairs` — the stream ⊎ `withheld`
//! — is the serial one byte for byte at any worker count, partition and claim order. [`TreeDiag`]
//! and `LeafListCounts` count the leaves each path completed, so they are the serial ones too.
//!
//! # Output regions and the tail
//!
//! There is no count pass: it would repeat the whole query. Chunk `c` writes into its own region of
//! the stream ([`Layout`]): a per-row budget `b = max(S5_MIN_ROW_ENTRIES, ⌈3E / 2R⌉)` cut from the
//! last tree-path step's stream entries `E` and queried rows `R`, plus [`LANES`] entries of slack.
//! A leaf whose emits would pass its region's end, or whose collection passes the cap (the serial
//! pass's fallback case), stops its chunk there, and its partial writes and staged counts are
//! abandoned. After the join the calling thread answers every chunk's unfinished leaves, in
//! ascending chunk order, from the region total onward, with the serial pass's rules — growth and
//! the fallback walk (the tail) — and each record it writes replaces the abandoned one.
//!
//! # The wave
//!
//! One `pool.scope` per dispatched step (the lever rulings' L5 OQ1, extended to S5's broadphase
//! scope by ruling 7 of 2026-09-26). `P = min(lanes, C)` participants — the caller inline and
//! `P − 1` spawned helpers — claim chunks from one counter until it is exhausted. Helpers never
//! wait. The caller, once its claims run out, spins until every chunk is done — [`S5_SPIN_BURST`]
//! `PAUSE`s, then a `yield_now`, repeated — and never parks while a chunk is in flight (the owner's
//! spin value, rulings 2026-09-29 W8B item 1); `Scope::drop`'s join can still park for the
//! instants after the last chunk (accepted, rulings 2026-10-01 item 19 Q6). A chunk's completion is
//! counted by a guard's `Drop`, so it advances on unwind as well: a helper that panics mid-chunk
//! ends the caller's wait, and the scope's join re-raises the panic on the calling thread instead
//! of leaving the caller spinning (the cut's critique, C1).
//!
//! # Threads, borrows and allocation
//!
//! * **Shared and read-only while the chunks run:** the three trees (`&PackedBvh8`; nothing writes
//!   them between the active build and the join).
//! * **Exclusive per chunk:** its stream region, the `(seg, nrev, nfwd)` of its rows and its
//!   `resume` slot, written through the columns' solve views (`ScratchColumn::solve_base`
//!   provenance). From the scope's opening to its join no `&[T]` or `&mut [T]` over the stream or
//!   the records exists but each chunk's own region slice, and the regions are disjoint (the SP4
//!   rule).
//! * **Synchronisation:** the claim and completion counters are `Relaxed`: claims are exactly-once
//!   by the read-modify-write total order, and every read of a chunk's output follows the scope's
//!   join, which is the happens-before edge (the narrowphase's `meta` rule). No new protocol, so no
//!   new loom model.
//! * **Allocation:** one `pool.scope` — a boxed shared frame and one task block for its `P − 1 ≤ 63`
//!   cells. The per-chunk slots are function-local arrays, and the stream grows only when a region
//!   total passes every earlier step's length.
//! * **Instruments:** no zone opens inside a task; the dispatched step stays inside
//!   `phys_bp_query`.

use core::mem::MaybeUninit;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
#[cfg(any(test, feature = "bp-query-counts"))]
use std::sync::atomic::AtomicU64;

use boyko_ecs::ecs::core::component::scratch::{ScratchBuildView, ScratchSolveView};
use boyko_threadpool::{MAX_WORKERS, PoolInner, try_with_active_pool};

use super::bvh::{LANES, LEAF_R, LEAF_ROW, LEAF_X, LEAF_Y, LEAF_Z, Node8};
use super::kernel::{CandList, QueryBox, leaf_mask, leaf_mask_above};
#[cfg(any(test, feature = "bp-query-counts"))]
use super::{LeafListCounts, inversions};
use super::{
    BroadphaseTree, RowRec, STREAM_GROW, TreeDiag, Trees, emit_lanes, fallback_leaf, grow_stream,
    sort_leaf_list_segment,
};

/// Active leaf nodes below which a step runs the serial leaf-list pass at every worker count: the
/// inline threshold, 32 leaf nodes (256 Q rows).
///
/// `[DERIVED, not swept: rulings 2026-10-01 item 19 Q7]`. A leaf node costs about 8 × 158.0 ns =
/// 1.26 µs serially (window 9a, G5, W1); opening a scope after a serial gap costs 6.3–6.6 µs at W8
/// and the tail about one chunk (≈ 5.1 µs), so W2 breaks even near 18.6 leaf nodes. 32 is ≈ 1.7×
/// that, about 40 µs of serial work: the narrowphase's per-chunk floor (Box2D's task floor).
pub const S5_MIN_LEAVES: usize = 32;

/// The fewest active leaf nodes a chunk may own (the grain clamp), from the measured pair counts:
/// the J snapshot emits 9,570 partners over 155 leaf nodes (G-LL3), 61.7 a leaf node, so a 4-leaf
/// chunk expects ≈ 247 entries, and the 3/2 budget slack covers ≈ 124 more, ≈ 7.9 σ of a Poisson
/// count at that mean. A claim costs one `fetch_add`, so dispatch cost does not set the grain.
pub const S5_MIN_LEAVES_PER_CHUNK: usize = 4;

/// Chunks per pool lane: oversubscription for late recruitment under dynamic claims.
pub const S5_CHUNKS_PER_LANE: usize = 4;

/// The most chunks one step dispatches; also the length of the per-chunk slot array (64 lanes × 4).
pub const S5_MAX_CHUNKS: usize = 256;

/// The per-row budget's slack over the last tree-path step's mean, as `(numerator, denominator)`.
pub const S5_ROW_SLACK: (u64, u64) = (3, 2);

/// The smallest per-row budget, in stream entries.
pub const S5_MIN_ROW_ENTRIES: usize = 8;

/// `PAUSE`s the caller spins between two `yield_now`s while it waits for the last chunks (the
/// `PAUSE`-then-yield ladder of rulings 2026-09-30 V2, item 8, Q6).
pub const S5_SPIN_BURST: u32 = 64;

/// Which caller a `PackedBvh8::collect_leaves_in` instantiation belongs to: the S5 chunk kernel.
/// Each caller keeps an instantiation of its own (the narrowphase's `STAMPED` model), so the
/// serial pass's instantiation keeps its single caller and the inlining the W1 path had.
pub(super) const SITE_CHUNK: u8 = 1;
/// The S5 tail's `collect_leaves_in` instantiation (see [`SITE_CHUNK`]).
pub(super) const SITE_TAIL: u8 = 2;

/// The chunk count for `leaves` active leaf nodes on a pool of `lanes` workers, or `0` when the
/// step runs the serial pass: `min(lanes × S5_CHUNKS_PER_LANE, leaves / S5_MIN_LEAVES_PER_CHUNK,
/// S5_MAX_CHUNKS)`, and `0` below two lanes or two chunks. The inline threshold
/// ([`S5_MIN_LEAVES`]) is checked before it.
///
/// A runner that checks a run's structure re-implements this from the exported constants; the two
/// must agree on every term, the lanes term included.
#[inline]
pub const fn s5_chunk_count(leaves: usize, lanes: usize) -> usize {
    chunk_count(leaves, lanes, S5_MIN_LEAVES_PER_CHUNK)
}

/// [`s5_chunk_count`] with the grain as a parameter (the unit gates lower it).
#[inline]
pub(super) const fn chunk_count(leaves: usize, lanes: usize, grain: usize) -> usize {
    if lanes < 2 {
        return 0;
    }
    let by_lanes = lanes * S5_CHUNKS_PER_LANE;
    let by_work = leaves / grain;
    let chunks = if by_lanes < by_work { by_lanes } else { by_work };
    let chunks = if chunks > S5_MAX_CHUNKS { S5_MAX_CHUNKS } else { chunks };
    if chunks < 2 { 0 } else { chunks }
}

/// The per-row budget from the last tree-path step's `entries` (its stream's `fwd + rev` total)
/// and `rows` (its queried rows, `> 0`): `max(S5_MIN_ROW_ENTRIES, ⌈3·entries / (2·rows)⌉)`.
#[inline]
pub(super) fn row_budget(entries: u64, rows: u64) -> usize {
    let (num, den) = S5_ROW_SLACK;
    let b = (num * entries).div_ceil(den * rows);
    usize::try_from(b).unwrap_or(usize::MAX).max(S5_MIN_ROW_ENTRIES)
}

/// A dispatched step's partition and stream layout, all closed forms of four integers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Layout {
    /// The chunk count `C`.
    pub(super) chunks: usize,
    /// The active leaf nodes `L`.
    pub(super) leaves: usize,
    /// The active leaf slots: the Q rows (only the last leaf node can be partial).
    pub(super) slots: usize,
    /// The per-row budget `b`.
    pub(super) budget: usize,
}

impl Layout {
    /// The active leaf nodes chunk `c` owns: `[c·L/C, (c+1)·L/C)`.
    #[inline]
    pub(super) const fn leaf_range(&self, c: usize) -> (usize, usize) {
        (c * self.leaves / self.chunks, (c + 1) * self.leaves / self.chunks)
    }

    /// Chunk `c`'s stream region `(start, len)`: `start = 8·lo·b + 8c`, `len = rows·b + 8`. Every
    /// leaf node before the last is full, so the rows before chunk `c` are `8·lo`, and the regions
    /// tile `[0, total)` in chunk order.
    #[inline]
    pub(super) const fn region(&self, c: usize) -> (usize, usize) {
        let (lo, hi) = self.leaf_range(c);
        let end_row = if hi * LANES < self.slots { hi * LANES } else { self.slots };
        let rows = end_row - lo * LANES;
        (lo * LANES * self.budget + c * LANES, rows * self.budget + LANES)
    }

    /// The regions' total, `slots·b + 8·C`: where the tail starts.
    #[inline]
    pub(super) const fn total(&self) -> usize {
        self.slots * self.budget + self.chunks * LANES
    }
}

/// The per-pass constants of the leaf-list query: the trees' leaf nodes and the static and sleeper
/// trees' shapes.
#[derive(Clone, Copy)]
struct Tables<'a> {
    a_leaves: &'a [Node8],
    s_leaves: &'a [Node8],
    z_leaves: &'a [Node8],
    s_single: bool,
    z_on: bool,
    z_single: bool,
}

impl<'a> Tables<'a> {
    #[inline]
    fn of(trees: Trees<'a>) -> Self {
        let Trees { active, statics, sleepers } = trees;
        Self {
            a_leaves: active.leaf_nodes(),
            s_leaves: statics.leaf_nodes(),
            z_leaves: sleepers.leaf_nodes(),
            s_single: statics.levels() == 1,
            z_on: sleepers.levels() > 0,
            z_single: sleepers.levels() == 1,
        }
    }
}

/// The three candidate lists of one participant, on its stack (about 4.4 KB each, never initialised
/// as a whole: `CandList::init_in`).
struct Lists {
    act: MaybeUninit<CandList>,
    sta: MaybeUninit<CandList>,
    slp: MaybeUninit<CandList>,
}

/// The step's counts staged by the chunks, summed after the join (test and `bp-query-counts`
/// builds only): one `fetch_add` per field per participant.
#[cfg(any(test, feature = "bp-query-counts"))]
struct SharedCounts([AtomicU64; 11]);

#[cfg(any(test, feature = "bp-query-counts"))]
impl SharedCounts {
    fn new() -> Self {
        Self([const { AtomicU64::new(0) }; 11])
    }

    fn fields(c: &LeafListCounts) -> [u64; 11] {
        [
            c.leaves,
            c.rows,
            c.collect_box_active,
            c.collect_box_static,
            c.cands_active,
            c.cands_static,
            c.prefilter_chunks,
            c.kept_active,
            c.kept_static,
            c.emitted,
            c.sort_shifts,
        ]
    }

    /// Adds one participant's completed leaves. Relaxed: read after the scope's join.
    fn add(&self, c: &LeafListCounts) {
        for (slot, v) in self.0.iter().zip(Self::fields(c)) {
            slot.fetch_add(v, Ordering::Relaxed);
        }
    }

    /// The sum, after the join.
    fn load(&self) -> LeafListCounts {
        let v = self.0.each_ref().map(|a| a.load(Ordering::Relaxed));
        LeafListCounts {
            leaves: v[0],
            rows: v[1],
            collect_box_active: v[2],
            collect_box_static: v[3],
            cands_active: v[4],
            cands_static: v[5],
            prefilter_chunks: v[6],
            kept_active: v[7],
            kept_static: v[8],
            emitted: v[9],
            sort_shifts: v[10],
        }
    }
}

#[cfg(any(test, feature = "bp-query-counts"))]
impl LeafListCounts {
    /// Adds `other`'s counts to these.
    fn absorb(&mut self, other: &Self) {
        self.leaves += other.leaves;
        self.rows += other.rows;
        self.collect_box_active += other.collect_box_active;
        self.collect_box_static += other.collect_box_static;
        self.cands_active += other.cands_active;
        self.cands_static += other.cands_static;
        self.prefilter_chunks += other.prefilter_chunks;
        self.kept_active += other.kept_active;
        self.kept_static += other.kept_static;
        self.emitted += other.emitted;
        self.sort_shifts += other.sort_shifts;
    }
}

/// The unit gates' knobs and the last dispatch's witness (test builds only). Every knob's default
/// is the production value, so a tree nobody configured runs the production rules.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(super) struct TestHooks {
    /// Lowers the inline threshold ([`S5_MIN_LEAVES`]) to this many leaf nodes.
    pub(super) min_leaves: usize,
    /// Lowers the grain ([`S5_MIN_LEAVES_PER_CHUNK`]) to this many leaf nodes.
    pub(super) grain: usize,
    /// Replaces the per-row budget (`1` forces the tail on almost every leaf).
    pub(super) budget: Option<usize>,
    /// Holds every participant after its first claim until all `P` have claimed one, so each
    /// participant runs a chunk and at least two threads run chunks concurrently (critique W3 (a)).
    pub(super) spread: bool,
    /// Panics in every chunk a helper (participant `p > 0`) runs (critique C1's regression test).
    pub(super) panic_in_helper: bool,
    /// Dispatches without a pool as if it had this many lanes, the calling thread the only
    /// participant (the Miri subset's stacked-borrows leg).
    pub(super) pool_free_lanes: Option<usize>,
    /// The last dispatch's participant count.
    pub(super) last_participants: usize,
    /// The last dispatch's chunks per participant (`[0]` the caller).
    pub(super) last_ran: [u32; MAX_WORKERS],
    /// The per-row budget the last dispatch cut its regions with (`Layout::budget`; triage r2 F1).
    pub(super) last_budget: usize,
}

#[cfg(test)]
impl Default for TestHooks {
    fn default() -> Self {
        Self {
            min_leaves: usize::MAX,
            grain: usize::MAX,
            budget: None,
            spread: false,
            panic_in_helper: false,
            pool_free_lanes: None,
            last_participants: 0,
            last_ran: [0; MAX_WORKERS],
            last_budget: 0,
        }
    }
}

/// The test build's per-dispatch witness and knobs as the wave reads them.
#[cfg(test)]
struct TestWave<'a> {
    hooks: TestHooks,
    /// Chunks each participant ran.
    ran: &'a [AtomicU32],
    /// Participants past their first claim (the spread barrier).
    arrived: &'a AtomicUsize,
}

/// What every participant reads and writes. `Send + Sync` without an `unsafe impl`: shared trees,
/// the two solve views (whose own impls state the per-row discipline) and shared atomics.
struct Wave<'a> {
    trees: Trees<'a>,
    /// The record column's per-row write view.
    recs: ScratchSolveView<'a, RowRec>,
    /// The stream column's per-row write view, at least [`Layout::total`] long.
    stream: ScratchSolveView<'a, u32>,
    /// The leaf-list collection cap.
    cap: usize,
    layout: Layout,
    /// The participant count `P`.
    participants: usize,
    /// The next unclaimed chunk.
    next: &'a AtomicUsize,
    /// Chunks finished, on return or on unwind.
    done: &'a AtomicUsize,
    /// Per chunk: the first of its leaf nodes it did not complete (`hi` when it completed all).
    resume: &'a [AtomicU32],
    #[cfg(any(test, feature = "bp-query-counts"))]
    counts: &'a SharedCounts,
    #[cfg(test)]
    test: TestWave<'a>,
}

/// Counts a chunk as done when dropped — on return and on unwind alike, so a panic in the chunk
/// cannot leave the caller's wait spinning (the critique's C1).
struct ChunkDone<'a>(&'a AtomicUsize);

impl Drop for ChunkDone<'_> {
    #[inline]
    fn drop(&mut self) {
        // Relaxed: the count only ends the caller's wait; the scope's join orders the chunk's
        // writes before every read of them.
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

impl BroadphaseTree {
    /// S5's dispatch decision and, when it dispatches, the whole leaf-list stage on the pool.
    /// Returns `true` when the stage ran — every Q row's segment is in the stream, which is cut to
    /// its written length — and `false` when the caller must run the serial pass. Called with the
    /// switch on and the leaf-list kernel selected, before any view of the stream or the records
    /// is taken.
    ///
    /// `#[inline(never)]`: one call per step with the switch on, and `query_all`'s switch-off path
    /// stays a test of one flag.
    #[inline(never)]
    pub(super) fn try_parallel_query(&mut self, cap: usize) -> bool {
        let leaves = self.active.leaf_nodes().len();
        let min_leaves = S5_MIN_LEAVES;
        #[cfg(test)]
        let min_leaves = min_leaves.min(self.s5_hooks.min_leaves);
        if leaves < min_leaves || self.hist_rows == 0 {
            return false;
        }
        let grain = S5_MIN_LEAVES_PER_CHUNK;
        #[cfg(test)]
        let grain = grain.min(self.s5_hooks.grain);
        // The Miri subset's stacked-borrows leg: no pool exists, and the calling thread alone runs
        // every chunk through the production chunk kernel (the pool's own worker loop trips a
        // pre-existing crossbeam-epoch retag under stacked borrows; `tests.rs`, S5).
        #[cfg(test)]
        if let Some(lanes) = self.s5_hooks.pool_free_lanes {
            return self.dispatch(cap, leaves, grain, lanes, 1, |wave| participate(wave, 0));
        }
        try_with_active_pool(|pool| {
            let lanes = pool.num_threads();
            self.dispatch(cap, leaves, grain, lanes, lanes, |wave| run_wave(pool, wave))
        })
        .unwrap_or(false)
    }

    /// The chunk count for `lanes` lanes, the layout and the reserve check; when the step
    /// dispatches, the stage with at most `max_participants` participants, its wave run by `run`.
    /// Returns whether it dispatched.
    #[inline]
    fn dispatch(
        &mut self,
        cap: usize,
        leaves: usize,
        grain: usize,
        lanes: usize,
        max_participants: usize,
        run: impl FnOnce(&Wave<'_>),
    ) -> bool {
        let chunks = chunk_count(leaves, lanes, grain);
        if chunks == 0 {
            return false;
        }
        let budget = row_budget(self.hist_entries, self.hist_rows);
        #[cfg(test)]
        let budget = self.s5_hooks.budget.unwrap_or(budget);
        let layout = Layout { chunks, leaves, slots: self.active.leaves() as usize, budget };
        if layout.total() > self.aux.capacity() / 2 {
            return beyond_reserve();
        }
        let participants = max_participants.min(chunks).min(MAX_WORKERS);
        self.parallel_leaf_list(cap, layout, participants, run);
        true
    }

    /// The dispatched stage: the stream grown to the region total, the wave (`run`, over
    /// `participants` participants), then the tail and the counters on the calling thread.
    fn parallel_leaf_list(
        &mut self,
        cap: usize,
        layout: Layout,
        participants: usize,
        run: impl FnOnce(&Wave<'_>),
    ) {
        let total = layout.total();
        {
            let mut stream = self.aux.build_view();
            if stream.len() < total {
                grow_stream(&mut stream, total);
            }
        }
        debug_assert!(layout.chunks <= S5_MAX_CHUNKS, "invariant: the chunk count clamps to S5_MAX_CHUNKS");
        debug_assert!(
            (1..=layout.chunks.min(MAX_WORKERS)).contains(&participants),
            "invariant: one to min(C, MAX_WORKERS) participants"
        );
        let next = AtomicUsize::new(0);
        let done = AtomicUsize::new(0);
        let resume: [AtomicU32; S5_MAX_CHUNKS] = [const { AtomicU32::new(0) }; S5_MAX_CHUNKS];
        #[cfg(any(test, feature = "bp-query-counts"))]
        let counts = SharedCounts::new();
        #[cfg(test)]
        let ran: [AtomicU32; MAX_WORKERS] = [const { AtomicU32::new(0) }; MAX_WORKERS];
        #[cfg(test)]
        let arrived = AtomicUsize::new(0);
        {
            let wave = Wave {
                trees: Trees { active: &self.active, statics: &self.statics, sleepers: &self.sleepers },
                recs: self.rec[usize::from(self.cur)].solve_view(),
                stream: self.aux.solve_view(),
                cap,
                layout,
                participants,
                next: &next,
                done: &done,
                resume: &resume[..layout.chunks],
                #[cfg(any(test, feature = "bp-query-counts"))]
                counts: &counts,
                #[cfg(test)]
                test: TestWave { hooks: self.s5_hooks, ran: &ran[..participants], arrived: &arrived },
            };
            debug_assert!(wave.stream.len() >= total, "invariant: the stream holds every region");
            run(&wave);
        }
        #[cfg(test)]
        {
            self.s5_hooks.last_participants = participants;
            self.s5_hooks.last_budget = layout.budget;
            for (slot, r) in self.s5_hooks.last_ran.iter_mut().zip(&ran) {
                *slot = r.load(Ordering::Relaxed);
            }
        }

        let Self {
            active,
            statics,
            sleepers,
            rec,
            cur,
            aux,
            diag,
            query_dispatches,
            query_tail_leaves,
            #[cfg(any(test, feature = "bp-query-counts"))]
            ll_counts,
            ..
        } = self;
        #[cfg(any(test, feature = "bp-query-counts"))]
        {
            // Relaxed: the scope's join ordered every participant's adds before these loads.
            *ll_counts = counts.load();
        }
        let mut recs_view = rec[usize::from(*cur)].build_view();
        let recs = recs_view.as_mut_slice();
        let mut stream = aux.build_view();
        let trees = Trees { active, statics, sleepers };
        let mut w = total;
        let mut served = 0u64;
        let mut tail_leaves = 0u64;
        for (c, slot) in resume.iter().enumerate().take(layout.chunks) {
            let (lo, hi) = layout.leaf_range(c);
            // Relaxed: stored by chunk `c` before the join.
            let first = slot.load(Ordering::Relaxed) as usize;
            debug_assert!(lo <= first && first <= hi, "invariant: a chunk resumes inside its own leaf range");
            served += (first - lo) as u64;
            if first < hi {
                tail_leaves += (hi - first) as u64;
                w = leaf_list_tail(
                    trees,
                    recs,
                    &mut stream,
                    cap,
                    layout.slots,
                    (first, hi),
                    w,
                    diag,
                    &mut served,
                    #[cfg(any(test, feature = "bp-query-counts"))]
                    ll_counts,
                );
            }
        }
        stream.truncate(w);
        diag.leaf_list_leaves += served;
        *query_dispatches += 1;
        *query_tail_leaves += tail_leaves;
    }
}

/// The region total would pass half the stream column's reserve: the step runs the serial pass,
/// which keeps the whole reserve for its own growth and the assembly's.
#[cold]
#[inline(never)]
fn beyond_reserve() -> bool {
    false
}

/// The step's ONE `pool.scope`: `P − 1` helpers and the caller claim chunks; the caller then waits
/// for the last ones.
#[inline]
fn run_wave(pool: &PoolInner, wave: &Wave<'_>) {
    pool.scope(|scope| {
        for p in 1..wave.participants {
            scope.spawn(move || participate(wave, p));
        }
        participate(wave, 0);
        wait_for_chunks(wave);
    });
}

/// Participant `p`'s claim loop: chunks until the counter is exhausted, then its counts.
///
/// `#[inline(never)]`: the chunk kernel is inlined here, and both the helpers' task and the
/// caller's scope body call it, so one copy serves both instead of two in the instruction cache.
#[inline(never)]
fn participate(wave: &Wave<'_>, p: usize) {
    let mut lists = Lists { act: MaybeUninit::uninit(), sta: MaybeUninit::uninit(), slp: MaybeUninit::uninit() };
    let act = CandList::init_in(&mut lists.act);
    let sta = CandList::init_in(&mut lists.sta);
    let slp = CandList::init_in(&mut lists.slp);
    let tables = Tables::of(wave.trees);
    #[cfg(any(test, feature = "bp-query-counts"))]
    let mut acc = LeafListCounts::default();
    #[cfg(test)]
    let mut ran = 0u32;
    loop {
        // Relaxed: exactly-once by the RMW total order; nothing else is published through it.
        let c = wave.next.fetch_add(1, Ordering::Relaxed);
        if c >= wave.layout.chunks {
            break;
        }
        let _done = ChunkDone(wave.done);
        #[cfg(test)]
        {
            wave.test.before_chunk(p, ran == 0, wave.participants);
            ran += 1;
        }
        let first = run_chunk(
            wave,
            &tables,
            c,
            act,
            sta,
            slp,
            #[cfg(any(test, feature = "bp-query-counts"))]
            &mut acc,
        );
        // Relaxed: read after the scope's join.
        wave.resume[c].store(first as u32, Ordering::Relaxed);
    }
    #[cfg(any(test, feature = "bp-query-counts"))]
    wave.counts.add(&acc);
    #[cfg(test)]
    wave.test.ran[p].store(ran, Ordering::Relaxed);
    let _ = p;
}

#[cfg(test)]
impl TestWave<'_> {
    /// The test knobs before a chunk: the spread barrier on a participant's first claim, then the
    /// helper panic.
    fn before_chunk(&self, p: usize, first: bool, participants: usize) {
        if self.hooks.spread && first {
            self.arrived.fetch_add(1, Ordering::Relaxed);
            let mut burst = 0u32;
            while self.arrived.load(Ordering::Relaxed) < participants {
                if burst < S5_SPIN_BURST {
                    core::hint::spin_loop();
                    burst += 1;
                } else {
                    burst = 0;
                    std::thread::yield_now();
                }
            }
        }
        if self.hooks.panic_in_helper && p != 0 {
            panic!("S5 test hook: a helper's chunk panics");
        }
    }
}

/// The caller's wait for the chunks other participants still run: `S5_SPIN_BURST` `PAUSE`s, then a
/// `yield_now`, repeated; never a park.
#[inline]
fn wait_for_chunks(wave: &Wave<'_>) {
    let chunks = wave.layout.chunks;
    let mut burst = 0u32;
    // Relaxed: the wait only decides when the scope's body returns; the join that follows orders
    // every chunk's writes before the caller reads them.
    while wave.done.load(Ordering::Relaxed) < chunks {
        if burst < S5_SPIN_BURST {
            core::hint::spin_loop();
            burst += 1;
        } else {
            burst = 0;
            std::thread::yield_now();
        }
    }
}

/// Collects active leaf node `l`'s three candidate lists, as the serial pass does: the active
/// tree, then the static tree unless it is one leaf node, then the sleeper tree unless it is empty
/// or one leaf node. `false` when a collection passes `cap` (the fallback case).
#[inline]
#[allow(clippy::too_many_arguments)] // the serial pass's own collection inputs, one each
fn collect_leaf<const SITE: u8>(
    trees: Trees<'_>,
    tab: &Tables<'_>,
    l: usize,
    act: &mut CandList,
    sta: &mut CandList,
    slp: &mut CandList,
    cap: usize,
    #[cfg(any(test, feature = "bp-query-counts"))] counts: &mut LeafListCounts,
) -> bool {
    let Trees { active, statics, sleepers } = trees;
    let box_l = active.leaf_box(l);
    // The sleeper collection's box tests: walked like the static collection, not reported.
    #[cfg(any(test, feature = "bp-query-counts"))]
    let mut z_box_tests = 0u64;
    active.collect_leaves_in::<true, SITE>(
        &box_l,
        act,
        cap,
        #[cfg(any(test, feature = "bp-query-counts"))]
        &mut counts.collect_box_active,
    ) && (tab.s_single
        || statics.collect_leaves_in::<false, SITE>(
            &box_l,
            sta,
            cap,
            #[cfg(any(test, feature = "bp-query-counts"))]
            &mut counts.collect_box_static,
        ))
        && (!tab.z_on
            || tab.z_single
            || sleepers.collect_leaves_in::<false, SITE>(
                &box_l,
                slp,
                cap,
                #[cfg(any(test, feature = "bp-query-counts"))]
                &mut z_box_tests,
            ))
}

/// One emit of the lanes of `node` in `mask` at `out[w..]`. `BOUNDED` (a chunk's region): when
/// eight entries might not fit, nothing is written and `over` is set.
#[inline]
fn emit_into<const BOUNDED: bool>(out: &mut [u32], w: usize, node: &Node8, mask: u32, over: &mut bool) -> usize {
    if BOUNDED && w + LANES > out.len() {
        *over = true;
        return w;
    }
    emit_lanes(out, w, node, mask)
}

/// The partners of the row in lane `k` of active leaf node `node` into `out[w..]`, in the serial
/// pass's emit order — the static candidates (or the static tree's one leaf node), the sleeper
/// candidates (or its one leaf node), then the active candidates with the max-row cut and
/// `row > query` — unsorted. Returns the new write index, or `None` when `BOUNDED` and an emit
/// could pass `out`'s end.
#[inline]
#[allow(clippy::too_many_arguments)] // the serial pass's per-row inputs, one each
fn emit_row<const BOUNDED: bool>(
    tab: &Tables<'_>,
    act: &CandList,
    sta: &CandList,
    slp: &CandList,
    node: &Node8,
    k: usize,
    out: &mut [u32],
    w: usize,
    #[cfg(any(test, feature = "bp-query-counts"))] counts: &mut LeafListCounts,
) -> Option<usize> {
    let (x, y, z, r) = (node.p[LEAF_X][k], node.p[LEAF_Y][k], node.p[LEAF_Z][k], node.p[LEAF_R][k]);
    let row = node.p[LEAF_ROW][k].to_bits();
    let q = QueryBox::of(x, y, z, r);
    let mut w = w;
    let mut over = false;
    #[cfg(any(test, feature = "bp-query-counts"))]
    {
        counts.prefilter_chunks += (act.chunks() + if tab.s_single { 0 } else { sta.chunks() }) as u64;
    }
    if tab.s_single {
        let s = &tab.s_leaves[0];
        w = emit_into::<BOUNDED>(out, w, s, leaf_mask(s, x, y, z, r), &mut over);
        #[cfg(any(test, feature = "bp-query-counts"))]
        {
            counts.kept_static += 1;
        }
    } else {
        sta.for_each_kept::<false>(&q, row, |leaf| {
            let s = &tab.s_leaves[leaf as usize];
            w = emit_into::<BOUNDED>(out, w, s, leaf_mask(s, x, y, z, r), &mut over);
            #[cfg(any(test, feature = "bp-query-counts"))]
            {
                counts.kept_static += 1;
            }
        });
    }
    if tab.z_single {
        let s = &tab.z_leaves[0];
        w = emit_into::<BOUNDED>(out, w, s, leaf_mask(s, x, y, z, r), &mut over);
    } else if tab.z_on {
        slp.for_each_kept::<false>(&q, row, |leaf| {
            let s = &tab.z_leaves[leaf as usize];
            w = emit_into::<BOUNDED>(out, w, s, leaf_mask(s, x, y, z, r), &mut over);
        });
    }
    act.for_each_kept::<true>(&q, row, |leaf| {
        #[cfg(any(test, feature = "bp-query-counts"))]
        {
            counts.kept_active += 1;
        }
        debug_assert!((leaf as usize) < tab.a_leaves.len(), "a candidate is a leaf node of the tree");
        // SAFETY: `act` was filled by `active.collect_leaves_in` over this tree, unchanged since,
        // and holds leaf node indices only: `8·i + k` for a lane `k` of level-1 node `i` whose box
        // met the leaf's box, and `build` writes a finite box into lane `k` only when
        // `8·i + k < count[0]` (an empty lane's `+inf / −inf` box meets no finite box, and the
        // leaf's box is a Normal row's, finite), or `0` in a one-level tree. A pad lane (leaf `0`)
        // is never kept: its empty box meets no query box. So `leaf < count[0] = a_leaves.len()`.
        let a = unsafe { tab.a_leaves.get_unchecked(leaf as usize) };
        w = emit_into::<BOUNDED>(out, w, a, leaf_mask_above(a, x, y, z, r, row), &mut over);
    });
    if BOUNDED && over { None } else { Some(w) }
}

/// Writes row `row`'s segment `(seg, nrev, nfwd)` through the record column's solve view, leaving
/// the record's other fields alone.
///
/// # Safety
///
/// `row < recs.len()`, and no other thread reads or writes row `row`'s record while this runs.
#[inline]
unsafe fn write_segment(recs: ScratchSolveView<'_, RowRec>, row: u32, seg: usize, nrev: usize, nfwd: usize) {
    debug_assert!((row as usize) < recs.len(), "invariant: an active row is a current row");
    // SAFETY: `row < recs.len()` (the caller's first condition), so `row_ptr` names the row's
    // record inside the column's committed, address-stable reservation, with the column base's
    // write-capable provenance. The three `&raw mut` projections write the fields without forming a
    // reference to the record, and no other thread touches this record now (the caller's second
    // condition), so the writes race with nothing.
    unsafe {
        let p = recs.row_ptr(row as usize);
        (&raw mut (*p).seg).write(seg as u32);
        (&raw mut (*p).nrev).write(nrev as u32);
        (&raw mut (*p).nfwd).write(nfwd as u32);
    }
}

/// Chunk `c`: its active leaf nodes, in slot order, into its stream region, each row's segment
/// sorted and recorded at `start + w` exactly as the serial pass records it at `w`. Returns the
/// first leaf node it did not complete — its range's end when it completed every one — and adds
/// the completed leaves' counts to `acc`.
#[inline]
fn run_chunk(
    wave: &Wave<'_>,
    tab: &Tables<'_>,
    c: usize,
    act: &mut CandList,
    sta: &mut CandList,
    slp: &mut CandList,
    #[cfg(any(test, feature = "bp-query-counts"))] acc: &mut LeafListCounts,
) -> usize {
    let (lo, hi) = wave.layout.leaf_range(c);
    let (start, len) = wave.layout.region(c);
    debug_assert!(start + len <= wave.stream.len(), "invariant: the region is inside the grown stream");
    // SAFETY: `start + len <= stream.len()` (the stream was grown to the region total, and the
    // regions tile `[0, total)`; `Layout::region`), and `len >= LANES > 0`, so `start < len()` for
    // `row_ptr` and the slice covers initialised `u32`s of the column's committed reservation,
    // aligned for `u32`, with the column base's write-capable provenance. The regions of distinct
    // chunks are disjoint and each chunk is run by one participant, and nothing else forms a
    // reference over the stream until the scope's join (`parallel_leaf_list`), so this slice is
    // the only reference to these entries for its lifetime, which ends with this call.
    let out: &mut [u32] = unsafe { core::slice::from_raw_parts_mut(wave.stream.row_ptr(start), len) };
    let mut w = 0usize;
    for (l, node) in tab.a_leaves.iter().enumerate().take(hi).skip(lo) {
        let live = (wave.layout.slots - l * LANES).min(LANES);
        // A leaf's counts are staged and committed only when it completes, so an abandoned leaf's
        // die with it and the tail's recount is the only one.
        #[cfg(any(test, feature = "bp-query-counts"))]
        let mut leaf = LeafListCounts::default();
        if !collect_leaf::<SITE_CHUNK>(
            wave.trees,
            tab,
            l,
            act,
            sta,
            slp,
            wave.cap,
            #[cfg(any(test, feature = "bp-query-counts"))]
            &mut leaf,
        ) {
            return l;
        }
        #[cfg(any(test, feature = "bp-query-counts"))]
        {
            leaf.leaves = 1;
            leaf.rows = live as u64;
            leaf.cands_active = act.len() as u64;
            leaf.cands_static = if tab.s_single { 1 } else { sta.len() as u64 };
        }
        for k in 0..live {
            let row = node.p[LEAF_ROW][k].to_bits();
            let seg = w;
            let Some(end) = emit_row::<true>(
                tab,
                act,
                sta,
                slp,
                node,
                k,
                out,
                w,
                #[cfg(any(test, feature = "bp-query-counts"))]
                &mut leaf,
            ) else {
                return l;
            };
            w = end;
            let segment = &mut out[seg..w];
            #[cfg(any(test, feature = "bp-query-counts"))]
            {
                leaf.emitted += segment.len() as u64;
                leaf.sort_shifts += inversions(segment);
            }
            sort_leaf_list_segment(segment);
            let nrev = segment.partition_point(|&t| t < row);
            debug_assert!(segment.get(nrev).is_none_or(|&t| t > row), "a row is not its own partner");
            // SAFETY: `row` is a live lane of the active tree, built over current rows, so
            // `row < n = recs.len()`; every row is in exactly one leaf lane and every leaf node in
            // exactly one chunk's range, and each chunk is run by one participant, so no other
            // thread touches this row's record until the scope's join.
            unsafe { write_segment(wave.recs, row, start + seg, nrev, segment.len() - nrev) };
        }
        #[cfg(any(test, feature = "bp-query-counts"))]
        acc.absorb(&leaf);
    }
    hi
}

/// The tail: active leaf nodes `[lo, hi)` answered on the calling thread after the join, from
/// `w` onward, with the serial pass's rules — the stream grown before a leaf whose worst case could
/// pass its length, the per-row walk for a leaf whose collection passes the cap. Returns the new
/// write index; adds its completed leaves to `served` and its fallbacks to `diag`.
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)] // the serial pass's inputs plus the range and the counters
fn leaf_list_tail(
    trees: Trees<'_>,
    recs: &mut [RowRec],
    stream: &mut ScratchBuildView<'_, u32>,
    cap: usize,
    slots: usize,
    (lo, hi): (usize, usize),
    mut w: usize,
    diag: &mut TreeDiag,
    served: &mut u64,
    #[cfg(any(test, feature = "bp-query-counts"))] counts: &mut LeafListCounts,
) -> usize {
    let mut lists = Lists { act: MaybeUninit::uninit(), sta: MaybeUninit::uninit(), slp: MaybeUninit::uninit() };
    let act = CandList::init_in(&mut lists.act);
    let sta = CandList::init_in(&mut lists.sta);
    let slp = CandList::init_in(&mut lists.slp);
    let tab = Tables::of(trees);
    let mut len = stream.len();
    debug_assert!(w <= len, "invariant: the tail starts inside the stream");
    for (l, node) in tab.a_leaves.iter().enumerate().take(hi).skip(lo) {
        let live = (slots - l * LANES).min(LANES);
        if !collect_leaf::<SITE_TAIL>(
            trees,
            &tab,
            l,
            act,
            sta,
            slp,
            cap,
            #[cfg(any(test, feature = "bp-query-counts"))]
            counts,
        ) {
            w = fallback_leaf(trees, recs, stream, w, l, live);
            len = w;
            diag.fallback_leaves += 1;
            continue;
        }
        let z_count = if !tab.z_on {
            0
        } else if tab.z_single {
            1
        } else {
            slp.len()
        };
        let need = live * LANES * (act.len() + if tab.s_single { 1 } else { sta.len() } + z_count);
        if w + need > len {
            len = grow_stream(stream, w + need.max(STREAM_GROW));
        }
        #[cfg(any(test, feature = "bp-query-counts"))]
        {
            counts.leaves += 1;
            counts.rows += live as u64;
            counts.cands_active += act.len() as u64;
            counts.cands_static += if tab.s_single { 1 } else { sta.len() as u64 };
        }
        let out = stream.as_mut_slice();
        for k in 0..live {
            let row = node.p[LEAF_ROW][k].to_bits();
            let seg = w;
            w = emit_row::<false>(
                &tab,
                act,
                sta,
                slp,
                node,
                k,
                out,
                w,
                #[cfg(any(test, feature = "bp-query-counts"))]
                counts,
            )
            .expect("invariant: an unbounded emit always completes");
            let segment = &mut out[seg..w];
            #[cfg(any(test, feature = "bp-query-counts"))]
            {
                counts.emitted += segment.len() as u64;
                counts.sort_shifts += inversions(segment);
            }
            sort_leaf_list_segment(segment);
            let nrev = segment.partition_point(|&t| t < row);
            debug_assert!(segment.get(nrev).is_none_or(|&t| t > row), "a row is not its own partner");
            let rec = &mut recs[row as usize];
            rec.seg = seg as u32;
            rec.nrev = nrev as u32;
            rec.nfwd = (segment.len() - nrev) as u32;
        }
        *served += 1;
    }
    w
}
