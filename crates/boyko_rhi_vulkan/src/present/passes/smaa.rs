//! `Renderer::record_smaa`: the anti-aliasing Stage 2 SMAA 1x post-process pass body (3
//! sub-passes: edge detection → blending-weight calculation → neighborhood blending),
//! recorded between the deferred resolve's `present_sample` graph pass and pass C (the
//! present-blit) — see [`gbuffer`](super::gbuffer)'s record-site gate. Mirrors
//! [`record_fxaa`](super::fxaa)'s barrier→begin_rendering→bind→push→draw→end→barrier shape,
//! repeated three times.

use core::ptr;

use crate::ffi::*;

use super::super::frame_driver::Renderer;
use super::super::scene_types::SmaaActivation;
use super::super::targets::GBufferTargets;
use super::super::COLOR_SUBRESOURCE_RANGE;

impl Renderer<'_> {
    /// Records the SMAA 1x fullscreen 3-pass into `cmd`: pass 1 (edge detection, CLEAR,
    /// writes `smaa_edges[fi]`), pass 2 (blending-weight calculation, CLEAR, writes
    /// `smaa_weights[fi]`), pass 3 (neighborhood blending, DONT_CARE, writes `aa_out[fi]`).
    /// Each sub-pass mirrors [`record_fxaa`](super::fxaa::Renderer::record_fxaa)'s
    /// barrier→begin_rendering→bind→push→draw(3,1,0,0)→end→barrier shape; the SAME stage
    /// masks (`TOP_OF_PIPE→COLOR_ATTACHMENT_OUTPUT` in, `COLOR_ATTACHMENT_OUTPUT→
    /// FRAGMENT_SHADER` out) on all 6 barriers.
    ///
    /// `lit[fi]` needs NO barrier here (RAR; the `present_sample` graph pass recorded
    /// immediately before this call already left it `SHADER_READ_ONLY_OPTIMAL`, and this
    /// pass's passes 1 + 3 read it). `area_tex`/`search_tex` need NO barrier (boot-permanent
    /// `SHADER_READ_ONLY_OPTIMAL`).
    ///
    /// `rt_metrics = [1/w, 1/h, w, h]` (16 bytes) is pushed FRAGMENT offset 0 to all three
    /// pipelines.
    ///
    /// # Safety
    ///
    /// `cmd` must be recordable (recording open, within the caller's begin/end bracket);
    /// `targets.aa_out` / `targets.smaa_edges` / `targets.smaa_weights` /
    /// `targets.smaa_{edge,weight,blend}_set` must all be `Some` (the caller gates on
    /// `targets.aa_out.is_some()`, kept in lockstep with `scene.smaa.is_some()` by
    /// [`GBufferTargets::sync_gbuffer`]); `activation`'s pipelines/layouts are live on this
    /// device and match the sets' layouts; `present_extent` sizes every SMAA target (the
    /// extent [`GBufferTargets::create`] allocated them at); `fi` is this frame's in-flight
    /// slot.
    pub(crate) unsafe fn record_smaa(
        &self,
        cmd: VkCommandBuffer,
        targets: &GBufferTargets,
        activation: &SmaaActivation<'_>,
        present_extent: VkExtent2D,
        fi: usize,
    ) {
        debug_assert!(
            present_extent.width > 0 && present_extent.height > 0,
            "invariant: every SMAA target is sized to a non-empty present_extent"
        );
        let aa_out = targets
            .aa_out
            .as_ref()
            .expect("invariant: record gate (targets.aa_out.is_some()) proved aa_out Some");
        let edges = targets
            .smaa_edges
            .as_ref()
            .expect("invariant: armed SMAA implies smaa_edges was built alongside aa_out");
        let weights = targets
            .smaa_weights
            .as_ref()
            .expect("invariant: armed SMAA implies smaa_weights was built alongside aa_out");
        let edge_set = targets
            .smaa_edge_set
            .as_ref()
            .expect("invariant: armed SMAA implies smaa_edge_set was built alongside aa_out");
        let weight_set = targets
            .smaa_weight_set
            .as_ref()
            .expect("invariant: armed SMAA implies smaa_weight_set was built alongside aa_out");
        let blend_set = targets
            .smaa_blend_set
            .as_ref()
            .expect("invariant: armed SMAA implies smaa_blend_set was built alongside aa_out");

        let rt_metrics: [f32; 4] = [
            1.0 / present_extent.width as f32,
            1.0 / present_extent.height as f32,
            present_extent.width as f32,
            present_extent.height as f32,
        ];
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

        // --- Pass 1: edge detection. lit[fi] -> edges[fi]. CLEAR (Decision 4 — the PS
        // `discard`s non-edge texels, so an unwritten prior-frame texel must not leak into
        // pass 2's read). ---
        // SAFETY: recording is open (caller contract); one image barrier on the live
        // `edges[fi]` texture (built by `create()` when `scene.smaa` was armed);
        // TOP_OF_PIPE→COLOR_ATTACHMENT_OUTPUT with UNDEFINED→COLOR is the superset-correct
        // pre-render transition; the barrier struct outlives the call.
        unsafe {
            self.smaa_barrier_to_color(cmd, edges[fi].image);
        }
        let edge_color = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: edges[fi].view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue { color: VkClearColorValue { float32: [0.0; 4] } },
        };
        let edge_rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: VkRect2D { offset: VkOffset2D { x: 0, y: 0 }, extent: present_extent },
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: 1,
            p_color_attachments: &edge_color,
            p_depth_attachment: ptr::null(),
            p_stencil_attachment: ptr::null(),
        };
        // SAFETY: recording is open; `edge_rendering`'s color attachment names the live
        // `edges[fi]` view (now COLOR_ATTACHMENT_OPTIMAL); dynamic rendering is enabled on
        // this device. `activation.edge_pipeline` + its layout (`scene.present_layout`)
        // belong to this device (caller contract); `edge_set[fi]` binds `lit[fi]` (already
        // SHADER_READ_ONLY_OPTIMAL) + `activation.sampler` at set 0. `rt_metrics` is pushed
        // to the FRAGMENT subset of the pipeline's 16-byte push range.
        // `viewport`/`scissor`/`rt_metrics` outlive the bracketed calls; `draw(3, 1, 0, 0)`
        // is the `SV_VertexID` fullscreen triangle (no vertex buffer). Begin/End bracket the
        // pass exactly.
        unsafe {
            (self.fns.cmd_begin_rendering)(cmd, &edge_rendering);
            (self.fns.cmd_bind_pipeline)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                activation.edge_pipeline.pipeline,
            );
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                activation.edge_pipeline.layout,
                0,
                1,
                &edge_set[fi].descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_push_constants)(
                cmd,
                activation.edge_pipeline.layout,
                VK_SHADER_STAGE_FRAGMENT_BIT,
                0,
                16,
                rt_metrics.as_ptr().cast(),
            );
            (self.fns.cmd_set_viewport)(cmd, 0, 1, &viewport);
            (self.fns.cmd_set_scissor)(cmd, 0, 1, &scissor);
            (self.fns.cmd_draw)(cmd, 3, 1, 0, 0);
            (self.fns.cmd_end_rendering)(cmd);
        }
        // SAFETY: recording is open; COLOR_ATTACHMENT_OUTPUT→FRAGMENT_SHADER with
        // COLOR→SHADER_READ_ONLY makes pass 1's store available + visible to pass 2's sample
        // of `edges[fi]` (recorded immediately after).
        unsafe {
            self.smaa_barrier_to_shader_read(cmd, edges[fi].image);
        }

        // --- Pass 2: blending-weight calculation. edges[fi] + area_tex + search_tex ->
        // weights[fi]. CLEAR (same discard-driven correctness requirement as pass 1). ---
        // SAFETY: same reasoning as pass 1's in-barrier, on the live `weights[fi]` texture.
        unsafe {
            self.smaa_barrier_to_color(cmd, weights[fi].image);
        }
        let weight_color = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: weights[fi].view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue { color: VkClearColorValue { float32: [0.0; 4] } },
        };
        let weight_rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: VkRect2D { offset: VkOffset2D { x: 0, y: 0 }, extent: present_extent },
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: 1,
            p_color_attachments: &weight_color,
            p_depth_attachment: ptr::null(),
            p_stencil_attachment: ptr::null(),
        };
        // SAFETY: same reasoning as pass 1's render bracket; `weight_set[fi]` binds
        // `edges[fi]` (now SHADER_READ_ONLY_OPTIMAL) + the boot-permanent `area_tex`/
        // `search_tex` (never barriered per-frame) + `activation.sampler` at set 0 of
        // `activation.weight_layout`.
        unsafe {
            (self.fns.cmd_begin_rendering)(cmd, &weight_rendering);
            (self.fns.cmd_bind_pipeline)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                activation.weight_pipeline.pipeline,
            );
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                activation.weight_pipeline.layout,
                0,
                1,
                &weight_set[fi].descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_push_constants)(
                cmd,
                activation.weight_pipeline.layout,
                VK_SHADER_STAGE_FRAGMENT_BIT,
                0,
                16,
                rt_metrics.as_ptr().cast(),
            );
            (self.fns.cmd_set_viewport)(cmd, 0, 1, &viewport);
            (self.fns.cmd_set_scissor)(cmd, 0, 1, &scissor);
            (self.fns.cmd_draw)(cmd, 3, 1, 0, 0);
            (self.fns.cmd_end_rendering)(cmd);
        }
        // SAFETY: recording is open; makes pass 2's store available + visible to pass 3's
        // sample of `weights[fi]` (recorded immediately after).
        unsafe {
            self.smaa_barrier_to_shader_read(cmd, weights[fi].image);
        }

        // --- Pass 3: neighborhood blending. lit[fi] + weights[fi] -> aa_out[fi]. DONT_CARE
        // (every texel is written unconditionally — the FXAA precedent). ---
        // SAFETY: same reasoning as `record_fxaa`'s single in-barrier, on the live
        // `aa_out[fi]` texture.
        unsafe {
            self.smaa_barrier_to_color(cmd, aa_out[fi].image);
        }
        let blend_color = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: aa_out[fi].view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_DONT_CARE,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue { color: VkClearColorValue { float32: [0.0; 4] } },
        };
        let blend_rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: VkRect2D { offset: VkOffset2D { x: 0, y: 0 }, extent: present_extent },
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: 1,
            p_color_attachments: &blend_color,
            p_depth_attachment: ptr::null(),
            p_stencil_attachment: ptr::null(),
        };
        // SAFETY: same reasoning as pass 1/2's render brackets; `blend_set[fi]` binds
        // `lit[fi]` (SHADER_READ_ONLY_OPTIMAL, RAR) + `weights[fi]` (now
        // SHADER_READ_ONLY_OPTIMAL) + `activation.sampler` at set 0 of
        // `activation.blend_layout`.
        unsafe {
            (self.fns.cmd_begin_rendering)(cmd, &blend_rendering);
            (self.fns.cmd_bind_pipeline)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                activation.blend_pipeline.pipeline,
            );
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                activation.blend_pipeline.layout,
                0,
                1,
                &blend_set[fi].descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_push_constants)(
                cmd,
                activation.blend_pipeline.layout,
                VK_SHADER_STAGE_FRAGMENT_BIT,
                0,
                16,
                rt_metrics.as_ptr().cast(),
            );
            (self.fns.cmd_set_viewport)(cmd, 0, 1, &viewport);
            (self.fns.cmd_set_scissor)(cmd, 0, 1, &scissor);
            (self.fns.cmd_draw)(cmd, 3, 1, 0, 0);
            (self.fns.cmd_end_rendering)(cmd);
        }
        // SAFETY: recording is open; makes pass 3's store available + visible to pass C's
        // present-blit sample of `aa_out[fi]` (recorded immediately after this call, via the
        // repointed `present_set`).
        unsafe {
            self.smaa_barrier_to_shader_read(cmd, aa_out[fi].image);
        }
    }

    /// Barriers `image` UNDEFINED → COLOR_ATTACHMENT_OPTIMAL (TOP_OF_PIPE →
    /// COLOR_ATTACHMENT_OUTPUT) — the shared pre-render transition every SMAA sub-pass'
    /// output target needs. Factored out of [`Self::record_smaa`] (3 identical uses) to keep
    /// the recording body's I-cache footprint compact.
    ///
    /// # Safety
    /// `cmd` must be recordable; `image` must be a live color image whose current layout is
    /// UNDEFINED-compatible (a fresh-this-frame SMAA target, per [`Self::record_smaa`]'s
    /// contract).
    unsafe fn smaa_barrier_to_color(&self, cmd: VkCommandBuffer, image: VkImage) {
        let barrier = VkImageMemoryBarrier {
            s_type: VkStructureType::ImageMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: 0,
            dst_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image,
            subresource_range: COLOR_SUBRESOURCE_RANGE,
        };
        // SAFETY: recording is open (caller contract); one image barrier on the live
        // `image`; `&barrier` outlives the call.
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
                (&barrier as *const VkImageMemoryBarrier).cast(),
            );
        }
    }

    /// Barriers `image` COLOR_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL
    /// (COLOR_ATTACHMENT_OUTPUT → FRAGMENT_SHADER) — the shared post-render transition every
    /// SMAA sub-pass' output target needs before the next pass (or pass C) samples it.
    ///
    /// # Safety
    /// `cmd` must be recordable; `image` must be a live color image just written by a
    /// `vkCmdEndRendering` bracket at COLOR_ATTACHMENT_OPTIMAL (per [`Self::record_smaa`]'s
    /// contract).
    unsafe fn smaa_barrier_to_shader_read(&self, cmd: VkCommandBuffer, image: VkImage) {
        let barrier = VkImageMemoryBarrier {
            s_type: VkStructureType::ImageMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            dst_access_mask: VK_ACCESS_SHADER_READ_BIT,
            old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image,
            subresource_range: COLOR_SUBRESOURCE_RANGE,
        };
        // SAFETY: recording is open (caller contract); one image barrier on the live
        // `image`; `&barrier` outlives the call.
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
                (&barrier as *const VkImageMemoryBarrier).cast(),
            );
        }
    }
}
