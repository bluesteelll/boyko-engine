//! `Renderer::record_post_final`: render wave PX1a's post-seam tail (`px1a/cut.md` D-G4, D-G5) —
//! one compute dispatch that writes `display` from the frame's scene input (`taa_hist[fi]` under
//! TAA, `lit_native` under SSAA, `lit[fi]` otherwise), recorded by all three path recorders after
//! their last `lit` producer and before `present_sample`. The caller emits the `post_final` graph
//! pass's derived barriers through its own sink first (the per-path ResId tables differ).

use core::ptr;

use crate::ffi::*;

use super::super::frame_driver::Renderer;
use super::super::gpu_zone::ZONE_POST_FINAL;
use super::super::scene_types::GBufferScene;
use super::super::targets::GBufferTargets;

/// `post_final`'s `numthreads(8, 8, 1)` tile edge (`post_final.comp.hlsl`).
const POST_FINAL_TILE: u32 = 8;

impl Renderer<'_> {
    /// Records `post_final`'s bind and its `8 × 8`-tiled dispatch over `display_extent` (the
    /// extent `display` was allocated at), bracketed by [`ZONE_POST_FINAL`]. The shader's extent
    /// guard reads the UBO's extent — the words the host wrote — which a debug build checks
    /// against `display_extent` here.
    ///
    /// # Safety
    ///
    /// `cmd` is recording, outside any rendering scope; the caller has ALREADY emitted the
    /// frame's `post_final` graph-pass barriers (the scene input readable at `GENERAL` by
    /// COMPUTE, `display` at `GENERAL` for the write); `targets` were built by
    /// `GBufferTargets::create` for `scene` (its `post_final` ring binds this frame's scene input
    /// and `display` against `scene.post_final.layout`); `fi` is this frame's in-flight slot.
    pub(crate) unsafe fn record_post_final(
        &self,
        cmd: VkCommandBuffer,
        targets: &GBufferTargets,
        scene: &GBufferScene<'_>,
        display_extent: VkExtent2D,
        fi: usize,
    ) {
        debug_assert_eq!(
            [scene.post_final_words[2], scene.post_final_words[3]],
            [display_extent.width, display_extent.height],
            "invariant: post_final's UBO extent is the extent display was allocated at"
        );
        let ring = targets
            .post_final
            .as_ref()
            .expect("invariant: GBufferTargets::create builds the post_final ring or fails");
        // Render wave PXt (OPEN-ITEMS 19): the module this frame's mode word selects — the RCAS
        // module under the sharpen bit, the sharpen-off module otherwise.
        let pipeline = scene.post_final.pipeline_for(scene.post_rcas());
        let zone = super::zone_arm::ZoneArm::from_scene(scene);
        // SAFETY: recording is open, outside any rendering scope (this fn's contract); the path
        // recorder's witness reset this frame's pool and seals the slot after this body.
        let zone_pair = unsafe { zone.begin(self.fns, cmd, ZONE_POST_FINAL) };
        // SAFETY: recording is open (this fn's contract); `pipeline` (either of
        // `scene.post_final`'s two pipelines, both live on this device for the frame) and the set
        // layout `ring.set[fi]` was written against (`scene.post_final.layout`) are live; both
        // pipelines' layouts were built from that one set layout with the same 4-byte COMPUTE push
        // range, so the set is compatible with whichever is bound here and is bound through that
        // pipeline's own layout; `ring.set[fi]` binds this frame's scene input @0, `display` @1 and
        // `ring.ubo[fi]` @2, all live for the frame, in the layouts the caller's graph barriers
        // left; the shader reads no push constant (its 4-byte range is declared, never pushed);
        // `&ring.set[fi].descriptor_set` is a single element alive for the call; the group counts
        // cover `display_extent` with `8 × 8` tiles and the shader's guard drops the overhang.
        unsafe {
            (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline.pipeline);
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                pipeline.layout,
                0,
                1,
                &ring.set[fi].descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_dispatch)(
                cmd,
                display_extent.width.div_ceil(POST_FINAL_TILE),
                display_extent.height.div_ceil(POST_FINAL_TILE),
                1,
            );
        }
        // SAFETY: as the begin above; `zone_pair` is the pair it returned for this zone.
        unsafe { zone.end(self.fns, cmd, ZONE_POST_FINAL, zone_pair) };
    }
}
