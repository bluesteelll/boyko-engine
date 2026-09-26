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
//!
//! **The material-table upload plan (design F1/F3, cut C-4).** Each frame the runner copies ONE of:
//! nothing; this frame's compact edited rows (`MaterialUploadStaging`, drained by the stager);
//! or the FULL image of the table, rebuilt from the CPU authority. The full image is owed on frame
//! 0 (the table is created empty), on a grow frame (the grown buffer is created empty — this
//! frame's runs alone would leave every untouched row unseeded), and on the first recorded frame
//! after a frame that drained edits but never recorded their copy (a minimized window, or a
//! swapchain-recreate skip): the stager already cleared those marks, so only a rebuild from the
//! authority carries them.

/// What the runner copies into the material table this frame ([`material_upload_plan`]'s
/// verdict).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialUploadPlan {
    /// Nothing staged and no full image owed: no staging write, no pass declared, no copy.
    Idle,
    /// This frame's compact edited rows, copied at their runs.
    Compact,
    /// The full image `[0, capacity·48)`, rebuilt from the authority.
    Full,
}

/// Decides this frame's material-table upload from the host's full-image-owed flag and the number
/// of rows the stager staged this frame. A full image supersedes the compact rows: it is built
/// after the stager ran, from the same authority, so it already carries them.
#[inline]
pub fn material_upload_plan(full_pending: bool, staged_rows: usize) -> MaterialUploadPlan {
    if full_pending {
        MaterialUploadPlan::Full
    } else if staged_rows > 0 {
        MaterialUploadPlan::Compact
    } else {
        MaterialUploadPlan::Idle
    }
}

/// The full-image-owed flag after a frame that planned `plan` and reached the renderer.
/// `recorded` is `true` iff the frame's commands were recorded and submitted (the render call
/// returned `Ok(true)`). A frame that planned a copy and did not record it owes a full image: its
/// staged rows are gone. A recorded frame owes nothing — a planned full image was just copied.
#[inline]
pub fn full_pending_after(plan: MaterialUploadPlan, recorded: bool) -> bool {
    plan != MaterialUploadPlan::Idle && !recorded
}

/// The full-image-owed flag after a frame skipped BEFORE any upload (a minimized window): the
/// stager drained `staged_rows` edits into staging this frame will never copy.
#[inline]
pub fn full_pending_after_skip(full_pending: bool, staged_rows: usize) -> bool {
    full_pending || staged_rows > 0
}

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

    /// Frame 0 (the flag starts `true` because the table is created empty) and a grow frame (the
    /// grow sets the flag) copy the full image, whatever the stager staged.
    #[test]
    fn an_owed_full_image_supersedes_the_compact_rows() {
        assert_eq!(material_upload_plan(true, 0), MaterialUploadPlan::Full);
        assert_eq!(material_upload_plan(true, 7), MaterialUploadPlan::Full);
    }

    #[test]
    fn staged_rows_alone_give_a_compact_upload_and_nothing_gives_nothing() {
        assert_eq!(material_upload_plan(false, 1), MaterialUploadPlan::Compact);
        assert_eq!(material_upload_plan(false, 0), MaterialUploadPlan::Idle);
    }

    /// A recorded frame owes nothing afterwards — including the full image it just copied.
    #[test]
    fn a_recorded_frame_clears_the_flag() {
        for plan in [MaterialUploadPlan::Idle, MaterialUploadPlan::Compact, MaterialUploadPlan::Full] {
            assert!(!full_pending_after(plan, true), "{plan:?} recorded");
        }
    }

    /// Cut C-4: a frame that planned a copy and did not record it (a recreate skip) lost its staged
    /// rows, so the next recorded frame owes the full image; an idle skip owes nothing.
    #[test]
    fn an_unrecorded_frame_that_planned_a_copy_owes_the_full_image() {
        assert!(full_pending_after(MaterialUploadPlan::Compact, false));
        assert!(full_pending_after(MaterialUploadPlan::Full, false));
        assert!(!full_pending_after(MaterialUploadPlan::Idle, false));
    }

    /// Cut C-4: a minimized frame skips before any upload, after the stager drained its edits.
    #[test]
    fn a_minimized_skip_with_staged_rows_owes_the_full_image() {
        assert!(full_pending_after_skip(false, 3));
        assert!(!full_pending_after_skip(false, 0));
        assert!(full_pending_after_skip(true, 0), "an owed full image stays owed");
    }

    /// The protocol over a run: frame 0 full; idle; an edit; an edit frame lost to a recreate skip;
    /// the next frame repays it with a full image; idle again.
    #[test]
    fn the_flag_carries_a_lost_edit_to_the_next_recorded_frame() {
        let mut pending = true;
        let frames: [(usize, bool, MaterialUploadPlan); 6] = [
            (4, true, MaterialUploadPlan::Full),
            (0, true, MaterialUploadPlan::Idle),
            (2, true, MaterialUploadPlan::Compact),
            (1, false, MaterialUploadPlan::Compact),
            (0, true, MaterialUploadPlan::Full),
            (0, true, MaterialUploadPlan::Idle),
        ];
        for (i, (staged, recorded, want)) in frames.into_iter().enumerate() {
            let plan = material_upload_plan(pending, staged);
            assert_eq!(plan, want, "frame {i}");
            pending = full_pending_after(plan, recorded);
        }
        assert!(!pending);
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
