//! The POD by-value cross-frame handoff (`UiFramePlan`) — GUI P5a Decision 9.
//!
//! The dispatcher-solo upload system stashes a `UiFramePlan` (POD, BORROWS NO RHI
//! HANDLE) that the swapchain recorder reads later in the SAME dispatcher window.
//! The recorder RE-RESOLVES the current-frame pipeline + bind-group by `frame_index`
//! (MF-7) — so a grow that rebuilt that slot's bind-group between upload and draw
//! cannot leave a stale handle, and nothing `!Send`/`!Sync` crosses the
//! `nonsend_resource_mut` token drop (the soundness fix that eliminated the unsound
//! `UiDrawData<'a>` borrow).

use crate::ui::instance::UiOrtho;

/// The POD by-value plan handed from the UI upload system to the swapchain draw
/// recorder (GUI P5a Decision 9). `#[derive(Clone, Copy)]` POD — it carries ONLY
/// the instance count, the ortho (16 B POD), and the frame index that selects the
/// ring slot + bind-group; it borrows NO RHI handle, so it is sound to stash across
/// the token projection and re-read in the same dispatcher-solo window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiFramePlan {
    /// The number of UI instances uploaded for this frame (the draw's
    /// `instance_count`; 0 ⇒ the recorder draws nothing).
    pub instance_count: u32,
    /// The pixel→NDC ortho for the swapchain extent the UI pass renders into.
    pub ortho: UiOrtho,
    /// The frame-in-flight index selecting the ring slot + bind-group the recorder
    /// re-resolves (MF-7) — never a cached raw device handle.
    pub frame_index: usize,
}

impl UiFramePlan {
    /// An empty plan (no instances) for a frame with no visible UI — the recorder
    /// draws nothing. `frame_index` still selects the (idle) slot.
    #[inline]
    pub fn empty(frame_index: usize, ortho: UiOrtho) -> Self {
        UiFramePlan {
            instance_count: 0,
            ortho,
            frame_index,
        }
    }
}
