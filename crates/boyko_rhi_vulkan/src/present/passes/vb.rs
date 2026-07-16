//! `Renderer::record_vb`: the on-screen FUSED VisibilityBuffer v1 record body behind
//! [`Renderer::render_gbuffer_frame`] — the `VisibilityBuffer` sibling of
//! [`record_forward`](super::forward), driven through
//! [`VbBarrierSink`](super::super::graph_bridge::VbBarrierSink) via [`Renderer::record_vb_pass`].
//!
//! # v1 SCOPE CUT (mirrors `vb_resolve.comp.hlsl`'s own doc)
//!
//! NO SSAO/DDGI/shadow-denoise/motion-vector/TAA (`cap_vb_v1_consumers` forces every one of
//! those consumers off structurally); NO froxel (VB v1 shades ALL-LIGHTS, mirrors plain
//! `Forward`'s own base compile); NO `interp` (the VB instance ring is a plain CPU `bytemuck`
//! upload, no GPU-side interpolation this rung); NO depth prepass (`mesh_geo_shade_split ==
//! false`, fused only); NO SDF leg (`VisibilityBuffer × {Both, Sdf}` collapses to `Mesh` until
//! R10 — `LegsCollapsedToMeshPreVbSdf`). Shadows (CSM + punctual atlas) ARE in scope —
//! `vb_resolve.comp.hlsl` samples them inline via `shadow_apply.hlsli`.
//!
//! # Why a SEPARATE record body, not a `record_forward` branch
//!
//! Mirrors `record_forward`'s own "own private ResId space, zero edits to Deferred/Forward's
//! reachable code" trade-off — see that file's doc for the rationale. Every duplicated block
//! (light_upload/csm/atlas/present-blit) is a byte-for-byte port of `record_forward`'s
//! counterpart, adapted only for the VB-private barrier sink (`record_vb_pass` vs
//! `record_forward_pass`) and targets.

use core::ptr;

use crate::ffi::*;
use crate::memory::BoundBuffer;
use crate::texture::{MAX_CASCADES, MAX_TEXTURE_LAYERS};

use super::super::frame_driver::Renderer;
use super::super::scene_types::{GBUFFER_PUSH_BASE_INSTANCE_OFFSET, GBufferScene};
use super::super::targets::{ForwardTargets, GBufferTargets, VB_CLASSIFY_MAX_MATERIAL_ROWS, VbTargets};
use super::super::{COLOR_SUBRESOURCE_RANGE, SwapchainError};

/// The VB `lit` clear color — byte-identical to `record_forward`'s own `FORWARD_LIT_CLEAR`
/// (`record_gbuffer`'s albedo clear, the marcher's `BACKGROUND` base). VB v1 has no SDF leg
/// (mesh-only, `LegsCollapsedToMeshPreVbSdf` until R10) and paints the sky FIRST (`vb_sky`), so
/// this clear is only ever visible transiently before `vb_sky` overwrites every pixel.
const VB_LIT_CLEAR: [f32; 4] = [0.05, 0.05, 0.1, 1.0];

/// The VB reverse-Z depth CLEAR (Decision 4): `0.0`, the "nothing drawn yet" sentinel — mirrors
/// `record_forward`'s own `FORWARD_DEPTH_CLEAR` (paired with `VK_COMPARE_OP_GREATER`).
const VB_DEPTH_CLEAR: f32 = 0.0;

/// The `vb_id` CLEAR value (Decision 9 / plan §F): `uint2(0xFFFFFFFF, 0)` — the SDF-owned /
/// unwritten-pixel sentinel (`VB_ID_SENTINEL`, `boyko_render::render_path_config` +
/// `vb_pack.hlsli`). A miss reads `instance_id == VB_ID_SENTINEL` and `vb_resolve` writes
/// nothing for that pixel (the sky color already painted by `vb_sky` stands).
const VB_ID_CLEAR: [u32; 4] = [0xFFFF_FFFF, 0, 0, 0];

impl Renderer<'_> {
    /// Records the VisibilityBuffer on-screen frame: `light_upload? → csm? → atlas? → vb_sky →
    /// vb_raster → vb_resolve → present-blit` — EXACTLY [`Renderer::declare_vb_graph`]'s
    /// declaration order (the SAME "declare/record order parity" invariant `record_forward`'s
    /// doc explains).
    ///
    /// # Safety
    ///
    /// `cmd` must be recordable (waited free); `image`/`view` must belong to the swapchain image
    /// presented this frame; `scene`'s pipelines/buffers/samplers are live on this device;
    /// `targets`/`forward`/`vb` were synced to `present_extent` (the SAME contract
    /// [`record_gbuffer`](super::gbuffer)'s doc states, restricted to VB's own images/sets).
    /// `extent` is the swapchain extent and governs ONLY the present-blit's clear render-area and
    /// the readback region; a `Some(readback)` buffer is host-visible and ≥ the swapchain image's
    /// (`extent`-sized) byte size.
    #[allow(clippy::too_many_arguments)]
    pub(crate) unsafe fn record_vb(
        &self,
        cmd: VkCommandBuffer,
        image: VkImage,
        view: VkImageView,
        extent: VkExtent2D,
        present_extent: VkExtent2D,
        aa_extent: VkExtent2D,
        clear: [f32; 4],
        scene: &GBufferScene<'_>,
        targets: &GBufferTargets,
        forward: &ForwardTargets,
        vb: &VbTargets,
        readback: Option<&BoundBuffer>,
    ) -> Result<(), SwapchainError> {
        let begin = VkCommandBufferBeginInfo {
            s_type: VkStructureType::CommandBufferBeginInfo,
            p_next: ptr::null(),
            flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            p_inheritance_info: ptr::null(),
        };
        // SAFETY: `cmd` is recordable per this fn's contract; `begin` is a fully-initialized
        // one-time-submit begin-info.
        let raw = unsafe { (self.fns.begin_command_buffer)(cmd, &begin) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkBeginCommandBuffer", result));
        }

        let fi = self.frame_index;

        let plan = self.vb_pass_plan.as_ref().expect("invariant: declare_frame_graph ran before record_vb");

        let vertex_offset: VkDeviceSize = 0;

        // === Lighting L0-r0: ASYNC light-table re-upload — byte-for-byte port of
        // `record_forward`'s own `light_upload` block. Recorded ONLY on a dirty frame. ===
        if scene.light_dirty && scene.light_upload_bytes > 0 {
            let light_upload =
                plan.light_upload.expect("invariant: light_dirty ⇒ light_upload pass declared");
            // SAFETY: recording is open; `record_vb_pass` records the graph's derived
            // cross-frame seed-WAR buffer barrier for the "light_upload" pass into `cmd`, ahead
            // of the copy it guards.
            self.record_vb_pass(light_upload, cmd, targets, forward, vb, scene, fi);
            let region = VkBufferCopy { src_offset: 0, dst_offset: 0, size: scene.light_upload_bytes };
            // SAFETY: recording is open; the copy names the live host-coherent staging +
            // device-local table buffers; the copy region spans `[0, light_upload_bytes)` ≤ both
            // buffer sizes (caller contract). `&region` outlives the call.
            unsafe {
                (self.fns.cmd_copy_buffer)(cmd, scene.light_staging.buffer, scene.light_table.buffer, 1, &region);
            }
        }

        // === CSM cascade DEPTH pass — byte-for-byte port of `record_forward`'s own `csm` block.
        // Recorded ONLY when `scene.csm.is_some()`; runs BEFORE `vb_resolve` (which samples the
        // cascade inline). ===
        if let Some(csm) = &scene.csm {
            let cascade = scene.csm_cascade_texture;
            let active = (csm.active_count as usize).clamp(1, MAX_CASCADES) as u32;
            let csm_pass = plan.csm.expect("invariant: scene.csm.is_some() ⇒ csm pass declared");
            // SAFETY: recording is open; `record_vb_pass` records the graph's derived
            // UNDEFINED→DEPTH barrier-in for the "csm_depth" pass into `cmd`.
            self.record_vb_pass(csm_pass, cmd, targets, forward, vb, scene, fi);

            let cascade_extent = VkExtent2D { width: csm.shadow_dim, height: csm.shadow_dim };
            let csm_area = VkRect2D { offset: VkOffset2D { x: 0, y: 0 }, extent: cascade_extent };
            let csm_viewport = VkViewport {
                x: 0.0,
                y: 0.0,
                width: cascade_extent.width as f32,
                height: cascade_extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            let mut csm_push = csm.push;
            for c in 0..active {
                csm_push[0..64].copy_from_slice(&csm.cascade_view_proj[c as usize]);
                let csm_depth_attachment = VkRenderingAttachmentInfo {
                    s_type: VkStructureType::RenderingAttachmentInfo,
                    p_next: ptr::null(),
                    image_view: cascade.layer_render_view(c),
                    image_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                    resolve_mode: 0,
                    resolve_image_view: VkImageView::NULL,
                    resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                    load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
                    store_op: VK_ATTACHMENT_STORE_OP_STORE,
                    clear_value: VkClearValue {
                        depth_stencil: VkClearDepthStencilValue { depth: 1.0, stencil: 0 },
                    },
                };
                let csm_rendering = VkRenderingInfo {
                    s_type: VkStructureType::RenderingInfo,
                    p_next: ptr::null(),
                    flags: 0,
                    render_area: csm_area,
                    layer_count: 1,
                    view_mask: 0,
                    color_attachment_count: 0,
                    p_color_attachments: ptr::null(),
                    p_depth_attachment: (&csm_depth_attachment as *const VkRenderingAttachmentInfo).cast(),
                    p_stencil_attachment: ptr::null(),
                };
                // SAFETY: recording is open; `csm_rendering` names the live cascade layer-`c`
                // render view (now DEPTH_ATTACHMENT_OPTIMAL; `c < active <= MAX_CASCADES`),
                // depth-only; the depth-only pipeline + the SAME instance SSBO
                // (`scene.instance_bind_group`) satisfy the depth VS's static `instances`
                // reference; the 88-byte push carries cascade `c`'s `view_proj` +
                // `use_model_matrix == 1`; per caster batch the recorder re-pushes
                // `base_instance` then `draw_indexed` reads that batch's bound vertex+index
                // buffers. Begin/End bracket each cascade.
                unsafe {
                    (self.fns.cmd_begin_rendering)(cmd, &csm_rendering);
                    (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, csm.pipeline.pipeline);
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_GRAPHICS,
                        csm.pipeline.layout,
                        0,
                        1,
                        &scene.instance_bind_group.descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_push_constants)(
                        cmd,
                        csm.pipeline.layout,
                        VK_SHADER_STAGE_VERTEX_BIT,
                        0,
                        csm_push.len() as u32,
                        csm_push.as_ptr().cast(),
                    );
                    (self.fns.cmd_set_viewport)(cmd, 0, 1, &csm_viewport);
                    (self.fns.cmd_set_scissor)(cmd, 0, 1, &csm_area);
                    for batch in scene.mesh_draw {
                        if !batch.casts_shadow {
                            continue;
                        }
                        let base = batch.base_instance;
                        csm_push[GBUFFER_PUSH_BASE_INSTANCE_OFFSET as usize
                            ..GBUFFER_PUSH_BASE_INSTANCE_OFFSET as usize + 4]
                            .copy_from_slice(&base.to_le_bytes());
                        (self.fns.cmd_push_constants)(
                            cmd,
                            csm.pipeline.layout,
                            VK_SHADER_STAGE_VERTEX_BIT,
                            GBUFFER_PUSH_BASE_INSTANCE_OFFSET,
                            4,
                            (&base as *const u32).cast(),
                        );
                        (self.fns.cmd_bind_vertex_buffers)(cmd, 0, 1, &batch.vertex_buffer.buffer, &vertex_offset);
                        (self.fns.cmd_bind_index_buffer)(cmd, batch.index_buffer.buffer, 0, batch.index_type);
                        (self.fns.cmd_draw_indexed)(cmd, batch.index_count, batch.instance_count, 0, 0, 0);
                    }
                    (self.fns.cmd_end_rendering)(cmd);
                }
            }
        }

        // === Punctual (spot/point) shadow-atlas DEPTH pass — byte-for-byte port of
        // `record_forward`'s own `atlas` block. Recorded ONLY when `scene.atlas_punctual.is_some()`.
        // ===
        if let Some(atlas_act) = &scene.atlas_punctual {
            let atlas = scene.shadow_atlas_texture;
            let active = (atlas_act.active_layers as usize).clamp(1, MAX_TEXTURE_LAYERS) as u32;
            let atlas_pass =
                plan.atlas.expect("invariant: scene.atlas_punctual.is_some() ⇒ atlas pass declared");
            // SAFETY: recording is open; `record_vb_pass` records the graph's derived
            // UNDEFINED→DEPTH barrier-in for the "atlas_depth" pass into `cmd`.
            self.record_vb_pass(atlas_pass, cmd, targets, forward, vb, scene, fi);

            let atlas_extent = VkExtent2D { width: atlas_act.shadow_dim, height: atlas_act.shadow_dim };
            let atlas_area = VkRect2D { offset: VkOffset2D { x: 0, y: 0 }, extent: atlas_extent };
            let atlas_viewport = VkViewport {
                x: 0.0,
                y: 0.0,
                width: atlas_extent.width as f32,
                height: atlas_extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            let mut atlas_push = atlas_act.push;
            let mut bound_point: Option<bool> = None;
            for s in 0..active {
                let is_point = atlas_act.face_is_point[s as usize];
                let face_pipeline = if is_point { atlas_act.point_pipeline } else { atlas_act.pipeline };
                atlas_push[0..64].copy_from_slice(&atlas_act.face_view_proj[s as usize]);
                if is_point {
                    atlas_push[64..80].copy_from_slice(&atlas_act.face_light[s as usize]);
                }
                let atlas_depth_attachment = VkRenderingAttachmentInfo {
                    s_type: VkStructureType::RenderingAttachmentInfo,
                    p_next: ptr::null(),
                    image_view: atlas.layer_render_view(s),
                    image_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                    resolve_mode: 0,
                    resolve_image_view: VkImageView::NULL,
                    resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                    load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
                    store_op: VK_ATTACHMENT_STORE_OP_STORE,
                    clear_value: VkClearValue {
                        depth_stencil: VkClearDepthStencilValue { depth: 1.0, stencil: 0 },
                    },
                };
                let atlas_rendering = VkRenderingInfo {
                    s_type: VkStructureType::RenderingInfo,
                    p_next: ptr::null(),
                    flags: 0,
                    render_area: atlas_area,
                    layer_count: 1,
                    view_mask: 0,
                    color_attachment_count: 0,
                    p_color_attachments: ptr::null(),
                    p_depth_attachment: (&atlas_depth_attachment as *const VkRenderingAttachmentInfo).cast(),
                    p_stencil_attachment: ptr::null(),
                };
                // SAFETY: recording is open; `atlas_rendering` names the live atlas layer-`s`
                // render view (now DEPTH_ATTACHMENT_OPTIMAL; `s < active <= MAX_TEXTURE_LAYERS`),
                // depth-only; the selected SPOT/POINT depth-only pipeline shares the SAME layout
                // as the SAME instance SSBO set 0; the 88-byte push carries slot `s`'s
                // `view_proj` (+ the POINT `cam_eye` lane) + `use_model_matrix == 1`; per caster
                // batch the recorder re-pushes `base_instance` then `draw_indexed` reads that
                // batch's bound vertex+index buffers. Begin/End bracket each slot.
                unsafe {
                    (self.fns.cmd_begin_rendering)(cmd, &atlas_rendering);
                    if bound_point != Some(is_point) {
                        (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, face_pipeline.pipeline);
                        (self.fns.cmd_bind_descriptor_sets)(
                            cmd,
                            VK_PIPELINE_BIND_POINT_GRAPHICS,
                            face_pipeline.layout,
                            0,
                            1,
                            &scene.instance_bind_group.descriptor_set,
                            0,
                            ptr::null(),
                        );
                        bound_point = Some(is_point);
                    }
                    (self.fns.cmd_push_constants)(
                        cmd,
                        face_pipeline.layout,
                        VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
                        0,
                        atlas_push.len() as u32,
                        atlas_push.as_ptr().cast(),
                    );
                    (self.fns.cmd_set_viewport)(cmd, 0, 1, &atlas_viewport);
                    (self.fns.cmd_set_scissor)(cmd, 0, 1, &atlas_area);
                    for batch in scene.mesh_draw {
                        if !batch.casts_shadow {
                            continue;
                        }
                        let base = batch.base_instance;
                        atlas_push[GBUFFER_PUSH_BASE_INSTANCE_OFFSET as usize
                            ..GBUFFER_PUSH_BASE_INSTANCE_OFFSET as usize + 4]
                            .copy_from_slice(&base.to_le_bytes());
                        (self.fns.cmd_push_constants)(
                            cmd,
                            face_pipeline.layout,
                            VK_SHADER_STAGE_VERTEX_BIT,
                            GBUFFER_PUSH_BASE_INSTANCE_OFFSET,
                            4,
                            (&base as *const u32).cast(),
                        );
                        (self.fns.cmd_bind_vertex_buffers)(cmd, 0, 1, &batch.vertex_buffer.buffer, &vertex_offset);
                        (self.fns.cmd_bind_index_buffer)(cmd, batch.index_buffer.buffer, 0, batch.index_type);
                        (self.fns.cmd_draw_indexed)(cmd, batch.index_count, batch.instance_count, 0, 0, 0);
                    }
                    (self.fns.cmd_end_rendering)(cmd);
                }
            }
        }

        // === Pass `vb_sky`: the sky background pass — writes `lit` (COLOR, first-touch). ===
        // SAFETY: recording is open; `record_vb_pass` records the graph's derived
        // UNDEFINED→COLOR_ATTACHMENT_OPTIMAL barrier-in for `lit` (+ the cascade/atlas
        // →SHADER_READ_ONLY_OPTIMAL barriers-out, this pass's declared FRAGMENT reader are
        // actually consumed by `vb_resolve` below, not this pass — see `declare_vb_graph`) for
        // the "vb_sky" pass into `cmd`.
        self.record_vb_pass(plan.vb_sky, cmd, targets, forward, vb, scene, fi);

        let vb_sky_pipeline =
            scene.vb_sky_pipeline.expect("invariant: a VisibilityBuffer-resolved scene always carries vb_sky_pipeline");
        let vb_set0 =
            targets.vb_set0.as_ref().expect("invariant: TargetsProfile::VbMesh ⇒ targets.vb_set0 is built");

        let lit_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: targets.lit[fi].view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue { color: VkClearColorValue { float32: VB_LIT_CLEAR } },
        };
        let sky_rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: VkRect2D { offset: VkOffset2D { x: 0, y: 0 }, extent: present_extent },
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: 1,
            p_color_attachments: &lit_attachment,
            p_depth_attachment: ptr::null(),
            p_stencil_attachment: ptr::null(),
        };
        let full_viewport = VkViewport {
            x: 0.0,
            y: 0.0,
            width: present_extent.width as f32,
            height: present_extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let full_area = VkRect2D { offset: VkOffset2D { x: 0, y: 0 }, extent: present_extent };
        // SAFETY: recording is open; `sky_rendering` names the live `lit` view (now
        // COLOR_ATTACHMENT_OPTIMAL); `vb_sky_pipeline` (REUSING the compiled `forward_sky`
        // SPIR-V verbatim against a NEW pipeline object built for `vb_layout0` — declares NO
        // depth attachment, `GBufferScene::vb_sky_pipeline`'s doc); `vb_set0[fi]` is a live
        // descriptor set written once per extent (the sky FS reads only its `Camera`/`LightBuf`
        // subset — the SAME bound-but-unread-subset idiom `forward_sky_pipeline` establishes).
        // `draw(3, 1, 0, 0)` is the `SV_VertexID` fullscreen triangle (no vertex buffer). Begin/
        // End bracket the pass exactly.
        unsafe {
            (self.fns.cmd_begin_rendering)(cmd, &sky_rendering);
            (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, vb_sky_pipeline.pipeline);
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                vb_sky_pipeline.layout,
                0,
                1,
                &vb_set0[fi].descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_set_viewport)(cmd, 0, 1, &full_viewport);
            (self.fns.cmd_set_scissor)(cmd, 0, 1, &full_area);
            (self.fns.cmd_draw)(cmd, 3, 1, 0, 0);
            (self.fns.cmd_end_rendering)(cmd);
        }

        // === Passes `vb_raster` + `vb_resolve` — rung R10: recorded ONLY under `mesh_leg`
        // (a `VisibilityBuffer x Sdf` frame skips both — the Decision-0 geometry table they
        // re-fetch through carries no slot with no mesh leg; see `declare_vb_graph`'s matching
        // gate). `vb_sky`'s `lit` write above stands for the sky; `sdf_forward_march` below
        // composites the SDF field over whatever the mesh raster/resolve left. ===
        if scene.resolved_render_path.mesh_leg {
            // === Pass `vb_raster`: the mesh id-raster pass (Decision 9) — writes `vb_id` (COLOR,
            // R32G32_UINT) + `vb_depth` (DEPTH, HW reverse-Z, first-touch `GREATER`, write ON). ===
            // SAFETY: recording is open; `record_vb_pass` records the graph's derived
            // UNDEFINED→COLOR_ATTACHMENT_OPTIMAL (`vb_id`) + UNDEFINED→DEPTH_ATTACHMENT_OPTIMAL
            // (`vb_depth`) barriers-in for the "vb_raster" pass into `cmd`.
            self.record_vb_pass(
                plan.vb_raster.expect("invariant: mesh_leg => vb_raster pass declared (declare_vb_graph)"),
                cmd,
                targets,
                forward,
                vb,
                scene,
                fi,
            );

            let vb_raster_pipeline =
                scene.vb_raster_pipeline.expect("invariant: a VisibilityBuffer-resolved scene always carries vb_raster_pipeline");

            let vb_id_attachment = VkRenderingAttachmentInfo {
                s_type: VkStructureType::RenderingAttachmentInfo,
                p_next: ptr::null(),
                image_view: vb.vb_id[fi].view,
                image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                resolve_mode: 0,
                resolve_image_view: VkImageView::NULL,
                resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
                store_op: VK_ATTACHMENT_STORE_OP_STORE,
                clear_value: VkClearValue { color: VkClearColorValue { uint32: VB_ID_CLEAR } },
            };
            let vb_depth_attachment = VkRenderingAttachmentInfo {
                s_type: VkStructureType::RenderingAttachmentInfo,
                p_next: ptr::null(),
                image_view: forward.depth[fi].view,
                image_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                resolve_mode: 0,
                resolve_image_view: VkImageView::NULL,
                resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
                store_op: VK_ATTACHMENT_STORE_OP_STORE,
                clear_value: VkClearValue {
                    depth_stencil: VkClearDepthStencilValue { depth: VB_DEPTH_CLEAR, stencil: 0 },
                },
            };
            let vb_rendering = VkRenderingInfo {
                s_type: VkStructureType::RenderingInfo,
                p_next: ptr::null(),
                flags: 0,
                render_area: full_area,
                layer_count: 1,
                view_mask: 0,
                color_attachment_count: 1,
                p_color_attachments: &vb_id_attachment,
                p_depth_attachment: (&vb_depth_attachment as *const VkRenderingAttachmentInfo).cast(),
                p_stencil_attachment: ptr::null(),
            };
            // SAFETY: recording is open; `vb_rendering` names the live `vb_id` view (now
            // COLOR_ATTACHMENT_OPTIMAL) + the live `vb_depth` (`forward.depth[fi]`, REUSED verbatim)
            // view (now DEPTH_ATTACHMENT_OPTIMAL); `vb_raster_pipeline` (1-set, built against
            // `vb_layout0` — its VS references only `instances`/the push, a bound-but-unread subset
            // of `vb_set0`) + the 88-byte VERTEX push range belong to this device (caller contract);
            // `vb_set0[fi]` is a live descriptor set. `full_viewport`/`full_area` outlive the
            // bracketed calls; each `DrawBatch`'s per-instance draw reads that batch's bound
            // vertex+index buffers. Begin/End bracket the pass exactly.
            unsafe {
                (self.fns.cmd_begin_rendering)(cmd, &vb_rendering);
                (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, vb_raster_pipeline.pipeline);
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_GRAPHICS,
                    vb_raster_pipeline.layout,
                    0,
                    1,
                    &vb_set0[fi].descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_push_constants)(
                    cmd,
                    vb_raster_pipeline.layout,
                    VK_SHADER_STAGE_VERTEX_BIT,
                    0,
                    scene.mvp.len() as u32,
                    scene.mvp.as_ptr().cast(),
                );
                (self.fns.cmd_set_viewport)(cmd, 0, 1, &full_viewport);
                (self.fns.cmd_set_scissor)(cmd, 0, 1, &full_area);
                for batch in scene.mesh_draw {
                    let base = batch.base_instance;
                    (self.fns.cmd_push_constants)(
                        cmd,
                        vb_raster_pipeline.layout,
                        VK_SHADER_STAGE_VERTEX_BIT,
                        GBUFFER_PUSH_BASE_INSTANCE_OFFSET,
                        4,
                        (&base as *const u32).cast(),
                    );
                    (self.fns.cmd_bind_vertex_buffers)(cmd, 0, 1, &batch.vertex_buffer.buffer, &vertex_offset);
                    (self.fns.cmd_bind_index_buffer)(cmd, batch.index_buffer.buffer, 0, batch.index_type);
                    (self.fns.cmd_draw_indexed)(cmd, batch.index_count, batch.instance_count, 0, 0, 0);
                }
                (self.fns.cmd_end_rendering)(cmd);
            }

            // === VB-P2 classification plan (docs/VB-P2-CLASSIFICATION-PLAN.md), rung P2c: the
            // classify chain `fill -> count -> scan -> scatter` — populates `gClassify` on real
            // hardware ONLY when the classified path is selected (`scene.vb_use_classified`, plan
            // P1-4) — mirrors `declare_vb_graph`'s matching gate, so a `!vb_use_classified` frame
            // records NONE of these four passes (ZERO classify tax, not merely an unread output as
            // rung P2b left it). `vb_shade` (below) is the sole consumer of `gClassify`. ===
            if scene.vb_use_classified {
                // Pass `vb_classify_fill`: two `vkCmdFillBuffer`s zero `counts[MAX]` and sentinel
                // `group_to_mat[G+MAX]` with `0xFFFFFFFF` (critic P1-1 — decouples correctness from
                // the scan's per-frame loop bound). Word offsets mirror
                // `vb_classify_common.hlsli`'s own sync-pin: `counts_off = 0`, `group_to_mat_off =
                // 4*MAX` (both word offsets, ×4 for bytes); `group_to_mat`'s reserved CAPACITY is
                // `G+MAX` (P1-2 — fixed per extent, not per frame), so the sentinel fill covers the
                // WHOLE reserved region regardless of this frame's live material count.
                // SAFETY: recording is open; `record_vb_pass` records the graph's derived
                // TRANSFER_WRITE producer access for the "vb_classify_fill" pass into `cmd` (the
                // FIRST access on `gclassify` this frame).
                self.record_vb_pass(
                    plan.vb_classify_fill.expect(
                        "invariant: mesh_leg && vb_use_classified => vb_classify_fill pass declared (declare_vb_graph)",
                    ),
                    cmd,
                    targets,
                    forward,
                    vb,
                    scene,
                    fi,
                );
                let gclassify_buf = targets
                    .vb_classify
                    .as_ref()
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries targets.vb_classify")
                    .gclassify[fi]
                    .buffer;
                let counts_bytes: VkDeviceSize = VB_CLASSIFY_MAX_MATERIAL_ROWS * 4;
                let group_to_mat_off_bytes: VkDeviceSize = 4 * VB_CLASSIFY_MAX_MATERIAL_ROWS * 4;
                let group_to_mat_bytes: VkDeviceSize =
                    (scene.dispatch_group_count_x as VkDeviceSize + VB_CLASSIFY_MAX_MATERIAL_ROWS) * 4;
                // SAFETY: recording is open; `gclassify_buf` is the live per-FIF `gClassify` buffer
                // (`STORAGE | TRANSFER_DST`, sized per `VbClassifyTargets::build`'s doc); both fill
                // regions lie within its bounds (`counts` is the buffer's first `MAX*4` bytes;
                // `group_to_mat`'s `[4*MAX*4, 4*MAX*4 + (G+MAX)*4)` range is its own reserved region,
                // per the sync-pin doc above — both fully inside the buffer `VbClassifyTargets::build`
                // allocates).
                unsafe {
                    (self.fns.cmd_fill_buffer)(cmd, gclassify_buf, 0, counts_bytes, 0);
                    (self.fns.cmd_fill_buffer)(
                        cmd,
                        gclassify_buf,
                        group_to_mat_off_bytes,
                        group_to_mat_bytes,
                        0xFFFF_FFFF,
                    );
                }

                // Pass `vb_classify_count`: one thread per composite pixel, `InterlockedAdd
                // (counts[mat], 1)` for every non-SENTINEL pixel's material id.
                // SAFETY: recording is open; `record_vb_pass` records the graph's derived
                // COLOR_ATTACHMENT_OPTIMAL->SHADER_READ_ONLY_OPTIMAL barrier (`vb_id`, the FIRST
                // reader this frame — `vb_shade`'s later same-layout read needs none) + the
                // TRANSFER_WRITE->SHADER_READ|SHADER_WRITE barrier (`gclassify`, chained from
                // `vb_classify_fill`) for the "vb_classify_count" pass.
                self.record_vb_pass(
                    plan.vb_classify_count.expect(
                        "invariant: mesh_leg && vb_use_classified => vb_classify_count pass declared (declare_vb_graph)",
                    ),
                    cmd,
                    targets,
                    forward,
                    vb,
                    scene,
                    fi,
                );
                let vb_classify_count_pipeline = scene
                    .vb_classify_count_pipeline
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_classify_count_pipeline");
                // SAFETY: recording is open; `vb_classify_count_pipeline` (1-set, built against
                // `vb_layout0`) belongs to this device (caller contract); `vb_set0[fi]` is a live
                // descriptor set; `scene.dispatch_group_count_x` covers `present_extent.width *
                // present_extent.height` pixels at the shader's `numthreads(64,1,1)` 1D grid (the
                // SAME grid `vb_shade` dispatches at). The pipeline's push-constant range (4 bytes,
                // declared but unread by this shader — the shared compute-push-range convention this
                // RHI mandates, `DdgiUpdateActivation`'s own doc) is never written; nothing pushes.
                unsafe {
                    (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, vb_classify_count_pipeline.pipeline);
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_classify_count_pipeline.layout,
                        0,
                        1,
                        &vb_set0[fi].descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
                }

                // Pass `vb_classify_scan`: a single workgroup performing the two chained
                // exclusive-prefix-sum phases (`counts -> offsets`/`cursors`, `gc -> gbase` +
                // `group_to_mat` fill) over the LIVE `[0, material_count)` prefix.
                // SAFETY: recording is open; `record_vb_pass` records the graph's derived
                // SHADER_WRITE->SHADER_READ|SHADER_WRITE barrier (`gclassify`, chained from
                // `vb_classify_count`) for the "vb_classify_scan" pass — P1-3.
                self.record_vb_pass(
                    plan.vb_classify_scan.expect(
                        "invariant: mesh_leg && vb_use_classified => vb_classify_scan pass declared (declare_vb_graph)",
                    ),
                    cmd,
                    targets,
                    forward,
                    vb,
                    scene,
                    fi,
                );
                let vb_classify_scan_pipeline = scene
                    .vb_classify_scan_pipeline
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_classify_scan_pipeline");
                // SAFETY: recording is open; `vb_classify_scan_pipeline`'s 4-byte push constant
                // (`PushConstants { uint material_count; }`, `vb_classify_scan.comp.hlsl`) is
                // written from `scene.vb_classify_material_count` (a plain `u32` local, `'static`
                // for the duration of this call); the pointer is valid and the push call happens
                // before the dispatch reads it.
                unsafe {
                    (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, vb_classify_scan_pipeline.pipeline);
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_classify_scan_pipeline.layout,
                        0,
                        1,
                        &vb_set0[fi].descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_push_constants)(
                        cmd,
                        vb_classify_scan_pipeline.layout,
                        VK_SHADER_STAGE_COMPUTE_BIT,
                        0,
                        4,
                        (&scene.vb_classify_material_count as *const u32).cast(),
                    );
                    (self.fns.cmd_dispatch)(cmd, 1, 1, 1);
                }

                // Pass `vb_classify_scatter`: one thread per composite pixel, claims a slot in its
                // material's `pixel_list` region and stores the pixel's linear index there.
                // SAFETY: recording is open; `record_vb_pass` records the graph's derived
                // SHADER_WRITE->SHADER_READ|SHADER_WRITE barrier (`gclassify`, chained from
                // `vb_classify_scan`) + the (already-SHADER_READ_ONLY_OPTIMAL, same-layout, no-op)
                // `vb_id` read for the "vb_classify_scatter" pass.
                self.record_vb_pass(
                    plan.vb_classify_scatter.expect(
                        "invariant: mesh_leg && vb_use_classified => vb_classify_scatter pass declared (declare_vb_graph)",
                    ),
                    cmd,
                    targets,
                    forward,
                    vb,
                    scene,
                    fi,
                );
                let vb_classify_scatter_pipeline = scene.vb_classify_scatter_pipeline.expect(
                    "invariant: a VisibilityBuffer-resolved scene always carries vb_classify_scatter_pipeline",
                );
                // SAFETY: recording is open; same contract as `vb_classify_count_pipeline` above —
                // 1-set pipeline, `vb_set0[fi]` live, `dispatch_group_count_x` covers every pixel;
                // this shader also declares no push constant, so nothing pushes.
                unsafe {
                    (self.fns.cmd_bind_pipeline)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_classify_scatter_pipeline.pipeline,
                    );
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_classify_scatter_pipeline.layout,
                        0,
                        1,
                        &vb_set0[fi].descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
                }
            }

            // === The `lit`-producer choice (plan P1-4): `vb_shade` (material-classified) when
            // `scene.vb_use_classified`, else the fused `vb_resolve` — mutually exclusive by
            // construction, exactly one runs per frame (mirrors `declare_vb_graph`'s matching
            // branch). ===
            if scene.vb_use_classified {
                // === Pass `vb_shade`: the material-classified shading compute pass (VB-P2
                // classification plan rung P2c) — re-fetches geometry via the Decision-0 table
                // (Set 2) for each classify-table pixel, shades, writes `lit` (STORAGE, extending
                // `vb_sky`'s COLOR write, C5). Byte-identical to `vb_resolve`'s own shading tail by
                // construction (plan D3). ===
                // SAFETY: recording is open; `record_vb_pass` records the graph's derived
                // (already-SHADER_READ_ONLY_OPTIMAL, no-op) `vb_id` read + the SHADER_WRITE->
                // SHADER_READ `gclassify` barrier (chained from `vb_classify_scatter`) + the
                // COLOR_ATTACHMENT_OPTIMAL→GENERAL barrier for `lit` (+ the cascade/atlas
                // →SHADER_READ_ONLY_OPTIMAL barriers when armed) for the "vb_shade" pass into `cmd`.
                self.record_vb_pass(
                    plan.vb_shade.expect(
                        "invariant: mesh_leg && vb_use_classified => vb_shade pass declared (declare_vb_graph)",
                    ),
                    cmd,
                    targets,
                    forward,
                    vb,
                    scene,
                    fi,
                );

                // Textured-PBR rung TV0 (`RENDER-PARITY-PLAN.md` §2.3): select the TEXTURED
                // `vb_shade` pipeline + its OWN Set-0 vocabulary set (`vb_set0_tex`, the wider
                // `PerInstanceMaterialTex` ring at b1) when this frame's VB gather bound a
                // non-zero material texture slot (`GBufferScene::vb_tex_active`'s own doc) — the
                // base classified pipeline/set otherwise. Mutually exclusive by construction
                // (`vb_tex_active` implies `vb_use_classified`, since it feeds that selector's
                // OR-in at the `GpuSceneBundles::scene()` assembly seam).
                let textured = scene.vb_tex_active();
                let (vb_shade_pipeline, vb_shade_set0) = if textured {
                    (
                        scene.vb_shade_tex_pipeline.expect(
                            "invariant: GBufferScene::vb_tex_active() => scene.vb_shade_tex_pipeline is Some",
                        ),
                        targets.vb_set0_tex.as_ref().expect(
                            "invariant: GBufferScene::vb_tex_active() => targets.vb_set0_tex is built",
                        ),
                    )
                } else {
                    (
                        scene.vb_shade_pipeline.expect(
                            "invariant: a VisibilityBuffer-resolved scene always carries vb_shade_pipeline",
                        ),
                        vb_set0,
                    )
                };
                let vb_geometry_set = scene
                    .vb_geometry_set
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_geometry_set");
                // SAFETY: recording is open; `vb_shade_pipeline` (the compute pipeline: Set 0 =
                // `vb_shade_set0[fi]` (`vb_set0[fi]` or, when `textured`, `vb_set0_tex[fi]`), Set 1 =
                // `forward.set1[fi]` — the Forward-family shadow set REUSED VERBATIM, Set 2 =
                // `vb_geometry_set` — the Decision-0 geometry table, bound directly, no ring, the
                // SAME triple `vb_resolve_pipeline` binds — PLUS, when `textured`, a 4th Set 3 =
                // `scene.bindless_set` — the shared bindless texture-array table, its LAYOUT already
                // baked into the TEXTURED pipeline's `VkPipelineLayout` at boot, mirroring
                // `record_gbuffer`'s own `tex_active` bindless-set bind) belongs to this device
                // (caller contract); `scene.mvp`'s leading 64 bytes are the SAME `view_proj` matrix
                // `vb_resolve.comp.hlsl`'s push constant reads (`vb_shade.comp.hlsl`'s push is the
                // identical 64-byte shape regardless of `-D TEXTURED`, plan D3); `scene.dispatch_group_count_x +
                // scene.vb_classify_material_count` is the D2 over-dispatch (`G +
                // present_material_count` groups — the classify chain's `scan` pass populated
                // `group_to_mat[0..total_groups)` with real material ids and left
                // `[total_groups, G+MAX)` SENTINEL from `fill`; `vb_shade`/`vb_shade_tex` early-outs
                // on every surplus group's SENTINEL read).
                unsafe {
                    (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, vb_shade_pipeline.pipeline);
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_shade_pipeline.layout,
                        0,
                        1,
                        &vb_shade_set0[fi].descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_shade_pipeline.layout,
                        1,
                        1,
                        &forward.set1[fi].descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_shade_pipeline.layout,
                        2,
                        1,
                        &vb_geometry_set.set(),
                        0,
                        ptr::null(),
                    );
                    if textured {
                        let bindless_set = scene
                            .bindless_set
                            .expect("invariant: GBufferScene::vb_tex_active() => scene.bindless_set is Some");
                        (self.fns.cmd_bind_descriptor_sets)(
                            cmd,
                            VK_PIPELINE_BIND_POINT_COMPUTE,
                            vb_shade_pipeline.layout,
                            3,
                            1,
                            &bindless_set,
                            0,
                            ptr::null(),
                        );
                    }
                    (self.fns.cmd_push_constants)(
                        cmd,
                        vb_shade_pipeline.layout,
                        VK_SHADER_STAGE_COMPUTE_BIT,
                        0,
                        64,
                        scene.mvp.as_ptr().cast(),
                    );
                    (self.fns.cmd_dispatch)(
                        cmd,
                        scene.dispatch_group_count_x + scene.vb_classify_material_count,
                        1,
                        1,
                    );
                }
            } else {
                // === Pass `vb_resolve`: the FUSED resolve compute pass (Decision 5) — reads `vb_id`,
                // re-fetches geometry via the Decision-0 table (Set 2), shades, writes `lit` (STORAGE,
                // extending `vb_sky`'s COLOR write, C5). ===
                // SAFETY: recording is open; `record_vb_pass` records the graph's derived
                // COLOR_ATTACHMENT_OPTIMAL→SHADER_READ_ONLY_OPTIMAL barrier for `vb_id` + the
                // COLOR_ATTACHMENT_OPTIMAL→GENERAL barrier for `lit` (+ the cascade/atlas
                // →SHADER_READ_ONLY_OPTIMAL barriers when armed) for the "vb_resolve" pass into `cmd`.
                self.record_vb_pass(
                    plan.vb_resolve.expect(
                        "invariant: mesh_leg && !vb_use_classified => vb_resolve pass declared (declare_vb_graph)",
                    ),
                    cmd,
                    targets,
                    forward,
                    vb,
                    scene,
                    fi,
                );

                let vb_resolve_pipeline = scene
                    .vb_resolve_pipeline
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_resolve_pipeline");
                let vb_geometry_set = scene
                    .vb_geometry_set
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_geometry_set");
                // SAFETY: recording is open; `vb_resolve_pipeline` (the 3-set compute pipeline: Set 0 =
                // `vb_set0[fi]`, Set 1 = `forward.set1[fi]` — the Forward-family shadow set REUSED
                // VERBATIM, Set 2 = `vb_geometry_set` — the Decision-0 geometry table, bound directly,
                // no ring) belongs to this device (caller contract); `scene.mvp` is the SAME 88-byte
                // push whose leading 64 bytes are the `view_proj` matrix `vb_resolve.comp.hlsl`'s
                // push constant reads (the SAME matrix `vb_raster.vs.hlsl` used, `GBUFFER_PUSH_BYTES`
                // layout parity); `scene.dispatch_group_count_x` covers `present_extent.width *
                // present_extent.height` pixels at the shader's `numthreads(64,1,1)` 1D grid (the SAME
                // grid the deferred marcher/resolve/`sdf_forward_march` dispatch at).
                unsafe {
                    (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, vb_resolve_pipeline.pipeline);
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_resolve_pipeline.layout,
                        0,
                        1,
                        &vb_set0[fi].descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_resolve_pipeline.layout,
                        1,
                        1,
                        &forward.set1[fi].descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_resolve_pipeline.layout,
                        2,
                        1,
                        &vb_geometry_set.set(),
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_push_constants)(
                        cmd,
                        vb_resolve_pipeline.layout,
                        VK_SHADER_STAGE_COMPUTE_BIT,
                        0,
                        64,
                        scene.mvp.as_ptr().cast(),
                    );
                    (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
                }
            }
        }

        // === Pass `sdf_forward_march` — rung R10: the fused SDF march-then-shade COMPUTE pass,
        // the SAME body `record_forward` records (that fn's own doc has the full C5 rationale).
        // Recorded ONLY when `scene.path_has_sdf_forward()` holds. Extends the `lit` write above
        // (`vb_resolve`'s GENERAL store under `Both`, or `vb_sky`'s COLOR write under `Sdf`) and
        // marches THIS frame's `lit` pixels the raster/sky did not already paint (a miss writes
        // nothing — the sky/mesh color stands, the pass's own doc). ===
        if scene.path_has_sdf_forward() {
            let sdf_forward_pass = plan
                .sdf_forward_march
                .expect("invariant: scene.path_has_sdf_forward() => sdf_forward_march pass declared");
            // SAFETY: recording is open; `record_vb_pass` records the graph's derived `lit`
            // ->GENERAL barrier (+ the `vb_depth` DEPTH_ATTACHMENT_OPTIMAL->SHADER_READ_ONLY_OPTIMAL
            // barrier under `mesh_leg`, or the cascade/atlas/light_table producer barriers under
            // `!mesh_leg`) for the "sdf_forward_march" pass into `cmd`.
            self.record_vb_pass(sdf_forward_pass, cmd, targets, forward, vb, scene, fi);

            // Decision 4's ownership gate: the `HAS_MESH` variant needs the reverse-Z decode
            // `A`/`B` (`scene.sdf_forward_view_z_a`/`_b`) to bound the march at the mesh surface;
            // the mesh-less variant never reads them (`SdfForwardMarchPush::sdf_only`'s doc).
            let (pipeline, push) = if scene.resolved_render_path.mesh_leg {
                let p = scene.sdf_forward_march_pipeline.expect(
                    "invariant: scene.path_has_sdf_forward() requires scene.sdf_forward_march_pipeline",
                );
                let push = crate::compute::SdfForwardMarchPush::has_mesh(
                    present_extent.width,
                    present_extent.height,
                    scene.sdf_forward_view_z_a,
                    scene.sdf_forward_view_z_b,
                    scene.light_dir,
                );
                (p, push)
            } else {
                let p = scene.sdf_forward_march_sdfonly_pipeline.expect(
                    "invariant: scene.path_has_sdf_forward() requires scene.sdf_forward_march_sdfonly_pipeline",
                );
                let push = crate::compute::SdfForwardMarchPush::sdf_only(
                    present_extent.width,
                    present_extent.height,
                    scene.light_dir,
                );
                (p, push)
            };
            let sdf_forward_set = targets
                .sdf_forward_set
                .as_ref()
                .expect("invariant: scene.path_has_sdf_forward() => targets.sdf_forward_set is built");
            let push_bytes = push.as_bytes();
            // SAFETY: recording is open; `pipeline` (the `HAS_MESH` or mesh-less compute variant,
            // selected by `mesh_leg`) + its 2-set layout (Set 0 = `sdf_forward_set[fi]`, the SAME
            // path-independent vocab ring the Forward family binds — it references `forward.depth`,
            // which VB reuses as `vb_depth`; Set 1 = `forward.set1[fi]`, the Forward-family shadow
            // set REUSED VERBATIM, the same one `vb_resolve` binds) belong to this device (caller
            // contract); both descriptor sets are live, written once per extent; `push_bytes` is
            // exactly `SDF_FORWARD_MARCH_PUSH_BYTES` (40) at offset 0; `scene.dispatch_group_count_x`
            // covers `present_extent.width * present_extent.height` pixels at the shader's
            // `numthreads(64,1,1)` 1D grid (the SAME grid `vb_resolve`/the deferred marcher use).
            unsafe {
                (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline.pipeline);
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    pipeline.layout,
                    0,
                    1,
                    &sdf_forward_set[fi].descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    pipeline.layout,
                    1,
                    1,
                    &forward.set1[fi].descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_push_constants)(
                    cmd,
                    pipeline.layout,
                    VK_SHADER_STAGE_COMPUTE_BIT,
                    0,
                    crate::compute::SDF_FORWARD_MARCH_PUSH_BYTES,
                    push_bytes.as_ptr().cast(),
                );
                (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
            }
        }

        // === Present-blit `lit` into the swapchain — byte-for-byte port of `record_forward`'s
        // own tail. ===
        // SAFETY: recording is open; `record_vb_pass` records the graph's derived
        // GENERAL→SHADER_READ_ONLY_OPTIMAL barrier for the "present_sample" pass into `cmd`.
        self.record_vb_pass(plan.present_sample, cmd, targets, forward, vb, scene, fi);

        // Anti-aliasing resolve (VB). When AA is armed, `sync_gbuffer` rewires `present_set` to
        // sample `aa_out` (path-agnostic), but the FXAA/SMAA/SSAA dispatch used to live ONLY in
        // `record_gbuffer` (Deferred) -- so under VB the resolved `lit` was never anti-aliased into
        // `aa_out`, and the present-blit below sampled a never-written (black) image. Mirror
        // `record_gbuffer`'s AA block verbatim: FXAA/SMAA read `present_extent`; SSAA reads the
        // BOOT-FIXED `aa_extent` (`present_extent` is 2x under SSAA). TAA cannot occur under VB
        // (`cap_vb_v1_consumers` forces `scene.taa` off), so the fall-through is a debug-only
        // invariant guard, never a live path. OFF (`aa_out` is `None`) records nothing -> the
        // AA-off VB command stream is byte-identical to before this block existed.
        if targets.aa_out.is_some() {
            if let Some(fxaa) = scene.aa.as_ref() {
                // SAFETY: recording is open; `present_sample` above left `lit` in
                // SHADER_READ_ONLY_OPTIMAL; `aa_out`/`fxaa_set` were built by `create()` under the
                // same `scene.aa` that gates this branch; `present_extent` sizes `aa_out`.
                unsafe { self.record_fxaa(cmd, targets, fxaa, present_extent, fi) };
            } else if let Some(smaa) = scene.smaa.as_ref() {
                // SAFETY: recording is open; `present_sample` above left `lit` in
                // SHADER_READ_ONLY_OPTIMAL; `aa_out`/the SMAA edge/weight targets + the three
                // `smaa_*_set` rings were built by `create()` under the same `scene.smaa` that
                // gates this branch; `present_extent` sizes every SMAA target.
                unsafe { self.record_smaa(cmd, targets, smaa, present_extent, fi) };
            } else if let Some(ssaa) = scene.ssaa.as_ref() {
                debug_assert!(targets.aa_out.is_some() && targets.downsample_set.is_some());
                // SAFETY: recording is open; `present_sample` above left `lit` (the 2x ring) in
                // SHADER_READ_ONLY_OPTIMAL; `aa_out`/`downsample_set` were built by `create()`
                // under the same `scene.ssaa` that gates this branch, sized to the BOOT-FIXED
                // `aa_extent` (NOT `present_extent`, which is 2x under SSAA).
                unsafe { self.record_ssaa(cmd, targets, ssaa, aa_extent, fi) };
            } else {
                // VB caps TAA off (`cap_vb_v1_consumers`), so an armed `aa_out` with none of
                // aa/smaa/ssaa matched is an invariant violation (unlike `record_gbuffer`'s
                // equivalent else, which documents "TAA already ran above").
                debug_assert!(
                    false,
                    "invariant: VB aa_out armed but none of aa/smaa/ssaa matched (taa is capped under VB)"
                );
            }
        }

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
        // TOP_OF_PIPE→COLOR_ATTACHMENT_OUTPUT with UNDEFINED→COLOR is the superset-correct
        // acquire→render transition; `&to_color` outlives the call.
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
            clear_value: VkClearValue { color: VkClearColorValue { float32: clear } },
        };
        let present_rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: VkRect2D { offset: VkOffset2D { x: 0, y: 0 }, extent },
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: 1,
            p_color_attachments: &color_attachment,
            p_depth_attachment: ptr::null(),
            p_stencil_attachment: ptr::null(),
        };
        let blit_extent = VkExtent2D {
            width: extent.width.min(present_extent.width),
            height: extent.height.min(present_extent.height),
        };
        let blit_viewport = VkViewport {
            x: 0.0,
            y: 0.0,
            width: blit_extent.width as f32,
            height: blit_extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let blit_scissor = VkRect2D { offset: VkOffset2D { x: 0, y: 0 }, extent: blit_extent };
        // SAFETY: recording is open; `present_rendering` names the live swapchain `view` (now
        // COLOR_ATTACHMENT_OPTIMAL); dynamic rendering is enabled. `scene.present_pipeline` +
        // its bind-group layout belong to this device and its declared color format equals the
        // swapchain's; `targets.present_set[fi]` binds `lit[fi]` (now SHADER_READ_ONLY_OPTIMAL —
        // the SAME slot `vb_resolve` just wrote), OR `aa_out[fi]` when AA is armed (the AA pass
        // above left it SHADER_READ_ONLY_OPTIMAL) + sampler at set 0; `blit_viewport`/
        // `blit_scissor` outlive the bracketed calls; `draw(3, 1, 0, 0)` is the `SV_VertexID`
        // fullscreen triangle. Begin/End bracket the pass exactly.
        unsafe {
            (self.fns.cmd_begin_rendering)(cmd, &present_rendering);
            (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, scene.present_pipeline.pipeline);
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                scene.present_pipeline.layout,
                0,
                1,
                &targets.present_set[fi].descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_set_viewport)(cmd, 0, 1, &blit_viewport);
            (self.fns.cmd_set_scissor)(cmd, 0, 1, &blit_scissor);
            (self.fns.cmd_draw)(cmd, 3, 1, 0, 0);
            (self.fns.cmd_end_rendering)(cmd);
        }

        match readback {
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
                // SAFETY: recording is open; COLOR_ATTACHMENT_OUTPUT→BOTTOM_OF_PIPE with
                // COLOR→PRESENT makes the blit's writes visible to the present engine;
                // `&to_present` outlives the call.
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
                // COLOR→TRANSFER_SRC makes the blit's writes available to the copy;
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
                    image_extent: VkExtent3D { width: extent.width, height: extent.height, depth: 1 },
                };
                // SAFETY: recording is open; the swapchain image is TRANSFER_SRC_OPTIMAL per the
                // barrier above; one full-image tightly-packed color region copies into the live
                // host-visible `staging.buffer` (≥ the image's byte size per this fn's contract);
                // `&region` outlives the call.
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
                // SAFETY: recording is open; TRANSFER→BOTTOM_OF_PIPE with TRANSFER_SRC→PRESENT
                // releases the image to the present engine after the readback copy; `&to_present`
                // outlives the call.
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
