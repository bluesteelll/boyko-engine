//! The particle pool-partition readback (`BOYKO_PARTICLE_READBACK_FRAME=<n>`) —
//! `docs/PARTICLES-PLAN.md` Rev 4, gates **#7** (the four-boundary partition) and **#9** (frame 0).
//!
//! Two things live here, and they are the two halves of one seam:
//!
//! * [`ParticleCountersReadback`] — the DECODED counters, a `World` Resource the runner inserts so
//!   an in-process gate can read them after `App::run` returns, plus the partition predicates
//!   stated as named methods. Pure: no device, no OS, no `cfg`, unit-testable headlessly.
//! * `ParticleReadbackProbe` — the settle → capture driver the windowed frame loop threads through
//!   its steady path, a sibling of `crate::host_dump::HostDump` (`#[cfg(windows)]`, like every
//!   other driver here, because its only caller is the windowed loop).
//!
//! # Why the values are a Resource and not a file
//!
//! Every sibling probe writes a text line, because its consumer is a separate process reading an
//! artifact. This one's consumer is the SAME process: `App::run(&mut self)` returns once the frame
//! loop exits, so the fixture that armed the capture can read the resource straight out of the
//! world. A file would add a format, a parser, and a way for the two to disagree — for a payload
//! whose reader is three lines further down the same function.
//!
//! # What "after N frames" means here
//!
//! `n` is a count of PRESENTED frames, the same clock `host_dump`'s settle window counts, and the
//! capture happens after frame `n` has presented. The counters therefore describe the state that
//! frame's `particle_kickoff`/`particle_sim` left behind — not a later frame's, because the loop
//! exits immediately afterwards.
//!
//! `n = 1` is gate #9's frame-0 case (the boot partition, `alive_count_cur == real_emit_count`,
//! nothing yet retired); `n = 30` is the settled case, at the same instant the image pin captures.

use boyko_rhi_vulkan::compute::{PARTICLE_SORT_BINS, PARTICLE_SORT_LOG_SPAN, particle_sort_key};
use boyko_render::ParticleRender;

use crate::gpu_scene::ParticleCountersRaw;

/// The wave width the artifact line's per-lane rate is computed against — 32, this part's
/// (NVIDIA RTX 3060 Laptop) warp size and the width every `WaveActiveCountBits` in the sim ballots
/// over.
///
/// It is a REPORTING constant, not a device query: nothing in the frame loop depends on it, and the
/// per-wave rate beside it needs no width at all. A reader on a 64-wide part must recompute the lane
/// column with [`ParticleCountersReadback::lane_skip_rate`], which takes the width as its argument
/// precisely so the assumption is visible at the call and not baked into the number.
pub const WAVE_WIDTH_ASSUMED: u32 = 32;

/// The decoded particle counters at the capture frame — a `World`-singleton Resource the runner
/// inserts once, on the frame it captured.
///
/// Every field is a plain `u32` read straight out of the device blocks; the ARITHMETIC over them
/// is the methods below, so a gate asserts a named property rather than re-deriving one.
#[derive(boyko_macros::Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleCountersReadback {
    /// Presented frames that had elapsed when the capture ran.
    pub frames_presented: u32,
    /// The boot-frozen pool capacity — the total the partition must account for.
    pub capacity: u32,
    /// `p_counters.alive_count_cur`: what THIS frame's kickoff published for the sim to walk
    /// (`alive_count_next` of the previous frame, plus this frame's real emit count).
    pub alive_count_cur: u32,
    /// `p_counters.alive_count_next`: the LIST counter the sim accumulated — the survivors, and
    /// what next frame's kickoff would read.
    pub alive_count_next: u32,
    /// `p_counters.dead_count`: the free-list depth after kickoff's pre-decrement and the sim's
    /// pushes.
    pub dead_count: u32,
    /// `p_counters.dead_base`: the base of the free-list window kickoff reserved for emit.
    pub dead_base: u32,
    /// `p_counters.emit_append_base`: the base of the alive-list window emit appended at.
    pub emit_append_base: u32,
    /// `p_counters.real_emit_count`: `min(requested_spawn, dead_count)` — what kickoff let through.
    pub real_emit_count: u32,
    /// `p_counters.clamped_spawns`: spawns kickoff refused because the POOL was full. Zero on a
    /// well-formed fixture, and NOT the same quantity as the host-side emitter-table clamp.
    pub clamped_spawns: u32,
    /// `p_draw_args.additive.instance_count`: the ADDITIVE render counter — the sim's own
    /// `InterlockedAdd` result, and the instance count the indirect draw fetched.
    pub additive_instance_count: u32,
    /// `p_draw_args.alpha.instance_count`: the ALPHA render counter. **Zero at P0** — the class
    /// exists in the layout, its draw is not declared, and a non-zero value here would mean an
    /// alpha survivor was written to a render slot nothing draws.
    pub alpha_instance_count: u32,
    /// `p_draw_args.additive.first_instance` — must be 0 on this device (F5b: `firstInstance` is a
    /// silent corruption class here), read back rather than trusted.
    pub additive_first_instance: u32,
    /// `p_draw_args.alpha.first_instance` — must likewise be 0.
    pub alpha_first_instance: u32,
    /// `p_draw_args.additive.index_count` — 6, the two-triangle billboard quad.
    pub additive_index_count: u32,
    /// `p_counters.waves_evaluated` — **rung P1b**: wave-substeps in which at least one lane needed
    /// the field, so the whole wave paid the edit-list walk.
    ///
    /// **Zero on every configuration except [`ParticleCollision::SdfStats`]**, which is the only arm
    /// that builds the `-D SDF_COLLIDE_STATS` module. A zero here is therefore "the instrument was
    /// not armed", NOT "the field was never evaluated" — which is why
    /// [`skip_census_is_armed`](Self::skip_census_is_armed) exists and why every rate method returns
    /// `None` rather than a number when it does not hold.
    ///
    /// Accumulated from boot across every frame, never reset (see
    /// [`ParticleCounters::waves_evaluated`](boyko_render::ParticleCounters::waves_evaluated)).
    pub waves_evaluated: u32,
    /// `p_counters.waves_skipped` — **rung P1b**: wave-substeps in which NO lane needed the field.
    /// Exclusive with [`waves_evaluated`](Self::waves_evaluated), so the two sum to the wave-substep
    /// count. Same arming and same accumulation.
    pub waves_skipped: u32,
    /// `p_counters.lanes_evaluated` — **rung P1b**: LANES that needed the field, summed over every
    /// wave-substep. The per-lane numerator beside the per-wave pair; same arming and same
    /// accumulation.
    pub lanes_evaluated: u32,
}

impl ParticleCountersReadback {
    /// Decodes one raw capture. `frames_presented` is the runner's own count at the capture.
    #[must_use]
    pub(crate) fn from_raw(raw: &ParticleCountersRaw, frames_presented: u32) -> Self {
        Self {
            frames_presented,
            capacity: raw.capacity,
            alive_count_cur: raw.counters.alive_count_cur,
            alive_count_next: raw.counters.alive_count_next,
            dead_count: raw.counters.dead_count,
            dead_base: raw.counters.dead_base,
            emit_append_base: raw.counters.emit_append_base,
            real_emit_count: raw.counters.real_emit_count,
            clamped_spawns: raw.counters.clamped_spawns,
            additive_instance_count: raw.draw_args.additive.instance_count,
            alpha_instance_count: raw.draw_args.alpha.instance_count,
            additive_first_instance: raw.draw_args.additive.first_instance,
            alpha_first_instance: raw.draw_args.alpha.first_instance,
            additive_index_count: raw.draw_args.additive.index_count,
            waves_evaluated: raw.counters.waves_evaluated,
            waves_skipped: raw.counters.waves_skipped,
            lanes_evaluated: raw.counters.lanes_evaluated,
        }
    }

    /// Whether rung P1b's skip census actually ran — i.e. whether the sim was built from the
    /// `-D SDF_COLLIDE_STATS` module.
    ///
    /// The test is `waves_evaluated + waves_skipped > 0`, and it is the reason the two wave counters
    /// are EXCLUSIVE: their sum is the wave-substep count, so a zero sum can only mean no wave ever
    /// reached the census. Every rate below is gated on it, because `0/0` reported as a rate is the
    /// defect this whole rung exists to remove — gate #17's finding was precisely that an
    /// unarmed-looking instrument and a real measurement were indistinguishable.
    #[must_use]
    pub fn skip_census_is_armed(&self) -> bool {
        u64::from(self.waves_evaluated) + u64::from(self.waves_skipped) > 0
    }

    /// Total wave-substeps the census saw — the denominator of
    /// [`wave_skip_rate`](Self::wave_skip_rate), stated as its own method so a reader can see the
    /// sample size beside the rate.
    #[must_use]
    pub fn wave_substeps(&self) -> u64 {
        u64::from(self.waves_evaluated) + u64::from(self.waves_skipped)
    }

    /// **The skip rate, at WAVE granularity** — `waves_skipped / (waves_skipped + waves_evaluated)`,
    /// in `[0, 1]`. `None` when the census was not armed.
    ///
    /// This is the number rung P1b exists to produce, and the granularity is load-bearing: the skip
    /// is a DIVERGENT branch, so a wave whose lanes disagree executes both sides and every lane in
    /// it pays the walk. Such a wave counts as EVALUATED here. That makes this figure the fraction
    /// of field walks the Lipschitz cache actually deleted — never an upper bound on it.
    #[must_use]
    pub fn wave_skip_rate(&self) -> Option<f64> {
        let total = self.wave_substeps();
        (total > 0).then(|| f64::from(self.waves_skipped) / total as f64)
    }

    /// **The same rate a naive PER-LANE counter would report**, for the same run — `1 −
    /// lanes_evaluated / (wave_substeps × lanes_per_wave)`. `None` when the census was not armed.
    ///
    /// `lanes_per_wave` is the device's wave width, which the host cannot read off these counters,
    /// so it is the caller's to supply (32 on this part). The two rates together are the point of
    /// the rung: the gap between them IS the wave's incoherence, and the plan's rule — *"a skip rate
    /// quoted per lane would overstate the win"* — is a statement about that gap.
    ///
    /// ⚠️ The denominator assumes FULL waves. A dispatch's last wave is partial whenever
    /// `alive_count_cur` is not a multiple of the wave width, so at small alive counts this figure
    /// is biased HIGH (it credits inactive lanes with skipping). At the saturated densities the
    /// measurement uses — 10 240 and up, all multiples of 32 — the bias is zero.
    #[must_use]
    pub fn lane_skip_rate(&self, lanes_per_wave: u32) -> Option<f64> {
        let waves = self.wave_substeps();
        if waves == 0 || lanes_per_wave == 0 {
            return None;
        }
        let lane_substeps = waves * u64::from(lanes_per_wave);
        Some(1.0 - (f64::from(self.lanes_evaluated) / lane_substeps as f64))
    }

    /// **Boundary B3**, the plan's two-term equality at the sim→draw edge: `alive_count_next +
    /// dead_count == CAP`.
    ///
    /// This is the whole pool accounted for exactly once — every slot is either on the alive list
    /// the next kickoff will walk, or on the free list emit will draw from. A LEAK (a slot on
    /// neither) makes the sum small; a DOUBLE-COUNT (a slot on both) makes it large. Both are
    /// silent in every image.
    #[must_use]
    pub fn partition_holds(&self) -> bool {
        u64::from(self.alive_count_next) + u64::from(self.dead_count) == u64::from(self.capacity)
    }

    /// The plan's M2 class-split assertion: the two RENDER counters sum to the LIST counter.
    ///
    /// At P0 the alpha term is structurally zero, so this reads "every survivor got an additive
    /// render slot". It is stated as the sum anyway because that is the form that catches rung
    /// P2's alpha leak — the defect where a class allocates its list index from a counter kickoff
    /// never reads, and its particles vanish from the next frame's walk.
    #[must_use]
    pub fn class_split_sums_to_list(&self) -> bool {
        u64::from(self.additive_instance_count) + u64::from(self.alpha_instance_count)
            == u64::from(self.alive_count_next)
    }

    /// The **frame-0** property (gate #9): with nothing alive before the first frame, kickoff's
    /// `A = alive_count_next + E` reduces to `A == E`, and the emit window opens at 0.
    ///
    /// Only meaningful at [`frames_presented`](Self::frames_presented) `== 1`; a later frame has
    /// survivors and the equality is expected to fail.
    #[must_use]
    pub fn frame_zero_shape_holds(&self) -> bool {
        self.alive_count_cur == self.real_emit_count && self.emit_append_base == 0
    }

    /// F5b, read back rather than trusted: `firstInstance` is 0 in BOTH draw slots, and the
    /// additive slot still names the 6-index quad.
    #[must_use]
    pub fn draw_args_are_well_formed(&self) -> bool {
        self.additive_first_instance == 0
            && self.alpha_first_instance == 0
            && self.additive_index_count == 6
    }

    /// The one-line artifact form — printed by the runner at the capture, so a run's partition is
    /// readable from its own log even when nothing in-process asserted it.
    ///
    /// The rung-P1b census tail is printed UNCONDITIONALLY, including its `census=false` state: a
    /// line that simply omitted the counters when they were zero would make an unarmed run and an
    /// armed one with nothing to report look the same in an artifact, which is the exact confusion
    /// gate #17 recorded as the reason this instrument had to be built.
    #[must_use]
    pub fn artifact_line(&self) -> String {
        let wave_rate = self.wave_skip_rate().map_or(-1.0, |r| r);
        let lane_rate = self.lane_skip_rate(WAVE_WIDTH_ASSUMED).map_or(-1.0, |r| r);
        format!(
            "particle_counters frames={} cap={} alive_cur={} alive_next={} dead={} dead_base={} \
             emit_base={} real_emit={} clamped={} additive_instances={} alpha_instances={} \
             partition={} class_split={} draw_args_ok={} census={} waves_evaluated={} \
             waves_skipped={} lanes_evaluated={} wave_skip_rate={:.4} lane_skip_rate={:.4}",
            self.frames_presented,
            self.capacity,
            self.alive_count_cur,
            self.alive_count_next,
            self.dead_count,
            self.dead_base,
            self.emit_append_base,
            self.real_emit_count,
            self.clamped_spawns,
            self.additive_instance_count,
            self.alpha_instance_count,
            self.partition_holds(),
            self.class_split_sums_to_list(),
            self.draw_args_are_well_formed(),
            self.skip_census_is_armed(),
            self.waves_evaluated,
            self.waves_skipped,
            self.lanes_evaluated,
            wave_rate,
            lane_rate,
        )
    }
}

// ---- Rung P2 item 3: the SORT MONOTONICITY readback (plan P2's named gate) -----------------

/// The largest number of alpha render records either half of the sort readback copies back —
/// 16 384, i.e. 512 KB per range and 1 MB for the pair.
///
/// A bound and not the whole class, because `p_render` is `CAP × 32 B` (8.4 MB at the default
/// capacity) and a gate that copies a scene-sized buffer is a gate nobody runs. Every fixture leg
/// in this tree is well inside it — the saturated alpha leg is 5 120 records — so on the legs that
/// gate this rung the readback sees the ENTIRE class and
/// [`ParticleSortRangeScan::is_complete`](ParticleSortRangeScan::is_complete) says so per capture
/// rather than leaving the reader to assume it.
pub const PARTICLE_SORT_READBACK_MAX_RECORDS: u32 = 16_384;

/// The sentinel [`ParticleSortRangeScan::first_inversion_rank`] carries when the scanned range is
/// monotone — `u32::MAX`, which is unreachable as a rank because
/// [`PARTICLE_SORT_READBACK_MAX_RECORDS`] bounds the scan.
pub const PARTICLE_SORT_NO_INVERSION: u32 = u32::MAX;

/// **The sort's correctness instrument** (plan P2, "sort monotonicity readback"): what a scan of
/// one contiguous alpha range says about its order.
///
/// # Why THIS is the gate, and why no image can be
///
/// Gate #16's order-independence argument does not transfer to a non-commutative blend, so no image
/// pin may be authored over overlapping alpha billboards — the plan records that at three code
/// sites. And rung P2 item 2 measured the harder half: a wrong alpha index transform produced a
/// dump BYTE-IDENTICAL to the `particle_additive` golden. **A byte-identical golden can hide a
/// wrong answer**, so the sort's gate has to be a statement about the ORDER itself.
///
/// # What it reports on a wrong range
///
/// * **UNSORTED** (the order the sim's waves retired in) — keys jump both ways, so
///   [`inversions`](Self::inversions) is large, [`first_inversion_rank`](Self::first_inversion_rank)
///   is small, and [`max_depth_ratio`](Self::max_depth_ratio) far exceeds one bin's width.
/// * **REVERSED** (front-to-back) — keys are non-INcreasing, so almost every adjacent pair is an
///   inversion: `inversions ≈ records_checked − 1` and `first_inversion_rank == 0` unless the first
///   two share a bin.
/// * **PARTIALLY sorted** (one pass of a two-pass radix, a lost group's reservation) —
///   `first_inversion_rank` is the exact rank of the first out-of-order pair, which is the number
///   that localizes the defect rather than merely reporting it.
///
/// # Two claims, one oracle-free
///
/// [`inversions`](Self::inversions) is computed from the HOST mirror of the device key
/// (`compute::particle_sort_key`), so it is exact only while the two agree — and `log2` is not a
/// correctly-rounded operation on either side, so a record sitting exactly on a bin boundary may
/// quantize one step differently here. [`max_depth_ratio`](Self::max_depth_ratio) needs no oracle
/// at all: it is a statement about the DEPTHS the records carry, and a correctly sorted range
/// satisfies `depth[r+1] ≤ depth[r] · 2^(SPAN/BINS)` because two elements out of depth order must
/// share a bin, and one bin is exactly that wide. Reported together so a boundary artefact is
/// diagnosable instead of being the one number a reader has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleSortRangeScan {
    /// The class's live length as `alpha.instanceCount` reported it — the population the scan is a
    /// prefix of.
    pub alpha_count: u32,
    /// How many records the scan actually walked: `min(alpha_count,
    /// PARTICLE_SORT_READBACK_MAX_RECORDS)`.
    pub records_checked: u32,
    /// Adjacent pairs whose host-recomputed key DECREASES with rank — zero for a correctly ordered
    /// range. The key is inverted (bin 0 is the farthest), so a non-decreasing key sequence is
    /// exactly back-to-front.
    pub inversions: u32,
    /// The rank `r` of the FIRST pair with `key[r] > key[r+1]`, or [`PARTICLE_SORT_NO_INVERSION`].
    pub first_inversion_rank: u32,
    /// The key at rank 0 — the farthest particle's bin on a correct range.
    pub key_first: u32,
    /// The key at rank `records_checked − 1`.
    pub key_last: u32,
    /// Distinct keys seen, in scan order. **The scan's own non-vacuity number**: a range whose
    /// particles all land in ONE bin is trivially monotone, so a gate that asserted monotonicity
    /// without reading this could pass on a fixture that proves nothing.
    pub distinct_keys: u32,
    /// The largest `depth[r+1] / depth[r]` over the scanned pairs — the ORACLE-FREE half. A
    /// correctly sorted range keeps it at or below one bin's width (`2^(SPAN/BINS)` ≈ 1.0415);
    /// `1.0` or less means every pair was strictly ordered.
    pub max_depth_ratio: f32,
    /// The camera distance at rank 0.
    pub depth_first: f32,
    /// The camera distance at rank `records_checked − 1`.
    pub depth_last: f32,
}

impl ParticleSortRangeScan {
    /// The empty scan — what a capture of a class with no live particles reports. Monotone and
    /// VACUOUS, which [`is_conclusive`](Self::is_conclusive) is what distinguishes.
    pub const EMPTY: Self = Self {
        alpha_count: 0,
        records_checked: 0,
        inversions: 0,
        first_inversion_rank: PARTICLE_SORT_NO_INVERSION,
        key_first: 0,
        key_last: 0,
        distinct_keys: 0,
        max_depth_ratio: 0.0,
        depth_first: 0.0,
        depth_last: 0.0,
    };

    /// Whether the whole class was walked rather than a prefix of it.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.records_checked == self.alpha_count
    }

    /// **The monotonicity property itself**: the key sequence never decreases with rank.
    #[must_use]
    pub fn is_monotone(&self) -> bool {
        self.inversions == 0
    }

    /// Whether this scan can support ANY verdict — at least two records and at least two distinct
    /// bins among them.
    ///
    /// A single-bin range is monotone for a reason that has nothing to do with the sort, and a
    /// one-record range is monotone by arithmetic. Asserting `is_monotone()` without this is the
    /// vacuous-green shape this campaign has found five times; the gate asserts BOTH.
    #[must_use]
    pub fn is_conclusive(&self) -> bool {
        self.records_checked >= 2 && self.distinct_keys >= 2
    }

    /// The oracle-free bound: no adjacent pair rises in depth by more than one bin's width.
    ///
    /// `tolerance` is that width, `2^(PARTICLE_SORT_LOG_SPAN / PARTICLE_SORT_BINS)`, which
    /// [`particle_sort_bin_depth_ratio`] computes from the same two constants the shaders are
    /// generated from.
    #[must_use]
    pub fn depth_order_holds(&self, tolerance: f32) -> bool {
        self.records_checked < 2 || self.max_depth_ratio <= tolerance
    }

    /// The one-line artifact form, for the runner's capture log.
    #[must_use]
    pub fn artifact_line(&self, label: &str) -> String {
        format!(
            "particle_sort[{label}] alpha={} checked={} complete={} inversions={} \
             first_inversion={} distinct_keys={} monotone={} conclusive={} key_first={} \
             key_last={} depth_first={:.4} depth_last={:.4} max_depth_ratio={:.6}",
            self.alpha_count,
            self.records_checked,
            self.is_complete(),
            self.inversions,
            self.first_inversion_rank,
            self.distinct_keys,
            self.is_monotone(),
            self.is_conclusive(),
            self.key_first,
            self.key_last,
            self.depth_first,
            self.depth_last,
            self.max_depth_ratio,
        )
    }
}

/// **The sort readback's non-vacuity CONTROL, taken in the same submit as the measurement.**
///
/// `sorted` scans `p_render_sorted` (the scatter's output) and `source` scans `p_render` (the
/// unsorted class the sim wrote), for the SAME frame and the SAME particles. The gate asserts that
/// `sorted` is monotone and conclusive **and that `source` is not monotone** — so the instrument
/// proves, in every run, that it can tell the two apart.
///
/// A control taken as a second RUN would not have this property: two runs do not share a spawn
/// seed, so a difference between them is a distribution comparison. Two ranges of one frame are a
/// per-record one.
#[derive(boyko_macros::Resource, Clone, Copy, Debug, PartialEq)]
pub struct ParticleSortReadback {
    /// Presented frames that had elapsed when the capture ran.
    pub frames_presented: u32,
    /// The scatter's destination — the range the alpha draw actually reads under a sorting arming.
    pub sorted: ParticleSortRangeScan,
    /// The sim's own unsorted output, still in `p_render` — the CONTROL.
    pub source: ParticleSortRangeScan,
}

impl ParticleSortReadback {
    /// **The whole gate as one predicate**: the destination is conclusively monotone and the source
    /// is not.
    ///
    /// Both halves are load-bearing, and the second is the one this campaign keeps having to add:
    /// without it a scatter that copied nothing at all — leaving `p_render_sorted` at its boot
    /// zeroes, every record at the origin, every key identical — would report `inversions == 0` and
    /// pass. `is_conclusive` on the destination refuses that, and `!source.is_monotone()` proves the
    /// fixture had an order to fix.
    #[must_use]
    pub fn sort_is_proven(&self) -> bool {
        self.sorted.is_monotone() && self.sorted.is_conclusive() && !self.source.is_monotone()
    }

    /// Both halves' artifact lines, newline-joined — printed by the runner at the capture.
    #[must_use]
    pub fn artifact_lines(&self) -> String {
        format!(
            "{}\n{}",
            self.sorted.artifact_line("sorted"),
            self.source.artifact_line("source")
        )
    }
}

/// One bin's width as a DEPTH RATIO — `2^(PARTICLE_SORT_LOG_SPAN / (PARTICLE_SORT_BINS − 1))`,
/// ≈ 1.041559.
///
/// The tolerance [`ParticleSortRangeScan::depth_order_holds`] is stated against, derived from the
/// two constants the shaders are generated from rather than written as a literal: moving the range
/// moves this with it.
///
/// # ⚠️ The divisor is `BINS − 1`, and getting it wrong is an off-by-one that only a DENSE range
/// reveals
///
/// The key quantizes with `round(t · 255)` over `t ∈ [0, 1]`, so the map has **255 steps, not 256**
/// — bin `b` covers `t ∈ [(b − ½)/255, (b + ½)/255)`, and the two end bins are half-width. One step
/// is therefore `SPAN/255` octaves, not `SPAN/256`.
///
/// **MEASURED, 2026-08-21**: the first cut of this function divided by `BINS` and gave 1.041450.
/// The 30-particle lab leg passed it (its widest adjacent pair was 1.033370 — no two particles were
/// at opposite ends of one bin), and the **saturated 32 256-particle leg reddened it at 1.041559**,
/// which is `2^(15/255)` to six figures — the correct width, produced by a correctly sorted range.
/// The bound was wrong, not the sort. Recorded because the sparse leg is the one a reader would
/// reach for first, and it cannot see this.
#[must_use]
pub fn particle_sort_bin_depth_ratio() -> f32 {
    (PARTICLE_SORT_LOG_SPAN / (PARTICLE_SORT_BINS - 1) as f32).exp2()
}

/// **The pure half of the sort readback**: scans one contiguous run of alpha render records, in
/// RANK order, and reports its order.
///
/// `records[r]` must be the record at rank `r` — the caller is responsible for undoing the class's
/// `capacity - 1 - rank` mirror, which it does by reading the range backwards. Device-free and
/// allocation-free, so the whole verdict is unit-testable against a hand-built range.
#[must_use]
pub fn scan_alpha_range(records: &[ParticleRender], cam_eye: [f32; 3]) -> ParticleSortRangeScan {
    let checked = records.len() as u32;
    if records.is_empty() {
        return ParticleSortRangeScan::EMPTY;
    }
    let depth_of = |r: &ParticleRender| -> f32 {
        let dx = cam_eye[0] - r.position[0];
        let dy = cam_eye[1] - r.position[1];
        let dz = cam_eye[2] - r.position[2];
        (dx * dx + dy * dy + dz * dz).sqrt()
    };

    let mut prev_key = particle_sort_key(records[0].position, cam_eye);
    let mut prev_depth = depth_of(&records[0]);
    let key_first = prev_key;
    let depth_first = prev_depth;
    let mut inversions = 0u32;
    let mut first_inversion_rank = PARTICLE_SORT_NO_INVERSION;
    // The keys are u8-valued, so "distinct in scan order" is a run count: a correctly sorted range
    // is non-decreasing, and a change of key is then a change of bin. On an UNSORTED range this
    // over-counts — which is the harmless direction, since the number only ever guards against a
    // range too uniform to prove anything.
    let mut distinct_keys = 1u32;
    let mut max_depth_ratio = 0.0f32;

    for (i, r) in records[1..].iter().enumerate() {
        let key = particle_sort_key(r.position, cam_eye);
        let depth = depth_of(r);
        if key < prev_key {
            if first_inversion_rank == PARTICLE_SORT_NO_INVERSION {
                // The rank of the pair's FIRST element: `r` sits at rank `i + 1`, so the pair
                // begins at `i`. Naming the pair's HEAD is what makes the number a place to look
                // rather than a place a break was noticed.
                first_inversion_rank = i as u32;
            }
            inversions += 1;
        }
        if key != prev_key {
            distinct_keys += 1;
        }
        // `prev_depth` can be zero only for a particle at the eye, which the key's own near clamp
        // already folds into bin 255; guard anyway so the ratio is a number rather than an inf.
        if prev_depth > 0.0 {
            let ratio = depth / prev_depth;
            if ratio > max_depth_ratio {
                max_depth_ratio = ratio;
            }
        }
        prev_key = key;
        prev_depth = depth;
    }

    ParticleSortRangeScan {
        // Filled by the caller, which is the side that knows the class's true length; the scan
        // itself only ever sees the prefix it was handed.
        alpha_count: checked,
        records_checked: checked,
        inversions,
        first_inversion_rank,
        key_first,
        key_last: prev_key,
        distinct_keys,
        max_depth_ratio,
        depth_first,
        depth_last: prev_depth,
    }
}

/// The settle → capture driver the windowed frame loop threads through its steady path.
///
/// Armed by `BOYKO_PARTICLE_READBACK_FRAME=<n>`; `n` is the presented-frame count to capture
/// after. No DRAIN phase, unlike its siblings: the capture is an out-of-band idled submit
/// (`ParticleGpuBundle::read_counters`), so there is no in-flight per-FIF staging whose fence has
/// to be re-waited — the device idle covers strictly more than a drain would.
#[cfg(windows)]
pub(crate) struct ParticleReadbackProbe {
    /// Presented frames still to elapse before the capture. Reaching zero arms it.
    remaining: u32,
    /// Presented frames counted so far — the number the capture is labelled with.
    presented: u32,
    /// Set once the capture has run, so the driver reports ready exactly once.
    done: bool,
}

#[cfg(windows)]
impl ParticleReadbackProbe {
    /// Arms the probe iff `BOYKO_PARTICLE_READBACK_FRAME` is set. The value is the presented-frame
    /// count to capture after; an unparsable or absent value falls back to
    /// [`DEFAULT_CAPTURE_FRAME`](Self::DEFAULT_CAPTURE_FRAME) rather than disarming, because a
    /// typo must not turn a gate into a run that quietly measures nothing.
    ///
    /// Cold: called once before the frame loop.
    pub(crate) fn from_env() -> Option<Self> {
        let raw = std::env::var("BOYKO_PARTICLE_READBACK_FRAME").ok()?;
        let frame: u32 = raw.trim().parse().unwrap_or(Self::DEFAULT_CAPTURE_FRAME);
        boyko_log::info!(
            boyko_log::Host,
            "BOYKO_PARTICLE_READBACK_FRAME armed -> capture after presented frame {}",
            frame
        );
        Some(Self { remaining: frame.max(1), presented: 0, done: false })
    }

    /// The settle length a bare/mistyped value falls back to — `host_dump`'s own settle window, so
    /// an unqualified "read the counters" lands on the same frame the image pin captures.
    pub(crate) const DEFAULT_CAPTURE_FRAME: u32 = 30;

    /// Counts one loop iteration and reports whether THIS iteration is the capture.
    ///
    /// `presented` is the frame loop's own `Ok(true)` signal — the same one every sibling driver
    /// counts, so a recreate-skip advances no driver's window.
    pub(crate) fn after_present(&mut self, presented: bool) -> bool {
        if self.done || !presented {
            return false;
        }
        self.presented += 1;
        self.remaining -= 1;
        if self.remaining == 0 {
            self.done = true;
            return true;
        }
        false
    }

    /// The presented-frame count at the capture — the label the readback carries.
    pub(crate) fn presented(&self) -> u32 {
        self.presented
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A readback with a chosen alive/dead split, everything else well-formed.
    fn readback(cap: u32, alive_next: u32, dead: u32, additive: u32) -> ParticleCountersReadback {
        ParticleCountersReadback {
            frames_presented: 30,
            capacity: cap,
            alive_count_cur: alive_next,
            alive_count_next: alive_next,
            dead_count: dead,
            dead_base: 0,
            emit_append_base: 0,
            real_emit_count: 0,
            clamped_spawns: 0,
            additive_instance_count: additive,
            alpha_instance_count: 0,
            additive_first_instance: 0,
            alpha_first_instance: 0,
            additive_index_count: 6,
            waves_evaluated: 0,
            waves_skipped: 0,
            lanes_evaluated: 0,
        }
    }

    // ---- Rung P2 item 3: the sort scan, device-free -------------------------------------

    /// A render record at `distance` units along +X from the origin — the only field
    /// [`scan_alpha_range`] reads is `position`, so the rest is left at its zero value.
    fn at(distance: f32) -> ParticleRender {
        ParticleRender {
            position: [distance, 0.0, 0.0],
            size: 1.0,
            color_rgba8: 0,
            rot_cs: 0,
            tex_index: 0,
            flags: 0,
        }
    }

    /// The eye every scan below measures from.
    const EYE: [f32; 3] = [0.0, 0.0, 0.0];

    /// A depth ladder wide enough to span many bins: each step is a factor of two, i.e. a whole
    /// octave, which is ~17 bins apart at the shipped 15-octave / 256-bin range.
    fn far_to_near() -> Vec<ParticleRender> {
        vec![at(64.0), at(32.0), at(16.0), at(8.0), at(4.0), at(2.0), at(1.0), at(0.5)]
    }

    /// **What the gate reports on a CORRECT range.** Rank 0 is the farthest; the key never
    /// decreases; the depths never rise.
    #[test]
    fn a_back_to_front_range_is_monotone_and_conclusive() {
        let scan = scan_alpha_range(&far_to_near(), EYE);
        assert_eq!(scan.inversions, 0, "a back-to-front range has no inversion");
        assert_eq!(scan.first_inversion_rank, PARTICLE_SORT_NO_INVERSION);
        assert!(scan.is_monotone() && scan.is_conclusive());
        assert!(
            scan.key_first < scan.key_last,
            "the key is INVERTED (bin 0 is the farthest), so it must RISE from far to near: \
             {} -> {}",
            scan.key_first,
            scan.key_last
        );
        assert!(scan.depth_first > scan.depth_last, "rank 0 is the farthest");
        // Eight octaves over the ladder ⇒ eight distinct bins, and the oracle-free bound holds
        // because every step goes DOWN in depth.
        assert_eq!(scan.distinct_keys, 8);
        assert!(scan.depth_order_holds(particle_sort_bin_depth_ratio()));
        assert!(scan.max_depth_ratio < 1.0, "every adjacent step falls in depth");
    }

    /// **What the gate reports on a REVERSED range** — front-to-back, the exact opposite of what
    /// `alpha_over` needs. Nearly every adjacent pair is an inversion and the first is at rank 0.
    #[test]
    fn a_front_to_back_range_inverts_at_every_step() {
        let mut records = far_to_near();
        records.reverse();
        let scan = scan_alpha_range(&records, EYE);
        assert_eq!(
            scan.inversions,
            (records.len() - 1) as u32,
            "a strictly reversed range inverts at every adjacent pair"
        );
        assert_eq!(scan.first_inversion_rank, 0, "the very first pair is already wrong");
        assert!(!scan.is_monotone());
        assert!(
            !scan.depth_order_holds(particle_sort_bin_depth_ratio()),
            "the oracle-free bound catches it too: each step DOUBLES the depth, which is ~17 bins"
        );
    }

    /// **What the gate reports on a PARTIALLY sorted range** — one pair out of order in the middle.
    /// `first_inversion_rank` localizes the defect rather than merely reporting it.
    #[test]
    fn a_partially_sorted_range_names_the_rank_of_its_first_break() {
        let mut records = far_to_near();
        records.swap(4, 5);
        let scan = scan_alpha_range(&records, EYE);
        assert_eq!(scan.inversions, 1, "one swapped pair is one inversion");
        assert_eq!(
            scan.first_inversion_rank, 4,
            "the break is the pair (rank 4, rank 5) — the rank NAMED is the pair's first element"
        );
        assert!(!scan.is_monotone());
    }

    /// **The vacuity the control exists to refuse**: a scatter that wrote NOTHING leaves
    /// `p_render_sorted` at its boot zeroes, so every record sits at the origin, every key is the
    /// same, and the range is monotone. `is_conclusive` is what refuses it.
    #[test]
    fn an_all_zero_range_is_monotone_and_inconclusive() {
        let records = vec![at(0.0); 16];
        let scan = scan_alpha_range(&records, EYE);
        assert!(scan.is_monotone(), "a run of identical keys has no inversion — that is the trap");
        assert_eq!(scan.distinct_keys, 1);
        assert!(
            !scan.is_conclusive(),
            "one distinct bin cannot support a verdict, which is exactly what a scatter that never \
             ran produces"
        );
        // ...and the near clamp is what keeps a particle AT the eye from producing -inf: it lands
        // in the nearest bin, deterministically.
        assert_eq!(scan.key_first, PARTICLE_SORT_BINS - 1);
    }

    /// A one-record range is monotone by arithmetic, and a zero-record one is
    /// [`ParticleSortRangeScan::EMPTY`]. Both are inconclusive, and both are states a live capture
    /// can be in (an alpha class with one particle, or none at all).
    #[test]
    fn short_ranges_are_monotone_and_inconclusive() {
        assert_eq!(scan_alpha_range(&[], EYE), ParticleSortRangeScan::EMPTY);
        assert!(ParticleSortRangeScan::EMPTY.is_monotone());
        assert!(!ParticleSortRangeScan::EMPTY.is_conclusive());
        let one = scan_alpha_range(&[at(3.0)], EYE);
        assert_eq!(one.records_checked, 1);
        assert!(one.is_monotone() && !one.is_conclusive());
    }

    /// **The whole gate as one predicate**, exercised in all four corners — because the composite
    /// is what the device-side gate asserts last, and a composite that disagreed with its parts
    /// would be a second opinion rather than a summary.
    #[test]
    fn sort_is_proven_needs_all_three_of_its_terms() {
        let good = scan_alpha_range(&far_to_near(), EYE);
        let mut reversed_records = far_to_near();
        reversed_records.reverse();
        let bad = scan_alpha_range(&reversed_records, EYE);
        let flat = scan_alpha_range(&vec![at(0.0); 16], EYE);

        let rb = |sorted, source| ParticleSortReadback { frames_presented: 30, sorted, source };
        // Monotone destination + disordered source ⇒ proven.
        assert!(rb(good, bad).sort_is_proven());
        // A disordered destination is the defect the gate exists for.
        assert!(!rb(bad, bad).sort_is_proven());
        // A monotone SOURCE means the frame could not distinguish a working sort from a verbatim
        // copy — the control is vacuous, so nothing is proven.
        assert!(!rb(good, good).sort_is_proven());
        // A flat destination is the scatter-never-ran case: monotone, and refused by conclusiveness.
        assert!(!rb(flat, bad).sort_is_proven());
    }

    /// The bin width the oracle-free bound is stated against is DERIVED from the two constants the
    /// shaders are generated from, so moving the range moves it.
    ///
    /// **The divisor is `BINS − 1`, and this test is written to red on `BINS`** — the off-by-one a
    /// 30-particle leg cannot see (see [`particle_sort_bin_depth_ratio`]'s doc for the measurement
    /// that found it). `2^(15/255) = 1.0415593` against `2^(15/256) = 1.0414502`: they differ in the
    /// fourth decimal, so the tolerance below is deliberately tight enough to tell them apart.
    #[test]
    fn one_bin_is_the_octave_span_divided_by_the_step_count() {
        let ratio = particle_sort_bin_depth_ratio();
        // The key quantizes with `round(t * 255)`, so the map has 255 STEPS over `t ∈ [0, 1]`.
        let steps = (PARTICLE_SORT_BINS - 1) as f32;
        let want = (PARTICLE_SORT_LOG_SPAN / steps).exp2();
        assert!(
            (ratio - want).abs() < 1e-7,
            "one bin is 2^(SPAN/(BINS-1)) = {want}, got {ratio} — dividing by BINS instead gives \
             {}, which is 1e-4 SMALLER and reddens on a correctly sorted DENSE range",
            (PARTICLE_SORT_LOG_SPAN / PARTICLE_SORT_BINS as f32).exp2()
        );
        // The 255 steps tile the octave span exactly, which is what "constant RELATIVE resolution"
        // means and why the key is logarithmic at all.
        let spanned = ratio.powf(steps);
        assert!(
            (spanned.log2() - PARTICLE_SORT_LOG_SPAN).abs() < 1e-2,
            "the steps must tile the octave span exactly: {} vs {PARTICLE_SORT_LOG_SPAN}",
            spanned.log2()
        );
        // And the wrong divisor does NOT tile it — the property that makes the assertion above a
        // discriminator rather than a restatement.
        let wrong = (PARTICLE_SORT_LOG_SPAN / PARTICLE_SORT_BINS as f32).exp2();
        assert!(wrong < ratio, "2^(SPAN/256) < 2^(SPAN/255), so the wrong divisor is the TIGHT one");
    }

    #[test]
    fn the_partition_is_an_equality_not_an_inequality() {
        assert!(readback(1024, 24, 1000, 24).partition_holds());
        // A LEAK: a slot on neither list.
        assert!(!readback(1024, 24, 999, 24).partition_holds());
        // A DOUBLE-COUNT: a slot on both.
        assert!(!readback(1024, 24, 1001, 24).partition_holds());
    }

    /// The partition is checked in 64-bit arithmetic, so a capacity near `u32::MAX` cannot make a
    /// wrapping sum look like a valid split.
    #[test]
    fn the_partition_does_not_wrap_at_u32_max() {
        let r = readback(1, u32::MAX, 2, 0);
        assert!(!r.partition_holds(), "u32::MAX + 2 must not wrap to 1");
    }

    #[test]
    fn the_class_split_must_sum_to_the_list_counter() {
        assert!(readback(1024, 24, 1000, 24).class_split_sums_to_list());
        // The alpha leak's fingerprint: survivors on the list that no class rendered.
        assert!(!readback(1024, 24, 1000, 20).class_split_sums_to_list());
    }

    #[test]
    fn the_frame_zero_shape_is_a_and_e_agreeing_at_a_zero_base() {
        let mut r = readback(1024, 8, 1016, 8);
        r.alive_count_cur = 8;
        r.real_emit_count = 8;
        r.emit_append_base = 0;
        assert!(r.frame_zero_shape_holds());

        // A frame that already had survivors: A > E, so the frame-0 shape must NOT hold.
        r.alive_count_cur = 12;
        assert!(!r.frame_zero_shape_holds());
    }

    #[test]
    fn a_nonzero_first_instance_is_rejected() {
        let mut r = readback(1024, 24, 1000, 24);
        assert!(r.draw_args_are_well_formed());
        r.additive_first_instance = 1;
        assert!(!r.draw_args_are_well_formed(), "F5b: a nonzero firstInstance is corruption");
        r.additive_first_instance = 0;
        r.additive_index_count = 4;
        assert!(!r.draw_args_are_well_formed(), "the quad is SIX indices");
    }

    /// The artifact line carries every counter the gate asserts over — a run whose assertions were
    /// not armed must still be diagnosable from its log alone.
    #[test]
    fn the_artifact_line_names_every_counter() {
        let line = readback(1024, 24, 1000, 24).artifact_line();
        for key in [
            "cap=1024",
            "alive_next=24",
            "dead=1000",
            "additive_instances=24",
            "alpha_instances=0",
            "partition=true",
            "class_split=true",
            "draw_args_ok=true",
            // Rung P1b's census tail, present even on a run that did not arm it — see
            // `artifact_line`'s doc for why the unarmed state is printed rather than omitted.
            "census=false",
            "waves_evaluated=0",
            "waves_skipped=0",
            "lanes_evaluated=0",
        ] {
            assert!(line.contains(key), "the artifact line is missing `{key}`: {line}");
        }
    }

    /// An UNARMED census reports nothing rather than reporting zero as a rate.
    ///
    /// The distinction is the rung's own subject: gate #17's finding was that an instrument which
    /// cannot see its subject and one that sees a subject with no signal are indistinguishable in
    /// the artifact. `None` is what makes them different here, and the artifact line's `-1.0000`
    /// sentinel is that `None` made printable.
    #[test]
    fn an_unarmed_census_yields_no_rate_at_all() {
        let r = readback(1024, 24, 1000, 24);
        assert!(!r.skip_census_is_armed());
        assert_eq!(r.wave_substeps(), 0);
        assert_eq!(r.wave_skip_rate(), None, "0/0 must never be reported as a rate");
        assert_eq!(r.lane_skip_rate(32), None);
        assert!(r.artifact_line().contains("wave_skip_rate=-1.0000"));
    }

    /// The two rates are DIFFERENT numbers on the same counters, and the gap is the wave's
    /// incoherence — the property the plan says a per-lane figure would hide.
    ///
    /// The fixture is one fully-coherent skipping wave against three waves in which a single lane
    /// of 32 needed the field: at wave granularity 1 of 4 wave-substeps skipped (25 %), while at
    /// lane granularity 125 of 128 lane-substeps skipped (97.7 %). A per-lane figure would claim
    /// the cache deleted 98 % of the work where it deleted 25 % of it.
    #[test]
    fn the_wave_rate_and_the_lane_rate_disagree_by_exactly_the_incoherence() {
        let mut r = readback(1024, 24, 1000, 24);
        r.waves_evaluated = 3;
        r.waves_skipped = 1;
        r.lanes_evaluated = 3;

        assert!(r.skip_census_is_armed());
        assert_eq!(r.wave_substeps(), 4);
        assert_eq!(r.wave_skip_rate(), Some(0.25));

        let lane = r.lane_skip_rate(32).expect("armed");
        assert!((lane - (1.0 - 3.0 / 128.0)).abs() < 1e-12, "lane rate was {lane}");
        assert!(
            lane > r.wave_skip_rate().expect("armed"),
            "the per-lane rate OVERSTATES the saving whenever a wave is incoherent — that is the \
             whole reason the wave pair is the reported figure"
        );
    }

    /// A fully COHERENT run puts the two rates back together: when every evaluating wave has all 32
    /// of its lanes evaluating, the lane rate equals the wave rate exactly.
    ///
    /// This is the control for the test above. Without it, "the lane rate is higher" would be
    /// satisfied by an arithmetic error as readily as by incoherence.
    #[test]
    fn perfect_wave_coherence_collapses_the_two_rates_onto_each_other() {
        let mut r = readback(1024, 24, 1000, 24);
        r.waves_evaluated = 3;
        r.waves_skipped = 1;
        r.lanes_evaluated = 3 * 32;

        assert_eq!(r.wave_skip_rate(), Some(0.25));
        let lane = r.lane_skip_rate(32).expect("armed");
        assert!((lane - 0.25).abs() < 1e-12, "lane rate was {lane}");
    }

    /// A zero wave width is a caller error, not a division by zero.
    #[test]
    fn a_zero_wave_width_yields_no_lane_rate() {
        let mut r = readback(1024, 24, 1000, 24);
        r.waves_evaluated = 1;
        assert_eq!(r.lane_skip_rate(0), None);
    }
}
