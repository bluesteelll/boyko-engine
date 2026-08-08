//! Concurrency analysis: what the schedule **declared** could run together, against what the
//! clock says actually did.
//!
//! # The two halves, and where each comes from
//!
//! **Declared** is the [`ConflictGraph`](crate::ecs::core::schedule::conflict_graph::ConflictGraph):
//! systems `i` and `j` are compatible exactly when `conflict_bits[i]` does not contain `j` — which
//! is already the union of "no access conflict" and "no ordering edge in either direction", the
//! same predicate the executor dispatches on.
//!
//! **Observed** is the interval ring: one appended record per span occurrence, carrying the open
//! stamp and the duration, for the last [`OVERLAP_FRAMES`] frames.
//!
//! # The corpus's `compat` snapshot is not here, and this is why
//!
//! Rev 4 specifies a 1024×1024-bit matrix snapshotted from the conflict graph **at arm** — 128 KiB
//! held for a session — because a graph that changed under the window would make the declared half
//! describe a schedule the observed half never ran. In this engine a `Schedule` is built once, at
//! [`ScheduleBuilder::build`](crate::ecs::core::schedule::schedule_builder::ScheduleBuilder::build),
//! and never rebuilt; `ConflictGraph` is `pub(crate)`, has no mutating method after `build`, and
//! the executor reads it every round. So the live graph *is* the snapshot, and taking a copy would
//! store 128 KiB to restate something immutable.
//!
//! **The residual is named, not waved away:** if a later rung makes schedules rebuildable, this
//! module must either snapshot at arm or refuse to report across a rebuild. It cannot silently
//! keep reading the live graph, because at that point the live graph stops being a statement about
//! the frames in the ring.
//!
//! # And the corpus's `sys_of` side table is not here either
//!
//! `sys_of: [u16; zone_stride]` — 2 KiB built at arm — mapped zone → system so the fold could
//! stamp `Interval.sys`. Rung 3a put the mapping on [`SystemMeta.zone`], which the schedule owns,
//! so the resolution happens here, at report time, in the one place that holds a schedule. The
//! fold stores the zone it already has and says nothing twice.
//!
//! [`SystemMeta.zone`]: crate::ecs::core::system::system_meta::SystemMeta::zone
//! [`OVERLAP_FRAMES`]: crate::ecs::core::profiling::store::OVERLAP_FRAMES
//!
//! # What this module deliberately does not do
//!
//! It does not materialise a per-pair table. At the kernel's own `MAX_SYSTEMS_PER_SCHEDULE = 1024`
//! that is 523 776 rows to answer an aggregate question, so the aggregate is computed in one pass
//! and the per-pair form is a separate call ([`pair_overlap`]) for the pair a caller actually
//! names.

use crate::ecs::core::profiling::ZONE_ID_UNASSIGNED;
use crate::ecs::core::profiling::store::{Interval, Profiler};
use crate::ecs::core::schedule::schedule::Schedule;

/// One pass of declared-versus-observed over the frames the interval ring retains.
///
/// Every field is a **count**, and the one ratio is a method that can refuse. A report that
/// printed a serialisation index of `1.0` for a window in which no compatible pair ever co-ran
/// would be reporting perfect serialisation where the honest answer is "no data".
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ConcurrencyReport {
    /// Systems in the analysed schedule.
    pub systems: u32,
    /// Systems carrying no zone id, so no interval of theirs can be attributed.
    ///
    /// Non-zero means the build folded the per-system tier away, or the zone registry refused
    /// (`W9201`). Either way the numbers below cover a **subset** of the schedule, and this is the
    /// field that says so.
    pub systems_unanalysed: u32,
    /// Frames the ring covered at the moment of the call.
    pub frames_analysed: u32,
    /// Intervals read, over every frame analysed.
    pub intervals_seen: u32,
    /// Spans a full bank refused, session-to-date. **Non-zero invalidates the ratio below**, and
    /// [`serialisation_index`](Self::serialisation_index) does not know that — a caller reporting
    /// the index must report this figure beside it.
    pub intervals_dropped: u64,
    /// Declared-compatible pairs that both ran in **the same frame** at least once.
    ///
    /// Same frame, not merely the same window: two systems that never ran in one frame never had
    /// the opportunity to overlap, and counting them in the denominator would charge the schedule
    /// for parallelism that was never on offer.
    pub compatible_co_ran: u32,
    /// Of those, the pairs whose intervals actually intersected.
    pub compatible_overlapped: u32,
    /// Pairs the graph declared **incompatible** whose intervals nevertheless intersected.
    ///
    /// Reported, never asserted on. Two spans measured on two cores are two `rdtsc` readings, and
    /// invariant TSC is synchronised across cores only to within a small skew — so a pair that
    /// abutted can read as a pair that overlapped by a handful of ticks. A non-zero figure here is
    /// a reason to look, not a proof of a dispatch bug, and no gate in this corpus reds on it.
    pub conflicting_overlapped: u32,
}

impl ConcurrencyReport {
    /// `1 − observed / declared-compatible-and-both-ran`, or `None` when nothing co-ran.
    ///
    /// `0.0` is "every pair that could overlap did"; `1.0` is "none did". `None` is neither, and
    /// is the answer for an empty window.
    #[must_use]
    pub fn serialisation_index(&self) -> Option<f32> {
        if self.compatible_co_ran == 0 {
            return None;
        }
        Some(1.0 - (self.compatible_overlapped as f32) / (self.compatible_co_ran as f32))
    }
}

/// One named pair's verdict, over the same frames [`concurrency`] reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PairVerdict {
    /// What the conflict graph says: `true` when neither an access conflict nor an ordering edge
    /// separates them.
    pub declared_compatible: bool,
    /// Frames in which both produced at least one interval.
    pub frames_co_ran: u32,
    /// Of those, frames in which some interval of one intersected some interval of the other.
    pub frames_overlapped: u32,
}

impl PairVerdict {
    /// The corpus's `observed_frac`: the share of co-running frames that actually overlapped, or
    /// `None` when the pair never co-ran.
    #[must_use]
    pub fn observed_frac(&self) -> Option<f32> {
        if self.frames_co_ran == 0 {
            return None;
        }
        Some((self.frames_overlapped as f32) / (self.frames_co_ran as f32))
    }
}

/// Whether two intervals intersect.
///
/// Half-open on both sides, so an interval that ends exactly where another begins does **not**
/// overlap it: back-to-back execution on one worker is serial, and calling it parallel would be
/// the single most common way for this number to flatter the scheduler. A zero-duration interval
/// therefore overlaps nothing, which is correct — a span too short for the clock to resolve is not
/// evidence of anything.
#[inline]
#[must_use]
fn intersects(a: &Interval, b: &Interval) -> bool {
    let a_end = a.begin.wrapping_add(u64::from(a.dur));
    let b_end = b.begin.wrapping_add(u64::from(b.dur));
    a.begin < b_end && b.begin < a_end
}

/// Map every zone id in the schedule back to its system index.
///
/// A `Vec` rather than a fixed array: the width is `zone_stride`, which is an arm-time value, and
/// this runs once per report on a `#[cold]` path. It is transient function-local scratch — the
/// durable mapping is `SystemMeta.zone`, and this is its inverse for the duration of one call.
fn system_of_zone(profiler: &Profiler, schedule: &Schedule) -> (Vec<u16>, u32) {
    const NO_SYSTEM: u16 = u16::MAX;
    let mut map = vec![NO_SYSTEM; profiler.zone_stride() as usize];
    let mut unanalysed = 0u32;
    for (i, sb) in schedule.systems.iter().enumerate() {
        let zone = sb.system.meta().zone();
        if zone == ZONE_ID_UNASSIGNED || (zone as usize) >= map.len() {
            unanalysed += 1;
            continue;
        }
        map[zone as usize] = i as u16;
    }
    (map, unanalysed)
}

/// Whether the graph lets `a` and `b` run at the same time.
///
/// The same predicate the executor dispatches on, read from the same bits — not a re-derivation,
/// which is how a "declared" half comes to describe a schedule the executor never ran.
#[inline]
#[must_use]
fn declared_compatible(schedule: &Schedule, a: usize, b: usize) -> bool {
    a != b && !schedule.conflict_graph.conflict_bits[a].contains(b)
}

/// Compute the report over every frame the interval ring still covers.
///
/// `#[cold]`: this is a reader's call, made once when somebody asks, and it is O(k²) in the
/// intervals of one frame. Nothing on the frame path reaches it.
#[cold]
#[must_use]
pub fn concurrency(profiler: &Profiler, schedule: &Schedule) -> ConcurrencyReport {
    let n = schedule.systems.len();
    let (map, systems_unanalysed) = system_of_zone(profiler, schedule);

    let mut report = ConcurrencyReport {
        systems: n as u32,
        systems_unanalysed,
        intervals_dropped: profiler.drops().intervals_dropped,
        ..ConcurrencyReport::default()
    };
    if n == 0 {
        return report;
    }

    // Two n×n bit matrices: pairs that co-ran in some frame, and pairs that overlapped in some
    // frame. They exist so a pair counted once stays counted once — a running tally over frames
    // would count a pair that overlapped in three frames three times, and the denominator and the
    // numerator would then be over different populations.
    let words = (n * n).div_ceil(64);
    let mut co_ran = vec![0u64; words];
    let mut overlapped = vec![0u64; words];
    let mut ran_this_frame = vec![false; n];
    let mut sysv: Vec<u16> = Vec::new();

    let frames: Vec<u32> = profiler.interval_frames().collect();
    for frame in frames {
        let ivals = profiler.intervals_of_frame(frame);
        if ivals.is_empty() {
            continue;
        }
        report.frames_analysed += 1;
        report.intervals_seen = report.intervals_seen.saturating_add(ivals.len() as u32);

        // Project the frame's intervals onto system indices once. An interval whose zone belongs
        // to no system — `__frame`, `__fold`, `__round`, a game's own zone — gets `NO_SYSTEM` and
        // is skipped by every test below, rather than being filtered into a second buffer.
        sysv.clear();
        sysv.reserve(ivals.len());
        ran_this_frame.iter_mut().for_each(|r| *r = false);
        for iv in ivals {
            let s = map.get(iv.zone as usize).copied().unwrap_or(u16::MAX);
            if s != u16::MAX {
                ran_this_frame[s as usize] = true;
            }
            sysv.push(s);
        }

        for a in 0..n {
            if !ran_this_frame[a] {
                continue;
            }
            for (b, ran_b) in ran_this_frame.iter().enumerate().skip(a + 1) {
                if *ran_b {
                    set_bit(&mut co_ran, n, a, b);
                }
            }
        }

        for (i, iv_i) in ivals.iter().enumerate() {
            let si = sysv[i];
            if si == u16::MAX {
                continue;
            }
            for (j, iv_j) in ivals.iter().enumerate().skip(i + 1) {
                let sj = sysv[j];
                if sj == u16::MAX || sj == si {
                    continue;
                }
                if intersects(iv_i, iv_j) {
                    let (a, b) = if si < sj { (si, sj) } else { (sj, si) };
                    set_bit(&mut overlapped, n, a as usize, b as usize);
                }
            }
        }
    }

    for a in 0..n {
        for b in (a + 1)..n {
            let compatible = declared_compatible(schedule, a, b);
            let over = get_bit(&overlapped, n, a, b);
            if compatible {
                if get_bit(&co_ran, n, a, b) {
                    report.compatible_co_ran += 1;
                    if over {
                        report.compatible_overlapped += 1;
                    }
                }
            } else if over {
                report.conflicting_overlapped += 1;
            }
        }
    }

    report
}

/// The verdict for one named pair of system indices.
///
/// Out of range or `a == b` yields the default verdict — `declared_compatible: false` and no
/// frames — rather than a panic: a reader asking about a system the schedule does not have has
/// asked a question with an answer.
#[cold]
#[must_use]
pub fn pair_overlap(profiler: &Profiler, schedule: &Schedule, a: u16, b: u16) -> PairVerdict {
    let n = schedule.systems.len();
    let (ai, bi) = (a as usize, b as usize);
    if ai >= n || bi >= n || ai == bi {
        return PairVerdict::default();
    }
    let (map, _) = system_of_zone(profiler, schedule);
    let mut verdict =
        PairVerdict { declared_compatible: declared_compatible(schedule, ai, bi), ..Default::default() };

    let frames: Vec<u32> = profiler.interval_frames().collect();
    for frame in frames {
        let ivals = profiler.intervals_of_frame(frame);
        let mine = |iv: &Interval, want: u16| -> bool {
            map.get(iv.zone as usize).copied().unwrap_or(u16::MAX) == want
        };
        let ran_a = ivals.iter().any(|iv| mine(iv, a));
        let ran_b = ivals.iter().any(|iv| mine(iv, b));
        if !(ran_a && ran_b) {
            continue;
        }
        verdict.frames_co_ran += 1;
        let overlapped = ivals
            .iter()
            .filter(|iv| mine(iv, a))
            .any(|ia| ivals.iter().filter(|iv| mine(iv, b)).any(|ib| intersects(ia, ib)));
        if overlapped {
            verdict.frames_overlapped += 1;
        }
    }
    verdict
}

#[inline]
fn set_bit(bits: &mut [u64], n: usize, a: usize, b: usize) {
    let idx = a * n + b;
    bits[idx / 64] |= 1u64 << (idx % 64);
}

#[inline]
#[must_use]
fn get_bit(bits: &[u64], n: usize, a: usize, b: usize) -> bool {
    let idx = a * n + b;
    bits[idx / 64] & (1u64 << (idx % 64)) != 0
}
