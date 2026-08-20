//! The host half of the particle EFFECT-TABLE generation protocol
//! (`docs/PARTICLES-PLAN.md` Rev 4, §Host → GPU).
//!
//! Writer side ([`boyko_render`]): `particle_bake_effects` bumps
//! `ParticleEffectScratch::rows_gen()` exactly once per ACTUAL rebuild — an asset edit, a new
//! effect, or a clock-timestep change (the bake is a function of BOTH inputs, since `damping` and
//! the rotation multiplier are per-substep constants). A static frame never bumps it. Host side
//! (here): a per-in-flight-slot record of the last generation whose bytes were written into that
//! slot's effect staging decides, deterministically, whether the fenced slot must be rewritten —
//! never a hash, never a byte-compare.
//!
//! # Why the emit-request half has NO gate function here
//!
//! The other per-frame particle upload — the emit-request table — is gated on
//! `ParticleEmitScratch::total_spawn() > 0`, which is a plain read of a value the writer already
//! computed, not a generation protocol. It deliberately has NO helper: the plan's
//! conditional-pass proof requires that predicate to be the SAME expression that decides whether
//! the `particle_emit` pass is declared, and wrapping it in a function whose result is stored
//! somewhere would be the second home the plan's reversal ledger warns about. One read, two
//! call sites, no state.
//!
//! Pure and GPU-free by design so the protocol is unit-testable headlessly; the runner is its
//! only production caller.

/// Decides whether in-flight slot `slot`'s particle effect staging must be rewritten this frame,
/// and records the catch-up when it must.
///
/// Returns `true` iff `uploaded[slot] != generation` — the slot's staging bytes lag the baked
/// table — and in that case stores `generation` into `uploaded[slot]` (the caller then performs
/// the actual `upload_particle_effects` write under its
/// [`FrameWriteToken`](boyko_rhi_vulkan::swapchain::FrameWriteToken)). Returns `false` on an
/// up-to-date slot: NO staging rewrite, and the recorder declares no effect-table copy — the
/// byte-identical idle command stream.
///
/// After a change on frame N, the two in-flight slots catch up over frames N and N+1 (each sees
/// its own stale record once), then every frame is a no-op until the next writer-side bump. The
/// `u64::MAX` boot seed is not a valid generation, so both slots upload the real table on their
/// first frames — which is what makes the effect table defined on frame 0 without a separate
/// boot-time upload path.
///
/// This is `light_upload_due`'s twin, deliberately a SEPARATE function rather than a shared
/// generic: the two protocols track different writers and must be able to diverge (the effect
/// table's generation folds in the clock's timestep, the light table's does not) without one
/// edit silently re-tuning both.
#[inline]
pub fn particle_effects_upload_due<const N: usize>(
    uploaded: &mut [u64; N],
    slot: usize,
    generation: u64,
) -> bool {
    debug_assert!(slot < N, "invariant: the frame slot indexes the in-flight ring");
    // Monotonicity witness: the writer only ever advances the generation, so a recorded slot is
    // never AHEAD of it (the boot `u64::MAX` seed is the one sentinel exception).
    debug_assert!(
        uploaded[slot] == u64::MAX || uploaded[slot] <= generation,
        "invariant: particle_effects_uploaded_gen[slot] <= ParticleEffectScratch::rows_gen() \
         (writer-side monotonic)"
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
    /// `boyko_rhi_vulkan::swapchain::FRAMES_IN_FLIGHT` without a GPU dependency in the headless
    /// test).
    const FIF: usize = 2;

    /// The boot catch-up: with the `u64::MAX` seed, BOTH slots upload the real table across the
    /// first two simulated frames — even at generation 0. This is what makes the device effect
    /// table defined on the first frames without a separate boot upload path.
    #[test]
    fn boot_seed_uploads_both_slots_once() {
        let mut uploaded = [u64::MAX; FIF];
        assert!(particle_effects_upload_due(&mut uploaded, 0, 0), "frame 1 (slot 0) uploads");
        assert!(particle_effects_upload_due(&mut uploaded, 1, 0), "frame 2 (slot 1) uploads");
        assert!(!particle_effects_upload_due(&mut uploaded, 0, 0), "frame 3 (slot 0) caught up");
        assert!(!particle_effects_upload_due(&mut uploaded, 1, 0), "frame 4 (slot 1) caught up");
    }

    /// A writer-side bump on frame N makes EXACTLY the two following occupancies of the two slots
    /// rewrite their staging, then the gate closes again.
    #[test]
    fn per_slot_catch_up_across_two_frames() {
        let mut uploaded = [u64::MAX; FIF];
        assert!(particle_effects_upload_due(&mut uploaded, 0, 1));
        assert!(particle_effects_upload_due(&mut uploaded, 1, 1));
        assert!(!particle_effects_upload_due(&mut uploaded, 0, 1));

        assert!(particle_effects_upload_due(&mut uploaded, 1, 2), "slot 1 catches up on frame N");
        assert!(particle_effects_upload_due(&mut uploaded, 0, 2), "slot 0 catches up on N+1");
        assert!(!particle_effects_upload_due(&mut uploaded, 1, 2), "slot 1 closed on N+2");
        assert!(!particle_effects_upload_due(&mut uploaded, 0, 2), "slot 0 closed on N+3");
    }

    /// The idle invariant: an unchanged generation NEVER rewrites a caught-up slot's staging. It
    /// is what makes the disarmed-adjacent "armed but static" frame declare no effect-table copy
    /// and therefore record a byte-identical stream.
    #[test]
    fn unchanged_generation_never_rewrites() {
        let mut uploaded = [7u64; FIF];
        for frame in 0..64 {
            assert!(
                !particle_effects_upload_due(&mut uploaded, frame % FIF, 7),
                "frame {frame}: an up-to-date slot must not rewrite its staging"
            );
        }
        assert_eq!(uploaded, [7u64; FIF], "the records are untouched by idle frames");
    }

    /// Two bumps between a slot's occupancies collapse into ONE rewrite carrying the LATEST
    /// generation — the compare is equality against the current generation, not a per-step
    /// replay, so the staged bytes are always the newest bake.
    #[test]
    fn skipped_generations_collapse_into_one_catch_up() {
        let mut uploaded = [3u64; FIF];
        assert!(particle_effects_upload_due(&mut uploaded, 0, 5), "slot 0 jumps 3 -> 5 at once");
        assert_eq!(uploaded[0], 5);
        assert!(!particle_effects_upload_due(&mut uploaded, 0, 5), "and is closed afterward");
    }
}
