//! `Renderer::record_rcas`: TAA rung T3 — the CONTRAST-ADAPTIVE SHARPEN (AMD FidelityFX CAS)
//! compute pass. Since render wave PX1a (D-G3) it runs on `display`, AFTER `post_final` and
//! BEFORE `present_sample`, inside the declared `cas` pass: it reads `display` as a STORAGE image
//! at `GENERAL` (the caller's sink emits `post_final`'s write → this read) and writes `aa_out[fi]`,
//! which the present blit samples. `rcas.comp.{hlsl,spv}` are unchanged.

use core::ptr;

use crate::compute::RcasPush;
use crate::ffi::*;

use super::super::frame_driver::Renderer;
use super::super::gpu_zone::ZONE_POST_CAS;
use super::super::scene_types::{GBufferScene, RcasActivation};
use super::super::targets::GBufferTargets;
use super::super::COLOR_SUBRESOURCE_RANGE;

impl Renderer<'_> {
    /// Records the CAS sharpen compute dispatch into `cmd`:
    ///
    /// 1. Barrier B: `aa_out[fi]` UNDEFINED → GENERAL — CAS writes every dispatched pixel of
    ///    `gAaOut` unconditionally, so UNDEFINED-discard is always valid.
    /// 2. Bind the CAS pipeline + `rcas_set[fi]`; push the 16-byte [`RcasPush`] `{ img_w, img_h,
    ///    sharpness, 0 }` (the extent from `present_extent`, sharpness from
    ///    `activation.sharpness`); dispatch `scene.dispatch_group_count_x` groups
    ///    (`numthreads(64,1,1)`, the same 1D pixel grid the resolve/marcher use).
    /// 3. Barrier C: `aa_out[fi]` GENERAL → SHADER_READ_ONLY_OPTIMAL — the present-blit's sample
    ///    (recorded after this call, via `present_set`) sees a valid, ordered read.
    ///
    /// # Safety
    ///
    /// `cmd` must be recordable (recording open, within the caller's begin/end bracket);
    /// `targets.aa_out` / `targets.rcas_set` must be `Some` (the caller gates on
    /// `scene.rcas.is_some() && targets.rcas_set.is_some()`, kept in lockstep by
    /// [`GBufferTargets::create`]); `activation.rcas_pipeline`'s layout matches `rcas_set`'s (2
    /// STORAGE bindings + a 16-byte COMPUTE push range); `present_extent` sizes `display` and
    /// `aa_out` (TAA arms no SSAA, so the two extents agree); `fi` is this frame's in-flight
    /// slot; the caller has ALREADY emitted the frame's `cas` graph-pass barriers (`post_final`'s
    /// `display` write → this COMPUTE read at `GENERAL`).
    pub(crate) unsafe fn record_rcas(
        &self,
        cmd: VkCommandBuffer,
        targets: &GBufferTargets,
        activation: &RcasActivation<'_>,
        present_extent: VkExtent2D,
        scene: &GBufferScene<'_>,
        fi: usize,
    ) {
        debug_assert!(
            present_extent.width > 0 && present_extent.height > 0,
            "invariant: display/aa_out are sized to a non-empty present_extent"
        );
        // Render wave PX0: `ZONE_POST_CAS` brackets the whole pass (barriers, bind, push,
        // dispatch, barrier C).
        let zone = super::zone_arm::ZoneArm::from_scene(scene);
        // SAFETY: recording is open (this fn's contract), outside any rendering scope; the path
        // recorder's witness reset this frame's pool and seals the slot after this pass.
        let zone_pair = unsafe { zone.begin(self.fns, cmd, ZONE_POST_CAS) };
        let aa = targets
            .aa_out
            .as_ref()
            .expect("invariant: scene.rcas.is_some() arms aa_out (GBufferTargets::create)");
        let set = targets
            .rcas_set
            .as_ref()
            .expect("invariant: armed scene.rcas implies rcas_set was built alongside it");

        // --- Barrier B: aa_out[fi] UNDEFINED → GENERAL (RCAS's STORAGE-image write). ---
        let to_general = VkImageMemoryBarrier {
            s_type: VkStructureType::ImageMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: 0,
            dst_access_mask: VK_ACCESS_SHADER_WRITE_BIT,
            old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            new_layout: VK_IMAGE_LAYOUT_GENERAL,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image: aa[fi].image,
            subresource_range: COLOR_SUBRESOURCE_RANGE,
        };
        // SAFETY: recording is open; one image barrier on the live `aa[fi]` texture (built by
        // `create()` whenever TAA is armed, so also present when RCAS is, per the lockstep
        // invariant `scene.rcas.is_some() implies scene.taa.is_some()`); TOP_OF_PIPE→
        // COMPUTE_SHADER with UNDEFINED→GENERAL is the superset-correct pre-dispatch transition
        // — RCAS writes every dispatched pixel of `gAaOut` unconditionally, so UNDEFINED-discard
        // is always valid, not just on the first frame; `&to_general` outlives the call.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                cmd,
                VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                0,
                0,
                ptr::null(),
                0,
                ptr::null(),
                1,
                (&to_general as *const VkImageMemoryBarrier).cast(),
            );
        }

        let push = RcasPush::new(present_extent.width, present_extent.height, activation.sharpness);
        let push_bytes = push.as_bytes();
        // SAFETY: recording is open; `activation.rcas_pipeline` + its 2-binding layout are live
        // on this device (caller contract); `set[fi]` binds `gRcasIn` = `display`, `gAaOut` =
        // `aa[fi]`; `scene.dispatch_group_count_x` covers `present_extent`'s pixel count (the
        // SAME grid the resolve/marcher dispatch); `&set[fi].descriptor_set` is a single-element
        // local alive for the call; `push_bytes` is exactly `RCAS_PUSH_BYTES` (16) at offset 0.
        unsafe {
            (self.fns.cmd_bind_pipeline)(
                cmd,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                activation.rcas_pipeline.pipeline,
            );
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                activation.rcas_pipeline.layout,
                0,
                1,
                &set[fi].descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_push_constants)(
                cmd,
                activation.rcas_pipeline.layout,
                VK_SHADER_STAGE_COMPUTE_BIT,
                0,
                crate::compute::RCAS_PUSH_BYTES,
                push_bytes.as_ptr().cast(),
            );
            (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
        }

        // --- Barrier C: aa_out[fi] GENERAL → SHADER_READ_ONLY_OPTIMAL (present-blit read). ---
        let to_shader_read = VkImageMemoryBarrier {
            s_type: VkStructureType::ImageMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: VK_ACCESS_SHADER_WRITE_BIT,
            dst_access_mask: VK_ACCESS_SHADER_READ_BIT,
            old_layout: VK_IMAGE_LAYOUT_GENERAL,
            new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image: aa[fi].image,
            subresource_range: COLOR_SUBRESOURCE_RANGE,
        };
        // SAFETY: recording is open; COMPUTE_SHADER→FRAGMENT_SHADER with GENERAL→
        // SHADER_READ_ONLY_OPTIMAL makes this dispatch's write available + visible to pass C's
        // present-blit sample of `aa_out[fi]` (recorded after this call, via the repointed
        // `present_set`); `&to_shader_read` outlives the call.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                cmd,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
                0,
                0,
                ptr::null(),
                0,
                ptr::null(),
                1,
                (&to_shader_read as *const VkImageMemoryBarrier).cast(),
            );
        }
        // SAFETY: as the begin above; `zone_pair` is the pair it returned for this zone.
        unsafe { zone.end(self.fns, cmd, ZONE_POST_CAS, zone_pair) };
    }
}
