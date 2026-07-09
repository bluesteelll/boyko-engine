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
