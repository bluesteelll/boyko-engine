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
