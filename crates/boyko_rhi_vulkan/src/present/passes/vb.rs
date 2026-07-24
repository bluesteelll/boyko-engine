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

#[cfg(feature = "hwrt")]
use crate::compute::LOCAL_SIZE_X;
use crate::ffi::*;
use crate::memory::BoundBuffer;
use crate::texture::{MAX_CASCADES, MAX_TEXTURE_LAYERS};

use super::super::frame_driver::Renderer;
use super::super::gpu_timing::VbTimedPass;
use super::super::scene_types::{
    CLUSTER_CULL_PUSH_BYTES, GBUFFER_PUSH_BASE_INSTANCE_OFFSET, GBufferScene, LIGHT_CULL_LOCAL_SIZE_X,
};
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
    /// (`extent`-sized) byte size. Rung R9d: `accel_fns` is the AS command table for the split's
    /// own per-frame TLAS build (`Some` only on an RT device) — the SAME contract
    /// [`record_gbuffer`](super::gbuffer)'s own `accel_fns` param documents.
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
        #[cfg(feature = "hwrt")] accel_fns: Option<&crate::accel::AccelFns>,
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

        // VB-P1d: reset ALL `2 * VB_PASS_COUNT` bench queries at the frame top — OUTSIDE any
        // render / dynamic-rendering scope (recording is open but no `begin_rendering` has run
        // yet), before the frame's first `write_timestamp`. GATED on `scene.vb_gpu_timing`:
        // `None` (every golden/host/interactive frame) records NOTHING, so the command stream
        // is byte-identical. A TIMESTAMP query is undefined until reset.
        if let Some(tc) = scene.vb_gpu_timing {
            // SAFETY: recording is open; `self.fns` is the live device fn-table; the reset is
            // recorded before any `begin_rendering` (outside a render pass, per
            // `VUID-vkCmdResetQueryPool-renderpass`); `fi` is this present's in-flight slot.
            unsafe { tc.reset_frame(self.fns, cmd, fi) };
        }

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

        // VB-P1d: open the LightCull bracket BEFORE the cull's `if let` gate, so it is written
        // EVERY bench-armed frame REGARDLESS of whether the froxel arm itself is boot-built
        // (`scene.cluster_cull.is_none()` on a flat-leg bench boot ⇒ a near-zero-width bracket
        // with no GPU work between begin/end) — the `VK_QUERY_RESULT_WAIT_BIT` readback
        // (`GpuSceneBundles::read_vb_bench_ns`) never blocks on an unwritten query this way,
        // whichever leg (flat vs froxel) this boot resolved. GATED — `None` records nothing.
        if let Some(tc) = scene.vb_gpu_timing {
            // SAFETY: recording is open; `self.fns` is the live device fn-table; the pool was
            // reset at the frame top; this write is outside any rendering scope; `fi` is this
            // present's in-flight slot.
            unsafe { tc.write_begin(self.fns, cmd, fi, VbTimedPass::LightCull) };
        }
        // === VB-P1a ("dark infra"): the L1 clustered froxel light-cull pass — byte-for-byte
        // port of `record_forward`'s own `light_cull` block. Recorded ONLY when
        // `scene.cluster_cull.is_some()` (hardcoded OFF this rung, so this block NEVER records
        // in production) AND the scene wires the cull set (the SAME "4-buffers-Some" gate
        // `declare_vb_graph` uses). ===
        if let (Some(cull_pipeline), Some(cull_set), Some(_grid), Some(_index), Some(alloc)) = (
            scene.cluster_cull,
            targets.cull_set.as_ref().map(|s| &s[fi]),
            scene.cluster_grid,
            scene.light_index,
            scene.light_index_alloc,
        ) {
            // (L1-0) Reset the global slice-allocation counter to 0 (a transfer fill), then
            // order the fill before the cull's atomic reads/writes (TRANSFER→COMPUTE). See
            // `record_forward`'s own L1-0 comment for the full rationale.
            // SAFETY: recording is open; `alloc` is a live device-local STORAGE buffer (≥ 4 B,
            // the single u32 counter); `cmd_fill_buffer` zero-fills it (Vulkan 1.0 core). The
            // FILL is GPU work (not a barrier), so it runs unconditionally — only the following
            // barrier is graph-driven.
            unsafe {
                (self.fns.cmd_fill_buffer)(cmd, alloc.buffer, 0, VK_WHOLE_SIZE, 0);
            }
            let light_cull = plan
                .light_cull
                .expect("invariant: cull wired ⇒ light_cull pass declared");
            // SAFETY: recording is open; `record_vb_pass` records the graph's derived
            // TRANSFER→COMPUTE barrier for the "light_cull" pass into `cmd`, ordering the fill's
            // TRANSFER write before the cull's COMPUTE atomics on the GPU timeline.
            self.record_vb_pass(light_cull, cmd, targets, forward, vb, scene, fi);

            // (L1-1) Bind the cull pipeline + the cull set (written ONCE at sync_gbuffer), push
            // the 16-byte ClusterCullPush, dispatch over CLUSTER_COUNT froxels.
            let cull_groups = scene.cluster_count.div_ceil(LIGHT_CULL_LOCAL_SIZE_X);
            // SAFETY: recording is open; the cull pipeline + its layout (declaring `cull_layout`
            // at set 0 + the 16-byte COMPUTE push range) are live on this device (caller
            // contract); the cull set binds the camera UBO + light table + the cluster buffers;
            // `cull_groups` covers `cluster_count` froxels at the 64-wide group; the push bytes
            // are exactly `CLUSTER_CULL_PUSH_BYTES` (16) at offset 0; `&cull_set.descriptor_set`
            // is a single-element local alive for the call (first_set 0, count 1).
            unsafe {
                (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, cull_pipeline.pipeline);
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    cull_pipeline.layout,
                    0,
                    1,
                    &cull_set.descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_push_constants)(
                    cmd,
                    cull_pipeline.layout,
                    VK_SHADER_STAGE_COMPUTE_BIT,
                    0,
                    CLUSTER_CULL_PUSH_BYTES,
                    scene.cluster_cull_push.as_ptr().cast(),
                );
                (self.fns.cmd_dispatch)(cmd, cull_groups, 1, 1);
            }
            // (L1-2) The cull's ClusterGrid + LightIndexList writes are made available + visible
            // to `vb_resolve`/`vb_shade`'s reads by the graph: derived at the reader — NOT here.
        }
        // VB-P1d: close the LightCull bracket. GATED.
        if let Some(tc) = scene.vb_gpu_timing {
            // SAFETY: recording is open; the pool was reset this frame; `fi` is this slot.
            unsafe { tc.write_end(self.fns, cmd, fi, VbTimedPass::LightCull) };
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
                        VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
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
                            VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
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
                            VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
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
                    VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
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
                        VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
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
            if scene.vb_use_classified && scene.path_vb_fused() {
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

            // === The `lit`-producer choice (plan P1-4) is THREE-way, not two: the split
            // `vb_shade_split` (below, when `scene.path_vb_split()`) DISPLACES both of the
            // other two — `vb_shade` (material-classified, when `scene.vb_use_classified`) and
            // the fused `vb_resolve` are mutually exclusive WITH EACH OTHER (exactly one of
            // THIS PAIR runs whenever `!scene.path_vb_split()`), but NEITHER runs on a split
            // frame — mirrors `declare_vb_graph`'s matching branch.
            //
            // VB-P1d: this three-way split is why the bench's `VbTimedPass::VbShade` bracket
            // is recorded in ONLY the classified/fused arms (below), never the split arm — the
            // bench is fused/classified-only by design (`boyko_app::runner`'s VB-P1d block
            // asserts `!scene.resolved_render_path.mesh_geo_shade_split` before ever reading a
            // bench pool, precisely because a split frame would reset-but-never-write the
            // VbShade pair and hang the `VK_QUERY_RESULT_WAIT_BIT` readback). ===
            if scene.path_vb_split() {
                // Rung R9b: the split DISPLACES the fused lit producer — `vb_shade_split`
                // (recorded in the split arm after this block) is the sole lit producer;
                // neither `vb_shade` nor `vb_resolve` records (mirrors the declarator).
            } else if scene.vb_use_classified {
                // VB-P1d: open the VbShade bracket — this branch is the classified lit
                // producer, mutually exclusive with the fused `vb_resolve` branch below (exactly
                // one of the two ever records per frame), so the SAME `VbTimedPass::VbShade`
                // pair is written by whichever runs. GATED.
                if let Some(tc) = scene.vb_gpu_timing {
                    // SAFETY: recording is open; `self.fns` is the live device fn-table; the
                    // pool was reset at the frame top; `fi` is this present's in-flight slot.
                    unsafe { tc.write_begin(self.fns, cmd, fi, VbTimedPass::VbShade) };
                }
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
                //
                // VB-P1c closes the VB-P1a scope cut: `textured`/`froxel` are INDEPENDENT axes
                // (`vb_tex_active` never reads `cluster_cull`), so all four combinations select
                // their own pipeline + Set-0 — `vb_set0_tex_froxel` (the TEXTURED+FROXEL combined
                // Set-0) exists exactly for the `(true, true)` cell. Inert today: the arm bit is
                // hardcoded OFF (`ResolvedRenderPath::froxel_light_cull`'s doc), so
                // `scene.cluster_cull` is ALWAYS `None` on every current boot, textured or not —
                // this frame's `(true, false)`/`(false, false)` cells are the only ones reachable
                // in production.
                let textured = scene.vb_tex_active();
                let froxel = scene.cluster_cull.is_some();
                let (vb_shade_pipeline, vb_shade_set0) = match (textured, froxel) {
                    (true, true) => (
                        scene.vb_shade_tex_froxel_pipeline.expect(
                            "invariant: vb_tex_active() && cluster_cull.is_some() => scene.vb_shade_tex_froxel_pipeline is Some",
                        ),
                        targets.vb_set0_tex_froxel.as_ref().expect(
                            "invariant: vb_tex_active() && cluster_cull.is_some() => targets.vb_set0_tex_froxel is built",
                        ),
                    ),
                    (true, false) => (
                        scene.vb_shade_tex_pipeline.expect(
                            "invariant: GBufferScene::vb_tex_active() => scene.vb_shade_tex_pipeline is Some",
                        ),
                        targets.vb_set0_tex.as_ref().expect(
                            "invariant: GBufferScene::vb_tex_active() => targets.vb_set0_tex is built",
                        ),
                    ),
                    (false, true) => (
                        scene.vb_shade_froxel_pipeline.expect(
                            "invariant: scene.cluster_cull.is_some() => scene.vb_shade_froxel_pipeline is Some",
                        ),
                        targets.vb_set0_froxel.as_ref().expect(
                            "invariant: scene.cluster_cull.is_some() => targets.vb_set0_froxel is built",
                        ),
                    ),
                    (false, false) => (
                        scene.vb_shade_pipeline.expect(
                            "invariant: a VisibilityBuffer-resolved scene always carries vb_shade_pipeline",
                        ),
                        vb_set0,
                    ),
                };
                let vb_geometry_set = scene
                    .vb_geometry_set
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_geometry_set");
                // SAFETY: recording is open; `vb_shade_pipeline` (the compute pipeline: Set 0 =
                // `vb_shade_set0[fi]` (`vb_set0[fi]`, or `vb_set0_tex[fi]`/`vb_set0_froxel[fi]`/
                // `vb_set0_tex_froxel[fi]` per the `(textured, froxel)` match above), Set 1 =
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
                // VB-P1d: close the VbShade bracket. GATED.
                if let Some(tc) = scene.vb_gpu_timing {
                    // SAFETY: recording is open; the pool was reset this frame; `fi` is this slot.
                    unsafe { tc.write_end(self.fns, cmd, fi, VbTimedPass::VbShade) };
                }
            } else {
                // VB-P1d: open the VbShade bracket — the fused lit producer, mutually exclusive
                // with the classified branch above (see that branch's own VB-P1d comment). GATED.
                if let Some(tc) = scene.vb_gpu_timing {
                    // SAFETY: recording is open; `self.fns` is the live device fn-table; the
                    // pool was reset at the frame top; `fi` is this present's in-flight slot.
                    unsafe { tc.write_begin(self.fns, cmd, fi, VbTimedPass::VbShade) };
                }
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

                // VB-P1a ("dark infra"): select the FROXEL-variant pipeline + its OWN WIDER
                // Set-0 (`vb_set0_froxel`, 10 bindings) when the arm is built
                // (`scene.cluster_cull.is_some()` — hardcoded OFF this rung, so this is ALWAYS
                // the base arm in production today), else the base `vb_resolve_pipeline` +
                // `vb_set0` — mutually exclusive by construction (mirrors `vb_shade`'s own
                // `textured` selector immediately above).
                let froxel = scene.cluster_cull.is_some();
                let (vb_resolve_pipeline, vb_resolve_set0) = if froxel {
                    (
                        scene.vb_resolve_froxel_pipeline.expect(
                            "invariant: scene.cluster_cull.is_some() => scene.vb_resolve_froxel_pipeline is Some",
                        ),
                        targets.vb_set0_froxel.as_ref().expect(
                            "invariant: scene.cluster_cull.is_some() => targets.vb_set0_froxel is built",
                        ),
                    )
                } else {
                    (
                        scene.vb_resolve_pipeline.expect(
                            "invariant: a VisibilityBuffer-resolved scene always carries vb_resolve_pipeline",
                        ),
                        vb_set0,
                    )
                };
                let vb_geometry_set = scene
                    .vb_geometry_set
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_geometry_set");
                // SAFETY: recording is open; `vb_resolve_pipeline` (the 3-set compute pipeline: Set 0 =
                // `vb_resolve_set0[fi]` (`vb_set0[fi]` or, when `froxel`, `vb_set0_froxel[fi]`), Set 1 =
                // `forward.set1[fi]` — the Forward-family shadow set REUSED VERBATIM, Set 2 =
                // `vb_geometry_set` — the Decision-0 geometry table, bound directly,
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
                        &vb_resolve_set0[fi].descriptor_set,
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
                // VB-P1d: close the VbShade bracket. GATED.
                if let Some(tc) = scene.vb_gpu_timing {
                    // SAFETY: recording is open; the pool was reset this frame; `fi` is this slot.
                    unsafe { tc.write_end(self.fns, cmd, fi, VbTimedPass::VbShade) };
                }
            }
        }

        // === Rung R9b: the SPLIT arm — recorded in EXACTLY `declare_vb_graph`'s order:
        // vb_viewt(pre-tail, iff ssao armed) → vb_geo → ssao gather → à-trous×N →
        // vb_shade_split. `sdf_forward_march` (below) then extends the split shade's `lit`
        // write under `Both`, exactly as it extends the fused producer's. ===
        if scene.path_vb_split() {
            // (1) The gViewT producer's PRE-TAIL slot (the gather's ray-metric depth source).
            if scene.ssao.is_some()
                && let Some(vb_viewt_pass) = plan.viewt_from_depth
            {
                self.record_vb_viewt_dispatch(vb_viewt_pass, cmd, targets, forward, vb, scene, present_extent, fi);
            }

            // (2) Pass `vb_geo` — the thin-aux producer: the first `vb_id` reader under split
            // (derives COLOR→SHADER_READ_ONLY), writes `thin_normal` (first-touch
            // UNDEFINED→GENERAL). SAFETY: recording is open; `record_vb_pass` records those
            // derived barriers for the "vb_geo" pass into `cmd`.
            let geo_pass = plan
                .vb_geo
                .expect("invariant: path_vb_split() => vb_geo pass declared (declare_vb_graph)");
            self.record_vb_pass(geo_pass, cmd, targets, forward, vb, scene, fi);
            // Rung R9d: select the `-D MOTION=1` sibling when the hwrt shadow chain's temporal
            // stage is armed this frame (`vb_geo_mv_active()` — the O1 single predicate
            // `declare_vb_graph`'s conditional `motion_vec` write also reads), else the base
            // pipeline (byte-identical to R9b).
            #[cfg(feature = "hwrt")]
            let vb_geo_pipeline = if scene.vb_geo_mv_active() {
                scene.vb_geo_mv_pipeline.expect(
                    "invariant: vb_geo_mv_active() ⇒ scene.vb_geo_mv_pipeline is Some (build_vb_split_pipelines ran on an RT device)",
                )
            } else {
                scene
                    .vb_geo_pipeline
                    .expect("invariant: a split-armed scene carries vb_geo_pipeline (build_vb_split_pipelines ran)")
            };
            #[cfg(not(feature = "hwrt"))]
            let vb_geo_pipeline = scene
                .vb_geo_pipeline
                .expect("invariant: a split-armed scene carries vb_geo_pipeline (build_vb_split_pipelines ran)");
            let vb_geometry_set = scene
                .vb_geometry_set
                .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_geometry_set");
            let vb_set0 = targets
                .vb_set0
                .as_ref()
                .expect("invariant: a VisibilityBuffer-resolved scene always builds vb_set0");
            let vb_geo_aux_set = targets
                .vb_geo_aux_set
                .as_ref()
                .expect("invariant: a split-armed boot builds vb_geo_aux_set (targets tail)");
            // SAFETY: recording is open; `vb_geo_pipeline` (3-set: Set 0 = `vb_set0[fi]` — the
            // BASE ring, `vb_geo` samples no textures; Set 1 = `vb_geo_aux_set[fi]`; Set 2 =
            // the geometry table) belongs to this device; the 64-byte push is `scene.mvp`'s
            // leading `view_proj` (the `vb_resolve` shape); `dispatch_group_count_x` covers the
            // pixel count at `numthreads(64,1,1)`.
            unsafe {
                (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, vb_geo_pipeline.pipeline);
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    vb_geo_pipeline.layout,
                    0,
                    1,
                    &vb_set0[fi].descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    vb_geo_pipeline.layout,
                    1,
                    1,
                    &vb_geo_aux_set[fi].descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    vb_geo_pipeline.layout,
                    2,
                    1,
                    &vb_geometry_set.set(),
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_push_constants)(
                    cmd,
                    vb_geo_pipeline.layout,
                    VK_SHADER_STAGE_COMPUTE_BIT,
                    0,
                    64,
                    scene.mvp.as_ptr().cast(),
                );
                (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
            }

            // (3) The SSAO gather + à-trous chain (`path_vb_ssao` — the declare-site predicate).
            if scene.path_vb_ssao() {
                let ssao_pass = plan
                    .vb_ssao
                    .expect("invariant: path_vb_ssao() => the VB ssao pass was declared");
                // SAFETY: recording is open; `record_vb_pass` records the thin_normal/viewt
                // store→load + the `ssao` seed-inert first-touch barriers for the "ssao" pass.
                self.record_vb_pass(ssao_pass, cmd, targets, forward, vb, scene, fi);
                let gather_pipeline = scene
                    .ssao_vb_pipeline
                    .expect("invariant: path_vb_ssao() => scene.ssao_vb_pipeline is Some");
                let vb_ssao_set = targets
                    .vb_ssao_set
                    .as_ref()
                    .expect("invariant: a split-armed boot builds vb_ssao_set (targets tail)");
                // SAFETY: recording is open; the VB_THIN gather reads its camera from the UBO
                // @3, so no push constant is recorded (the deferred gather's own discipline);
                // `dispatch_group_count_x` covers the pixel count.
                unsafe {
                    (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, gather_pipeline.pipeline);
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        gather_pipeline.layout,
                        0,
                        1,
                        &vb_ssao_set[fi].descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
                }

                // The à-trous chain — the deferred record loop MIRRORED (gbuffer.rs's own
                // block, incl. the graceful set-degrade + the 0-||-2..=MAX contract assert);
                // the role-keyed SETS are the SAME core-ring sets (core.viewt/ssao/rings are
                // path-shared), so no VB-specific set plumbing exists here.
                if let Some(activation) = &scene.ssao
                    && activation.atrous_levels > 0
                    && let (
                        Some(read8_set),
                        Some(interior_from0_set),
                        Some(interior_from1_set),
                        Some(write8_from0_set),
                        Some(write8_from1_set),
                    ) = (
                        targets.ssao_atrous_read8_set.as_ref(),
                        targets.ssao_atrous_interior_from0_set.as_ref(),
                        targets.ssao_atrous_interior_from1_set.as_ref(),
                        targets.ssao_atrous_write8_from0_set.as_ref(),
                        targets.ssao_atrous_write8_from1_set.as_ref(),
                    )
                {
                    let read8_pipeline = scene
                        .ssao_atrous_read8_pipeline
                        .expect("invariant: the SSAO à-trous sets built ⇒ the boot read8 pipeline is Some");
                    let interior_pipeline = scene.ssao_atrous_interior_pipeline.expect(
                        "invariant: the SSAO à-trous sets built ⇒ the boot interior pipeline is Some",
                    );
                    let write8_pipeline = scene
                        .ssao_atrous_write8_pipeline
                        .expect("invariant: the SSAO à-trous sets built ⇒ the boot write8 pipeline is Some");
                    let atrous_levels =
                        activation.atrous_levels.min(crate::present::MAX_SSAO_ATROUS_LEVELS);
                    debug_assert!(
                        atrous_levels == 0
                            || (2..=crate::present::MAX_SSAO_ATROUS_LEVELS).contains(&atrous_levels),
                        "invariant: ssao à-trous levels is 0 or 2..=MAX at the RHI boundary; got {atrous_levels}"
                    );
                    for level in 0..atrous_levels {
                        let atrous_pass = plan.ssao_atrous[level as usize].expect(
                            "invariant: level < ssao_atrous_levels ⇒ the VB ssao_atrous[level] declared",
                        );
                        // SAFETY: recording is open; the "ssao_atrous" pass's derived RAW
                        // barriers (gather-write→level-0-read, the ring ping-pong, the last
                        // level's write→shade-read) are recorded via the VB sink.
                        self.record_vb_pass(atrous_pass, cmd, targets, forward, vb, scene, fi);
                        let (pipeline, set) =
                            match crate::present::ssao_atrous_step(level, atrous_levels) {
                                crate::present::AtrousStepRole::Read8 => {
                                    (read8_pipeline, &read8_set[self.frame_index])
                                }
                                crate::present::AtrousStepRole::Interior { in_ring: 0 } => {
                                    (interior_pipeline, &interior_from0_set[self.frame_index])
                                }
                                crate::present::AtrousStepRole::Interior { .. } => {
                                    (interior_pipeline, &interior_from1_set[self.frame_index])
                                }
                                crate::present::AtrousStepRole::Write8 { in_ring: 0 } => {
                                    (write8_pipeline, &write8_from0_set[self.frame_index])
                                }
                                crate::present::AtrousStepRole::Write8 { .. } => {
                                    (write8_pipeline, &write8_from1_set[self.frame_index])
                                }
                            };
                        let step: u32 = 1u32 << level;
                        // SAFETY: recording is open; the selected variant + its shared 4-binding
                        // layout are live; the 4-byte `{ uint step }` push covers the declared
                        // COMPUTE range; `dispatch_group_count_x` covers the pixel count.
                        unsafe {
                            (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline.pipeline);
                            (self.fns.cmd_bind_descriptor_sets)(
                                cmd,
                                VK_PIPELINE_BIND_POINT_COMPUTE,
                                pipeline.layout,
                                0,
                                1,
                                &set.descriptor_set,
                                0,
                                ptr::null(),
                            );
                            (self.fns.cmd_push_constants)(
                                cmd,
                                pipeline.layout,
                                VK_SHADER_STAGE_COMPUTE_BIT,
                                0,
                                4,
                                (&step as *const u32).cast(),
                            );
                            (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
                        }
                    }
                }
            }

            // (3.6, rung R9d) The VB split's hardware shadow chain — TLAS pack/build + the RT
            // soft-shadow VIS pre-pass + the `levels` à-trous passes + the temporal reproject.
            // Recorded in the SAME order `declare_vb_graph` declares them (after the SSAO/
            // à-trous chain, before `ddgi_update`). Reuses the DEFERRED chain's shared pipeline
            // objects (`scene.shadow`'s own `atrous_pipeline`/`temporal_pipeline` — one pipeline
            // object, many per-path callers) against the split's OWN dedicated sets
            // (`thin_normal`/`viewt` instead of the fat `gNormal`/`gViewT`). The tlas pack/build
            // body is `record_gbuffer`'s own port, adapted to `record_vb_pass` for barriers.
            #[cfg(feature = "hwrt")]
            if let (Some(t), Some(fns)) = (scene.tlas.as_ref(), accel_fns) {
                let pack_pass =
                    plan.tlas_pack.expect("invariant: scene.tlas.is_some() ⇒ tlas_pack declared");
                let build_pass =
                    plan.tlas_build.expect("invariant: scene.tlas.is_some() ⇒ tlas_build declared");
                // SAFETY: recording is open; `record_vb_pass` records the "tlas_pack" pass's
                // derived input barriers into `cmd` against the live scene buffers.
                self.record_vb_pass(pack_pass, cmd, targets, forward, vb, scene, fi);
                let groups = t.count.div_ceil(LOCAL_SIZE_X);
                let push = t.count.to_le_bytes();
                // SAFETY: recording is open; the packer pipeline + its layout are live on this
                // device (caller contract); `t.bind_group` binds this frame slot's pack inputs;
                // `groups` covers `t.count` at the 64-wide group; `&t.bind_group.descriptor_set`
                // is a single-element local alive for the call; the push is exactly
                // `BUILD_TLAS_INSTANCES_PUSH_BYTES` (4) at offset 0 and `push` outlives the call.
                unsafe {
                    (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, t.pipeline.pipeline);
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        t.pipeline.layout,
                        0,
                        1,
                        &t.bind_group.descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_push_constants)(
                        cmd,
                        t.pipeline.layout,
                        VK_SHADER_STAGE_COMPUTE_BIT,
                        0,
                        crate::compute::BUILD_TLAS_INSTANCES_PUSH_BYTES,
                        push.as_ptr().cast(),
                    );
                    (self.fns.cmd_dispatch)(cmd, groups, 1, 1);
                }
                // SAFETY: recording is open; `record_vb_pass` records the "tlas_build" pass's
                // derived pack-WRITE → build-READ barrier on `tlas_instances`.
                self.record_vb_pass(build_pass, cmd, targets, forward, vb, scene, fi);
                let entry = boyko_rhi::AsBuildEntry {
                    kind: boyko_rhi::AsKind::Tlas,
                    geometry: boyko_rhi::AsGeometryDesc {
                        vertex_data: t.instance_array_addr,
                        index_data: 0,
                        vertex_stride: 0,
                        max_vertex: 0,
                        primitive_count: t.count,
                        index_type: boyko_rhi::AsIndexType::Uint32,
                    },
                    scratch_address: t.scratch_addr,
                };
                // SAFETY: recording is open; `fns` is the live device's AS table (resolved from
                // the RT `ctx`); `entry`'s `vertex_data` (the pack-written instance array) +
                // `scratch_address` + `t.dest.handle` are live, correctly-flagged resources; the
                // pack→build barrier just recorded orders the instance-array write before this
                // build's read; `entry`/`dest` are 1-element slices that outlive the call.
                unsafe {
                    crate::accel::cmd_build_acceleration_structures(
                        fns,
                        cmd,
                        core::slice::from_ref(&entry),
                        &[t.dest],
                    );
                }
                // SAFETY: recording is open; `self.fns` is the live device's core command table.
                // The barrier touches no resource beyond the execution/memory dependency
                // (AS_BUILD stage → COMPUTE_SHADER stage).
                unsafe {
                    crate::accel::cmd_acceleration_structure_barrier(self.fns, cmd);
                }
            }

            #[cfg(feature = "hwrt")]
            if let (Some(sh), Some(vis_set), Some(atrous_sets)) = (
                scene.shadow.as_ref(),
                targets.vb_shadow_vis_set.as_ref(),
                targets.vb_shadow_atrous_sets.as_ref(),
            ) {
                let vis_pipeline = scene.vb_shadow_vis_pipeline.expect(
                    "invariant: scene.shadow.is_some() under the split ⇒ vb_shadow_vis_pipeline is Some",
                );
                let vis_pass = plan
                    .shadow_vis
                    .expect("invariant: scene.shadow.is_some() ⇒ shadow_vis pass declared");
                // SAFETY: recording is open; `record_vb_pass` records the graph's derived input
                // barriers for the "shadow_vis" pass into `cmd`.
                self.record_vb_pass(vis_pass, cmd, targets, forward, vb, scene, fi);
                // SAFETY: recording is open; `vis_pipeline` + its 7-binding layout are live on
                // this device (caller contract); `vis_set[fi]` binds `thin_normal`/`viewt` +
                // `LightTable` + the camera UBO + the TLAS + the `ResolvedRayShadow` UBO +
                // `gShadowVis` (the write target); `dispatch_group_count_x` covers the pixel
                // count; the pipeline's declared 4-byte push is bound-but-unread (no push
                // recorded, mirroring the deferred VIS pass).
                unsafe {
                    (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, vis_pipeline.pipeline);
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vis_pipeline.layout,
                        0,
                        1,
                        &vis_set[fi].descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
                }

                let atrous_levels =
                    (sh.atrous_levels as usize).min(crate::present::MAX_ATROUS_LEVELS as usize);
                debug_assert!(
                    atrous_sets.len() >= atrous_levels,
                    "invariant: the VB à-trous set array must hold at least `atrous_levels` levels"
                );
                debug_assert_eq!(
                    sh.final_is_vis2,
                    atrous_levels % 2 == 1,
                    "invariant: the VB denoised/temporal bind ring must match the last à-trous parity"
                );
                for (level, level_ring) in atrous_sets.iter().enumerate().take(atrous_levels) {
                    let atrous_pass = plan.shadow_atrous[level].expect(
                        "invariant: level < scene.shadow.atrous_levels ⇒ the VB shadow_atrous[level] declared",
                    );
                    let step: u32 = 1u32 << level;
                    // SAFETY: recording is open; `record_vb_pass` records the "shadow_atrous"
                    // pass's derived RAW barriers on the ping-pong pair into `cmd`.
                    self.record_vb_pass(atrous_pass, cmd, targets, forward, vb, scene, fi);
                    let atrous_set = &level_ring[self.frame_index];
                    // SAFETY: recording is open; the SHARED à-trous pipeline + its 6-binding
                    // layout (the deferred boot object) are live on this device; `atrous_set`
                    // binds `gVisIn`/`gVisOut` (the ping-pong pair) + `thin_normal`/`viewt` + the
                    // `ResolvedShadowDenoise` UBO + the camera UBO; the 4-byte `{ uint step }`
                    // push covers the pipeline's declared COMPUTE range; `dispatch_group_count_x`
                    // covers the pixel count.
                    unsafe {
                        (self.fns.cmd_bind_pipeline)(
                            cmd,
                            VK_PIPELINE_BIND_POINT_COMPUTE,
                            sh.atrous_pipeline.pipeline,
                        );
                        (self.fns.cmd_bind_descriptor_sets)(
                            cmd,
                            VK_PIPELINE_BIND_POINT_COMPUTE,
                            sh.atrous_pipeline.layout,
                            0,
                            1,
                            &atrous_set.descriptor_set,
                            0,
                            ptr::null(),
                        );
                        (self.fns.cmd_push_constants)(
                            cmd,
                            sh.atrous_pipeline.layout,
                            VK_SHADER_STAGE_COMPUTE_BIT,
                            0,
                            4,
                            (&step as *const u32).cast(),
                        );
                        (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
                    }
                }
            }

            #[cfg(feature = "hwrt")]
            if scene.path_vb_hwrt_shadow()
                && scene.temporal_active()
                && let Some(temporal_sets) = targets.vb_shadow_temporal_set.as_ref()
            {
                let sh = scene
                    .shadow
                    .as_ref()
                    .expect("invariant: temporal_active() implies scene.shadow.is_some()");
                let temporal_pipeline = sh.temporal_pipeline.expect(
                    "invariant: temporal_active() + the VB temporal set built implies the temporal pipeline",
                );
                let temporal_pass = plan.shadow_temporal.expect(
                    "invariant: scene.temporal_active() under the split ⇒ shadow_temporal pass declared",
                );
                // SAFETY: recording is open; `record_vb_pass` records the "shadow_temporal"
                // pass's derived input/RAW barriers into `cmd`.
                self.record_vb_pass(temporal_pass, cmd, targets, forward, vb, scene, fi);
                let temporal_set = &temporal_sets[self.frame_index];
                // SAFETY: recording is open; the SHARED temporal pipeline + its 8-binding layout
                // (the deferred boot object) are live on this device; `temporal_set` binds
                // gVisIn/gMotionVec/gViewT/gHistIn/gHistOut/gTemporalOut + the
                // ResolvedTemporalShadow UBO + the camera UBO for `frame_index`;
                // `dispatch_group_count_x` covers the pixel count; the pipeline's declared 4-byte
                // COMPUTE range is bound-but-unread (no push recorded).
                unsafe {
                    (self.fns.cmd_bind_pipeline)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        temporal_pipeline.pipeline,
                    );
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        temporal_pipeline.layout,
                        0,
                        1,
                        &temporal_set.descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
                }
            }

            // (3.5, rung R9c) Pass `ddgi_update` — the probe update under VB (reachable only
            // VB×Both, `path_vb_ddgi()`): the DEFERRED record body mirrored (the single
            // non-ringed 7-binding set; the shader reads all params from the b6 UBO — no push).
            if scene.path_vb_ddgi() {
                let activation = scene
                    .ddgi_update
                    .as_ref()
                    .expect("invariant: path_vb_ddgi() ⇒ scene.ddgi_update.is_some()");
                let ddgi_update_set = targets
                    .ddgi_update_set
                    .as_ref()
                    .expect("invariant: scene.ddgi_update is Some ⇒ GBufferTargets wrote ddgi_update_set");
                let ddgi_pass = plan
                    .ddgi_update
                    .expect("invariant: path_vb_ddgi() ⇒ the VB ddgi_update pass was declared");
                // SAFETY: recording is open; `record_vb_pass` records the derived light/ray/
                // classification reads + the content-preserving SRO→GENERAL atlas transitions.
                self.record_vb_pass(ddgi_pass, cmd, targets, forward, vb, scene, fi);
                // SAFETY: recording is open; the update pipeline + its 7-binding layout are
                // live (caller contract); `dispatch_group_count_x` is the activation's own
                // probe-subset block count; no push (the b6 UBO carries every param).
                unsafe {
                    (self.fns.cmd_bind_pipeline)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        activation.pipeline.pipeline,
                    );
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        activation.pipeline.layout,
                        0,
                        1,
                        &ddgi_update_set.descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_dispatch)(cmd, activation.dispatch_group_count_x, 1, 1);
                }
            }

            // (4) Pass `vb_shade_split` — the split's lit producer (RE-fetch + shade + the
            // unconditional gSsao read + the rung-R9c header-gated DDGI probe sample). Per-frame
            // base/`_tex` pick mirrors the fused
            // `vb_resolve`/`vb_shade_tex` selection (boot-frozen split, per-frame textures).
            let shade_pass = plan
                .vb_shade_split
                .expect("invariant: path_vb_split() => vb_shade_split pass declared");
            // SAFETY: recording is open; `record_vb_pass` records the `lit`
            // COLOR_ATTACHMENT→GENERAL + cascade/atlas + `ssao` GENERAL-read barriers for the
            // "vb_shade_split" pass into `cmd`.
            self.record_vb_pass(shade_pass, cmd, targets, forward, vb, scene, fi);
            // Rung R9d: extend the base/`_tex` pick to the 2×2 matrix — the `-D HWRT=1` variants
            // when the hwrt shadow chain is armed (`path_vb_hwrt_shadow()`, the SAME predicate
            // the declare-site's conditional denoised-vis read uses), else the software pair
            // exactly as shipped.
            #[cfg(feature = "hwrt")]
            let (shade_pipeline, shade_set0) = if scene.path_vb_hwrt_shadow() {
                if scene.vb_tex_active() {
                    (
                        scene.vb_shade_split_tex_hwrt_pipeline.expect(
                            "invariant: vb_tex_active() + path_vb_hwrt_shadow() requires vb_shade_split_tex_hwrt_pipeline",
                        ),
                        targets.vb_set0_tex.as_ref().expect(
                            "invariant: GBufferScene::vb_tex_active() => targets.vb_set0_tex is built",
                        ),
                    )
                } else {
                    (
                        scene.vb_shade_split_hwrt_pipeline.expect(
                            "invariant: path_vb_hwrt_shadow() requires vb_shade_split_hwrt_pipeline",
                        ),
                        vb_set0,
                    )
                }
            } else if scene.vb_tex_active() {
                (
                    scene.vb_shade_split_tex_pipeline.expect(
                        "invariant: vb_tex_active() under split requires vb_shade_split_tex_pipeline",
                    ),
                    targets.vb_set0_tex.as_ref().expect(
                        "invariant: GBufferScene::vb_tex_active() => targets.vb_set0_tex is built",
                    ),
                )
            } else {
                (
                    scene.vb_shade_split_pipeline.expect(
                        "invariant: a split-armed scene carries vb_shade_split_pipeline",
                    ),
                    vb_set0,
                )
            };
            #[cfg(not(feature = "hwrt"))]
            let (shade_pipeline, shade_set0) = if scene.vb_tex_active() {
                (
                    scene.vb_shade_split_tex_pipeline.expect(
                        "invariant: vb_tex_active() under split requires vb_shade_split_tex_pipeline",
                    ),
                    targets.vb_set0_tex.as_ref().expect(
                        "invariant: GBufferScene::vb_tex_active() => targets.vb_set0_tex is built",
                    ),
                )
            } else {
                (
                    scene.vb_shade_split_pipeline.expect(
                        "invariant: a split-armed scene carries vb_shade_split_pipeline",
                    ),
                    vb_set0,
                )
            };
            let vb_split_set1 = targets
                .vb_split_set1
                .as_ref()
                .expect("invariant: a split-armed boot builds vb_split_set1 (targets tail)");
            // SAFETY: recording is open; `shade_pipeline` (Set 0 = the base/_tex vb ring, Set 1
            // = `vb_split_set1[fi]` — the shadow vocab + gSsao + the DDGI combined atlases, Set
            // 2 = the geometry table, `_tex` adds Set 3 = the bindless table) belongs to this
            // device; the 64-byte push is the SAME `view_proj`; `dispatch_group_count_x`
            // covers the pixel count.
            unsafe {
                (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, shade_pipeline.pipeline);
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    shade_pipeline.layout,
                    0,
                    1,
                    &shade_set0[fi].descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    shade_pipeline.layout,
                    1,
                    1,
                    &vb_split_set1[fi].descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    shade_pipeline.layout,
                    2,
                    1,
                    &vb_geometry_set.set(),
                    0,
                    ptr::null(),
                );
                if scene.vb_tex_active() {
                    let bindless_set = scene
                        .bindless_set
                        .expect("invariant: GBufferScene::vb_tex_active() => scene.bindless_set is Some");
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        shade_pipeline.layout,
                        3,
                        1,
                        &bindless_set,
                        0,
                        ptr::null(),
                    );
                }
                (self.fns.cmd_push_constants)(
                    cmd,
                    shade_pipeline.layout,
                    VK_SHADER_STAGE_COMPUTE_BIT,
                    0,
                    64,
                    scene.mvp.as_ptr().cast(),
                );
                (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
            }
        }

        // === Pass `sdf_forward_march` — rung R10: the fused SDF march-then-shade COMPUTE pass,
        // the SAME body `record_forward` records (that fn's own doc has the full C5 rationale).
        // Recorded ONLY when `scene.path_has_sdf_forward()` holds. Extends the `lit` write above
        // (`vb_resolve`'s GENERAL store under `Both`, or `vb_sky`'s COLOR write under `Sdf`) and
        // marches THIS frame's `lit` pixels the raster/sky did not already paint (a miss writes
        // nothing to `lit` — the sky/mesh color stands, the pass's own doc; the TAA-armed VIEWT
        // variants additionally write the `gViewT` lane for EVERY pixel, exactly once). ===
        if scene.path_has_sdf_forward() {
            let sdf_forward_pass = plan
                .sdf_forward_march
                .expect("invariant: scene.path_has_sdf_forward() => sdf_forward_march pass declared");
            // SAFETY: recording is open; `record_vb_pass` records the graph's derived `lit`
            // ->GENERAL barrier (+ the `vb_depth` DEPTH_ATTACHMENT_OPTIMAL->SHADER_READ_ONLY_OPTIMAL
            // barrier under `mesh_leg`, or the cascade/atlas/light_table producer barriers under
            // `!mesh_leg`) for the "sdf_forward_march" pass into `cmd`.
            self.record_vb_pass(sdf_forward_pass, cmd, targets, forward, vb, scene, fi);

            // Decision 4's ownership gate: the `HAS_MESH` variants need the reverse-Z decode
            // `A`/`B` (`scene.sdf_forward_view_z_a`/`_b`) to bound the march at the mesh surface;
            // the mesh-less variants never read them (`SdfForwardMarchPush::sdf_only`'s doc).
            // TAA-under-VB: `writes_viewt` (the SAME O1 predicate `declare_vb_graph` read to
            // declare this pass's conditional `viewt` write) selects the `VIEWT` gViewT-producing
            // sibling — same layout, same push, plus the binding-13 store.
            let writes_viewt = scene.path_sdf_forward_writes_viewt();
            let (pipeline, push) = if scene.resolved_render_path.mesh_leg {
                let p = if writes_viewt {
                    scene.sdf_forward_march_viewt_pipeline.expect(
                        "invariant: path_sdf_forward_writes_viewt() requires scene.sdf_forward_march_viewt_pipeline",
                    )
                } else {
                    scene.sdf_forward_march_pipeline.expect(
                        "invariant: scene.path_has_sdf_forward() requires scene.sdf_forward_march_pipeline",
                    )
                };
                let push = crate::compute::SdfForwardMarchPush::has_mesh(
                    present_extent.width,
                    present_extent.height,
                    scene.sdf_forward_view_z_a,
                    scene.sdf_forward_view_z_b,
                    scene.light_dir,
                );
                (p, push)
            } else {
                let p = if writes_viewt {
                    scene.sdf_forward_march_sdfonly_viewt_pipeline.expect(
                        "invariant: path_sdf_forward_writes_viewt() requires scene.sdf_forward_march_sdfonly_viewt_pipeline",
                    )
                } else {
                    scene.sdf_forward_march_sdfonly_pipeline.expect(
                        "invariant: scene.path_has_sdf_forward() requires scene.sdf_forward_march_sdfonly_pipeline",
                    )
                };
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
            // SAFETY: recording is open; `pipeline` (one of the four `{HAS_MESH} x {VIEWT}`
            // compute variants, selected by `mesh_leg` x `writes_viewt`) + its 2-set layout
            // (Set 0 = `sdf_forward_set[fi]`, the SAME
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

        // === TAA-under-VB: the `vb_viewt` gViewT-producer dispatch — AFTER every `lit`/depth
        // producer, BEFORE the TAA resolve below that consumes `viewt`. Mirrors
        // `record_gbuffer`'s own `viewt_from_depth` record site token-for-token, adapted to the
        // VB plan/sink + the 16-byte reverse-Z push. ===
        debug_assert_eq!(
            plan.viewt_from_depth.is_some(),
            scene.viewt_from_vb_depth.is_some(),
            "W1: declare/record predicate desync (viewt_from_vb_depth)"
        );
        // Rung R9b: this is the taa-only LATE slot — with the split's SSAO armed the pass was
        // recorded PRE-TAIL (inside the split arm above), matching `declare_vb_graph`'s slot
        // choice (ONE `scene.ssao.is_some()` predicate at both sites).
        if scene.ssao.is_none()
            && let Some(vb_viewt_pass) = plan.viewt_from_depth
        {
            self.record_vb_viewt_dispatch(vb_viewt_pass, cmd, targets, forward, vb, scene, present_extent, fi);
        }

        // === TAA-under-VB: the temporal-resolve (+ RCAS) — recorded HERE, BEFORE
        // `present_sample`'s `lit` GENERAL→SHADER_READ_ONLY transition (TAA is a COMPUTE
        // dispatch whose own graph pass derives that transition out of the `lit` producer — the
        // OPPOSITE ordering the FXAA/SMAA/SSAA fragment passes below use; `record_gbuffer`'s
        // TAA block ordering, ported). The pass barriers are emitted through the VB sink
        // (`record_vb_pass` — VB's OWN ResId table), then the graph-emit-free
        // `record_taa_body`/`record_rcas` bodies run unchanged (both are path-portable). ===
        if let Some(taa) = scene.taa.as_ref()
            && targets.taa_resolve_set.is_some()
        {
            let taa_pass = plan
                .taa_resolve
                .expect("invariant: scene.taa.is_some() ⇒ the VB taa_resolve pass was declared");
            self.record_vb_pass(taa_pass, cmd, targets, forward, vb, scene, fi);
            // SAFETY: recording is open; the taa_resolve pass barriers were just emitted via
            // the VB sink above (record_taa_body's caller contract); `aa_out`/`taa_hist`/
            // `taa_resolve_set` were built by `create()` under the same `scene.taa` that gates
            // this branch.
            unsafe { self.record_taa_body(cmd, targets, taa, scene, fi) };
            if let Some(rcas) = scene.rcas.as_ref()
                && targets.rcas_set.is_some()
            {
                // SAFETY: recording is open; `record_taa_body` (just above) already wrote
                // `taa_resolved[fi]`, leaving it in GENERAL; `taa_resolved`/`aa_out`/`rcas_set`
                // were built by `create()` under the same `scene.rcas` that gates this branch;
                // `present_extent` sizes both (the SAME extent the resolve dispatched over).
                unsafe { self.record_rcas(cmd, targets, rcas, present_extent, scene, fi) };
            }
        }

        // === Present-blit `lit` into the swapchain — byte-for-byte port of `record_forward`'s
        // own tail. ===
        // SAFETY: recording is open; `record_vb_pass` records the graph's derived
        // GENERAL→SHADER_READ_ONLY_OPTIMAL barrier for the "present_sample" pass into `cmd`
        // (with TAA armed, the taa_resolve pass above already left `lit` in SHADER_READ_ONLY —
        // this then derives no further barrier, the deferred precedent).
        self.record_vb_pass(plan.present_sample, cmd, targets, forward, vb, scene, fi);

        // Anti-aliasing resolve (VB). When AA is armed, `sync_gbuffer` rewires `present_set` to
        // sample `aa_out` (path-agnostic), but the FXAA/SMAA/SSAA dispatch used to live ONLY in
        // `record_gbuffer` (Deferred) -- so under VB the resolved `lit` was never anti-aliased into
        // `aa_out`, and the present-blit below sampled a never-written (black) image. Mirror
        // `record_gbuffer`'s AA block verbatim: FXAA/SMAA read `present_extent`; SSAA reads the
        // BOOT-FIXED `aa_extent` (`present_extent` is 2x under SSAA). TAA (VB × Mesh) was ALREADY
        // recorded above (before `present_sample` -- the compute-vs-fragment ordering, see that
        // block's comment), so the fall-through documents it exactly like `record_gbuffer`'s
        // equivalent else. OFF (`aa_out` is `None`) records nothing -> the AA-off VB command
        // stream is byte-identical to before this block existed.
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
                // `aa_out.is_some()` with none of aa/smaa/ssaa matched ⇒ TAA is the reason
                // (the four arms are mutually exclusive by construction); `record_taa_body`
                // already ran above (the TAA-under-VB block) — nothing left to do here, the
                // SAME fall-through `record_gbuffer`'s equivalent else documents.
                debug_assert!(
                    scene.taa.is_some(),
                    "invariant: VB aa_out armed but none of aa/smaa/ssaa/taa matched"
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

    /// Rung R9b: the `vb_viewt` (viewt_from_depth_rz) pass-barriers + dispatch body, extracted
    /// so BOTH record slots (the split's SSAO-armed PRE-TAIL slot and the taa-only LATE slot)
    /// share one implementation — `declare_vb_graph` declares the pass in exactly one position
    /// per frame (`scene.ssao.is_some()` picks it), and the recorder replays the SAME body at
    /// the matching site.
    #[allow(clippy::too_many_arguments)]
    fn record_vb_viewt_dispatch(
        &self,
        vb_viewt_pass: crate::framegraph::PassId,
        cmd: VkCommandBuffer,
        targets: &GBufferTargets,
        forward: &ForwardTargets,
        vb: &VbTargets,
        scene: &GBufferScene<'_>,
        present_extent: VkExtent2D,
        fi: usize,
    ) {
        // SAFETY: recording is open (caller contract); `record_vb_pass` records the graph's
        // derived DEPTH_ATTACHMENT→SHADER_READ_ONLY (vb_depth) + UNDEFINED→GENERAL (viewt)
        // barriers for the "vb_viewt" pass into `cmd`.
        self.record_vb_pass(vb_viewt_pass, cmd, targets, forward, vb, scene, fi);
        let activation = scene.viewt_from_vb_depth.as_ref().expect(
            "invariant: plan.viewt_from_depth.is_some() ⇒ scene.viewt_from_vb_depth.is_some() (W1)",
        );
        let push = crate::compute::ViewtFromDepthRzPush::new(
            present_extent.width,
            present_extent.height,
            activation.view_z_a,
            activation.view_z_b,
        );
        let push_bytes = push.as_bytes();
        let set = &targets.viewt_from_vb_depth_set.as_ref().expect(
            "invariant: scene.viewt_from_vb_depth.is_some() ⇒ create wrote viewt_from_vb_depth_set",
        )[fi];
        let group_x = present_extent.width.div_ceil(8);
        let group_y = present_extent.height.div_ceil(8);
        // SAFETY: recording is open; `activation.pipeline` + its layout (declaring
        // `activation.layout` at set 0 AND the 16-byte COMPUTE push range) are live on this
        // device (caller contract); `set` binds the now-transitioned reverse-Z depth
        // (SHADER_READ, by the `record_vb_pass` call above) + `gViewT` (GENERAL) + the
        // camera-ring slot `fi`; `group_x`/`group_y` cover `present_extent` (the 8×8-tile
        // ceiling of the SAME extent the depth/gViewT images are sized to);
        // `&set.descriptor_set` is a single-element local alive for the call; `push_bytes`
        // is `VIEWT_FROM_DEPTH_RZ_PUSH_BYTES` (16) bytes at offset 0, exactly the declared
        // push range, and the backing `push` local outlives the call.
        unsafe {
            (self.fns.cmd_bind_pipeline)(
                cmd,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                activation.pipeline.pipeline,
            );
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                activation.pipeline.layout,
                0,
                1,
                &set.descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_push_constants)(
                cmd,
                activation.pipeline.layout,
                VK_SHADER_STAGE_COMPUTE_BIT,
                0,
                crate::compute::VIEWT_FROM_DEPTH_RZ_PUSH_BYTES,
                push_bytes.as_ptr().cast(),
            );
            (self.fns.cmd_dispatch)(cmd, group_x, group_y, 1);
        }
    }
}
