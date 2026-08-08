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
//! The claim has to be about **commands**, so it is measured where the commands are: a `&mut`
//! counter incremented **at the `vkCmd*` call site**, exactly as [`VbRecordProbe`] is. Incremented
//! at the site and never derived from the arming predicate — *"the difference between a gate and a
//! tautology"*, in that struct's own words.
//!
//! [`VbRecordProbe`]: super::VbRecordProbe
//!
//! # The whole struct is behind `feature = "profiling-census"`, default OFF
//!
//! [`stream_pos`](CommandWitness::stream_pos) counts **every** recorded command in the witnessed
//! region, not only the profiling ones — that is what makes it a position rather than a tally, and
//! it is a perturbation of every recorder it is threaded through. So it is compiled only into the
//! gate binaries that read it.
//!
//! The increments are host-side `u32` adds that record no command and change no device state.
//! **That is why a census build records the same command stream as a non-census build**, and
//! therefore why a census measurement speaks about the shipped configuration rather than about
//! itself.
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
//! monotone "commands recorded so far in this region" counter at the moment each timestamp is
//! recorded. Both collectors produce it from the *same* instrumentation, so the cross-leg claim
//! becomes "same number of timestamps, each at the same position in the recorded stream" — and
//! shifting one bracket by a single command changes one entry. **No mapping table exists, so none
//! can be wrong.** That comparison is rung 5c's; this rung builds the instrument and gates its
//! two-sided arithmetic.

use super::gpu_zone::MAX_GPU_PAIRS;

/// What the recorder recorded, counted at the `vkCmd*` sites.
///
/// Host state, owned by whoever drives a frame's recording. `Clone`/`Copy` is deliberately **not**
/// derived: it holds two kilobyte-scale arrays, and a silent copy of those in a recorder's inner
/// loop is exactly the cost this struct is supposed to be too cheap to have.
pub struct CommandWitness {
    profiling_cmds: u32,
    query_resets: u32,
    timestamps: u32,
    recorded_pairs: u16,
    stream_pos: u32,
    /// Zone ids in the order their pairs were OPENED. `zone_open_order[k]` is the zone of the
    /// `k`-th pair to be opened, which is a statement about RECORD order and nothing else.
    zone_open_order: [u16; MAX_GPU_PAIRS],
    /// `stream_pos` at each recorded timestamp, in record order.
    stamp_positions: [u32; 2 * MAX_GPU_PAIRS],
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
            profiling_cmds: 0,
            query_resets: 0,
            timestamps: 0,
            recorded_pairs: 0,
            stream_pos: 0,
            zone_open_order: [0; MAX_GPU_PAIRS],
            stamp_positions: [0; 2 * MAX_GPU_PAIRS],
        }
    }

    /// One recorded command that is **not** the profiler's.
    ///
    /// Called at every `vkCmd*` in the witnessed region. Without these the positions below would
    /// be a timestamp index rather than a stream position, and two legs whose brackets sit at
    /// different points in the same command stream would be indistinguishable.
    #[inline]
    pub fn command(&mut self) {
        self.stream_pos = self.stream_pos.saturating_add(1);
    }

    /// One `vkCmdResetQueryPool` recorded by the profiler.
    #[inline]
    pub fn query_reset(&mut self) {
        self.query_resets = self.query_resets.saturating_add(1);
        self.profiling_cmds = self.profiling_cmds.saturating_add(1);
        self.stream_pos = self.stream_pos.saturating_add(1);
    }

    /// One `vkCmdWriteTimestamp` recorded by the profiler, at the current stream position.
    ///
    /// The position is recorded **before** the increment, so it is the position *of* this command
    /// rather than of the one after it. That is the convention the cross-leg comparison rests on,
    /// and it is stated because either choice is defensible and only one can be used by both legs.
    #[inline]
    pub fn timestamp(&mut self) {
        let slot = self.timestamps as usize;
        if slot < self.stamp_positions.len() {
            self.stamp_positions[slot] = self.stream_pos;
        }
        self.timestamps = self.timestamps.saturating_add(1);
        self.profiling_cmds = self.profiling_cmds.saturating_add(1);
        self.stream_pos = self.stream_pos.saturating_add(1);
    }

    /// One pair OPENED for `zone` — the record-order witness.
    ///
    /// Called where the pair's BEGIN is recorded, not where it is allocated: allocation order is
    /// the bump allocator's business, and the question this answers is which bracket the recorder
    /// reached first.
    #[inline]
    pub fn open_pair(&mut self, zone: u16) {
        let slot = self.recorded_pairs as usize;
        if slot < self.zone_open_order.len() {
            self.zone_open_order[slot] = zone;
        }
        self.recorded_pairs = self.recorded_pairs.saturating_add(1);
    }

    /// Every command the profiler recorded: resets plus timestamps.
    #[must_use]
    #[inline]
    pub fn profiling_cmds(&self) -> u32 {
        self.profiling_cmds
    }

    /// `vkCmdResetQueryPool` calls the profiler recorded.
    #[must_use]
    #[inline]
    pub fn query_resets(&self) -> u32 {
        self.query_resets
    }

    /// `vkCmdWriteTimestamp` calls the profiler recorded.
    #[must_use]
    #[inline]
    pub fn timestamps(&self) -> u32 {
        self.timestamps
    }

    /// Pairs the recorder OPENED.
    #[must_use]
    #[inline]
    pub fn recorded_pairs(&self) -> u16 {
        self.recorded_pairs
    }

    /// Commands recorded in the witnessed region, profiling and otherwise.
    ///
    /// **The instrument's own positive control.** A disarmed leg must show every profiling counter
    /// at zero — and so would a witness that was never threaded through anything. A non-zero
    /// `stream_pos` on that leg is what separates "the profiler recorded nothing" from "nothing
    /// recorded anything".
    #[must_use]
    #[inline]
    pub fn stream_pos(&self) -> u32 {
        self.stream_pos
    }

    /// Zone ids in the order their pairs were opened.
    #[must_use]
    #[inline]
    pub fn zone_open_order(&self) -> &[u16] {
        &self.zone_open_order[..self.recorded_pairs.min(MAX_GPU_PAIRS as u16) as usize]
    }

    /// The stream position of each recorded timestamp, in record order.
    #[must_use]
    #[inline]
    pub fn stamp_positions(&self) -> &[u32] {
        let n = (self.timestamps as usize).min(self.stamp_positions.len());
        &self.stamp_positions[..n]
    }

    /// Whether the arithmetic the census exists to assert holds: **two timestamps per opened
    /// pair**, and every one of them positioned.
    ///
    /// A method rather than a gate-local expression because both the G5 gate and rung 5c's
    /// cross-leg comparison need the same predicate, and two spellings of one equality are two
    /// things that can drift.
    #[must_use]
    pub fn timestamps_pair_up(&self) -> bool {
        self.timestamps == u32::from(self.recorded_pairs) * 2
            && self.stamp_positions().len() == self.timestamps as usize
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
        assert_eq!(w.recorded_pairs(), 0);
        assert_eq!(w.stream_pos(), 0);
        assert!(w.zone_open_order().is_empty());
        assert!(w.stamp_positions().is_empty());
        // Vacuously true, and that is the point of stating it: the equality alone is not a gate.
        assert!(w.timestamps_pair_up());
    }

    /// A timestamp's recorded position is the position **of** that command, not of the next one —
    /// and non-profiling commands move it.
    #[test]
    fn a_stamp_position_is_the_position_of_its_own_command() {
        let mut w = CommandWitness::new();
        w.command(); // stream_pos 0 -> 1
        w.command(); // 1 -> 2
        w.open_pair(9);
        w.timestamp(); // recorded AT 2, then 2 -> 3
        w.command(); // 3 -> 4
        w.timestamp(); // recorded AT 4, then 4 -> 5

        assert_eq!(w.stamp_positions(), &[2, 4]);
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
        let mut a = CommandWitness::new();
        a.open_pair(1);
        a.timestamp();
        a.command();
        a.timestamp();

        let mut b = CommandWitness::new();
        b.open_pair(1);
        b.timestamp();
        b.command();
        b.command(); // one extra command before the closing stamp
        b.timestamp();

        assert_eq!(a.stamp_positions()[0], b.stamp_positions()[0], "the open stamp did not move");
        assert_ne!(
            a.stamp_positions()[1],
            b.stamp_positions()[1],
            "a bracket that closed one command later reported the same position, so the witness \
             cannot see a shifted bracket at all"
        );
    }

    /// The pairing equality fails when a bracket is half-recorded, which is the arithmetic the
    /// armed clause of G5 asserts.
    #[test]
    fn a_half_recorded_bracket_breaks_the_pairing_equality() {
        let mut w = CommandWitness::new();
        w.open_pair(1);
        w.timestamp();
        w.timestamp();
        assert!(w.timestamps_pair_up());

        w.open_pair(2);
        w.timestamp(); // and no closing stamp
        assert!(
            !w.timestamps_pair_up(),
            "three timestamps over two pairs must not read as paired up"
        );
    }

    /// The record-order witness records ORDER, not allocation index.
    #[test]
    fn the_open_order_is_the_order_pairs_were_opened() {
        let mut w = CommandWitness::new();
        // Opened in an order that is not the zone ids' order, so a witness that sorted or indexed
        // by zone would disagree.
        w.open_pair(30);
        w.open_pair(10);
        w.open_pair(20);
        assert_eq!(w.zone_open_order(), &[30, 10, 20]);
    }
}
