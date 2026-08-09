//! Profiling rung 5b — `CommandWitness`, the host-side record of what the recorder actually
//! recorded.
//!
//! # Why a witness and not a golden pin
//!
//! `goldens/PINS.toml` pins *"the SHA-256 of a dumped BMP"*. A `vkCmdResetQueryPool` plus two
//! `vkCmdWriteTimestamp`s change **zero pixels**, so the obvious disarmed-byte-identity claim —
//! "record one profiling command on the disarmed path and the pins move" — is false as written.
//! The pins stay exactly where they were, and the frame is not byte-identical at all.
//!
//! The claim has to be about **commands**, so it is measured where the commands are: a counter
//! incremented **at the `vkCmd*` call site**, exactly as [`VbRecordProbe`] is. Incremented at the
//! site and never derived from the arming predicate — *"the difference between a gate and a
//! tautology"*, in that struct's own words.
//!
//! [`VbRecordProbe`]: super::VbRecordProbe
//!
//! # The counting is behind `feature = "profiling-census"`, default OFF
//!
//! [`stream_pos`](CommandWitness::stream_pos) counts **every** witnessed record site, not only the
//! profiling ones — that is what makes it a position rather than a tally, and it is a perturbation
//! of every recorder it is threaded through. So the ~200 increments at `vb.rs`'s record sites are
//! compiled only into the gate binaries that read them.
//!
//! The **type** is unconditional, and rung 5c made it so deliberately. `GBufferScene` must carry an
//! `Option<&CommandWitness>` for the witness to reach the recorder; features unify per PACKAGE, so
//! a `#[cfg]`'d field is present or absent for `boyko_app`'s construction site depending on a flag
//! no `boyko_app` source names — a build that enables this feature from anywhere would stop
//! compiling for a reason no crate shows. A type nobody constructs costs nothing; a field whose
//! existence depends on someone else's feature costs a build.
//!
//! The increments are host-side `u32` adds ([`Cell`], which compiles to the same store a plain
//! field would) that record no command and change no device state. **That is why a census build
//! records the same command stream as a non-census build**, and therefore why a census measurement
//! speaks about the shipped configuration rather than about itself.
//!
//! # `stamp_positions` exists because `zone_open_order` cannot cross a leg boundary
//!
//! The record-order witness ([`zone_open_order`](CommandWitness::zone_open_order)) is in the NEW
//! vocabulary: zone ids. The collector it is meant to be compared against has no zone ids at all —
//! only its own per-harness pass slots. Comparing the two would need a hand-written
//! `pass → zone` table, and a table written alongside the ported brackets makes the comparison a
//! tautology: it agrees with itself.
//!
//! [`stamp_positions`](CommandWitness::stamp_positions) has **no vocabulary**. It is the value of a
//! monotone "record sites passed so far in this region" counter at the moment each timestamp is
//! recorded. Both collectors produce it from the *same* instrumentation, so the cross-leg claim
//! becomes "same number of timestamps, each at the same position in the recorded stream" — and
//! shifting one bracket by a single command changes one entry. **No mapping table exists, so none
//! can be wrong.**
//!
//! # Rung 5c: three things the port settled, recorded here because they are the type's contract
//!
//! **1. The verbs take `&self`.** A frame's recording holds the instrument shared: `record_vb`
//! reaches it through `&GBufferScene`, and every recording verb below it therefore has a shared
//! borrow and nothing else. This is 5b's own `GpuZoneRecorder::record_reset` finding one rung
//! later and for the same reason — the first caller to record a *whole frame* through one borrow
//! is what exposes a `&mut self` verb as unreachable. The counters are [`Cell`], not atomics:
//! a command buffer is recorded by one thread, and an atomic here would be a claim about a
//! concurrency this type never has.
//!
//! **2. What `stream_pos` counts, stated exactly, because rung 5b's doc claimed more than the
//! instrument delivers.** It counts **record sites in `vb.rs`**: every `vkCmd*` recorded there
//! directly (MEASURED: 167 through `self.fns.cmd_*` plus 2 through `crate::accel::cmd_*`) *and*
//! every call to a helper that records commands elsewhere (`record_vb_pass`, the AA/post chain),
//! which counts as **one** whatever it records inside. It is therefore a position in `vb.rs`'s
//! record stream, not a count of `vkCmd` calls, and the resolution of the cross-leg claim is
//! exactly that: a bracket that moves across any witnessed site moves a position, and a bracket
//! that moves *within* one delegate's body does not. Rung 5b's *"every recorded command in the
//! witnessed region"* was wider than any instrument that does not also thread through the shared
//! post-process recorders.
//!
//! # Rung 7c: a position says WHERE a stamp was recorded and nothing about WHAT was recorded
//!
//! [`stamp_positions`](CommandWitness::stamp_positions) is blind to every argument of the
//! `vkCmdWriteTimestamp` it witnesses. That blindness is what let rungs 5c/6 port the VB brackets
//! while **silently changing the pipeline stage of seven of the ten begin stamps**:
//! `VbTimestampCollector::write_begin` consults `VbTimedPass::begin_stage` — `TOP_OF_PIPE` for slots
//! 0..2 and `BOTTOM_OF_PIPE` for the seven P4-2 partitioning brackets — while
//! `GpuZoneRecorder::record_begin` hardcoded `TOP_OF_PIPE` for all of them. Same commands, same
//! positions, same count; a different quantity measured, by the old collector's own argument that
//! *"only BOTTOM-vs-BOTTOM comparisons carry the partition property"*.
//!
//! G10 was green throughout, because a stage is not a position. So the witness now records the
//! stage each bracket stamp was written at ([`stamp_stages`](CommandWitness::stamp_stages)),
//! **taken from the value the recorder was handed at its own call site** rather than from the pass
//! table — a stage read back out of the table on both legs would agree with itself, which is the
//! same tautology `stamp_positions` exists to avoid.
//!
//! **3. A repair is not a bracket.** The old `VbTimestampCollector` closes every pair the frame
//! did not bracket, because its readback uses `VK_QUERY_RESULT_WAIT_BIT` and would block on an
//! unwritten query; the new `GpuZoneRecorder` labels that pair `NotBracketed` and records nothing.
//! So the two legs' *total* timestamp counts differ **by design**, and a cross-leg equality over
//! all of them would red on the very difference the port exists to make. [`Self::repair`] is the
//! epilogue's verb: it moves the stream position (the commands are real) and counts into
//! [`Self::profiling_cmds`], but it is not a bracket stamp and does not enter
//! [`Self::stamp_positions`]. The comparison is over brackets, and it compares their **count**
//! as well as their positions, so a port that dropped a bracket entirely still reds.

use core::cell::Cell;

use boyko_rhi::TimestampStage;

use super::gpu_zone::MAX_GPU_PAIRS;

/// What the recorder recorded, counted at the `vkCmd*` sites.
///
/// Host state, owned by whoever drives a frame's recording. `Clone`/`Copy` is deliberately **not**
/// derived: it holds two kilobyte-scale arrays, and a silent copy of those in a recorder's inner
/// loop is exactly the cost this struct is supposed to be too cheap to have.
pub struct CommandWitness {
    profiling_cmds: Cell<u32>,
    query_resets: Cell<u32>,
    timestamps: Cell<u32>,
    repairs: Cell<u32>,
    recorded_pairs: Cell<u16>,
    stream_pos: Cell<u32>,
    /// Zone ids in the order their pairs were OPENED. `zone_open_order[k]` is the zone of the
    /// `k`-th pair to be opened, which is a statement about RECORD order and nothing else.
    zone_open_order: [Cell<u16>; MAX_GPU_PAIRS],
    /// `stream_pos` at each recorded BRACKET timestamp, in record order. Repairs are absent by
    /// construction — see the module doc's point 3.
    stamp_positions: [Cell<u32>; 2 * MAX_GPU_PAIRS],
    /// The pipeline stage each of those stamps was written at, same index, same order — rung 7c.
    /// Parallel to [`Self::stamp_positions`] rather than folded into it because the two answer
    /// different questions and a reader comparing one must be able to compare the other alone.
    stamp_stages: [Cell<TimestampStage>; 2 * MAX_GPU_PAIRS],
}

impl Default for CommandWitness {
    fn default() -> CommandWitness {
        CommandWitness::new()
    }
}

impl CommandWitness {
    /// A witness that has seen nothing.
    #[must_use]
    pub fn new() -> CommandWitness {
        CommandWitness {
            profiling_cmds: Cell::new(0),
            query_resets: Cell::new(0),
            timestamps: Cell::new(0),
            repairs: Cell::new(0),
            recorded_pairs: Cell::new(0),
            stream_pos: Cell::new(0),
            zone_open_order: core::array::from_fn(|_| Cell::new(0)),
            stamp_positions: core::array::from_fn(|_| Cell::new(0)),
            stamp_stages: core::array::from_fn(|_| Cell::new(TimestampStage::TopOfPipe)),
        }
    }

    /// Forget every frame but the next one.
    ///
    /// A stream position is a statement about ONE frame's recording. Left to accumulate over a
    /// leg's K frames, the two legs' positions would still be comparable — and would agree for the
    /// wrong reason, because a K-frame total is insensitive to a bracket that moved earlier in one
    /// frame and later in another. Called at the top of the witnessed region, so what the reader
    /// sees after a frame is that frame.
    pub fn begin_frame(&self) {
        self.profiling_cmds.set(0);
        self.query_resets.set(0);
        self.timestamps.set(0);
        self.repairs.set(0);
        self.recorded_pairs.set(0);
        self.stream_pos.set(0);
    }

    /// One witnessed record site that is **not** the profiler's.
    ///
    /// Called at every `vkCmd*` in the region and at every call to a helper that records commands
    /// elsewhere. Without these the positions below would be a timestamp index rather than a
    /// stream position, and two legs whose brackets sit at different points in the same command
    /// stream would be indistinguishable.
    #[inline]
    pub fn command(&self) {
        self.stream_pos.set(self.stream_pos.get().saturating_add(1));
    }

    /// One `vkCmdResetQueryPool` recorded by the profiler.
    #[inline]
    pub fn query_reset(&self) {
        self.query_resets.set(self.query_resets.get().saturating_add(1));
        self.profiling_cmds.set(self.profiling_cmds.get().saturating_add(1));
        self.stream_pos.set(self.stream_pos.get().saturating_add(1));
    }

    /// One `vkCmdWriteTimestamp` recorded by the profiler **as a bracket**, at the current stream
    /// position and at `stage`.
    ///
    /// The position is recorded **before** the increment, so it is the position *of* this command
    /// rather than of the one after it. That is the convention the cross-leg comparison rests on,
    /// and it is stated because either choice is defensible and only one can be used by both legs.
    ///
    /// `stage` must be the stage the caller handed the RECORDER for this very command, never one
    /// looked up again from a pass table: the two legs consult the same table, so a witness that
    /// re-read it would agree with itself while the recorders disagreed — which is exactly what
    /// happened for seven passes between rungs 5c and 7c (module doc).
    #[inline]
    pub fn timestamp(&self, stage: TimestampStage) {
        let slot = self.timestamps.get() as usize;
        if slot < self.stamp_positions.len() {
            self.stamp_positions[slot].set(self.stream_pos.get());
            self.stamp_stages[slot].set(stage);
        }
        self.timestamps.set(self.timestamps.get().saturating_add(1));
        self.profiling_cmds.set(self.profiling_cmds.get().saturating_add(1));
        self.stream_pos.set(self.stream_pos.get().saturating_add(1));
    }

    /// One `vkCmdWriteTimestamp` recorded by a totality **epilogue** rather than by a bracket.
    ///
    /// It is a real command — it moves the stream position and it is the profiler's — but it is
    /// not a bracket, so it does not enter [`Self::stamp_positions`]. A collector whose readback
    /// cannot tolerate an unwritten query has these; one that labels the pair instead does not,
    /// and that difference must not read as a disagreement about where the brackets are.
    #[inline]
    pub fn repair(&self) {
        self.repairs.set(self.repairs.get().saturating_add(1));
        self.profiling_cmds.set(self.profiling_cmds.get().saturating_add(1));
        self.stream_pos.set(self.stream_pos.get().saturating_add(1));
    }

    /// One pair OPENED for `zone` — the record-order witness.
    ///
    /// Called where the pair's BEGIN is recorded, not where it is allocated: allocation order is
    /// the bump allocator's business, and the question this answers is which bracket the recorder
    /// reached first.
    #[inline]
    pub fn open_pair(&self, zone: u16) {
        let slot = self.recorded_pairs.get() as usize;
        if slot < self.zone_open_order.len() {
            self.zone_open_order[slot].set(zone);
        }
        self.recorded_pairs.set(self.recorded_pairs.get().saturating_add(1));
    }

    /// Every command the profiler recorded: resets plus bracket timestamps plus repairs.
    #[must_use]
    #[inline]
    pub fn profiling_cmds(&self) -> u32 {
        self.profiling_cmds.get()
    }

    /// `vkCmdResetQueryPool` calls the profiler recorded.
    #[must_use]
    #[inline]
    pub fn query_resets(&self) -> u32 {
        self.query_resets.get()
    }

    /// `vkCmdWriteTimestamp` calls the profiler recorded **as brackets**.
    #[must_use]
    #[inline]
    pub fn timestamps(&self) -> u32 {
        self.timestamps.get()
    }

    /// `vkCmdWriteTimestamp` calls a totality epilogue recorded to close pairs the frame never
    /// bracketed.
    #[must_use]
    #[inline]
    pub fn repairs(&self) -> u32 {
        self.repairs.get()
    }

    /// Pairs the recorder OPENED.
    #[must_use]
    #[inline]
    pub fn recorded_pairs(&self) -> u16 {
        self.recorded_pairs.get()
    }

    /// Record sites passed in the witnessed region, profiling and otherwise.
    ///
    /// **The instrument's own positive control.** A disarmed leg must show every profiling counter
    /// at zero — and so would a witness that was never threaded through anything. A non-zero
    /// `stream_pos` on that leg is what separates "the profiler recorded nothing" from "nothing
    /// recorded anything".
    #[must_use]
    #[inline]
    pub fn stream_pos(&self) -> u32 {
        self.stream_pos.get()
    }

    /// Zone ids in the order their pairs were opened.
    ///
    /// An iterator rather than a slice because the cells cannot be reborrowed as one, and every
    /// use is a comparison: `w.zone_open_order().eq([30, 10, 20])`.
    pub fn zone_open_order(&self) -> impl Iterator<Item = u16> + '_ {
        let n = (self.recorded_pairs.get() as usize).min(self.zone_open_order.len());
        self.zone_open_order[..n].iter().map(Cell::get)
    }

    /// The stream position of each recorded bracket timestamp, in record order.
    ///
    /// An iterator, for [`Self::zone_open_order`]'s reason. The cross-leg comparison is
    /// `a.stamp_positions().eq(b.stamp_positions())`, which compares length as well as contents —
    /// so a port that dropped a bracket reds here and not only in the arithmetic below.
    pub fn stamp_positions(&self) -> impl Iterator<Item = u32> + '_ {
        let n = (self.timestamps.get() as usize).min(self.stamp_positions.len());
        self.stamp_positions[..n].iter().map(Cell::get)
    }

    /// The pipeline stage of each recorded bracket timestamp, in the same order as
    /// [`Self::stamp_positions`] — rung 7c.
    ///
    /// A bracket is *(where, what)*: two collectors that stamp at identical stream positions can
    /// still be measuring different quantities, because a `TOP_OF_PIPE` stamp fires when prior work
    /// REACHES the pipe and a `BOTTOM_OF_PIPE` one only when prior work has COMPLETED. Comparing
    /// this beside the positions is what makes the port's claim *"the same brackets"* rather than
    /// *"brackets in the same places"*.
    pub fn stamp_stages(&self) -> impl Iterator<Item = TimestampStage> + '_ {
        let n = (self.timestamps.get() as usize).min(self.stamp_stages.len());
        self.stamp_stages[..n].iter().map(Cell::get)
    }

    /// Whether the arithmetic the census exists to assert holds: **two bracket timestamps per
    /// opened pair**, and every one of them positioned.
    ///
    /// A method rather than a gate-local expression because both the G5 gate and rung 5c's
    /// cross-leg comparison need the same predicate, and two spellings of one equality are two
    /// things that can drift.
    #[must_use]
    pub fn timestamps_pair_up(&self) -> bool {
        self.timestamps.get() == u32::from(self.recorded_pairs.get()) * 2
            && self.stamp_positions().count() == self.timestamps.get() as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh witness claims nothing, including no stream position.
    #[test]
    fn a_fresh_witness_has_seen_nothing() {
        let w = CommandWitness::new();
        assert_eq!(w.profiling_cmds(), 0);
        assert_eq!(w.query_resets(), 0);
        assert_eq!(w.timestamps(), 0);
        assert_eq!(w.repairs(), 0);
        assert_eq!(w.recorded_pairs(), 0);
        assert_eq!(w.stream_pos(), 0);
        assert_eq!(w.zone_open_order().count(), 0);
        assert_eq!(w.stamp_positions().count(), 0);
        assert_eq!(w.stamp_stages().count(), 0);
        // Vacuously true, and that is the point of stating it: the equality alone is not a gate.
        assert!(w.timestamps_pair_up());
    }

    /// A timestamp's recorded position is the position **of** that command, not of the next one —
    /// and non-profiling commands move it.
    #[test]
    fn a_stamp_position_is_the_position_of_its_own_command() {
        let w = CommandWitness::new();
        w.command(); // stream_pos 0 -> 1
        w.command(); // 1 -> 2
        w.open_pair(9);
        w.timestamp(TimestampStage::TopOfPipe); // recorded AT 2, then 2 -> 3
        w.command(); // 3 -> 4
        w.timestamp(TimestampStage::BottomOfPipe); // recorded AT 4, then 4 -> 5

        assert!(w.stamp_positions().eq([2, 4]));
        assert!(w.stamp_stages().eq([TimestampStage::TopOfPipe, TimestampStage::BottomOfPipe]));
        assert_eq!(w.stream_pos(), 5);
        assert_eq!(w.profiling_cmds(), 2, "a timestamp is itself a recorded command");
        assert_eq!(w.timestamps(), 2);
        assert_eq!(w.recorded_pairs(), 1);
        assert!(w.timestamps_pair_up());
    }

    /// Shifting one bracket by a single command changes exactly one entry — the property the
    /// cross-leg comparison rests on.
    #[test]
    fn moving_a_bracket_by_one_command_moves_one_position() {
        let a = CommandWitness::new();
        a.open_pair(1);
        a.timestamp(TimestampStage::TopOfPipe);
        a.command();
        a.timestamp(TimestampStage::BottomOfPipe);

        let b = CommandWitness::new();
        b.open_pair(1);
        b.timestamp(TimestampStage::TopOfPipe);
        b.command();
        b.command(); // one extra command before the closing stamp
        b.timestamp(TimestampStage::BottomOfPipe);

        let (pa, pb): (Vec<u32>, Vec<u32>) =
            (a.stamp_positions().collect(), b.stamp_positions().collect());
        assert_eq!(pa[0], pb[0], "the open stamp did not move");
        assert_ne!(
            pa[1], pb[1],
            "a bracket that closed one command later reported the same position, so the witness \
             cannot see a shifted bracket at all"
        );
    }

    /// The pairing equality fails when a bracket is half-recorded, which is the arithmetic the
    /// armed clause of G5 asserts.
    #[test]
    fn a_half_recorded_bracket_breaks_the_pairing_equality() {
        let w = CommandWitness::new();
        w.open_pair(1);
        w.timestamp(TimestampStage::TopOfPipe);
        w.timestamp(TimestampStage::BottomOfPipe);
        assert!(w.timestamps_pair_up());

        w.open_pair(2);
        w.timestamp(TimestampStage::TopOfPipe); // and no closing stamp
        assert!(
            !w.timestamps_pair_up(),
            "three timestamps over two pairs must not read as paired up"
        );
    }

    /// The record-order witness records ORDER, not allocation index.
    #[test]
    fn the_open_order_is_the_order_pairs_were_opened() {
        let w = CommandWitness::new();
        // Opened in an order that is not the zone ids' order, so a witness that sorted or indexed
        // by zone would disagree.
        w.open_pair(30);
        w.open_pair(10);
        w.open_pair(20);
        assert!(w.zone_open_order().eq([30, 10, 20]));
    }

    /// A repair moves the stream and counts as the profiler's, but is not a bracket — the property
    /// that lets a leg WITH a totality epilogue be compared against a leg without one.
    #[test]
    fn a_repair_is_a_command_but_not_a_bracket() {
        let bracketed = CommandWitness::new();
        bracketed.open_pair(1);
        bracketed.timestamp(TimestampStage::TopOfPipe);
        bracketed.command();
        bracketed.timestamp(TimestampStage::BottomOfPipe);
        // ...and the epilogue closes a pass this frame never bracketed.
        bracketed.repair();
        bracketed.repair();

        let labelled = CommandWitness::new();
        labelled.open_pair(1);
        labelled.timestamp(TimestampStage::TopOfPipe);
        labelled.command();
        labelled.timestamp(TimestampStage::BottomOfPipe);

        assert!(
            bracketed.stamp_positions().eq(labelled.stamp_positions()),
            "the epilogue's fillers entered the bracket positions, so a collector that repairs \
             can never be compared against one that labels"
        );
        assert_eq!(bracketed.repairs(), 2);
        assert_eq!(labelled.repairs(), 0);
        assert_ne!(
            bracketed.profiling_cmds(),
            labelled.profiling_cmds(),
            "a repair is a real recorded command and must be counted as one"
        );
        assert!(bracketed.timestamps_pair_up() && labelled.timestamps_pair_up());
    }

    /// `begin_frame` makes the witness a statement about ONE frame.
    #[test]
    fn begin_frame_forgets_the_previous_frame() {
        let w = CommandWitness::new();
        w.command();
        w.open_pair(4);
        w.timestamp(TimestampStage::TopOfPipe);
        w.timestamp(TimestampStage::BottomOfPipe);
        assert_eq!(w.stream_pos(), 3);

        w.begin_frame();
        assert_eq!(w.stream_pos(), 0);
        assert_eq!(w.timestamps(), 0);
        assert_eq!(w.recorded_pairs(), 0);
        assert_eq!(w.stamp_positions().count(), 0);
        assert_eq!(w.stamp_stages().count(), 0);

        w.command();
        w.open_pair(4);
        w.timestamp(TimestampStage::TopOfPipe);
        assert!(
            w.stamp_positions().eq([1]),
            "the second frame's first stamp carried the first frame's offset"
        );
    }

    /// **Rung 7c's property, and the one rungs 5c/6 shipped without.** Two legs can agree on every
    /// stream position and still be measuring different things, because a `TOP_OF_PIPE` stamp and a
    /// `BOTTOM_OF_PIPE` stamp at the same point in the stream fire at different moments.
    ///
    /// This is the shape of the real defect, in miniature: `VbTimestampCollector` opened the seven
    /// P4-2 brackets at `BOTTOM_OF_PIPE` and `GpuZoneRecorder` opened them at `TOP_OF_PIPE`, and
    /// G10's position equality was green for five commits across the difference.
    #[test]
    fn equal_positions_do_not_imply_equal_brackets() {
        let bottom_open = CommandWitness::new();
        bottom_open.open_pair(3);
        bottom_open.timestamp(TimestampStage::BottomOfPipe);
        bottom_open.command();
        bottom_open.timestamp(TimestampStage::BottomOfPipe);

        let top_open = CommandWitness::new();
        top_open.open_pair(3);
        top_open.timestamp(TimestampStage::TopOfPipe);
        top_open.command();
        top_open.timestamp(TimestampStage::BottomOfPipe);

        assert!(
            bottom_open.stamp_positions().eq(top_open.stamp_positions()),
            "the fixture must agree on positions, or it is not showing what positions miss"
        );
        assert!(
            !bottom_open.stamp_stages().eq(top_open.stamp_stages()),
            "a bracket opened at BOTTOM_OF_PIPE compared equal to one opened at TOP_OF_PIPE, so \
             the witness still cannot see the difference between the two collectors"
        );
    }

    /// A stage is recorded in RECORD order and paired with its own position, so a reader zipping
    /// the two iterators gets `(where, what)` for one command rather than two commands' halves.
    #[test]
    fn stages_and_positions_are_the_same_stamps_in_the_same_order() {
        let w = CommandWitness::new();
        w.open_pair(1);
        w.timestamp(TimestampStage::BottomOfPipe);
        w.command();
        w.command();
        w.timestamp(TimestampStage::BottomOfPipe);
        w.open_pair(2);
        w.timestamp(TimestampStage::TopOfPipe);
        w.timestamp(TimestampStage::BottomOfPipe);

        let zipped: Vec<(u32, TimestampStage)> =
            w.stamp_positions().zip(w.stamp_stages()).collect();
        assert_eq!(
            zipped,
            vec![
                (0, TimestampStage::BottomOfPipe),
                (3, TimestampStage::BottomOfPipe),
                (4, TimestampStage::TopOfPipe),
                (5, TimestampStage::BottomOfPipe),
            ]
        );
    }
}
