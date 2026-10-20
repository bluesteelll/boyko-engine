//! `Renderer::record_ssaa`: the anti-aliasing Stage 3 SSAA downsample — since render wave PX1a
//! (D-G2) the declared `ssaa_downsample` pass, recorded after the frame's last `lit` producer and
//! before `post_final`: it box-filters the 2× `lit` (left at `SHADER_READ_ONLY` by the pass's
//! derived barrier) into `lit_native` at the native `aa_extent`, which `post_final` reads.

use core::ptr;

use crate::ffi::*;

use super::super::frame_driver::Renderer;
use super::super::gpu_zone::ZONE_POST_SSAA_DOWNSAMPLE;
use super::super::scene_types::SsaaActivation;
use super::zone_arm::ZoneArm;
use super::super::targets::GBufferTargets;

impl Renderer<'_> {
    /// Records the SSAA downsample into `cmd`: one dynamic-rendering scope over `lit_native`
    /// (`loadOp = CLEAR` — the `FrameLocal` initialiser D-G2 declares), the fullscreen-triangle
    /// draw sampling `lit[fi]` through `downsample_set[fi]`, scope closed. Every barrier is the
    /// declared pass's: the caller emits them first (`lit` → `SHADER_READ_ONLY`, `lit_native` →
    /// `COLOR_ATTACHMENT_OPTIMAL`), and `post_final`'s pass orders the attachment write before its
    /// COMPUTE read.
    ///
    /// # Safety
    ///
    /// `cmd` is recording, outside any rendering scope; the caller has ALREADY emitted the
    /// frame's `ssaa_downsample` graph-pass barriers; `targets.lit_native` and
    /// `targets.downsample_set` are `Some` (both built by `GBufferTargets::create` under the same
    /// `scene.ssaa` that gates the call); `aa_extent` is the extent `lit_native` was allocated at;
    /// `activation.pipeline` targets `lit_native`'s format; `fi` is this frame's in-flight slot.
    pub(crate) unsafe fn record_ssaa(
        &self,
        cmd: VkCommandBuffer,
        targets: &GBufferTargets,
        activation: &SsaaActivation<'_>,
        aa_extent: VkExtent2D,
        fi: usize,
        zone: ZoneArm<'_>,
    ) {
        debug_assert!(
            aa_extent.width > 0 && aa_extent.height > 0,
            "invariant: lit_native is sized to a non-empty native aa_extent under SSAA"
        );
        // SAFETY: recording is open (this fn's contract), outside any rendering scope; `zone`'s
        // contract (pool reset this frame, sealed after) is the caller's.
        let zone_pair = unsafe { zone.begin(self.fns, cmd, ZONE_POST_SSAA_DOWNSAMPLE) };
        let native = targets
            .lit_native
            .as_ref()
            .expect("invariant: scene.ssaa armed ⇒ GBufferTargets::create built lit_native");
        let set = targets
            .downsample_set
            .as_ref()
            .expect("invariant: scene.ssaa armed ⇒ GBufferTargets::create built downsample_set");

        let color_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: native.view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
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
                extent: aa_extent,
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
            width: aa_extent.width as f32,
            height: aa_extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let scissor = VkRect2D {
            offset: VkOffset2D { x: 0, y: 0 },
            extent: aa_extent,
        };
        // SAFETY: recording is open; `rendering` is fully initialized — its color
        // attachment names the live `lit_native` view (COLOR_ATTACHMENT_OPTIMAL — the declared
        // pass's barrier, which the caller emitted);
        // dynamic rendering is enabled on this device. `activation.pipeline` + its layout
        // belong to this device (caller contract); `set[fi]` binds `lit[fi]` (the 2× ring
        // slot — the downsample's INPUT, already SHADER_READ_ONLY_OPTIMAL) + the NEAREST
        // sampler at set 0 of that layout (ignored by the shader's `.Load`). NO push
        // constants (`push_constant_bytes: 0` — the 2× ratio is compiled into the shader).
        // `viewport`/`scissor` outlive the bracketed calls; `draw(3, 1, 0, 0)` is the
        // `SV_VertexID` fullscreen triangle (no vertex buffer). Begin/End bracket the pass
        // exactly.
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
            (self.fns.cmd_set_viewport)(cmd, 0, 1, &viewport);
            (self.fns.cmd_set_scissor)(cmd, 0, 1, &scissor);
            (self.fns.cmd_draw)(cmd, 3, 1, 0, 0);
            (self.fns.cmd_end_rendering)(cmd);
        }

        // SAFETY: as the begin above; `zone_pair` is the pair it returned for this zone.
        unsafe { zone.end(self.fns, cmd, ZONE_POST_SSAA_DOWNSAMPLE, zone_pair) };
    }
}
