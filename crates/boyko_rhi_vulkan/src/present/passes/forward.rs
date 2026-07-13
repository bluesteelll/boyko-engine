//! `Renderer::record_forward`: the on-screen Forward v1 mesh raster + inline-shade record body
//! behind [`Renderer::render_gbuffer_frame`] — the `Forward` sibling of
//! [`record_gbuffer`](super::gbuffer), driven through
//! [`ForwardBarrierSink`](super::super::graph_bridge::ForwardBarrierSink) via
//! [`Renderer::record_forward_pass`].
//!
//! # v1 SCOPE CUT (mirrors `forward_opaque.fs.hlsl`'s own doc — R4b-a)
//!
//! Mesh-only (the resolver collapses `Forward × {Both, Sdf}` to `Mesh` pre-R-SDFFWD), all-lights
//! (no froxel), NO SSAO/DDGI/shadow-denoise/motion vector/TAA — `cap_forward_v1_consumers`
//! (`boyko_render::render_path_config`) forces every one of those consumers off structurally, so
//! this recorder has no prepass, no thin-aux MRT, and writes no motion. Shadows (CSM + punctual
//! atlas) ARE in scope — `forward_opaque.fs.hlsl` samples them inline via `shadow_apply.hlsli`.
//!
//! # Why a SEPARATE record body, not a `record_gbuffer` branch
//!
//! Reuses `record_gbuffer`'s exact `interp`/`csm`/`atlas`/`light_upload`/present-blit LOGIC
//! (duplicated here, not extracted into a shared helper — see
//! [`super::super::graph_bridge::ForwardPassPlan`]'s doc for the "own private ResId space, zero
//! edits to Deferred's reachable code" trade-off this rung made; extracting a shared draw-body
//! helper out of `record_gbuffer` was rejected as a higher-risk touch to the orchestrator's
//! golden-gated file for this v1 delivery). Every duplicated block is a byte-for-byte port of
//! its `record_gbuffer` counterpart (same Vulkan calls, same order), adapted only for the
//! Forward-private barrier sink (`record_forward_pass` vs `record_graph_pass`) and targets.

use core::ptr;

use crate::ffi::*;
use crate::memory::BoundBuffer;
use crate::texture::{MAX_CASCADES, MAX_TEXTURE_LAYERS};

use super::super::frame_driver::Renderer;
use super::super::scene_types::{GBUFFER_PUSH_BASE_INSTANCE_OFFSET, GBufferScene};
use super::super::targets::{ForwardTargets, GBufferTargets};
use super::super::{COLOR_SUBRESOURCE_RANGE, SwapchainError};

/// The Forward `lit` clear color — byte-identical to `record_gbuffer`'s albedo clear
/// (`gbuffer.rs`: `[0.05, 0.05, 0.1, 1.0]`, the marcher's `BACKGROUND` base,
/// `sdf_gbuffer_composite.hlsl`). Forward v1 has no SDF leg (mesh-only), so this is purely a
/// visual-parity choice — a mesh-only Deferred frame's uncovered pixels show the SAME
/// background this constant reproduces.
const FORWARD_LIT_CLEAR: [f32; 4] = [0.05, 0.05, 0.1, 1.0];

/// The Forward reverse-Z depth CLEAR (Decision 4): `0.0` is the "nothing drawn yet" sentinel —
/// farther than any real `depth ∈ (0, 1]` under `VK_COMPARE_OP_GREATER` (nearer fragment has
/// the LARGER stored depth). Paired with [`VulkanContext::create_graphics_pipeline_forward`]'s
/// `VK_COMPARE_OP_GREATER` pipeline state (`boyko_render::view::forward_view_proj_rows`'s doc).
const FORWARD_DEPTH_CLEAR: f32 = 0.0;

impl Renderer<'_> {
    /// Records the Forward v1 on-screen frame: `interp? → light_upload? → csm? → atlas? →
    /// forward_opaque → present-blit` — EXACTLY [`Renderer::declare_forward_graph`]'s declaration
    /// order (code-review P1-2: `compile()` derives barriers in declaration order, so this
    /// recorder's order must match it pass-for-pass, not merely "light_upload somewhere before
    /// forward_opaque"). Mirrors the Deferred recorder's own placement (`light_upload` right
    /// after the geometry pass, before `csm`/`atlas`).
    ///
    /// # Safety
    ///
    /// `cmd` must be recordable (waited free); `image`/`view` must belong to the swapchain image
    /// presented this frame; `scene`'s pipelines/buffers/samplers are live on this device;
    /// `targets`/`forward` were synced to `present_extent` (the SAME contract
    /// [`record_gbuffer`](super::gbuffer)'s doc states, restricted to Forward's own images/sets).
    /// `extent` is the swapchain extent and governs ONLY the present-blit's clear render-area and
    /// the readback region; a `Some(readback)` buffer is host-visible and ≥ the swapchain image's
    /// (`extent`-sized) byte size.
    #[allow(clippy::too_many_arguments)]
    pub(crate) unsafe fn record_forward(
        &self,
        cmd: VkCommandBuffer,
        image: VkImage,
        view: VkImageView,
        extent: VkExtent2D,
        present_extent: VkExtent2D,
        clear: [f32; 4],
        scene: &GBufferScene<'_>,
        targets: &GBufferTargets,
        forward: &ForwardTargets,
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

        // The lock-free cross-frame ring index — see `record_gbuffer`'s doc (the SAME
        // per-in-flight-frame ringing discipline: `forward.depth[fi]`/`forward.set0[fi]`/
        // `forward.set1[fi]`/`targets.lit[fi]`).
        let fi = self.frame_index;

        let plan = self
            .forward_pass_plan
            .as_ref()
            .expect("invariant: declare_frame_graph ran before record_forward");

        // === Pillar B B3: the per-instance TRS interpolation compute PRE-PASS — byte-for-byte
        // port of `record_gbuffer`'s own interp block (see this module's doc), adapted to the
        // Forward-private barrier sink. Recorded ONLY when `scene.interp.is_some()`. ===
        if let Some(interp) = &scene.interp
            && interp.instance_count > 0
        {
            let interp_pass =
                plan.interp.expect("invariant: scene.interp.is_some() ⇒ interp pass declared");
            // SAFETY: recording is open; `record_forward_pass` records the graph's derived input
            // barriers (currently none — the pair read is a frame-private first touch) for the
            // "interp" pass into `cmd`.
            self.record_forward_pass(interp_pass, cmd, targets, forward, scene, fi);
            let groups = interp.instance_count.div_ceil(crate::compute::LOCAL_SIZE_X);
            let mut push = [0u8; crate::compute::INTERP_INSTANCES_PUSH_BYTES as usize];
            push[0..4].copy_from_slice(&interp.instance_count.to_le_bytes());
            push[4..8].copy_from_slice(&interp.alpha.to_le_bytes());
            // SAFETY: recording is open; the interp pipeline + its layout (the 3-binding interp
            // set at set 0 + the 8-byte COMPUTE push range) are live on this device (caller
            // contract); `interp.interp_set` binds this frame slot's pair/out-slot SSBOs +
            // the SHARED model-out ring (the SAME `instance_rings[fi]`
            // `scene.forward_instance_ring[fi]` also references); `groups` covers the dynamic
            // `instance_count`; `&interp.interp_set.descriptor_set` is a single-element local
            // alive for the call; the push is exactly `INTERP_INSTANCES_PUSH_BYTES` at offset 0.
            unsafe {
                (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, interp.pipeline.pipeline);
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    interp.pipeline.layout,
                    0,
                    1,
                    &interp.interp_set.descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_push_constants)(
                    cmd,
                    interp.pipeline.layout,
                    VK_SHADER_STAGE_COMPUTE_BIT,
                    0,
                    crate::compute::INTERP_INSTANCES_PUSH_BYTES,
                    push.as_ptr().cast(),
                );
                (self.fns.cmd_dispatch)(cmd, groups, 1, 1);
            }
            // The model-out WRITE is ordered before `forward_opaque`'s VS READ by the graph
            // (the COMPUTE→VERTEX RAW barrier derived at `forward_opaque`, the reader) — NOT here.
        }

        // === Lighting L0-r0: ASYNC light-table re-upload — byte-for-byte port of
        // `record_gbuffer`'s own `light_upload` block. Recorded ONLY on a dirty frame.
        // Code-review P1-2: recorded HERE (right after `interp`, before `csm`/`atlas`) to match
        // `declare_forward_graph`'s DECLARATION order exactly (`compile()` derives barriers in
        // declaration order — a pass recorded out of that order still emits a materially correct
        // stream here since `light_table` is disjoint from `cascade`/`atlas`, but keeping
        // declare/record order identical is the load-bearing invariant this fix restores, the
        // SAME position the Deferred recorder uses: `light_upload` right after the geometry
        // pass, before `csm`/`atlas`). ===
        if scene.light_dirty && scene.light_upload_bytes > 0 {
            let light_upload =
                plan.light_upload.expect("invariant: light_dirty ⇒ light_upload pass declared");
            // SAFETY: recording is open; `record_forward_pass` records the graph's derived
            // cross-frame seed-WAR buffer barrier for the "light_upload" pass into `cmd`, ahead
            // of the copy it guards.
            self.record_forward_pass(light_upload, cmd, targets, forward, scene, fi);
            let region =
                VkBufferCopy { src_offset: 0, dst_offset: 0, size: scene.light_upload_bytes };
            // SAFETY: recording is open; the copy names the live host-coherent staging +
            // device-local table buffers; the copy region spans `[0, light_upload_bytes)` ≤ both
            // buffer sizes (caller contract). `&region` outlives the call.
            unsafe {
                (self.fns.cmd_copy_buffer)(
                    cmd,
                    scene.light_staging.buffer,
                    scene.light_table.buffer,
                    1,
                    &region,
                );
            }
        }

        let vertex_offset: VkDeviceSize = 0;

        // === CSM cascade DEPTH pass — byte-for-byte port of `record_gbuffer`'s own `csm` block
        // (see this module's doc). Recorded ONLY when `scene.csm.is_some()`; runs BEFORE
        // `forward_opaque` (which samples the cascade inline), unlike Deferred's placement
        // relative to its shading pass (the resolve) — see `declare_forward_graph`'s doc for the
        // dependency-order rationale. ===
        if let Some(csm) = &scene.csm {
            let cascade = scene.csm_cascade_texture;
            let active = (csm.active_count as usize).clamp(1, MAX_CASCADES) as u32;
            let csm_pass = plan.csm.expect("invariant: scene.csm.is_some() ⇒ csm pass declared");
            // SAFETY: recording is open; `record_forward_pass` records the graph's derived
            // UNDEFINED→DEPTH barrier-in for the "csm_depth" pass into `cmd`.
            self.record_forward_pass(csm_pass, cmd, targets, forward, scene, fi);

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
                // depth-only (no color attachment); the depth-only pipeline + the SAME instance
                // SSBO (`scene.instance_bind_group`) satisfy the depth VS's static `instances`
                // reference; the 88-byte push carries cascade `c`'s `view_proj` + `use_model_matrix
                // == 1`; per caster batch the recorder re-pushes `base_instance` then
                // `draw_indexed` reads that batch's bound vertex+index buffers. Begin/End bracket
                // each cascade — byte-identical to `record_gbuffer`'s own csm loop.
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
            // The graph derives the dual-use depth barrier-out (→SHADER_READ_ONLY_OPTIMAL) at
            // `forward_opaque` (the cascade reader) — NOT here.
        }

        // === Punctual (spot/point) shadow-atlas DEPTH pass — byte-for-byte port of
        // `record_gbuffer`'s own `atlas` block. Recorded ONLY when `scene.atlas_punctual.is_some()`.
        // ===
        if let Some(atlas_act) = &scene.atlas_punctual {
            let atlas = scene.shadow_atlas_texture;
            let active = (atlas_act.active_layers as usize).clamp(1, MAX_TEXTURE_LAYERS) as u32;
            let atlas_pass =
                plan.atlas.expect("invariant: scene.atlas_punctual.is_some() ⇒ atlas pass declared");
            // SAFETY: recording is open; `record_forward_pass` records the graph's derived
            // UNDEFINED→DEPTH barrier-in for the "atlas_depth" pass into `cmd`.
            self.record_forward_pass(atlas_pass, cmd, targets, forward, scene, fi);

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
                // as the SAME instance SSBO set 0; the 88-byte push carries slot `s`'s `view_proj`
                // (+ the POINT `cam_eye` lane) + `use_model_matrix == 1`; per caster batch the
                // recorder re-pushes `base_instance` then `draw_indexed` reads that batch's bound
                // vertex+index buffers. The pipeline/set are (re)bound only when the face TYPE
                // changes. Begin/End bracket each slot — byte-identical to `record_gbuffer`'s own
                // atlas loop.
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
            // The graph derives the dual-use depth barrier-out at `forward_opaque` — NOT here.
        }

        // === Pass `forward_opaque`: the mesh raster + inline-shade pass. Writes `lit` (COLOR,
        // Decision 2's C5 per-path producer access) + `forward_depth` (HW reverse-Z, Decision 4);
        // no `SV_Depth`/`discard`/UAV in `forward_opaque.fs.hlsl` ⇒ early-Z stays live. ===
        // SAFETY: recording is open; `record_forward_pass` records the graph's derived
        // UNDEFINED→COLOR_ATTACHMENT_OPTIMAL (`lit`) + UNDEFINED→DEPTH_ATTACHMENT_OPTIMAL
        // (`forward_depth`) barriers-in, plus the cascade/atlas →SHADER_READ_ONLY_OPTIMAL
        // barriers-out (this pass is their declared FRAGMENT reader) for the "forward_opaque"
        // pass into `cmd`.
        self.record_forward_pass(plan.forward_opaque, cmd, targets, forward, scene, fi);

        // `scene.path_is_forward()` gated this call site (`Renderer::render_gbuffer_frame`'s
        // dispatch) — production ALWAYS threads `Some(...)` for a `Forward`-resolved scene
        // (`GBufferScene::forward_pipeline`'s doc: built unconditionally at boot).
        let forward_pipeline = scene
            .forward_pipeline
            .expect("invariant: a Forward-resolved scene always carries forward_pipeline");
        // Code-review follow-up (rung R4b-b): the sky background pipeline — same production
        // invariant as `forward_pipeline` above.
        let forward_sky_pipeline = scene
            .forward_sky_pipeline
            .expect("invariant: a Forward-resolved scene always carries forward_sky_pipeline");

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
            clear_value: VkClearValue { color: VkClearColorValue { float32: FORWARD_LIT_CLEAR } },
        };
        let forward_depth_attachment = VkRenderingAttachmentInfo {
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
                depth_stencil: VkClearDepthStencilValue { depth: FORWARD_DEPTH_CLEAR, stencil: 0 },
            },
        };
        let forward_area = VkRect2D { offset: VkOffset2D { x: 0, y: 0 }, extent: present_extent };
        let forward_rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: forward_area,
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: 1,
            p_color_attachments: &lit_attachment,
            p_depth_attachment: (&forward_depth_attachment as *const VkRenderingAttachmentInfo).cast(),
            p_stencil_attachment: ptr::null(),
        };
        let forward_viewport = VkViewport {
            x: 0.0,
            y: 0.0,
            width: present_extent.width as f32,
            height: present_extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        // SAFETY: recording is open; `forward_rendering` names the live `lit` view (now
        // COLOR_ATTACHMENT_OPTIMAL) + the live `forward_depth` view (now
        // DEPTH_ATTACHMENT_OPTIMAL); `lit_attachment`/`forward_depth_attachment` outlive the
        // bracketed calls; dynamic rendering is enabled on this device.
        //
        // `forward_sky_pipeline` (declaring ONE color format + `depth_format: None` — no depth
        // attachment at all, which Vulkan permits inside a rendering scope that DOES bind one;
        // the SAME `forward_layout0` set-0 layout, no push constants) draws FIRST: bound-but-
        // unread past its own 2-binding FS subset, `forward.set0[fi]` is a live descriptor set
        // written once per extent; `draw(3, 1, 0, 0)` is the `SV_VertexID` fullscreen triangle
        // (no vertex buffer).
        //
        // `forward_pipeline` (declaring ONE color format + `Format::D32Sfloat` +
        // `VK_COMPARE_OP_GREATER` + the plain 2-set `[Set0, Set1]` layout, built by
        // `VulkanContext::create_graphics_pipeline_forward` — boot-panic fix: no placeholder set,
        // `forward_opaque.fs.hlsl`'s shadow bindings live at Set 1, not Set 2) + the 88-byte
        // VERTEX push range belong to this device (caller contract); `forward.set0[fi]` (bound
        // at set 0) and `forward.set1[fi]` (bound at set 1) are live descriptor sets written once
        // per extent against `scene.forward_layout0`/`scene.forward_layout1`. `scene.mvp` is the
        // SAME 88-byte push [`Renderer::record_gbuffer`]'s raster pass reads, host-assembled with
        // `boyko_render::view::forward_view_proj_rows` instead of `gbuffer_push_from_view` on a
        // `Forward`-resolved boot (the two paths are boot-mutually-exclusive, so reusing the
        // field is sound). `vertex_offset`/`forward_viewport`/`forward_area` outlive the
        // bracketed calls; the legacy arm's `draw` reads the merged vertex buffer, the instanced
        // arm's per-batch `draw_indexed` reads each batch's bound vertex+index buffers (created
        // on this device, VERTEX/INDEX usage). Begin/End bracket the WHOLE pass (both pipelines)
        // exactly.
        unsafe {
            (self.fns.cmd_begin_rendering)(cmd, &forward_rendering);
            // Code-review follow-up (rung R4b-b): draw the sky BACKGROUND first, inside this
            // SAME `begin_rendering` scope — `forward_sky_pipeline` declares NO depth attachment
            // (`depth_format: None` at boot), so it neither tests nor writes `forward_depth`;
            // the opaque mesh loop below (its OWN real `VK_COMPARE_OP_GREATER` depth test/write)
            // then draws over exactly the pixels it covers, leaving the sky's color everywhere
            // else. `forward.set0[fi]` is REUSED (the sky FS reads only Camera @2 + LightBuf @3,
            // a subset of that layout's 5 bindings — `GBufferScene::forward_sky_pipeline`'s doc).
            // `forward_viewport`/`forward_area` (set once here) stay the active dynamic state for
            // the `forward_pipeline` draws below too (both pipelines declare the SAME
            // `VK_DYNAMIC_STATE_VIEWPORT`/`SCISSOR` pair, and Vulkan dynamic state persists
            // across a pipeline bind within one command buffer until next overwritten).
            (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, forward_sky_pipeline.pipeline);
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                forward_sky_pipeline.layout,
                0,
                1,
                &forward.set0[fi].descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_set_viewport)(cmd, 0, 1, &forward_viewport);
            (self.fns.cmd_set_scissor)(cmd, 0, 1, &forward_area);
            (self.fns.cmd_draw)(cmd, 3, 1, 0, 0);

            (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, forward_pipeline.pipeline);
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                forward_pipeline.layout,
                0,
                1,
                &forward.set0[fi].descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                forward_pipeline.layout,
                1,
                1,
                &forward.set1[fi].descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_push_constants)(
                cmd,
                forward_pipeline.layout,
                VK_SHADER_STAGE_VERTEX_BIT,
                0,
                scene.mvp.len() as u32,
                scene.mvp.as_ptr().cast(),
            );
            (self.fns.cmd_set_viewport)(cmd, 0, 1, &forward_viewport);
            (self.fns.cmd_set_scissor)(cmd, 0, 1, &forward_area);
            if scene.mesh_draw.is_empty() {
                (self.fns.cmd_bind_vertex_buffers)(cmd, 0, 1, &scene.vertex_buffer.buffer, &vertex_offset);
                (self.fns.cmd_draw)(cmd, scene.vertex_count, 1, 0, 0);
            } else {
                for batch in scene.mesh_draw {
                    let base = batch.base_instance;
                    (self.fns.cmd_push_constants)(
                        cmd,
                        forward_pipeline.layout,
                        VK_SHADER_STAGE_VERTEX_BIT,
                        GBUFFER_PUSH_BASE_INSTANCE_OFFSET,
                        4,
                        (&base as *const u32).cast(),
                    );
                    (self.fns.cmd_bind_vertex_buffers)(cmd, 0, 1, &batch.vertex_buffer.buffer, &vertex_offset);
                    (self.fns.cmd_bind_index_buffer)(cmd, batch.index_buffer.buffer, 0, batch.index_type);
                    (self.fns.cmd_draw_indexed)(cmd, batch.index_count, batch.instance_count, 0, 0, 0);
                }
            }
            (self.fns.cmd_end_rendering)(cmd);
        }

        // === Present-blit `lit` into the swapchain — byte-for-byte port of `record_gbuffer`'s
        // own Pass C (see this module's doc): the `present_sample` barrier (COLOR_ATTACHMENT_
        // OPTIMAL → SHADER_READ_ONLY_OPTIMAL, C5-derived from `forward_opaque`'s producer access),
        // then the swapchain blit, then the readback/present tail. ===
        // SAFETY: recording is open; `record_forward_pass` records the graph's derived
        // COLOR_ATTACHMENT_OPTIMAL→SHADER_READ_ONLY_OPTIMAL barrier for the "present_sample" pass
        // into `cmd`.
        self.record_forward_pass(plan.present_sample, cmd, targets, forward, scene, fi);

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
        // the SAME slot `forward_opaque` just wrote) + sampler at set 0; `blit_viewport`/
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
