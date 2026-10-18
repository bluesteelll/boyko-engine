//! The host half of the light-table GENERATION protocol (host plan D5/R4).
//!
//! Writer side ([`boyko_render`]): `collect_lights` bumps the world's
//! `LightTableGeneration` exactly once per ACTUAL staging rewrite (spawn / toggle /
//! removal / header-gate flip); a static frame never bumps it. Host side (here): a
//! per-in-flight-slot record of the last generation whose bytes were written into
//! that slot's light-staging ring buffer decides, deterministically, whether the
//! fenced slot's staging must be rewritten this frame — never a hash, never a
//! byte-compare (the D5 pin: deterministic writer-side generations only).
//!
//! Pure and GPU-free by design so the protocol is unit-testable headlessly; the
//! runner is its only production caller.

/// Decides whether in-flight slot `slot`'s light staging must be rewritten this
/// frame, and records the catch-up when it must.
///
/// Returns `true` iff `uploaded[slot] != generation` — the slot's staging bytes lag
/// the staged table — and in that case stores `generation` into `uploaded[slot]`
/// (the caller then performs the actual `upload_light_table` write under its
/// [`FrameWriteToken`](boyko_rhi_vulkan::swapchain::FrameWriteToken)). Returns
/// `false` on an up-to-date slot: NO staging rewrite, NO recorded copy — the
/// byte-identical idle command stream (the rung L0-r0 0%-gate).
///
/// After a change on frame N, the two in-flight slots catch up over frames N and
/// N+1 (each sees its own stale record once), then every frame is a no-op until the
/// next writer-side bump. The `u64::MAX` boot seed is not a valid generation, so
/// both slots upload the real table on their first frames.
#[inline]
pub fn light_upload_due<const N: usize>(
    uploaded: &mut [u64; N],
    slot: usize,
    generation: u64,
) -> bool {
    debug_assert!(slot < N, "invariant: the frame slot indexes the in-flight ring");
    // Monotonicity witness (plan validation set): the writer only ever advances the
    // generation, so a recorded slot is never AHEAD of it (the boot u64::MAX seed is
    // the one sentinel exception).
    debug_assert!(
        uploaded[slot] == u64::MAX || uploaded[slot] <= generation,
        "invariant: light_uploaded_gen[slot] <= LightTableGeneration (writer-side monotonic)"
    );
    if uploaded[slot] != generation {
        uploaded[slot] = generation;
        true
    } else {
        false
    }
}

/// Which part of the light table a plan step touches: nothing, the 64-B header alone, or the
/// whole table (header + rows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightRegion {
    /// Nothing.
    None,
    /// Bytes `[0, 64)` — the `LightHeaderGpu` (exposure, counts, the gate words).
    Header,
    /// The whole staged table.
    Full,
}

/// One frame's light-table work (render wave PX1a, H4 Scope 7, arch W8 / perf O11): what to
/// write into the fenced slot's staging, and what to copy from it into the device table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightCopyPlan {
    /// The staging write into the frame's slot.
    pub staging: LightRegion,
    /// The recorded staging → device-table copy.
    pub copy: LightRegion,
}

/// Per in-flight slot: the row and header generations whose bytes that slot's staging holds.
/// Seeded `u64::MAX` (not a generation), so every slot writes the whole table on its first frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightSlotState<const N: usize> {
    /// `LightTableGeneration` of the rows in each slot's staging.
    pub staging_rows_gen: [u64; N],
    /// `LightHeaderGeneration` of the header in each slot's staging.
    pub staging_header_gen: [u64; N],
}

impl<const N: usize> LightSlotState<N> {
    /// The boot seed: no slot holds a generation.
    pub const BOOT: Self = Self { staging_rows_gen: [u64::MAX; N], staging_header_gen: [u64::MAX; N] };
}

/// The generations the DEVICE light table holds — advanced only after a frame that presented
/// (`Ok(true)`), so a skipped or recreated frame leaves the record behind and the next presented
/// frame copies what the device missed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightDeviceState {
    /// `LightTableGeneration` of the rows the device table holds.
    pub device_rows_gen: u64,
    /// `LightHeaderGeneration` of the header the device table holds.
    pub device_header_gen: u64,
}

impl LightDeviceState {
    /// The boot seed: the device table holds nothing the writer produced.
    pub const BOOT: Self = Self { device_rows_gen: u64::MAX, device_header_gen: u64::MAX };
}

/// The light-table work for in-flight slot `slot` this frame, keyed on what the slot's staging
/// and the device table last received — never on this frame's delta (a change staged on a frame
/// the render skipped is still copied by the next presented frame). Records the slot's staging
/// as current; the caller advances `device` after the frame presents.
pub fn light_copy_plan<const N: usize>(
    slots: &mut LightSlotState<N>,
    slot: usize,
    device: &LightDeviceState,
    rows_gen: u64,
    header_gen: u64,
) -> LightCopyPlan {
    let _ = (slots, slot, device, rows_gen, header_gen);
    unimplemented!("PX1a C8: light_copy_plan")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The number of in-flight slots the production host rings (mirrors
    /// `boyko_rhi_vulkan::swapchain::FRAMES_IN_FLIGHT` without a GPU dependency in
    /// the headless test).
    const FIF: usize = 2;

    /// The boot catch-up: with the `u64::MAX` seed, BOTH slots upload the real
    /// table across the first two simulated frames — even at generation 0 (a world
    /// whose `collect_lights` never rebuilt still replaces the boot placeholder).
    #[test]
    fn boot_seed_uploads_both_slots_once() {
        let mut uploaded = [u64::MAX; FIF];
        assert!(light_upload_due(&mut uploaded, 0, 0), "frame 1 (slot 0) uploads at boot");
        assert!(light_upload_due(&mut uploaded, 1, 0), "frame 2 (slot 1) uploads at boot");
        assert!(!light_upload_due(&mut uploaded, 0, 0), "frame 3 (slot 0) is caught up");
        assert!(!light_upload_due(&mut uploaded, 1, 0), "frame 4 (slot 1) is caught up");
    }

    /// The steady-state change: a writer-side bump on frame N makes EXACTLY the two
    /// following occupancies of the two slots rewrite their staging (the per-slot
    /// catch-up across 2 simulated frames), then the gate closes again.
    #[test]
    fn per_slot_catch_up_across_two_frames() {
        let mut uploaded = [u64::MAX; FIF];
        // Settle the boot catch-up at generation 1.
        assert!(light_upload_due(&mut uploaded, 0, 1));
        assert!(light_upload_due(&mut uploaded, 1, 1));
        assert!(!light_upload_due(&mut uploaded, 0, 1));

        // The writer rebuilds (generation 1 -> 2) before slot 1's next frame.
        assert!(light_upload_due(&mut uploaded, 1, 2), "slot 1 catches up on frame N");
        assert!(light_upload_due(&mut uploaded, 0, 2), "slot 0 catches up on frame N+1");
        assert!(!light_upload_due(&mut uploaded, 1, 2), "slot 1 closed on frame N+2");
        assert!(!light_upload_due(&mut uploaded, 0, 2), "slot 0 closed on frame N+3");
    }

    /// The idle invariant: an unchanged generation NEVER rewrites a caught-up slot's
    /// staging (the no-rewrite half of the D5 pin — the recorded copy is gated on
    /// this same decision, so an idle frame's command stream is byte-identical).
    #[test]
    fn unchanged_generation_never_rewrites() {
        let mut uploaded = [7u64; FIF];
        for frame in 0..64 {
            assert!(
                !light_upload_due(&mut uploaded, frame % FIF, 7),
                "frame {frame}: an up-to-date slot must not rewrite its staging"
            );
        }
        assert_eq!(uploaded, [7u64; FIF], "the records are untouched by idle frames");
    }

    /// Two bumps between a slot's occupancies collapse into ONE rewrite carrying the
    /// LATEST generation (the compare is equality against the current generation,
    /// not a per-step replay — the staged bytes are always the newest table).
    #[test]
    fn skipped_generations_collapse_into_one_catch_up() {
        let mut uploaded = [3u64; FIF];
        assert!(light_upload_due(&mut uploaded, 0, 5), "slot 0 jumps 3 -> 5 in one rewrite");
        assert_eq!(uploaded[0], 5);
        assert!(!light_upload_due(&mut uploaded, 0, 5), "and is closed afterward");
    }
}
