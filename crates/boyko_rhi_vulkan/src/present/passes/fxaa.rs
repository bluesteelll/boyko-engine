//! `Renderer::record_fxaa`: the anti-aliasing Stage 1 FXAA post-process pass body,
//! recorded between the deferred resolve's `present_sample` graph pass and pass C
//! (the present-blit) — see [`gbuffer`](super::gbuffer)'s record-site gate.

use core::ptr;

use crate::ffi::*;

use super::super::frame_driver::Renderer;
use super::super::scene_types::AaActivation;
use super::super::targets::GBufferTargets;
use super::super::COLOR_SUBRESOURCE_RANGE;

impl Renderer<'_> {
    /// Records the FXAA fullscreen pass into `cmd`: barrier `aa_out[fi]` (UNDEFINED →
    /// COLOR_ATTACHMENT_OPTIMAL), `vkCmdBeginRendering` (DONT_CARE load — the fullscreen
    /// triangle overwrites every texel), bind the FXAA pipeline + `fxaa_set[fi]` (INPUT =
    /// `lit[fi]`, never `aa_out`) + dynamic viewport/scissor, push `rcp_frame`,
    /// `vkCmdDraw(3, 1, 0, 0)`, `vkCmdEndRendering`, then barrier `aa_out[fi]`
    /// (COLOR_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL) so the present-blit's sample
    /// (pass C, recorded right after this call) sees a valid, ordered read.
    ///
    /// `lit[fi]` needs NO barrier here: the `present_sample` graph pass recorded
    /// immediately before this call already left it in `SHADER_READ_ONLY_OPTIMAL`, and
    /// this pass's sample of it is a valid read-after-read.
    ///
    /// # Safety
    ///
    /// `cmd` must be recordable (recording open, within the caller's begin/end bracket);
    /// `targets.aa_out` / `targets.fxaa_set` must be `Some` (the caller gates on
    /// `targets.aa_out.is_some()`, kept in lockstep with `scene.aa.is_some()` by
    /// [`GBufferTargets::sync_gbuffer`]); `activation.pipeline` is live on this device and
    /// its layout matches `fxaa_set`'s (`scene.present_layout`) plus an 8-byte
    /// VERTEX|FRAGMENT push range; `present_extent` sizes `aa_out` (the extent
    /// [`GBufferTargets::create`] allocated it at); `fi` is this frame's in-flight slot.
    pub(crate) unsafe fn record_fxaa(
        &self,
        cmd: VkCommandBuffer,
        targets: &GBufferTargets,
        activation: &AaActivation<'_>,
        present_extent: VkExtent2D,
        fi: usize,
    ) {
        debug_assert!(
            present_extent.width > 0 && present_extent.height > 0,
            "invariant: aa_out is sized to a non-empty present_extent"
        );
        let aa = targets
            .aa_out
            .as_ref()
            .expect("invariant: record gate (targets.aa_out.is_some()) proved aa_out Some");
        let set = targets
            .fxaa_set
            .as_ref()
            .expect("invariant: armed aa_out implies fxaa_set was built alongside it");

        // --- Barrier 1: aa_out[fi] UNDEFINED → COLOR_ATTACHMENT_OPTIMAL. ---
        let to_color = VkImageMemoryBarrier {
            s_type: VkStructureType::ImageMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: 0,
            dst_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image: aa[fi].image,
            subresource_range: COLOR_SUBRESOURCE_RANGE,
        };
        // SAFETY: recording is open (caller contract); one image barrier on the live
        // `aa[fi]` texture (built by `create()` when `scene.aa` was armed);
        // TOP_OF_PIPE→COLOR_ATTACHMENT_OUTPUT with UNDEFINED→COLOR is the
        // superset-correct pre-render transition; `&to_color` outlives the call.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                cmd,
                VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                0,
                0,
                ptr::null(),
                0,
                ptr::null(),
                1,
                (&to_color as *const VkImageMemoryBarrier).cast(),
            );
        }

        // Dynamic rendering: one color attachment (`aa_out[fi]`), DONT_CARE load — the
        // fullscreen triangle overwrites every texel, so no clear/preserve is needed
        // (research B). `render_area` is the FULL `present_extent` (no `min()` clamp,
        // unlike pass C: `aa_out` IS `present_extent`, never a wider swapchain image).
        let color_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: aa[fi].view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_DONT_CARE,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue {
                color: VkClearColorValue { float32: [0.0; 4] },
            },
        };
        let rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: VkRect2D {
                offset: VkOffset2D { x: 0, y: 0 },
                extent: present_extent,
            },
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: 1,
            p_color_attachments: &color_attachment,
            p_depth_attachment: ptr::null(),
            p_stencil_attachment: ptr::null(),
        };
        let viewport = VkViewport {
            x: 0.0,
            y: 0.0,
            width: present_extent.width as f32,
            height: present_extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let scissor = VkRect2D {
            offset: VkOffset2D { x: 0, y: 0 },
            extent: present_extent,
        };
        let rcp_frame: [f32; 2] =
            [1.0 / present_extent.width as f32, 1.0 / present_extent.height as f32];
        // SAFETY: recording is open; `rendering` is fully initialized — its color
        // attachment names the live `aa[fi]` view (now COLOR_ATTACHMENT_OPTIMAL);
        // dynamic rendering is enabled on this device. `activation.pipeline` + its layout
        // belong to this device (caller contract); `set[fi]` binds `lit[fi]` (the FXAA
        // INPUT, already SHADER_READ_ONLY_OPTIMAL) + the LINEAR sampler at set 0 of that
        // layout. `rcp_frame` is pushed to the FRAGMENT subset of the pipeline's
        // VERTEX|FRAGMENT 8-byte push range (the present_blit UI-pass precedent pushes a
        // stage subset against a wider range). `viewport`/`scissor`/`rcp_frame` outlive
        // the bracketed calls; `draw(3, 1, 0, 0)` is the `SV_VertexID` fullscreen triangle
        // (no vertex buffer). Begin/End bracket the pass exactly.
        unsafe {
            (self.fns.cmd_begin_rendering)(cmd, &rendering);
            (self.fns.cmd_bind_pipeline)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                activation.pipeline.pipeline,
            );
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                activation.pipeline.layout,
                0,
                1,
                &set[fi].descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_push_constants)(
                cmd,
                activation.pipeline.layout,
                VK_SHADER_STAGE_FRAGMENT_BIT,
                0,
                8,
                rcp_frame.as_ptr().cast(),
            );
            (self.fns.cmd_set_viewport)(cmd, 0, 1, &viewport);
            (self.fns.cmd_set_scissor)(cmd, 0, 1, &scissor);
            (self.fns.cmd_draw)(cmd, 3, 1, 0, 0);
            (self.fns.cmd_end_rendering)(cmd);
        }

        // --- Barrier 2: aa_out[fi] COLOR_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL. ---
        let to_shader_read = VkImageMemoryBarrier {
            s_type: VkStructureType::ImageMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            dst_access_mask: VK_ACCESS_SHADER_READ_BIT,
            old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image: aa[fi].image,
            subresource_range: COLOR_SUBRESOURCE_RANGE,
        };
        // SAFETY: recording is open; COLOR_ATTACHMENT_OUTPUT→FRAGMENT_SHADER with
        // COLOR→SHADER_READ_ONLY makes this pass's draw available + visible to pass C's
        // present-blit sample of `aa_out[fi]` (recorded immediately after this call, via
        // the repointed `present_set`); `&to_shader_read` outlives the call.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                cmd,
                VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
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
    }
}
