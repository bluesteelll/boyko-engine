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
        }
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
    #[must_use]
    pub fn artifact_line(&self) -> String {
        format!(
            "particle_counters frames={} cap={} alive_cur={} alive_next={} dead={} dead_base={} \
             emit_base={} real_emit={} clamped={} additive_instances={} alpha_instances={} \
             partition={} class_split={} draw_args_ok={}",
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
        ] {
            assert!(line.contains(key), "the artifact line is missing `{key}`: {line}");
        }
    }
}
