//! `Renderer::record_present_sampled`: the fullscreen composite-sample body behind
//! [`Renderer::present_sampled`].

use core::ptr;

use crate::ffi::*;
use crate::memory::BoundBuffer;

use super::super::frame_driver::Renderer;
use super::super::scene_types::{SampledComposite, UiPass};
use super::super::{COLOR_SUBRESOURCE_RANGE, SwapchainError};

impl Renderer<'_> {
    /// Records the rung-11 fullscreen-sample present into `cmd`: barrier the swapchain
    /// image (UNDEFINED → COLOR), `vkCmdBeginRendering` (color CLEAR), bind the
    /// fullscreen pipeline + the composite-texture bind group + dynamic
    /// viewport/scissor, `vkCmdDraw(3, 1, 0, 0)`, `vkCmdEndRendering`, then either
    /// color COLOR → PRESENT (steady) or color COLOR → TRANSFER_SRC, copy-to-buffer,
    /// TRANSFER_SRC → PRESENT (the test readback path).
    ///
    /// The composite texture is NOT touched as a write target here: the caller
    /// uploaded it once before the present loop and left it in
    /// `SHADER_READ_ONLY_OPTIMAL`, so this records only a `FRAGMENT_SHADER` sample of
    /// it (no upload copy, no composite-texture barrier). That is what keeps the
    /// multi-frame-in-flight present loop free of any write-after-read hazard on the
    /// shared composite texture.
    ///
    /// # Safety
    ///
    /// `cmd` must be recordable (waited free); `image`/`view` must belong to the
    /// swapchain image presented this frame; every `composite` resource is live on
    /// this device and `composite.texture` is already resident in
    /// `SHADER_READ_ONLY_OPTIMAL` (uploaded once by the caller, never written again);
    /// the pipeline's declared color format equals the swapchain image's (W2-b); a
    /// `Some(readback)` buffer is host-visible and ≥ the swapchain image's byte size.
    #[allow(clippy::too_many_arguments)]
    pub(crate) unsafe fn record_present_sampled(
        &self,
        cmd: VkCommandBuffer,
        image: VkImage,
        view: VkImageView,
        extent: VkExtent2D,
        clear: [f32; 4],
        composite: &SampledComposite<'_>,
        readback: Option<&BoundBuffer>,
        ui: Option<&UiPass<'_>>,
    ) -> Result<(), SwapchainError> {
        let begin = VkCommandBufferBeginInfo {
            s_type: VkStructureType::CommandBufferBeginInfo,
            p_next: ptr::null(),
            flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            p_inheritance_info: ptr::null(),
        };
        // SAFETY: `cmd` is recordable per this fn's contract; `begin` is a
        // fully-initialized one-time-submit begin-info.
        let raw = unsafe { (self.fns.begin_command_buffer)(cmd, &begin) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkBeginCommandBuffer", result));
        }

        // The composite texture is already resident in SHADER_READ_ONLY_OPTIMAL (the
        // caller's pre-loop one-time upload). This path only SAMPLES it, so it records
        // no barrier on the composite texture — a read-only image shared across
        // frames-in-flight needs none, and re-uploading/re-transitioning it per frame
        // would be the cross-frame write-after-read hazard this restructure removes.

        // --- Barrier (swapchain color): UNDEFINED → COLOR_ATTACHMENT_OPTIMAL. ---
        let to_color = VkImageMemoryBarrier {
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
        // SAFETY: recording is open; one image barrier on the live swapchain `image`;
        // TOP_OF_PIPE→COLOR_ATTACHMENT_OUTPUT with UNDEFINED→COLOR is the
        // superset-correct acquire→render transition; `&to_color` outlives the call.
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

        // Dynamic rendering: one color attachment (the swapchain image, CLEAR/STORE),
        // no depth (the fullscreen triangle is depth-less). The pipeline's declared
        // color format equals the swapchain format (W2-b, upheld by the caller).
        let color_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue {
                color: VkClearColorValue { float32: clear },
            },
        };
        let rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: VkRect2D {
                offset: VkOffset2D { x: 0, y: 0 },
                extent,
            },
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: 1,
            p_color_attachments: &color_attachment,
            p_depth_attachment: ptr::null(),
            p_stencil_attachment: ptr::null(),
        };
        // Present the composite at its NATIVE size in the TOP-LEFT of the swapchain
        // image, NOT stretched to the full swapchain extent. The viewport/scissor are
        // clamped to `min(swapchain_extent, texture_extent)` at origin (0, 0): the
        // fullscreen triangle then writes exactly the composite's pixels 1:1, and the
        // rest of a wider WSI-clamped swapchain image keeps the clear color (the
        // begin-rendering `render_area` above stays the full swapchain extent so the
        // CLEAR covers it). A 1:1 top-left mapping makes a per-texel golden exact
        // regardless of any `current_extent` clamp.
        let present_extent = VkExtent2D {
            width: extent.width.min(composite.texture_extent.width),
            height: extent.height.min(composite.texture_extent.height),
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
        // SAFETY: recording is open; `rendering` is fully initialized — its color
        // attachment names the live swapchain `view` (now COLOR_ATTACHMENT_OPTIMAL);
        // dynamic rendering is enabled on this device. The pipeline + its bind-group
        // layout belong to this device (caller contract) and the pipeline's declared
        // color format equals the swapchain image's (W2-b). The bind group binds the
        // composite texture (now SHADER_READ_ONLY_OPTIMAL) + sampler at set 0 of the
        // pipeline's layout; `viewport`/`scissor` locals outlive the bracketed calls;
        // `draw(3, 1, 0, 0)` is the `SV_VertexID` fullscreen triangle (no vertex
        // buffer). Begin/End bracket the pass exactly.
        unsafe {
            (self.fns.cmd_begin_rendering)(cmd, &rendering);
            (self.fns.cmd_bind_pipeline)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                composite.pipeline.pipeline,
            );
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                composite.pipeline.layout,
                0,
                1,
                &composite.bind_group.descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_set_viewport)(cmd, 0, 1, &viewport);
            (self.fns.cmd_set_scissor)(cmd, 0, 1, &scissor);
            (self.fns.cmd_draw)(cmd, 3, 1, 0, 0);
            (self.fns.cmd_end_rendering)(cmd);
        }

        // --- GUI P5a Rung 5 / Decision 9: the UI rect sub-pass. After the composite
        //     scope ENDED above, open a FRESH `begin_rendering(LoadOp::Load)` at the
        //     FULL swapchain extent (preserve the composite, do NOT re-clear) and
        //     record ONE instanced draw of the current frame's UI rects. The image is
        //     still COLOR_ATTACHMENT_OPTIMAL (the composite scope only ended the render
        //     pass, not the layout) — but an EXPLICIT barrier between the two rendering
        //     instances IS required: `vkCmdEndRendering`/`vkCmdBeginRendering` perform
        //     NO implicit synchronization, so without it the UI pass's LOAD + draws race
        //     the composite pass's CLEAR + draw on the same image (WAW/RAW) and the
        //     composite can clobber the UI — a timing-dependent loss the validation
        //     layer's pacing reliably exposed (ui_rect_swapchain_golden: the RED-rect
        //     texel read back as the bare scene). The COLOR→PRESENT/TRANSFER transition
        //     below still covers BOTH passes' writes for what follows. A pass with
        //     `instance_count == 0` records nothing. ---
        if let Some(ui) = ui
            && ui.instance_count > 0
        {
            // Barrier: composite writes → UI loadOp read + draws (same image, no layout
            // change — a pure COLOR_ATTACHMENT_OUTPUT→COLOR_ATTACHMENT_OUTPUT ordering
            // + availability/visibility dependency).
            let composite_to_ui = VkImageMemoryBarrier {
                s_type: VkStructureType::ImageMemoryBarrier,
                p_next: ptr::null(),
                src_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                dst_access_mask: VK_ACCESS_COLOR_ATTACHMENT_READ_BIT
                    | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                image,
                subresource_range: COLOR_SUBRESOURCE_RANGE,
            };
            // SAFETY: recording is open (between the two rendering scopes); one image
            // barrier on the live swapchain `image`; same-layout COLOR→COLOR with
            // COLOR_ATTACHMENT_OUTPUT on both sides orders the composite's stores
            // before the UI pass's loadOp read + stores; `&composite_to_ui` outlives
            // the call.
            unsafe {
                (self.fns.cmd_pipeline_barrier)(
                    cmd,
                    VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                    VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                    0,
                    0,
                    ptr::null(),
                    0,
                    ptr::null(),
                    1,
                    (&composite_to_ui as *const VkImageMemoryBarrier).cast(),
                );
            }
            let ui_color = VkRenderingAttachmentInfo {
                s_type: VkStructureType::RenderingAttachmentInfo,
                p_next: ptr::null(),
                image_view: view,
                image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                resolve_mode: 0,
                resolve_image_view: VkImageView::NULL,
                resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                // LOAD preserves the composited scene; STORE keeps the UI result.
                load_op: VK_ATTACHMENT_LOAD_OP_LOAD,
                store_op: VK_ATTACHMENT_STORE_OP_STORE,
                clear_value: VkClearValue {
                    color: VkClearColorValue { float32: [0.0; 4] },
                },
            };
            // The UI pass covers the FULL swapchain extent (NOT `present_extent`): the
            // ortho denominator the host computed is the swapchain extent, so a rect at
            // the bottom-right corner must reach the bottom-right swapchain texel.
            let ui_rendering = VkRenderingInfo {
                s_type: VkStructureType::RenderingInfo,
                p_next: ptr::null(),
                flags: 0,
                render_area: VkRect2D {
                    offset: VkOffset2D { x: 0, y: 0 },
                    extent,
                },
                layer_count: 1,
                view_mask: 0,
                color_attachment_count: 1,
                p_color_attachments: &ui_color,
                p_depth_attachment: ptr::null(),
                p_stencil_attachment: ptr::null(),
            };
            let ui_viewport = VkViewport {
                x: 0.0,
                y: 0.0,
                width: extent.width as f32,
                height: extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            let ui_scissor = VkRect2D {
                offset: VkOffset2D { x: 0, y: 0 },
                extent,
            };
            debug_assert_eq!(
                ui.ortho_bytes.len(),
                16,
                "invariant: the UI ortho push block is 16 bytes (UiOrtho)"
            );
            // SAFETY: recording is open; `ui_rendering` is fully initialized — its color
            // attachment names the live swapchain `view` (still COLOR_ATTACHMENT_OPTIMAL
            // from the to_color barrier) with LoadOp::LOAD (preserving the composite).
            // `ui.pipeline`/`ui.bind_group` are the caller's live, current-frame-
            // re-resolved (MF-7) UI handles (their `RhiContext` outlives this submit per
            // the caller contract); the pipeline's `color_formats[0]` equals the
            // swapchain format (W2-b). The ortho is pushed to the pipeline's VERTEX range
            // (16 B, asserted); the bind-group's STORAGE ring holds `instance_count`
            // valid records uploaded for this frame index. `ui_viewport`/`ui_scissor`
            // span the full swapchain extent and outlive the bracketed calls; the
            // vertexless `draw(6, N, 0, 0)` reads the SSBO by `SV_InstanceID`. Begin/End
            // bracket the pass exactly.
            unsafe {
                (self.fns.cmd_begin_rendering)(cmd, &ui_rendering);
                (self.fns.cmd_bind_pipeline)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_GRAPHICS,
                    ui.pipeline.pipeline,
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_GRAPHICS,
                    ui.pipeline.layout,
                    0,
                    1,
                    &ui.bind_group.descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_push_constants)(
                    cmd,
                    ui.pipeline.layout,
                    VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
                    0,
                    ui.ortho_bytes.len() as u32,
                    ui.ortho_bytes.as_ptr().cast(),
                );
                (self.fns.cmd_set_viewport)(cmd, 0, 1, &ui_viewport);
                (self.fns.cmd_set_scissor)(cmd, 0, 1, &ui_scissor);
                (self.fns.cmd_draw)(cmd, 6, ui.instance_count, 0, 0);
                (self.fns.cmd_end_rendering)(cmd);
            }
        }

        // The post-draw color transition depends on whether a readback is requested
        // (identical to `record_scene`'s branch).
        match readback {
            // Steady present path: COLOR_ATTACHMENT_OPTIMAL → PRESENT_SRC_KHR.
            None => {
                let to_present = VkImageMemoryBarrier {
                    s_type: VkStructureType::ImageMemoryBarrier,
                    p_next: ptr::null(),
                    src_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                    dst_access_mask: 0,
                    old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    new_layout: VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
                    src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    image,
                    subresource_range: COLOR_SUBRESOURCE_RANGE,
                };
                // SAFETY: recording is open; COLOR_ATTACHMENT_OUTPUT→BOTTOM_OF_PIPE
                // with COLOR→PRESENT makes the draw's writes visible to the present
                // engine; `&to_present` outlives the call.
                unsafe {
                    (self.fns.cmd_pipeline_barrier)(
                        cmd,
                        VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                        VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
                        0,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        1,
                        (&to_present as *const VkImageMemoryBarrier).cast(),
                    );
                }
            }
            // Test readback path: COLOR → TRANSFER_SRC, copy image → buffer, then
            // TRANSFER_SRC → PRESENT (the image is still presented after the copy).
            Some(staging) => {
                let to_transfer = VkImageMemoryBarrier {
                    s_type: VkStructureType::ImageMemoryBarrier,
                    p_next: ptr::null(),
                    src_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                    dst_access_mask: VK_ACCESS_TRANSFER_READ_BIT,
                    old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    new_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    image,
                    subresource_range: COLOR_SUBRESOURCE_RANGE,
                };
                // SAFETY: recording is open; COLOR_ATTACHMENT_OUTPUT→TRANSFER with
                // COLOR→TRANSFER_SRC makes the draw's writes available to the copy;
                // `&to_transfer` outlives the call.
                unsafe {
                    (self.fns.cmd_pipeline_barrier)(
                        cmd,
                        VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                        VK_PIPELINE_STAGE_TRANSFER_BIT,
                        0,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        1,
                        (&to_transfer as *const VkImageMemoryBarrier).cast(),
                    );
                }

                let region = VkBufferImageCopy {
                    buffer_offset: 0,
                    buffer_row_length: 0,
                    buffer_image_height: 0,
                    image_subresource: VkImageSubresourceLayers {
                        aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    },
                    image_offset: VkOffset3D { x: 0, y: 0, z: 0 },
                    image_extent: VkExtent3D {
                        width: extent.width,
                        height: extent.height,
                        depth: 1,
                    },
                };
                // SAFETY: recording is open; the image is TRANSFER_SRC_OPTIMAL per the
                // barrier above; one full-image tightly-packed color region copies
                // into the live host-visible `staging.buffer` (≥ the image's byte size
                // per this fn's contract); `&region` outlives the call.
                unsafe {
                    (self.fns.cmd_copy_image_to_buffer)(
                        cmd,
                        image,
                        VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                        staging.buffer,
                        1,
                        &region,
                    );
                }

                let to_present = VkImageMemoryBarrier {
                    s_type: VkStructureType::ImageMemoryBarrier,
                    p_next: ptr::null(),
                    src_access_mask: VK_ACCESS_TRANSFER_READ_BIT,
                    dst_access_mask: 0,
                    old_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    new_layout: VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
                    src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    image,
                    subresource_range: COLOR_SUBRESOURCE_RANGE,
                };
                // SAFETY: recording is open; TRANSFER→BOTTOM_OF_PIPE with
                // TRANSFER_SRC→PRESENT releases the image to the present engine after
                // the readback copy; `&to_present` outlives the call.
                unsafe {
                    (self.fns.cmd_pipeline_barrier)(
                        cmd,
                        VK_PIPELINE_STAGE_TRANSFER_BIT,
                        VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
                        0,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        1,
                        (&to_present as *const VkImageMemoryBarrier).cast(),
                    );
                }
            }
        }

        // SAFETY: recording is open; ending it matches the `begin` above.
        let raw = unsafe { (self.fns.end_command_buffer)(cmd) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkEndCommandBuffer", result));
        }
        Ok(())
    }

}
