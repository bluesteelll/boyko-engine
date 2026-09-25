//! The host half of the material upload protocol (dynamic-materials DM1).
//!
//! Pure and GPU-free by design, so every decision the runner takes about the material
//! table and the per-instance material ring is unit-testable headlessly (the
//! [`crate::light_gate`] precedent). The runner is the only production caller.
//!
//! **The `PerInstanceMaterial` ring rule (design F7, live defect D-2).** The runner uploads
//! the gathered `PerInstanceMaterial` lane only on a frame with a non-default material. The VB
//! shading and classify shaders and `forward_opaque.vs` read that ring **unconditionally**, so
//! when the last non-default-material instance disappears, the renumbered default instances
//! would keep reading the stale ids the ring still holds. Per ring slot, the runner therefore
//! tracks the high-water row count written since the slot was last all-zero, and zero-fills that
//! prefix once on the falling edge. Zero is exact: on a non-live frame every instance's
//! material id is 0.

/// What the runner does to the fenced slot's `PerInstanceMaterial` ring this frame
/// ([`pm_ring_action`]'s verdict).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PmRingAction {
    /// A live frame: upload the gathered lane `[0, n)`.
    Upload,
    /// The falling edge: zero-fill rows `[0, rows)` of the slot, once.
    Zero {
        /// The slot's high-water row count — every row written since it was last all-zero.
        rows: u32,
    },
    /// Nothing to write: a non-live frame on an already-clean slot.
    Idle,
}

/// Decides the fenced slot's `PerInstanceMaterial` ring action and advances its high-water
/// record `hw` (the runner's per-slot `WindowHost` field).
///
/// - `live` (a frame with a non-default material): [`PmRingAction::Upload`], and
///   `hw = max(hw, n)`, where `n` is the gathered lane length.
/// - not live, `hw > 0`: [`PmRingAction::Zero`] over `[0, hw)`, and `hw = 0`.
/// - not live, `hw == 0`: [`PmRingAction::Idle`] — a default-only scene does no ring work.
///
/// After a falling edge, each in-flight slot is zeroed on its own next occupancy, so after
/// `FRAMES_IN_FLIGHT` frames both slots are clean and the gate is idle again.
#[inline]
pub fn pm_ring_action(live: bool, hw: &mut u32, n: u32) -> PmRingAction {
    if live {
        *hw = (*hw).max(n);
        PmRingAction::Upload
    } else if *hw > 0 {
        let rows = *hw;
        *hw = 0;
        PmRingAction::Zero { rows }
    } else {
        PmRingAction::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The number of in-flight slots the production host rings (mirrors
    /// `boyko_rhi_vulkan::swapchain::FRAMES_IN_FLIGHT` without a GPU dependency).
    const FIF: usize = 2;

    /// A default-only scene never touches the ring: no upload, no zero-fill, on any slot.
    #[test]
    fn a_default_only_scene_never_writes_the_ring() {
        let mut hw = [0u32; FIF];
        for frame in 0..16 {
            let s = frame % FIF;
            assert_eq!(pm_ring_action(false, &mut hw[s], 100), PmRingAction::Idle, "frame {frame}");
        }
        assert_eq!(hw, [0; FIF]);
    }

    /// A live frame uploads and raises the slot's high-water to the largest lane written.
    #[test]
    fn a_live_frame_uploads_and_raises_the_high_water() {
        let mut hw = 0u32;
        assert_eq!(pm_ring_action(true, &mut hw, 7), PmRingAction::Upload);
        assert_eq!(hw, 7);
        assert_eq!(pm_ring_action(true, &mut hw, 3), PmRingAction::Upload);
        assert_eq!(hw, 7, "a shorter lane leaves rows [3, 7) stale, so the high-water keeps them");
        assert_eq!(pm_ring_action(true, &mut hw, 12), PmRingAction::Upload);
        assert_eq!(hw, 12);
    }

    /// D-2's falling edge: after the last non-default material leaves, each slot zero-fills its
    /// own high-water prefix exactly once, on its own next occupancy, then goes idle.
    #[test]
    fn the_falling_edge_zeroes_each_slot_once_on_its_own_occupancy() {
        let mut hw = [0u32; FIF];
        // Frames 0..4 are live with 5 instances; frame 3 (slot 1) grew to 9.
        assert_eq!(pm_ring_action(true, &mut hw[0], 5), PmRingAction::Upload);
        assert_eq!(pm_ring_action(true, &mut hw[1], 5), PmRingAction::Upload);
        assert_eq!(pm_ring_action(true, &mut hw[0], 5), PmRingAction::Upload);
        assert_eq!(pm_ring_action(true, &mut hw[1], 9), PmRingAction::Upload);
        // The falling edge on frame 4 (slot 0), then frame 5 (slot 1).
        assert_eq!(pm_ring_action(false, &mut hw[0], 9), PmRingAction::Zero { rows: 5 });
        assert_eq!(pm_ring_action(false, &mut hw[1], 9), PmRingAction::Zero { rows: 9 });
        assert_eq!(hw, [0; FIF], "both slots are clean after FRAMES_IN_FLIGHT frames");
        // Idle from then on.
        assert_eq!(pm_ring_action(false, &mut hw[0], 9), PmRingAction::Idle);
        assert_eq!(pm_ring_action(false, &mut hw[1], 9), PmRingAction::Idle);
    }

    /// A rising edge after a falling edge starts a fresh high-water record.
    #[test]
    fn a_new_rising_edge_starts_a_fresh_record() {
        let mut hw = 0u32;
        assert_eq!(pm_ring_action(true, &mut hw, 20), PmRingAction::Upload);
        assert_eq!(pm_ring_action(false, &mut hw, 20), PmRingAction::Zero { rows: 20 });
        assert_eq!(pm_ring_action(true, &mut hw, 4), PmRingAction::Upload);
        assert_eq!(hw, 4);
        assert_eq!(pm_ring_action(false, &mut hw, 4), PmRingAction::Zero { rows: 4 });
    }
}
