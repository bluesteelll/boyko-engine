//! `Renderer::record_gbuffer`: the on-screen 3-pass G-buffer body
//! (raster → depth-sample → march/SSAO → deferred resolve → present-blit) behind
//! [`Renderer::render_gbuffer_frame`], with every derived barrier driven through the
//! [`GbufferBarrierSink`](super::super::graph_bridge::GbufferBarrierSink).

use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::compute::{
    CoarseMode, DEFAULT_MARCHER_OMEGA, FineMarcherPush, INTERP_INSTANCES_PUSH_BYTES, LOCAL_SIZE_X,
    VIEWT_FROM_DEPTH_PUSH_BYTES, ViewtFromDepthPush, tile_grid_extent,
};
use crate::ffi::*;
use crate::memory::BoundBuffer;
use crate::texture::{MAX_CASCADES, MAX_TEXTURE_LAYERS};

use super::super::frame_driver::Renderer;
use super::super::gpu_timing::TimedPass;
use super::super::scene_types::{
    CLUSTER_CULL_HIER_PUSH_BYTES, CLUSTER_CULL_PUSH_BYTES, GBUFFER_MARCHER_PUSH_BYTES,
    GBUFFER_PUSH_BASE_INSTANCE_OFFSET, GBufferScene, LIGHT_CULL_LOCAL_SIZE_X,
};
use super::super::targets::GBufferTargets;
use super::super::{COLOR_SUBRESOURCE_RANGE, SwapchainError};

/// Textured-PBR T6c (plan Decision D4): prints a ONE-TIME diagnostic when a textured
/// material is active on a frame that also has the temporal motion-vector pipeline
/// active — TEXTURED is never compiled with MOTION_VECTORS, so that frame renders the
/// material's `base_color`/scalar `mrr` instead of sampled textures (the MV/mvpm arm
/// takes priority). A process-wide latch keeps this off the steady-state hot path
/// (Principle 1: no per-frame `AtomicBool` load cost beyond the one `swap` on the FIRST
/// occurrence — every subsequent call short-circuits `WARNED.load` first) and avoids
/// spamming stderr every frame the two features overlap.
#[cold]
#[inline(never)]
fn warn_textured_suppressed_by_motion_vectors() {
    static WARNED: AtomicBool = AtomicBool::new(false);
    // Relaxed: a diagnostic latch, not a cross-thread synchronization point — the render
    // loop is single-threaded through this call site, and a racing double-print (were this
    // ever called from multiple threads) would be a harmless cosmetic duplicate, not UB.
    if WARNED.load(Ordering::Relaxed) {
        return;
    }
    WARNED.store(true, Ordering::Relaxed);
    eprintln!(
        "boyko_rhi_vulkan: a textured material is active while the temporal motion-vector \
         gbuffer pipeline is also active this frame — TEXTURED is never compiled with \
         MOTION_VECTORS (textured-PBR T6c plan Decision D4), so textured material(s) render \
         base_color/scalar mrr instead of sampled textures until temporal denoise is off."
    );
}

impl Renderer<'_> {
    /// Records the Render-P1c on-screen 3-pass G-buffer frame into `cmd`. The barrier
    /// sequence (one hand-FFI barrier per transition — correct-but-unbatched; P3a
    /// batches later):
    ///
    /// 0. throwaway raster color `UNDEFINED → COLOR_ATTACHMENT_OPTIMAL` (the raster
    ///    pipeline declares one color format, so the prepass binds a format-compatible
    ///    throwaway color attachment whose result is discarded — only the depth matters)
    /// 1. depth `UNDEFINED → DEPTH_ATTACHMENT_OPTIMAL` (TOP_OF_PIPE → (EARLY|LATE)_FRAGMENT_TESTS)
    /// 2. **(pass A)** `vkCmdBeginRendering` (throwaway color CLEAR/STORE + depth CLEAR
    ///    to the far plane / STORE), draw the mesh quad — the depth prepass (the
    ///    swapchain image becomes a color attachment only at pass C)
    /// 3. depth `DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL` (DEPTH aspect,
    ///    (EARLY|LATE)_FRAGMENT_TESTS → COMPUTE_SHADER) — the single dual-use depth
    ///    barrier (REPLACES the packed path's depth copy + its two transfer barriers)
    /// 4. the 3 G-buffer images `UNDEFINED → GENERAL` (TOP_OF_PIPE → COMPUTE_SHADER)
    /// 5. **(P0 coarse cull, OPTIONAL — only when `scene.coarse` is `Some`)** bind the
    ///    coarse-cull pipeline + the vocabulary set, dispatch one group per `LOCAL_SIZE_X`
    ///    tiles (each invocation writes a `TileBound` into binding 6), then a COMPUTE→COMPUTE
    ///    buffer barrier on `tiles_buffer` (SHADER_WRITE → SHADER_READ); the marcher then runs
    ///    with `coarse_enabled == scene.coarse_mode` (`1` = full / `2` = empty-skip-only). When
    ///    `scene.coarse` is `None` this step records NOTHING (`coarse_enabled == 0`).
    /// 6. **(pass B)** bind the marcher + the vocabulary set, dispatch (the marcher
    ///    SAMPLES the depth image, STORES the final composite into ALBEDO)
    /// 7. ALBEDO `GENERAL → SHADER_READ_ONLY_OPTIMAL` (COMPUTE_SHADER → FRAGMENT_SHADER)
    /// 8. swapchain `UNDEFINED → COLOR_ATTACHMENT_OPTIMAL` (TOP_OF_PIPE → COLOR_ATTACHMENT_OUTPUT)
    /// 9. **(pass C)** `vkCmdBeginRendering` (swapchain color CLEAR), fullscreen-sample
    ///    the ALBEDO 1:1 in the top-left, end
    /// 10. swapchain `COLOR_ATTACHMENT_OPTIMAL → PRESENT_SRC_KHR` (steady) or
    ///     `→ TRANSFER_SRC`, copy-to-buffer, `→ PRESENT` (the readback path)
    ///
    /// NO `copy_image_to_buffer(depth)` (step 3 replaces it) and NO
    /// `vkUpdateDescriptorSets` (both sets were written once at `sync_gbuffer`).
    ///
    /// Extents: passes A (prepass raster/depth) and B (the marcher dispatch → composite)
    /// run at `present_extent` (the composite size the G-buffer/depth images, the dispatch
    /// grid, and the camera UBO `count` were all sized to in `sync_gbuffer`). `extent` is
    /// the swapchain extent and governs ONLY pass C's clear render-area (step 8) and the
    /// readback region (step 9); the present-blit viewport is `min(extent, present_extent)`
    /// at the origin for the exact 1:1 top-left composite present. `aa_extent` (SSAA) is the
    /// BOOT-FIXED native extent `aa_out` was actually allocated at (`sync_gbuffer`'s
    /// `aa_extent` param) — the SSAA downsample pass's render-area/viewport MUST use this,
    /// NOT `extent` (which tracks live window resizes while `aa_out` stays boot-fixed,
    /// exactly like `present_extent`). Unread when `scene.ssaa` is `None`.
    ///
    /// # Safety
    ///
    /// `cmd` must be recordable (waited free); `image`/`view` must belong to the
    /// swapchain image presented this frame; `scene`'s pipelines / buffers / samplers
    /// are live on this device; `targets` was synced to `present_extent` (the composite
    /// size — its descriptor sets bind `scene`'s SSBO/UBO + its own images, and its
    /// G-buffer/depth images are allocated at `present_extent`); `scene.dispatch_group_count_x`
    /// (and `scene.camera_uniform`'s `count`) cover `present_extent`'s pixel count.
    /// `extent` is the swapchain extent and governs ONLY pass C's clear render-area and the
    /// readback region; a `Some(readback)` buffer is host-visible and ≥ the swapchain
    /// image's (`extent`-sized) byte size.
    #[allow(clippy::too_many_arguments)]
    #[cfg_attr(feature = "hwrt", allow(clippy::too_many_arguments))]
    pub(crate) unsafe fn record_gbuffer(
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
        readback: Option<&BoundBuffer>,
        // HW-RT rung R2a-3: the resolved AS command table for the per-frame TLAS build; `None`
        // on a non-RT device. The whole parameter is `hwrt`-gated, so the `not(hwrt)` signature
        // is unchanged (byte-identity).
        #[cfg(feature = "hwrt")] accel_fns: Option<&crate::accel::AccelFns>,
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

        // The lock-free cross-frame ring index: this present's slot. EVERY G-buffer render-
        // target IMAGE is RINGED to `FRAMES_IN_FLIGHT` copies, so this frame writes (and the
        // matching `*_set[fi]` descriptor binds) `<image>[fi]` while a sibling in-flight frame
        // reads its OWN slot — the cross-frame Write-After-Read fix. The per-slot `in_flight`
        // fence (waited at the top of `render_gbuffer_frame`) already freed this slot's previous
        // images, so no new wait is introduced. Index every image barrier / attachment by `[fi]`.
        let fi = self.frame_index;

        // HW-RT rung R0: reset ALL `2 * PASS_COUNT` timestamp queries at the frame top —
        // OUTSIDE any render / dynamic-rendering scope (recording is open but no
        // `begin_rendering` has run yet), before the frame's first `write_timestamp`. GATED
        // on `scene.gpu_timing`: `None` (every golden/host frame) records NOTHING, so the
        // command stream is byte-identical. A TIMESTAMP query is undefined until reset.
        if let Some(tc) = scene.gpu_timing {
            // SAFETY: recording is open; `self.fns` is the live device fn-table; the reset is
            // recorded before any `begin_rendering` (outside a render pass, per
            // `VUID-vkCmdResetQueryPool-renderpass`); `fi` is this present's in-flight slot.
            unsafe { tc.reset_frame(self.fns, cmd, fi) };
        }

        // === Pass A (Render P5-r0): rasterize the mesh quad as a 3-MRT G-buffer PRODUCER
        // (albedo@0, normal@1, material@2) + the D32 depth. The marcher's attribute
        // encoding is the contract; pass A writes mesh fragments in it (mask=1) so the
        // deferred resolve lights mesh pixels first-class and the r1 ownership gate yields
        // to them. gViewT is UNTOUCHED by r0 (still wholly marcher-produced). ===

        // (0)+(1) Barrier-in: the 3 RGBA8 G-buffer images UNDEFINED → COLOR_ATTACHMENT_OPTIMAL,
        // then the depth image UNDEFINED → DEPTH_ATTACHMENT_OPTIMAL.
        // `src=0`/`TOP_OF_PIPE` is the superset-correct FIRST transition for a freshly
        // re-`UNDEFINED`'d image (no prior content to make available).
        // Step 1a (sync1 array-batching): the 3 color barriers share one global
        // TOP_OF_PIPE→COLOR_ATTACHMENT_OUTPUT scope over independent (non-aliasing) images,
        // so ONE array-form `vkCmdPipelineBarrier` is byte-identical GPU semantics to the
        // former three `count=1` calls — same masks/layouts/subresource, fewer API calls.
        //
        // These two batched barriers are DRIVEN by `frame_graph`'s "raster" pass — the
        // graph derives the color + depth transitions, and `GbufferBarrierSink` records
        // them into the two `vkCmdPipelineBarrier` calls. The per-frame plan is set by
        // `declare_deferred_graph` just before this record; every barrier site below
        // fetches it the same way.
        // SAFETY: recording is open; `record_graph_pass` records the graph's derived
        // barriers for the "raster" pass into `cmd` against the live G-buffer targets.
        let plan = self
            .gbuffer_pass_plan
            .as_ref()
            .expect("invariant: declare_frame_graph ran before record_gbuffer");
        // Multi-paradigm render-path plan, rung R2 (O1 hard rule / W1 lesson): the declare site
        // (`declare_deferred_graph`) and this record site MUST agree on whether the raster pass
        // exists — both call the SAME `scene.path_has_raster()` predicate, so this can never
        // trip unless the two sites diverge.
        debug_assert_eq!(
            plan.raster.is_some(),
            scene.path_has_raster(),
            "W1: declare/record predicate desync (raster)"
        );

        // === Pillar B B3: the per-instance TRS INTERPOLATION compute PRE-PASS. Recorded ONLY
        // when the scene wires the activation (`scene.interp.is_some()`); otherwise skipped
        // entirely — NO bind, NO dispatch, NO barrier — so the command stream is BYTE-IDENTICAL
        // to the interp-OFF (dump) path. Runs FIRST (before the raster pass): one invocation per
        // instance reads its prev/curr TRS pair (bound at the interp set @0), interpolates at the
        // frame-wide `alpha`, and STORES the interpolated 48-byte model column into the draw SSBO
        // (bound at @1). The raster + shadow VS then read that draw SSBO as `instances[...]`
        // (the caller set `scene.instance_bind_group` to the SAME draw-SSBO slot), so the drawn
        // geometry tracks the interpolated pose. The COMPUTE→VERTEX RAW barrier ordering the
        // interp WRITE before the raster VS READ is derived by the graph at the raster pass (the
        // draw reader), emitted by the `record_graph_pass(plan.raster)` just below — after this
        // dispatch's write. A `count == 0` frame records NO dispatch (an empty scene skips it). ===
        if let Some(interp) = &scene.interp
            && interp.instance_count > 0
        {
            let interp_pass = plan
                .interp
                .expect("invariant: scene.interp.is_some() ⇒ interp pass declared");
            // The interp pass's INPUT barriers are DRIVEN by the graph's "interp" pass,
            // recorded HERE before the dispatch. On the current declaration it derives NONE
            // (the pair read is a first touch on a frame-private slot), so this emits zero
            // `vkCmdPipelineBarrier` calls — but it keeps the per-pass record symmetric with
            // every other pass and future-proofs an added interp input hazard.
            // SAFETY: recording is open; `record_graph_pass` records the graph's derived
            // input barriers (currently none) for the "interp" pass into `cmd`.
            self.record_graph_pass(interp_pass, cmd, targets, scene, fi);
            let groups = interp.instance_count.div_ceil(LOCAL_SIZE_X);
            let mut push = [0u8; INTERP_INSTANCES_PUSH_BYTES as usize];
            push[0..4].copy_from_slice(&interp.instance_count.to_le_bytes());
            push[4..8].copy_from_slice(&interp.alpha.to_le_bytes());
            // SAFETY: recording is open; the interp pipeline + its layout (declaring the
            // 3-binding interp set at set 0 + the 8-byte COMPUTE push range) are live on this
            // device (caller contract); `interp.interp_set` binds this frame slot's pair SSBO
            // @0 (the host-written prev/curr pairs) + the out-slot SSBO @1 (the host-written
            // ring offsets) + the SHARED model-out ring @2 (the compute write target, the SAME
            // buffer the raster VS reads); `groups` covers the dynamic `instance_count` at the
            // 64-wide group; `&interp.interp_set.descriptor_set` is a single-element local alive
            // for the call (first_set 0, count 1, zero dynamic offsets); the push is exactly
            // `INTERP_INSTANCES_PUSH_BYTES` (8) at offset 0 and `push` outlives the call. The
            // interp pass reads frame-private pair + out-slot slots (first touches — the graph
            // derives NO input barrier), so no barrier is recorded before this dispatch.
            unsafe {
                (self.fns.cmd_bind_pipeline)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    interp.pipeline.pipeline,
                );
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
                    INTERP_INSTANCES_PUSH_BYTES,
                    push.as_ptr().cast(),
                );
                (self.fns.cmd_dispatch)(cmd, groups, 1, 1);
            }
            // The interp pass's model-out WRITES (COMPUTE/SHADER_WRITE, the dynamic slots of
            // the SHARED instance ring) are ordered before the raster VS's READS
            // (VERTEX/SHADER_READ) by the graph: it derives the COMPUTE→VERTEX RAW
            // `interp_model_out` barrier at the raster pass (the model_out reader), so
            // `record_graph_pass(plan.raster)` below emits it BEFORE the raster begins — still
            // AFTER this dispatch's write. NOT recorded here.
        }

        // === HW-RT rung R2a-3: the GPU-resident per-frame TLAS PACK + BUILD. Recorded ONLY when
        // the scene wires the activation (`scene.tlas.is_some()` — armed under hwrt + ray_query +
        // count > 0) AND the AS command table resolved (`accel_fns.is_some()`); otherwise skipped
        // entirely — NO bind, NO dispatch, NO build, NO barrier — so the command stream is
        // BYTE-IDENTICAL to the tlas-OFF path. Runs after interp, BEFORE the raster pass. The
        // pack pre-pass writes one 64-byte `VkAccelerationStructureInstanceKHR` per instance into
        // the device-local array (reading the shared M3 ring + the mesh-id lane + the BLAS-address
        // table); the graph derives the pack-WRITE → build-READ barrier; the build then builds the
        // TLAS into the UNTRACKED backing/scratch. Nothing traces the TLAS yet (R2a-4), so the
        // render stays byte-identical even when armed. ===
        #[cfg(feature = "hwrt")]
        if let (Some(t), Some(fns)) = (scene.tlas.as_ref(), accel_fns) {
            let pack_pass = plan
                .tlas_pack
                .expect("invariant: scene.tlas.is_some() ⇒ tlas_pack declared");
            let build_pass = plan
                .tlas_build
                .expect("invariant: scene.tlas.is_some() ⇒ tlas_build declared");
            // Pack: emit the graph's derived input barriers (interp→pack on the shared ring when
            // interp ran; else none), then bind the packer + dispatch `ceil(count / LOCAL_SIZE_X)`.
            // SAFETY: recording is open; `record_graph_pass` records the "tlas_pack" pass's derived
            // barriers into `cmd` against the live scene buffers.
            self.record_graph_pass(pack_pass, cmd, targets, scene, fi);
            let groups = t.count.div_ceil(LOCAL_SIZE_X);
            let push = t.count.to_le_bytes();
            // SAFETY: recording is open; the packer pipeline + its layout (declaring the 4-binding
            // pack set at set 0 + the 4-byte COMPUTE push range) are live on this device (caller
            // contract); `t.bind_group` binds this frame slot's { M3 ring @0, mesh-ids @1,
            // blas-addr @2, instance-array @3 }; `groups` covers `t.count` at the 64-wide group;
            // `&t.bind_group.descriptor_set` is a single-element local alive for the call
            // (first_set 0, count 1, zero dynamic offsets); the push is exactly
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
            // Build: emit the graph's derived pack-WRITE → build-READ barrier on the instance
            // array, then record the TLAS build into the UNTRACKED backing/scratch (the DDGI-update
            // discipline — the BARRIER is graph-emitted, only the GPU work is raw).
            // SAFETY: recording is open; `record_graph_pass` records the "tlas_build" pass's derived
            // barrier (pack COMPUTE/SHADER_WRITE → build AS_BUILD/SHADER_READ on `tlas_instances`).
            self.record_graph_pass(build_pass, cmd, targets, scene, fi);
            let entry = boyko_rhi::AsBuildEntry {
                kind: boyko_rhi::AsKind::Tlas,
                geometry: boyko_rhi::AsGeometryDesc {
                    vertex_data: t.instance_array_addr,
                    index_data: 0,
                    vertex_stride: 0,
                    max_vertex: 0,
                    primitive_count: t.count,
                    // A TLAS geometry ignores the index type (instance array, not triangles).
                    index_type: boyko_rhi::AsIndexType::Uint32,
                },
                scratch_address: t.scratch_addr,
            };
            // SAFETY: recording is open; `fns` is the live device's AS table (resolved from the RT
            // `ctx`); `entry`'s `vertex_data` (the pack-written instance array) + `scratch_address`
            // (aligned to `as_scratch_align` at create) + `t.dest.handle` (this slot's persistent
            // TLAS, its backing sized for `capacity >= count` at create) are live, correctly-flagged
            // resources; the pack→build barrier just recorded orders the instance-array write before
            // this build's read; `entry`/`dest` are 1-element slices that outlive the call.
            unsafe {
                crate::accel::cmd_build_acceleration_structures(
                    fns,
                    cmd,
                    core::slice::from_ref(&entry),
                    &[t.dest],
                );
            }
            // R2a-4b: order this TLAS build's AS write against the deferred resolve's `rayQuery`
            // read (the shadow trace at gbuffer.rs's resolve dispatch). The TLAS backing/scratch is
            // UNTRACKED by the framegraph (the build's AS write is invisible to the graph), so this
            // is a raw AS-write → AS-read global barrier — NOT a double-transition of any tracked
            // resource. Inside the same `if let (Some(t), Some(fns))` gate ⇒ the tlas-OFF path emits
            // NOTHING (byte-identical).
            // SAFETY: recording is open; `self.fns` is the live device's core command table (the
            // same table the pack bind/dispatch above used). The barrier touches no resource beyond
            // the execution/memory dependency (AS_BUILD stage → COMPUTE_SHADER stage).
            unsafe {
                crate::accel::cmd_acceleration_structure_barrier(self.fns, cmd);
            }
        }

        // SAFETY: recording is open; `record_graph_pass` records the graph's derived
        // barriers for the "raster" pass into `cmd` against the live G-buffer targets. When the
        // interp pass ran, this also emits the COMPUTE→VERTEX RAW barrier on the SHARED interp
        // model-out ring (the instance ring the raster VS reads).
        //
        // Multi-paradigm render-path plan, rung R2: `Some` iff `scene.path_has_raster()` (the
        // `debug_assert_eq!` above already pinned this) — `None` under `Deferred × Sdf` (rung R3;
        // `mesh_depth_neutral_clear` below is its depth-clear replacement), `Some` on every other
        // currently reachable frame, so the `if let` is byte-identical to the pre-R2
        // unconditional call there.
        if let Some(raster_pass) = plan.raster {
            self.record_graph_pass(raster_pass, cmd, targets, scene, fi);
        }

        // (2) Dynamic rendering at the marcher's extent: 3 MRT color attachments
        // (albedo@0, normal@1, material@2; CLEAR/STORE) + the depth attachment (CLEAR to
        // the far plane / STORE). The render area is the marcher's extent so the
        // rasterized fragments cover exactly the dispatched pixels; the swapchain may be
        // WSI-clamped wider (the present-blit handles that).
        //
        // Render P5-r0 / Decision r0-2: each color clear IS the marcher's mask=0 neutral
        // G-buffer, so a pixel with NO mesh fragment holds the cleared neutral, which the
        // marcher (owning that pixel) overwrites anyway — making the no-mesh 0%-gate
        // trivial AND a depth-failed/missed mesh fragment fall back to a valid mask=0
        // neutral. The clears pass through the SAME float→UNORM8 `round(c*255)` quantizer
        // the marcher store uses; 0.05/0.10/0.5/1.0/0.0 are all exact, so the cleared
        // neutral is bit-identical to a marcher-written neutral.
        //   albedo  clear = (BACKGROUND.rgb, 1.0)  — the marcher's background base.
        //   normal  clear = (0.5, 0.5, 0.0, 0.0)   — neutral oct + id=0.
        //   material clear = (1.0, 1.0, 0.0, 1.0)  — shadow=1, ao=1, mask=0, 1.
        // These MUST equal the marcher's background-arm constants (sdf_gbuffer_composite.hlsl:
        // BACKGROUND = (0.05, 0.05, 0.1); the Site-A/B mask=0 neutrals).
        let albedo_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: targets.albedo[fi].view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue {
                color: VkClearColorValue {
                    float32: [0.05, 0.05, 0.1, 1.0],
                },
            },
        };
        let normal_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: targets.normal[fi].view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue {
                color: VkClearColorValue {
                    float32: [0.5, 0.5, 0.0, 0.0],
                },
            },
        };
        let material_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: targets.material[fi].view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue {
                color: VkClearColorValue {
                    float32: [1.0, 1.0, 0.0, 1.0],
                },
            },
        };
        // HW-RT Rung 3b step 5a: decide whether this frame uses the MESH motion-vector pipeline (a
        // 4th MRT writing Δuv). ON iff temporal is enabled AND the MV pipeline + bind group exist
        // (an RT + storage device). OFF (the default / non-hwrt build) ⇒ the base 3-MRT raster ⇒
        // byte-identical. Evaluated ONCE; drives the attachment count, the color-array ptr, the
        // pipeline/layout, and the set-0 bind below.
        // `mesh_mv_active()` is the SINGLE source shared with `declare_deferred_graph` (W1: the
        // barrier declaration and this write must never disagree).
        #[cfg(feature = "hwrt")]
        let mv_active = scene.mesh_mv_active();
        #[cfg(not(feature = "hwrt"))]
        let mv_active = false;
        // Asset-streaming plan F8: decide whether this frame uses the PER_INSTANCE_MATERIAL
        // pipeline. Present on BOTH cfg legs (materials are device-agnostic, unlike `mv`) —
        // `mesh_pm_active()` is the SINGLE source shared with the pipeline/set selection below.
        // MV takes priority over PM (F8 §2.3) UNLESS both are active, in which case the
        // combined mvpm pipeline (below) renders BOTH correctly (F8-mv).
        let pm_active = scene.mesh_pm_active();
        // F8-mv: decide whether this frame uses the COMBINED MOTION_VECTORS +
        // PER_INSTANCE_MATERIAL pipeline. `mesh_mvpm_active()` is the SINGLE source shared with
        // the pipeline/set selection below; it is a strict AND of `mv_active`/`pm_active`'s
        // gates plus the mvpm pipeline/bind-group presence, so it can only be true when both
        // would otherwise fire. Non-hwrt build: `false` (mvpm is an MV extension, hwrt-only).
        #[cfg(feature = "hwrt")]
        let mvpm_active = scene.mesh_mvpm_active();
        #[cfg(not(feature = "hwrt"))]
        let mvpm_active = false;
        // Textured-PBR T6c: decide whether this frame uses the TEXTURED pipeline. Present on
        // BOTH cfg legs (materials/textures are device-agnostic, like `pm`) — `mesh_tex_active()`
        // is the SINGLE source shared with `declare_deferred_graph`'s `pbr` write declaration
        // (W1). `mesh_tex_active()` is ALREADY `false` whenever `mv_active` holds (T6c plan
        // Decision D4: TEXTURED is never compiled with MOTION_VECTORS), so this tier check needs
        // no explicit `!mv_active` guard of its own.
        let tex_active = scene.mesh_tex_active();
        // T6c plan Decision D4: under an active MV/mvpm frame, a textured material renders
        // base_color/scalar via the MV/mvpm pipeline instead of sampled textures (`tex_active`
        // above is false). Warn ONCE per process (not every frame — Principle 1, avoid I-cache/
        // hot-path bloat + stderr spam) so the suppression is visible without a per-frame cost.
        if mv_active && scene.tex_enabled {
            warn_textured_suppressed_by_motion_vectors();
        }
        // The 4th MRT: the motion_vec Δuv target (R16G16Sfloat), CLEAR to (0,0) / STORE — a pixel
        // with no mesh fragment holds zero motion (the marcher overwrites SDF pixels in step 5b).
        // Built unconditionally so it outlives `cmd_begin_rendering`; the driver reads it ONLY when
        // `color_attachment_count == 4` (the `mv_active` branch). On the OFF path its `image_view`
        // is NULL and it is never read (count stays 3), so no motion_vec target need exist.
        // `mv_active` (⇒ `raster_pipeline_mv.is_some()`) implies the MV boot gate held
        // (`ray_query_enabled() && shadow_denoise_storage_ok()`), a strict superset of the
        // `motion_vec` target gate (`shadow_denoise_storage_ok()`), so the target MUST exist here.
        // `expect` (not `unwrap_or(NULL)`) so a future loosening of the MV gate trips loudly rather
        // than binding a NULL 4th color attachment (O1).
        #[cfg(feature = "hwrt")]
        let motion_vec_view = if mv_active {
            targets
                .motion_vec
                .as_ref()
                .map(|r| r[fi].view)
                .expect("invariant: mesh_mv_active implies the motion_vec target was allocated")
        } else {
            VkImageView::NULL
        };
        #[cfg(not(feature = "hwrt"))]
        let motion_vec_view = VkImageView::NULL;
        let motion_vec_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: motion_vec_view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue {
                color: VkClearColorValue { float32: [0.0, 0.0, 0.0, 0.0] },
            },
        };
        // Textured-PBR T6c: the 4th MRT under the TEXTURED path — `gPbr`
        // (`R16G16B16A16_SFLOAT`), CLEAR to the T6a neutral (metallic 0, roughness 0.5, ao 1,
        // emissive 1) / STORE. Mutually exclusive with `motion_vec_attachment` above (TEXTURED
        // is never compiled with MOTION_VECTORS, T6c plan Decision D4), so at most one of the
        // two is ever selected into the 4th array slot below. `image_view` is NULL (present-
        // but-unread) when `tex_active` is false.
        let pbr_view = if tex_active { targets.pbr[fi].view } else { VkImageView::NULL };
        let pbr_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: pbr_view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue {
                color: VkClearColorValue { float32: [0.0, 0.5, 1.0, 1.0] },
            },
        };
        // The color-attachment array is ALWAYS 4 elements (so the ptr is valid for both counts and
        // the array outlives the bracketed calls — the lifetime caution). `color_attachment_count`
        // selects 3 (base) vs 4 (MV or TEXTURED); on the base path the 4th element is
        // present-but-unread. `mv_active`/`tex_active` are mutually exclusive (D4), so the 4th
        // slot picks EITHER the motion_vec OR the pbr attachment, never a mix of the two.
        let fourth_attachment = if mv_active { motion_vec_attachment } else { pbr_attachment };
        let raster_color_attachments = [
            albedo_attachment,
            normal_attachment,
            material_attachment,
            fourth_attachment,
        ];
        let color_attachment_count: u32 = if mv_active || tex_active { 4 } else { 3 };
        let depth_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: targets.depth[fi].view,
            image_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue {
                depth_stencil: VkClearDepthStencilValue {
                    depth: 1.0,
                    stencil: 0,
                },
            },
        };
        let raster_area = VkRect2D {
            offset: VkOffset2D { x: 0, y: 0 },
            extent: present_extent,
        };
        let raster_rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: raster_area,
            layer_count: 1,
            view_mask: 0,
            color_attachment_count,
            p_color_attachments: raster_color_attachments.as_ptr(),
            p_depth_attachment: (&depth_attachment as *const VkRenderingAttachmentInfo).cast(),
            p_stencil_attachment: ptr::null(),
        };
        let raster_viewport = VkViewport {
            x: 0.0,
            y: 0.0,
            width: present_extent.width as f32,
            height: present_extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let vertex_offset: VkDeviceSize = 0;
        // HW-RT Rung 3b step 5a: select the pipeline + its set-0 bind group. When `mv_active`, bind
        // the MESH motion-vector pipeline (its own 3-binding set-0 layout: current @0 / prev @1 /
        // motion-cam @2) + this frame's MV bind group; else the base raster pipeline + the shared
        // instance bind group (byte-identical). Both pipelines carry `.pipeline` + `.layout`; the
        // push (88 B) + the per-batch `base_instance` re-push are UNCHANGED across both.
        // Asset-streaming plan F8 §2.3 / F8-mv: the `pm_active`/`mvpm_active` arms are present
        // on BOTH cfg legs (materials are device-agnostic); only the `mv_active` arm is
        // cfg-gated (`mvpm_active` itself resolves to `false` on a non-hwrt build, so its arm
        // never fires there). Priority mvpm > mv > pm > base: `mvpm_active` implies both
        // `mv_active` and `pm_active` would otherwise fire, so checking it FIRST renders both
        // deltas together instead of falling into the mv-only (default-material) arm.
        let raster_pipeline = if mvpm_active {
            #[cfg(feature = "hwrt")]
            {
                scene
                    .raster_pipeline_mvpm
                    .expect("invariant: mvpm_active implies raster_pipeline_mvpm is Some")
            }
            #[cfg(not(feature = "hwrt"))]
            {
                scene.raster_pipeline
            }
        } else if mv_active {
            #[cfg(feature = "hwrt")]
            {
                scene
                    .raster_pipeline_mv
                    .expect("invariant: mv_active implies raster_pipeline_mv is Some")
            }
            #[cfg(not(feature = "hwrt"))]
            {
                scene.raster_pipeline
            }
        } else if tex_active {
            scene
                .raster_pipeline_tex
                .expect("invariant: tex_active implies raster_pipeline_tex is Some")
        } else if pm_active {
            scene
                .raster_pipeline_pm
                .expect("invariant: pm_active implies raster_pipeline_pm is Some")
        } else {
            scene.raster_pipeline
        };
        let raster_set = if mvpm_active {
            #[cfg(feature = "hwrt")]
            {
                scene
                    .mvpm_bind_group
                    .expect("invariant: mvpm_active implies mvpm_bind_group is Some")
            }
            #[cfg(not(feature = "hwrt"))]
            {
                scene.instance_bind_group
            }
        } else if mv_active {
            #[cfg(feature = "hwrt")]
            {
                scene
                    .mv_bind_group
                    .expect("invariant: mv_active implies mv_bind_group is Some")
            }
            #[cfg(not(feature = "hwrt"))]
            {
                scene.instance_bind_group
            }
        } else if tex_active {
            scene
                .tex_bind_group
                .expect("invariant: tex_active implies tex_bind_group is Some")
        } else if pm_active {
            scene
                .pm_bind_group
                .expect("invariant: pm_active implies pm_bind_group is Some")
        } else {
            scene.instance_bind_group
        };
        // SAFETY: recording is open; `raster_rendering` is fully initialized — its 3 color
        // attachments name the live albedo/normal/material views (now
        // COLOR_ATTACHMENT_OPTIMAL) and its depth attachment the live depth view (now
        // DEPTH_ATTACHMENT_OPTIMAL); `raster_color_attachments` outlives the bracketed
        // calls; dynamic rendering is enabled on this device. The raster pipeline (declaring
        // 3 matching color formats + 3 blend states, P5-r0) + its VERTEX push range + the
        // vertex buffer all belong to this device (caller contract) and the pipeline's
        // declared color/depth formats equal the bound attachments'. The 88-byte push is
        // `GBUFFER_PUSH_BYTES` at offset 0 into the VERTEX range (M1: its trailing
        // `use_model_matrix` selects the VS arm — `0` legacy / `1` instanced). A VALID set 0
        // is bound before the draw to satisfy the VS's static `instances` reference:
        // `scene.instance_bind_group` (the shared N-instance SSBO — the 1-element identity
        // dummy on the legacy empty-slice arm, the gather-filled ring on the M3 instanced
        // arm), bound ONCE for both arms. Asset-streaming plan F8: when `pm_active`, set 0
        // instead binds the PM group's TWO bindings — `instances[s]` @0 (the SAME
        // gather-filled model ring) + `instance_materials[s]` @1 (the gather-filled,
        // OOB-clamped id ring); both buffers are live (boot-minted or F7/F8-grown) and, on
        // any grow, `grow_instance_family_if_needed` rebound BOTH the PM set's @0 and @1
        // against slot `s`'s fence-waited buffers (F8 §7i), so neither descriptor points at
        // a freed buffer. F8-mv: when `mvpm_active`, set 0 instead binds the combined group's
        // FOUR bindings (`instances[s]` @0, `prev_instances[s]` @1, `MotionCam[s]` @2,
        // `instance_materials[s]` @3) and the pipeline declares 4 color formats matching the
        // 4-attachment `raster_rendering` (`color_attachment_count == 4` via `mv_active`,
        // which `mesh_mvpm_active()` implies). All four are live, `INSTANCE_CAPACITY`-fixed
        // rings on the RT leg (`grow_instance_family_if_needed`'s W3 gate never grows them
        // there, so they are never rebound and never dangle). Textured-PBR T6c: when
        // `tex_active`, set 0 instead binds the TEX group's TWO bindings — `instances[s]` @0
        // (the SAME gather-filled model ring) + `instance_materials_tex[s]` @1 (the
        // gather-filled `PerInstanceMaterialTex` ring) — AND, immediately after, set 1 is
        // ALSO bound to the bindless texture-array descriptor SET (`scene.bindless_set`, a
        // live `VkDescriptorSet` allocated by `BindlessTextureTable::new` and never
        // destroyed before this point — its owning `BindlessTextureTable` outlives every
        // frame until the runner's teardown); the pipeline's LAYOUT already declares this
        // set (built via `create_graphics_pipeline_bindless` at boot), so this bind's
        // `first_set = 1` matches a real layout slot. `raster_pipeline.layout` is the SAME
        // 2-set layout in that case, and the pipeline declares 4 color formats (3 base +
        // `gPbr`) matching the 4-attachment `raster_rendering` (`color_attachment_count ==
        // 4` via `tex_active`). `vertex_offset`/
        // `raster_viewport`/`raster_area` locals outlive the bracketed calls. On the legacy arm
        // `draw(vertex_count, 1, 0, 0)` reads the merged vertex buffer; on the M3 arm the
        // batch loop re-pushes each batch's
        // `base_instance` (4 bytes at offset 80, in-range of the declared 88-byte VERTEX push)
        // then `draw_indexed(index_count, instance_count, 0, 0, 0)` reads that batch's bound
        // vertex + index buffers (created on this device, carrying VERTEX/INDEX usage;
        // `index_type` a valid `VkIndexType`). Begin/End bracket pass A exactly.
        //
        // Multi-paradigm render-path plan, rung R2 (Decision 2 / O1): the whole raster
        // begin/end-rendering block is gated on `plan.raster.is_some()` — the SAME gate the
        // raster barriers use, single-sourced from `scene.path_has_raster()` at declare time
        // (the `debug_assert_eq!` at this fn's top guards declare/record never diverging;
        // review R2/P2-2: one gate expression per leg, mirroring the marcher). Under the R2
        // resolver guard this is `Some` on every currently reachable frame, so the `if` is
        // byte-identical to the pre-R2 unconditional block.
        if plan.raster.is_some() {
            unsafe {
                (self.fns.cmd_begin_rendering)(cmd, &raster_rendering);
                (self.fns.cmd_bind_pipeline)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_GRAPHICS,
                    raster_pipeline.pipeline,
                );
                // M3: the instanced batch loop binds the SHARED N-instance model SSBO ONCE
                // (set 0); the legacy (empty-slice) arm binds the 1-element identity dummy
                // (bound-but-unread). Both bind a VALID set 0 so the VS's static `instances`
                // reference is satisfied. The shared SSBO is `scene.instance_bind_group` for
                // both arms (M3 repurposed it as the gather-filled N-instance ring on the
                // instanced path); every batch indexes it by `base_instance + SV_InstanceID`.
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_GRAPHICS,
                    raster_pipeline.layout,
                    0,
                    1,
                    &raster_set.descriptor_set,
                    0,
                    ptr::null(),
                );
                // Textured-PBR T6c: when the TEXTURED pipeline is selected, ALSO bind the bindless
                // texture-array descriptor SET at set 1 (FRAGMENT-visible) — its LAYOUT is already
                // baked into `raster_pipeline.layout` at boot via
                // `VulkanContext::create_graphics_pipeline_bindless`, so this is purely a per-frame
                // set bind, mirroring the set-0 bind immediately above. `bindless_set` is a local so
                // `&bindless_set` is a valid single-element pointer for the call.
                if tex_active {
                    let bindless_set = scene
                        .bindless_set
                        .expect("invariant: tex_active implies bindless_set is Some");
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_GRAPHICS,
                        raster_pipeline.layout,
                        1,
                        1,
                        &bindless_set,
                        0,
                        ptr::null(),
                    );
                }
                (self.fns.cmd_push_constants)(
                    cmd,
                    raster_pipeline.layout,
                    VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
                    0,
                    scene.mvp.len() as u32,
                    scene.mvp.as_ptr().cast(),
                );
                (self.fns.cmd_set_viewport)(cmd, 0, 1, &raster_viewport);
                (self.fns.cmd_set_scissor)(cmd, 0, 1, &raster_area);
                if scene.mesh_draw.is_empty() {
                    // LEGACY arm: byte-identical to the pre-M2 stream — a non-indexed,
                    // single-instance draw over the scene's merged vertex buffer. The shared
                    // set 0 + the `use_model_matrix == 0` push (caller contract) make the bound
                    // SSBO bound-but-unread.
                    (self.fns.cmd_bind_vertex_buffers)(
                        cmd,
                        0,
                        1,
                        &scene.vertex_buffer.buffer,
                        &vertex_offset,
                    );
                    (self.fns.cmd_draw)(cmd, scene.vertex_count, 1, 0, 0);
                } else {
                    // M3 INSTANCED batch loop: one indexed draw per registered mesh. `scene.
                    // mvp`'s `use_model_matrix == 1` (caller contract) selects the VS arm that
                    // reads `instances[base_instance + SV_InstanceID]`. Each batch overwrites
                    // the push's `base_instance` word (offset 80, 4 bytes) with its bucket
                    // offset — NONZERO for every mesh after the first (the C1 proof) — then
                    // binds its own vertex+index buffers (with its O3 index width) and draws
                    // its instance bucket.
                    for batch in scene.mesh_draw {
                        let base = batch.base_instance;
                        (self.fns.cmd_push_constants)(
                            cmd,
                            raster_pipeline.layout,
                            VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
                            GBUFFER_PUSH_BASE_INSTANCE_OFFSET,
                            4,
                            (&base as *const u32).cast(),
                        );
                        (self.fns.cmd_bind_vertex_buffers)(
                            cmd,
                            0,
                            1,
                            &batch.vertex_buffer.buffer,
                            &vertex_offset,
                        );
                        (self.fns.cmd_bind_index_buffer)(
                            cmd,
                            batch.index_buffer.buffer,
                            0,
                            batch.index_type,
                        );
                        (self.fns.cmd_draw_indexed)(
                            cmd,
                            batch.index_count,
                            batch.instance_count,
                            0,
                            0,
                            0,
                        );
                    }
                }
                (self.fns.cmd_end_rendering)(cmd);
            }
        }

        // Multi-paradigm render-path plan, rung R3 (§E leg-disable / the O2 audit finding): the
        // mesh-depth NEUTRAL CLEAR — `Deferred × Sdf`'s replacement for the raster pass's own
        // depth-clear producer (see `mesh_depth_neutral_clear`'s doc in `graph_bridge.rs` +
        // `GBufferScene::path_has_mesh_depth_neutral_clear`'s doc for the full rationale). `Some`
        // iff `scene.path_has_mesh_depth_neutral_clear()`, mutually exclusive with `plan.raster`
        // by construction (W1 parity, the SAME predicate `declare_deferred_graph` checks).
        debug_assert_eq!(
            plan.mesh_depth_neutral_clear.is_some(),
            scene.path_has_mesh_depth_neutral_clear(),
            "W1: declare/record predicate desync (mesh_depth_neutral_clear)"
        );
        if let Some(depth_clear_pass) = plan.mesh_depth_neutral_clear {
            self.record_graph_pass(depth_clear_pass, cmd, targets, scene, fi);
            let depth_only_attachment = VkRenderingAttachmentInfo {
                s_type: VkStructureType::RenderingAttachmentInfo,
                p_next: ptr::null(),
                image_view: targets.depth[fi].view,
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
            let depth_only_area =
                VkRect2D { offset: VkOffset2D { x: 0, y: 0 }, extent: present_extent };
            let depth_only_rendering = VkRenderingInfo {
                s_type: VkStructureType::RenderingInfo,
                p_next: ptr::null(),
                flags: 0,
                render_area: depth_only_area,
                layer_count: 1,
                view_mask: 0,
                color_attachment_count: 0,
                p_color_attachments: ptr::null(),
                p_depth_attachment: (&depth_only_attachment as *const VkRenderingAttachmentInfo)
                    .cast(),
                p_stencil_attachment: ptr::null(),
            };
            // SAFETY: recording is open; `depth_only_rendering` names only the live depth view
            // (now `DEPTH_ATTACHMENT_OPTIMAL`, transitioned by the barrier
            // `record_graph_pass` just emitted) with `color_attachment_count == 0` (no color
            // attachment array is needed — `p_color_attachments` is a valid null ptr for a
            // zero-length array per the Vulkan spec); dynamic rendering is enabled on this
            // device (the SAME capability every other `cmd_begin_rendering` call in this fn
            // relies on); `depth_only_area` is within the depth image's extent (==
            // `present_extent`, matching every other G-buffer target this frame). No draw is
            // recorded — the clear alone (LOAD_OP_CLEAR/STORE_OP_STORE, depth = 1.0) leaves the
            // whole image at the far-plane sentinel, reproducing the raster pass's OWN depth
            // clear value exactly (`MESH_DEPTH_CLEAR` / `DEPTH_CLEAR` == 1.0), so the
            // byte-UNCHANGED marcher reads "no mesh" for every pixel this frame.
            unsafe {
                (self.fns.cmd_begin_rendering)(cmd, &depth_only_rendering);
                (self.fns.cmd_end_rendering)(cmd);
            }
        }

        // Multi-paradigm render-path plan, rung R3b (§E leg-disable / the R3 audit finding): the
        // `viewt_from_depth` `gViewT`-producer — `Deferred × Mesh`'s replacement for the
        // (undispatched) marcher's `gViewT` write (see `viewt_from_depth`'s doc in
        // `graph_bridge.rs` + `ViewtFromDepthActivation`'s doc for the full rationale). `Some`
        // iff `scene.viewt_from_depth.is_some()`, mutually exclusive with
        // `plan.mesh_depth_neutral_clear` by construction (W1 parity: `plan.viewt_from_depth`
        // and `scene.viewt_from_depth` are TWO separate `Option`s that must move in lock-step —
        // `declare_deferred_graph` derives the former directly from the latter).
        debug_assert_eq!(
            plan.viewt_from_depth.is_some(),
            scene.viewt_from_depth.is_some(),
            "W1: declare/record predicate desync (viewt_from_depth)"
        );
        if let Some(viewt_from_depth_pass) = plan.viewt_from_depth {
            self.record_graph_pass(viewt_from_depth_pass, cmd, targets, scene, fi);
            let activation = scene
                .viewt_from_depth
                .as_ref()
                .expect("invariant: plan.viewt_from_depth.is_some() ⇒ scene.viewt_from_depth.is_some() (W1)");
            let push = ViewtFromDepthPush::new(
                present_extent.width,
                present_extent.height,
                activation.mesh_view_t_norm,
            );
            let push_bytes = push.as_bytes();
            let set = &targets
                .viewt_from_depth_set
                .as_ref()
                .expect(
                    "invariant: scene.viewt_from_depth.is_some() ⇒ GBufferTargets::create wrote viewt_from_depth_set",
                )[self.frame_index];
            let group_x = present_extent.width.div_ceil(8);
            let group_y = present_extent.height.div_ceil(8);
            // SAFETY: recording is open; `activation.pipeline` + its layout (declaring
            // `activation.layout` at set 0 AND the 12-byte COMPUTE push range) are live on this
            // device (caller contract); `set` binds the now-transitioned depth (SHADER_READ, by
            // the `record_graph_pass` call above) + `gViewT` (GENERAL) images; `group_x`/`group_y`
            // cover `present_extent` (the 8×8-tile ceiling of the SAME extent the depth/gViewT
            // images are sized to); `&set.descriptor_set` is a single-element local alive for the
            // call (first_set 0, count 1, zero dynamic offsets); `push_bytes` is
            // `VIEWT_FROM_DEPTH_PUSH_BYTES` (12) bytes at offset 0, exactly the declared push
            // range, and the backing `push` local outlives the call.
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
                    VIEWT_FROM_DEPTH_PUSH_BYTES,
                    push_bytes.as_ptr().cast(),
                );
                (self.fns.cmd_dispatch)(cmd, group_x, group_y, 1);
            }
        }

        // The marcher's INPUT barriers — depth→sampled, color→general, lit/viewt/ssao
        // first-touch — are DRIVEN by the graph's "marcher" pass, but that `record_pass`
        // is emitted just BEFORE the marcher DISPATCH (site 5), NOT here. This is
        // REQUIRED for the coarse-ON case: the graph derives the `tiles` cull→marcher
        // barrier at the marcher (the reader), so it must fire AFTER the coarse dispatch
        // WRITES tiles — recording the marcher pass here (before the coarse dispatch)
        // would order a not-yet-issued write. So this site records NOTHING; the graph
        // re-orders lit/ssao's first-touch to their true first-use (resolve / ssao) — a
        // sound superset the equivalence tests lock in.

        // === Lighting L0-r0: ASYNC light-table re-upload (C3), recorded only on a dirty
        // frame, BEFORE the marcher/resolve reads. The graph-driven pre-copy barrier +
        // a staging→device `cmd_copy_buffer` into the SAME `cmd` — fence-free, no
        // readback. An idle (non-dirty) frame records NOTHING — byte-identical command
        // stream to before (the rung L0-r0 0%-gate). The collection system wrote the new
        // table into `light_staging`'s mapped bytes and set `light_dirty`. ===
        if scene.light_dirty && scene.light_upload_bytes > 0 {
            // The barrier group in the "light_upload" pass's range is the CROSS-FRAME
            // SEED-WAR: src = the SIBLING in-flight frame's still-pipelined
            // COMPUTE_SHADER/SHADER_READ of the light table (the
            // `add_buffer_seeded(seeded_readers(COMPUTE_SHADER, SHADER_READ))`
            // declaration — the table is a SINGLE instance shared by both frames),
            // dst = this copy's TRANSFER write. A `vkCmdPipelineBarrier` orders only
            // commands recorded BEFORE it against commands recorded AFTER it, so this
            // group MUST be emitted BEFORE the copy below (every other pass emits its
            // input barriers before its work — coarse, csm; this site had them
            // inverted, review R4-C1): emitted after the copy, the copy could race
            // frame N−1's in-flight resolve read (torn header/table bytes — and the D5
            // generation protocol makes dirty frames arrive in consecutive slot pairs,
            // so the race window was COMMON, not exotic). The TRANSFER_WRITE→
            // SHADER_READ flush is NOT in this group: the graph derives it at the
            // READERS' own ranges (the marcher/resolve passes) and their
            // `record_graph_pass` calls emit it before their dispatches.
            // SAFETY: recording is open; `record_graph_pass` records the graph's
            // derived COMPUTE→TRANSFER seed-WAR buffer barrier for the "light_upload"
            // pass into `cmd`, ahead of the copy it guards. The pass gate here is the
            // SAME predicate that declared the pass, so the plan slot is `Some`.
            let plan = self
                .gbuffer_pass_plan
                .as_ref()
                .expect("invariant: declare_frame_graph ran before record_gbuffer");
            let light_upload = plan
                .light_upload
                .expect("invariant: light_dirty ⇒ light_upload pass declared");
            self.record_graph_pass(light_upload, cmd, targets, scene, fi);

            let region = VkBufferCopy {
                src_offset: 0,
                dst_offset: 0,
                size: scene.light_upload_bytes,
            };
            // SAFETY: recording is open; the copy names the live host-coherent staging +
            // device-local table buffers; the copy region spans `[0, light_upload_bytes)`
            // ≤ both buffer sizes (caller contract — the table is sized for MAX_LIGHTS).
            // The seed-WAR barrier recorded ABOVE orders this transfer write after the
            // sibling frame's pipelined table reads; the readers' own graph passes order
            // the marcher/resolve reads after this write. `&region` outlives the call.
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

        // === Render P0: the P4b COARSE-CULL pass (Decision: mirror the offscreen
        // `run_gbuffer_hybrid_ex` coarse dispatch + the `cluster_cull` optional-compute
        // recorder shape). Recorded ONLY when the scene wires the coarse pipeline; otherwise
        // skipped entirely — NO dispatch, NO barrier — so the command stream is byte-identical
        // to the pre-P0 windowed path (the 0%-gate). The coarse pass binds the SAME vocabulary
        // set (the cull shader declares only a subset — valid), SAMPLES the depth (already
        // SHADER_READ from barrier 3, which it shares with the marcher), and WRITES one
        // `TileBound` per 8×8 tile into vocab binding 6. The fine marcher then READS those
        // bounds (gated by `coarse_enabled == 1` in its push) to skip empty / cone-rejected
        // tiles — the SAME pixels, fewer marches. A COMPUTE→COMPUTE buffer barrier on
        // `tiles_buffer` orders the cull WRITE before the marcher READ. ===
        let coarse_enabled = scene.coarse.is_some();
        if let Some(coarse_pipeline) = scene.coarse {
            // The 1D coarse dispatch element count = the full tile grid at the COMPOSITE
            // extent (the marcher dispatches + the camera UBO `count` are sized to it). One
            // group per `LOCAL_SIZE_X` tiles, mirroring the offscreen `coarse_group_count_x`.
            let (tw, th) = tile_grid_extent(present_extent.width, present_extent.height);
            let coarse_groups = (tw * th).div_ceil(LOCAL_SIZE_X);
            // The coarse pass's INPUT barrier (depth→sampled — the coarse pass is the
            // graph's FIRST COMPUTE depth reader, so it owns the
            // DEPTH_ATTACHMENT_OPTIMAL→SHADER_READ_ONLY transition) is DRIVEN by the
            // graph's "coarse" pass, recorded HERE, immediately before the coarse dispatch
            // that samples depth. The `tiles` cull→marcher barrier is derived at the
            // marcher (the reader), so it is emitted later by `record_pass(marcher)`
            // (after this dispatch writes tiles) — NOT here.
            // SAFETY: recording is open; `record_graph_pass` records the graph's derived
            // depth→sampled barrier for the "coarse" pass into `cmd`.
            let coarse = self
                .gbuffer_pass_plan
                .as_ref()
                .expect("invariant: declare_frame_graph ran before record_gbuffer")
                .coarse
                .expect("invariant: scene.coarse.is_some() ⇒ coarse pass declared");
            self.record_graph_pass(coarse, cmd, targets, scene, fi);
            // SAFETY: recording is open; the coarse pipeline + its layout (declaring
            // `vocab_layout` at set 0 + the shared COMPUTE push range) are live on this device
            // (caller contract); the vocabulary set binds the SSBO/UBO + the now-transitioned
            // depth (SHADER_READ) + a valid Tiles SSBO @6 (the cull's write target) + the valid
            // brick descriptors @9..=14; the cull shader uses only a subset of those bindings
            // (valid); `coarse_groups` covers the full tile grid at the 64-wide group;
            // `&...vocab_set.descriptor_set` is a single-element local alive for the call
            // (first_set 0, count 1, zero dynamic offsets). The cull declares no push it reads,
            // but the layout's push range matches the marcher's, so no constant is pushed here.
            unsafe {
                (self.fns.cmd_bind_pipeline)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    coarse_pipeline.pipeline,
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    coarse_pipeline.layout,
                    0,
                    1,
                    &targets.vocab_set[self.frame_index].descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_dispatch)(cmd, coarse_groups, 1, 1);
            }

            // The coarse pass's `TileBound` WRITES (binding 6, COMPUTE/SHADER_WRITE) are
            // ordered before the fine marcher's READS (COMPUTE/SHADER_READ) by the graph:
            // it derives this COMPUTE→COMPUTE `tiles_buffer` barrier at the marcher (the
            // `tiles` reader), so `record_pass(marcher)` emits it just before the marcher
            // dispatch (still after this coarse dispatch's write) — NOT here.
        }

        // === Pass B: the marcher SAMPLES the depth image, STORES the G-buffer. ===
        // (5) Bind the marcher + the vocabulary set (written ONCE at sync_gbuffer; NO
        // per-frame update) against the marcher's OWN dedicated layout, push the 32-byte
        // P4b/B1 constants, dispatch.
        //
        // The marcher's 32-byte compute push range is `FineMarcherPush`
        // `{ coarse_enabled: u32 @0, omega: f32 @4, lighting_flags: u32 @8, light_dir: float3 @16 }`.
        // Render P0: `coarse_enabled` is a 3-value `CoarseMode` — `0` on the OFF path (no coarse
        // dispatch, the tile read is gated off), else `scene.coarse_mode`: `1` (full = EMPTY-skip +
        // `near_t` seed) or `2` (empty-skip-only = EMPTY-skip, NO seed → lit-transparent, no rim).
        // When the cull pass above ran the marcher reads the per-tile bounds it wrote into binding 6
        // (skipping empty tiles). Either way the marcher DECLARES binding 6, so the (valid) Tiles descriptor
        // is always bound in the vocabulary set. `omega` carries the B1 over-relaxation
        // factor (`DEFAULT_MARCHER_OMEGA`, the provably hole-free speedup). Render A1/A2:
        // the on-screen demo turns lighting ON (A1 soft shadows + A2 AO) with the default
        // directional light.
        // SDF brick-cache activation (campaign M1/M2/M4): the empty-skip + trilinear/cubic surface
        // cache + clip-map LOD gates live ENTIRELY in this per-frame push (the bound descriptors at
        // 9..=14 are static), so `scene.brick` selects ON/OFF at runtime with no re-record — the
        // owner's A/B toggle.
        //
        // - `None` (the default / OFF path): `brick_enabled == 0` / `brick_trilinear == 0` /
        //   `brick_levels == 1` — the marcher's `select_level` loops once over level 0 and never reads
        //   the brick grids/atlas, byte-identical to the pre-brick M2 marcher. `with_brick_levels(1)`
        //   is REQUIRED (the recompiled shader treats `brick_levels == 0` as no-level).
        // - `Some(a)` (the ON path): `with_brick(a.grid_origin, a.grid_dims, a.brick_world)` stamps the
        //   level-0 empty-skip grid uniforms (the `lvl == 0` arm indexes binding 9 with them),
        //   `with_brick_trilinear(true)` turns on the surface-brick cubic, and `with_brick_levels(a.levels)`
        //   loops the clip-map ladder. The caller MUST have bound the real BrickClipmap per-level
        //   resources at 9..=14 + written its `M4GridParams` tail into the b5 UBO. This mirrors the
        //   offscreen RTX-verified `run_gbuffer_hybrid_m4` push exactly.
        // Render P0: the marcher's coarse-cull mode. OFF (no `coarse` pipeline ⇒ no dispatch) forces
        // `CoarseMode::Off` so the push byte is 0 and the marcher never reads the (un-dispatched)
        // tile bounds — byte-identical to the pre-P0 stream. ON uses `scene.coarse_mode`: `Full`
        // keeps the historical EMPTY-skip + `near_t` seed (the offscreen goldens' mode);
        // `EmptySkipOnly` is the LIT-TRANSPARENT on-screen cull (EMPTY-skip only, no seed → no
        // grazing-silhouette AO/shadow rim).
        let coarse_mode = if coarse_enabled { scene.coarse_mode } else { CoarseMode::Off };
        let base = FineMarcherPush::new_mode(
            coarse_mode,
            DEFAULT_MARCHER_OMEGA,
            scene.lighting_flags,
            // The marcher marches the A1 soft shadow toward the SCENE's primary directional `L`
            // (NOT a hardcoded head-on `[0,0,1]`), so an angled sun casts a real shadow that the
            // resolve's primary directional then consumes via `gMaterial.r`. See `light_dir`.
            scene.light_dir,
        );
        let marcher_push = match scene.brick {
            Some(a) => base
                .with_brick(a.grid_origin, a.grid_dims, a.brick_world)
                .with_brick_trilinear(true)
                .with_brick_levels(a.levels),
            None => base.with_brick_levels(1),
        }
        // MDF Stage-2c: arm the mesh-distance-field SHADOW path. `false` (the default for every
        // non-MDF scene) leaves the push byte-identical — the mesh-SDF texture @binding 15 is
        // bound-but-unread, the shadow march stays the frozen analytic `sdf_soft_shadow` (the
        // 0%-gate keeping the 41 hybrid goldens byte-exact). `true` marches `sdf_soft_shadow_mesh`.
        .with_mesh_sdf(scene.mesh_sdf_enabled);
        // SAFETY: recording is open; the marcher pipeline + its layout (declaring
        // `vocab_layout` at set 0 AND the 80-byte COMPUTE push range) are live on this
        // device (caller contract); the vocabulary set binds the SSBO/UBO + the
        // now-transitioned depth (SHADER_READ) + G-buffer (GENERAL) images + a valid
        // Tiles SSBO @6 + valid brick descriptors @9..=14 (whether the brick gates are ON
        // or OFF, those descriptors are always bound — caller contract); `dispatch_group_count_x`
        // covers `present_extent`'s pixel count (the G-buffer images + dispatch grid + camera UBO
        // `count` are all sized to `present_extent`, the composite — NOT the swapchain `extent`;
        // caller contract); `&...descriptor_set` is a single-element local alive for the call
        // (first_set 0, count 1, zero dynamic offsets); `marcher_push.as_bytes()` is
        // `GBUFFER_MARCHER_PUSH_BYTES` (80) bytes at offset 0, exactly the declared 80-byte range,
        // and the backing `marcher_push` local outlives the call.
        let marcher_push_bytes = marcher_push.as_bytes();
        // The marcher's INPUT barriers are DRIVEN by the graph's "marcher" pass, recorded
        // HERE, immediately before the marcher dispatch (the collapse of the former hand
        // depth→sampled / color→general / lit·viewt·ssao first-touch sites). It emits
        // color→general + viewt first-touch UNDEFINED→GENERAL and, on the coarse-ON path,
        // the `tiles` cull→marcher COMPUTE→COMPUTE barrier (correctly AFTER the coarse
        // dispatch wrote tiles). depth→sampled is free here (the coarse pass — or,
        // coarse-OFF, this pass would own it: the graph derives it wherever depth is first
        // COMPUTE-read).
        // SAFETY: recording is open; `record_graph_pass` records the graph's derived
        // input barriers for the "marcher" pass into `cmd` against the live G-buffer targets.
        //
        // Multi-paradigm render-path plan, rung R2 (Decision 2 / O1): `plan.marcher` is `Some`
        // iff `scene.path_has_marcher()` — the SAME predicate `declare_deferred_graph`'s
        // `marcher` pass declaration checks. The `debug_assert_eq!` guards the two never
        // diverging (W1); under the R2 resolver guard this is `Some`/`true` on every currently
        // reachable frame, so the `if let` is byte-identical to the pre-R2 unconditional
        // dispatch.
        let marcher_plan = self
            .gbuffer_pass_plan
            .as_ref()
            .expect("invariant: declare_frame_graph ran before record_gbuffer")
            .marcher;
        debug_assert_eq!(
            marcher_plan.is_some(),
            scene.path_has_marcher(),
            "W1: declare/record predicate desync (marcher)"
        );
        if let Some(marcher_pass) = marcher_plan {
            self.record_graph_pass(marcher_pass, cmd, targets, scene, fi);
            unsafe {
                (self.fns.cmd_bind_pipeline)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    scene.marcher.pipeline,
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    scene.marcher.layout,
                    0,
                    1,
                    &targets.vocab_set[self.frame_index].descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_push_constants)(
                    cmd,
                    scene.marcher.layout,
                    VK_SHADER_STAGE_COMPUTE_BIT,
                    0,
                    GBUFFER_MARCHER_PUSH_BYTES,
                    marcher_push_bytes.as_ptr().cast(),
                );
                (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
            }
        }

        // (5a) PBR MVP-2: make the marcher's gAlbedo + gNormal + gMaterial STORES available
        // + visible to the resolve's LOADS. A real memory+execution dependency
        // (SHADER_WRITE→SHADER_READ, COMPUTE→COMPUTE), GENERAL→GENERAL (no layout change).
        // gNormal is now READ by the resolve (oct-normal decode + 16-bit material id), so it
        // joins gAlbedo + gMaterial in the barrier (MVP-1 omitted it — gNormal was unread).
        // Lighting L0b: the gViewT lane is marcher-STORED + resolve-READ, so it joins too.
        // Step 1a (sync1 array-batching): the 4 store-to-load barriers share one global
        // COMPUTE_SHADER→COMPUTE_SHADER scope over independent images → ONE array-form call,
        // byte-identical GPU semantics to the former four `count=1` calls.
        // The graph derives each attribute's store→load at its FIRST reader —
        // normal/material/viewt at `record_pass(ssao)` (before the SSAO dispatch) when
        // SSAO is on, and albedo (+ any not read by SSAO) at `record_pass(resolve)`
        // (before the resolve dispatch). So the former single hand batch is split across
        // those two pass records; nothing is recorded here.

        // === Render P7: the SSAO compute pass. Recorded ONLY when the scene wires the SSAO
        // activation (`scene.ssao.is_some()`); otherwise skipped entirely — NO bind, NO dispatch,
        // NO barrier — so the command stream is byte-identical to the pre-P7 windowed path (the
        // 0%-gate; the `ssao` image is always allocated + transitioned by C1's batch regardless of
        // this branch). The SSAO pass gathers a horizon-based AO factor from the G-buffer (gNormal/
        // gMaterial/gViewT, READ) and STORES it into the `ssao` lane the resolve combines under
        // `ssao_mode != 0`. Its inputs are already SHADER_READ-visible: the marcher→resolve
        // store-to-load barrier above (5a) covers gNormal/gMaterial/gViewT (the SSAO reads the same
        // three the resolve reads), so NO new input barrier is needed. After the dispatch, a NEW
        // COMPUTE→COMPUTE / SHADER_WRITE→SHADER_READ / GENERAL→GENERAL barrier on `ssao` orders the
        // SSAO store before the resolve's `gSsao.Load` (the cull→resolve barrier shape, on the
        // `ssao` image). The SSAO pass reads its camera from the UBO bound at the SSAO set's binding
        // 4, so it pushes NO constant (unlike the marcher). ===
        if let Some(activation) = &scene.ssao {
            // The lock-free per-frame ring: bind this present's slot (`frame_index`).
            let ssao_set = &targets
                .ssao_set
                .as_ref()
                .expect("invariant: scene.ssao is Some ⇒ GBufferTargets::create wrote ssao_set")
                [self.frame_index];
            // The SSAO pass's INPUT barriers are DRIVEN by the graph's "ssao" pass,
            // recorded HERE, before the SSAO dispatch: the normal/material/viewt marcher
            // store→load (SSAO is their FIRST reader) + the `ssao` first-touch
            // UNDEFINED→GENERAL (the SSAO write). The `ssao` store→load
            // (SSAO-write→resolve-read) is derived at the resolve (the reader), so it is
            // emitted later by `record_pass(resolve)` — NOT here.
            // SAFETY: recording is open; `record_graph_pass` records the graph's derived
            // input barriers for the "ssao" pass into `cmd`.
            let ssao_pass = self
                .gbuffer_pass_plan
                .as_ref()
                .expect("invariant: declare_frame_graph ran before record_gbuffer")
                .ssao
                .expect("invariant: scene.ssao.is_some() ⇒ ssao pass declared");
            self.record_graph_pass(ssao_pass, cmd, targets, scene, fi);
            // SAFETY: recording is open; the SSAO pipeline + its layout (declaring the SSAO set
            // layout at set 0 + the shared 80-byte COMPUTE push range) are live on this device
            // (caller contract); `ssao_set` binds the now-stored (SHADER_READ-visible, GENERAL)
            // gNormal/gMaterial/gViewT + the `ssao` out (GENERAL) images + the scene's camera UBO;
            // `dispatch_group_count_x` covers `present_extent`'s pixel count (the same grid the
            // marcher/resolve dispatch); `&ssao_set.descriptor_set` is a single-element local alive
            // for the call (first_set 0, count 1, zero dynamic offsets). The SSAO shader reads its
            // camera from the UBO @4, so no push constant is recorded.
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
                    &ssao_set.descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
            }

            // The SSAO pass's `ssao` WRITES (COMPUTE/SHADER_WRITE) are ordered before the
            // resolve's `gSsao.Load` READS (COMPUTE/SHADER_READ) by the graph: it derives
            // this COMPUTE→COMPUTE, GENERAL→GENERAL image barrier on `ssao` at the resolve
            // (the `ssao` reader) UNLESS the à-trous chain below runs (`atrous_levels > 0`),
            // in which case the graph instead derives it at the à-trous chain's LEVEL 0 (the
            // FIRST reader of the raw gather) — `record_pass(resolve)` / this block's first
            // `record_graph_pass` call emits whichever applies, still after this dispatch's
            // write — NOT here.
        }

        // === The SSAO edge-avoiding à-trous denoise chain (RHI DISPATCH WIRING — the deferred
        // half of the R8<->R16 C1 endpoint solution). Recorded ONLY when `scene.ssao.is_some()`
        // (mirrors the gather pass's gate — à-trous cannot run without a fresh gather) AND
        // `activation.atrous_levels > 0` (the owner-authored 0%-gate:
        // `SsaoConfig::clamped_atrous_levels() == 0`) AND the FIVE role-keyed sets all exist
        // (`None` on a device lacking `R16_UNORM` storage — the graceful degrade,
        // `ssao_atrous_storage_ok()`). Otherwise skipped entirely — NO bind, NO dispatch, NO
        // barrier — so the resolve reads the raw, un-denoised gather (the byte-identical
        // pre-dispatch-wiring path).
        //
        // Belt-and-suspenders (the shadow à-trous precedent): the sets are built DECOUPLED from
        // this per-frame gate (on the STABLE boot signals, `GBufferTargets::build_ssao_atrous_
        // sets`), so `scene.ssao.is_some()` normally implies them; a future gate mismatch
        // DEGRADES GRACEFULLY (this whole block simply does not run) instead of an `expect`
        // panic on a `None` set.
        //
        // `N` (`atrous_levels`, clamped to `MAX_SSAO_ATROUS_LEVELS`) dispatches are recorded,
        // level `k`'s (pipeline, set) pair selected by [`crate::present::ssao_atrous_step`]'s
        // [`crate::present::AtrousStepRole`] — `Read8` (level 0: reads the frozen R8 `gSsao`,
        // writes ring 0) / `Interior` (ping-pongs the two R16 rings) / `Write8` (the last level:
        // reads a ring, writes BACK into `gSsao` — the resolve's UNCHANGED binding then reads the
        // FILTERED result). Each pushes `step = 1 << level` (a 4-byte `{ uint step }`). ===
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
            let atrous_levels = activation.atrous_levels.min(crate::present::MAX_SSAO_ATROUS_LEVELS);
            // O1: pin the `0 || 2..=MAX` contract (host `clamped_atrous_levels`) at the RHI boundary —
            // the SAME assert the declarator makes, so a raw `1` (which `ssao_atrous_step(0,1)` would
            // route as a lone `Read8` that never writes back to `gSsao`) trips loudly rather than
            // silently wasting a dispatch + leaving the resolve on the raw gather.
            debug_assert!(
                atrous_levels == 0
                    || (2..=crate::present::MAX_SSAO_ATROUS_LEVELS).contains(&atrous_levels),
                "invariant: ssao à-trous levels is 0 or 2..=MAX at the RHI boundary; got {atrous_levels}"
            );
            let plan = self
                .gbuffer_pass_plan
                .as_ref()
                .expect("invariant: declare_frame_graph ran before record_gbuffer");
            for level in 0..atrous_levels {
                let atrous_pass = plan.ssao_atrous[level as usize].expect(
                    "invariant: level < ssao_atrous_levels ⇒ ssao_atrous[level] declared",
                );
                // SAFETY: recording is open; `record_graph_pass` records the "ssao_atrous" pass's
                // derived RAW barriers (the gather-write → level-0-read on the first iteration,
                // the ring ping-pong on every iteration, the last level's write → resolve-read
                // implicitly ordering before the resolve's later `image_access`) into `cmd`.
                self.record_graph_pass(atrous_pass, cmd, targets, scene, fi);
                let (pipeline, set) = match crate::present::ssao_atrous_step(level, atrous_levels) {
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
                // SAFETY: recording is open; the selected à-trous pipeline variant + its shared
                // 4-binding layout are live on this device (caller contract); `set` binds
                // `gAoIn`/`gAoOut` (the role-keyed pair) + `gViewT` + the camera UBO; the 4-byte
                // `{ uint step }` push covers the pipeline's declared COMPUTE range;
                // `dispatch_group_count_x` covers the pixel count; `&set.descriptor_set` is a
                // single-element local alive for the call.
                unsafe {
                    (self.fns.cmd_bind_pipeline)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        pipeline.pipeline,
                    );
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

        // === SDFDDGI I2: the probe-update compute pass. Recorded ONLY when the scene wires the
        // update activation (`scene.ddgi_update.is_some()`) — i.e. only when `ResolvedDdgi::enabled()`
        // (the same predicate driving the LightBuf GI gate + the resolve read). Otherwise skipped
        // entirely — NO RDG pass, NO bind, NO dispatch, NO barrier — so the command stream is
        // byte-identical to the pre-I2 windowed path (the GI-OFF 0%-gate; the atlas + ray-table + UBO
        // are allocated regardless, staying in boot SHADER_READ_ONLY_OPTIMAL, unread). Placed AFTER
        // the marcher (edit-list SSBO warm) + AFTER the L0 light-table copy (`LightBuf`
        // COMPUTE-read-visible), BEFORE the resolve. The pass sphere-traces the CSG edit-list from
        // each active probe over the Fibonacci ray set + blends into the atlas storage images. Its
        // input barriers (light_table/ray-table/classification reads + the boot
        // SHADER_READ_ONLY_OPTIMAL → GENERAL atlas transition) are DERIVED by the graph's "ddgi_update"
        // pass recorded HERE; the update-write → resolve-read atlas barrier is DERIVED at the resolve
        // (the atlas reader) — NEITHER is hand-written (a hand `cmd_pipeline_barrier` would
        // double-transition against the RDG's derived barriers, a validation error). ===
        if let Some(activation) = &scene.ddgi_update {
            // The SINGLE (non-ringed) update bind group written once at sync_gbuffer — every input
            // is non-ringed per plan §2.2 (the update pass binds neither the ringed camera UBO nor
            // any ringed input), so one bind group captures no stale slot.
            let ddgi_update_set = targets
                .ddgi_update_set
                .as_ref()
                .expect("invariant: scene.ddgi_update is Some ⇒ GBufferTargets wrote ddgi_update_set");
            // SAFETY: recording is open; `record_graph_pass` records the graph's derived input
            // barriers for the "ddgi_update" pass into `cmd` (the light_table/ray-table reads, the
            // classification RW, and the two atlas storage images' boot SHADER_READ_ONLY_OPTIMAL →
            // GENERAL transition — all before the dispatch below).
            let ddgi_update_pass = self
                .gbuffer_pass_plan
                .as_ref()
                .expect("invariant: declare_frame_graph ran before record_gbuffer")
                .ddgi_update
                .expect("invariant: scene.ddgi_update.is_some() ⇒ ddgi_update pass declared");
            // HW-RT rung R0: open the DdgiUpdate bracket BEFORE the pass's input barriers +
            // dispatch. GATED — `None` records nothing (byte-identical).
            if let Some(tc) = scene.gpu_timing {
                // SAFETY: recording is open; `self.fns` is the live device fn-table; the pool was
                // reset at the frame top; `fi` is this present's in-flight slot.
                unsafe { tc.write_begin(self.fns, cmd, fi, TimedPass::DdgiUpdate) };
            }
            self.record_graph_pass(ddgi_update_pass, cmd, targets, scene, fi);
            // SAFETY: recording is open; the update pipeline + its layout (declaring the 7-binding
            // update set layout at set 0, NO push range) are live on this device (caller contract);
            // `ddgi_update_set` binds the edit-list SSBO @0 + the two atlas storage images @1/@2
            // (now GENERAL) + the classification @3 + the ray table @4 + the light table @5 + the
            // update UBO @6; `dispatch_group_count_x` = `DDGI_PROBE_COUNT / subset_n` blocks (one
            // `[numthreads(64,1,1)]` block per active probe in this frame's round-robin subset).
            // The shader reads all params from the b6 UBO, so no push constant is recorded.
            // `&ddgi_update_set.descriptor_set` is a single-element local alive for the call
            // (first_set 0, count 1, zero dynamic offsets).
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
            // HW-RT rung R0: close the DdgiUpdate bracket AFTER the dispatch. GATED.
            if let Some(tc) = scene.gpu_timing {
                // SAFETY: recording is open; the pool was reset this frame; `fi` is this slot.
                unsafe { tc.write_end(self.fns, cmd, fi, TimedPass::DdgiUpdate) };
            }

            // The update pass's atlas WRITES (COMPUTE/SHADER_WRITE, GENERAL) are ordered before the
            // resolve's atlas READS (COMPUTE/SHADER_READ) by the graph: it derives the
            // COMPUTE→COMPUTE, GENERAL→SHADER_READ_ONLY_OPTIMAL image barriers on `ddgi_irr`/
            // `ddgi_depth` at the resolve (the atlas reader — the resolve SAMPLES them through a
            // combined-image-sampler, so it reads at SHADER_READ_ONLY_OPTIMAL, ending the frame at
            // that layout = the cross-frame seed), so `record_pass(resolve)` emits them before the
            // resolve dispatch (still after this update dispatch's writes) — NOT here.
        }

        // === Lighting L1: the clustered froxel light-cull pass (Decision 6). Recorded ONLY
        // when the scene wires the cull pipeline + cull set; otherwise skipped entirely (the
        // resolve's `clusters_enabled` header gate then loops the flat table — the L1 OFF /
        // 0%-gate, byte-identical command stream). The cull reads the camera UBO + light table
        // (the L0-r0 copy above already ordered the table for COMPUTE reads) and writes the
        // ClusterGrid + LightIndexList; the resolve reads them, so a COMPUTE→COMPUTE buffer
        // barrier orders the cull WRITE before the resolve READ. The cull does NOT depend on
        // gViewT (it is geometric), so it can run after the marcher without further sync. ===
        // `_grid` / `_index` are matched only to GATE this L1 block on the cluster
        // buffers being wired; the graph's `light_cull` / `resolve` passes read them via
        // `scene`, so the bindings themselves are unused here.
        if let (Some(cull_pipeline), Some(cull_set), Some(_grid), Some(_index), Some(alloc)) = (
            scene.cluster_cull,
            // The lock-free per-frame ring: bind this present's slot (`frame_index`).
            targets.cull_set.as_ref().map(|s| &s[self.frame_index]),
            scene.cluster_grid,
            scene.light_index,
            scene.light_index_alloc,
        ) {
            // (L1-0) Reset the global slice-allocation counter to 0 (a transfer fill), then
            // order the fill before the cull's atomic reads/writes (TRANSFER→COMPUTE).
            // SAFETY: recording is open; `alloc` is a live device-local STORAGE buffer (≥ 4 B,
            // the single u32 counter); `cmd_fill_buffer` zero-fills it (Vulkan 1.0 core). The
            // FILL is GPU work (not a barrier), so it runs unconditionally — only the
            // following barrier is graph-driven when the flag is ON.
            unsafe {
                (self.fns.cmd_fill_buffer)(cmd, alloc.buffer, 0, VK_WHOLE_SIZE, 0);
            }
            // The alloc TRANSFER→COMPUTE(RW) barrier (+ the light-table TRANSFER→COMPUTE
            // flush, if `light_upload` left one pending — the cull is the first COMPUTE
            // reader of the table) is DRIVEN by the graph's "light_cull" pass, recorded
            // HERE, before the cull dispatch. The cull's grid/index writes are ordered to
            // the resolve by `record_pass(resolve)` (the reader) — NOT here.
            // SAFETY: recording is open; `record_graph_pass` records the graph's derived
            // TRANSFER→COMPUTE barrier for the "light_cull" pass into `cmd`, ordering the
            // fill's TRANSFER write before the cull's COMPUTE atomics on the GPU timeline.
            let light_cull = self
                .gbuffer_pass_plan
                .as_ref()
                .expect("invariant: declare_frame_graph ran before record_gbuffer")
                .light_cull
                .expect("invariant: cull wired ⇒ light_cull pass declared");
            self.record_graph_pass(light_cull, cmd, targets, scene, fi);

            // (L1-1) Bind the cull pipeline + the cull set (written ONCE at sync_gbuffer), push
            // this arm's own push image, dispatch this arm's own group count (VB-P1e H5, the
            // SAME `match` `vb.rs` uses per D11/H4): base = `cluster_count` froxels at the
            // 64-wide group + the 16-byte `ClusterCullPush`; hier = `h.groups` groups of 256 +
            // the 24-byte `ClusterCullHierPush`. `scene.cluster_cull_hier` selects BOTH halves
            // together, so the group count can never be paired with the other arm's push range.
            //
            // On every current Deferred boot this `match` always takes the `None` arm:
            // `GpuSceneBundles::build_froxel_light_cull` is the only writer of both
            // `scene.cluster_cull` and `scene.cluster_cull_hier`, and it is gated on
            // `ResolvedRenderPath::froxel_light_cull`, which resolves VB-only
            // (`consumers.clusters_wanted && matches!(path, RenderPath::VisibilityBuffer)`,
            // `boyko_render::render_path_config.rs:913`) — so `scene.cluster_cull` itself stays
            // `None` on a Deferred boot and this whole `if let` block does not execute. This
            // rung does not migrate a live path; it makes the record site CAPABLE of carrying a
            // hierarchical dispatch record, removing the interim `debug_assert`'s latent trap (a
            // future Deferred froxel-cull wiring landing without a matching push/dispatch update
            // here) ahead of that wiring existing.
            let (cull_groups, push_ptr, push_len) = match scene.cluster_cull_hier.as_ref() {
                Some(h) => (h.groups, h.push.as_ptr(), CLUSTER_CULL_HIER_PUSH_BYTES),
                None => (
                    scene.cluster_count.div_ceil(LIGHT_CULL_LOCAL_SIZE_X),
                    scene.cluster_cull_push.as_ptr(),
                    CLUSTER_CULL_PUSH_BYTES,
                ),
            };
            // SAFETY: recording is open; the cull pipeline + its layout (declaring `cull_layout`
            // at set 0 + a COMPUTE push range sized for the SAME arm) are live on this device
            // (caller contract); the cull set binds the camera UBO + light table + the cluster
            // buffers; the dispatch size and the push image are the SAME `Option` arm (base:
            // `cluster_count` froxels at the 64-wide group + the 16-byte `ClusterCullPush`;
            // hier: `h.groups` groups of 256 + the 24-byte `ClusterCullHierPush`), so the group
            // count can never be paired with the other arm's push range.
            //
            // The two arms' `ClusterGrid[fi]` write bounds are DIFFERENT quantities (P0-1,
            // adversarial review — the two must not be conflated). HIER: `cluster_cull.hlsl`'s
            // `#ifdef HIER` branch guards on `fi < pc.cluster_capacity`, a pushed BOOT-snapshot
            // word minted by `build_froxel_light_cull` from the SAME `ClusterConfig::
            // cluster_count()` binding the `ClusterGrid` buffer itself was allocated from
            // (`gpu_scene/mod.rs`) — a live edit to the `ClusterConfig` Resource cannot move
            // this arm's own write bound, by construction (D11). BASE: the `#else` branch
            // carries NO `cluster_capacity` push word at all — its push is 16 B / 4 words
            // (`z_near`, `z_far`, `max_lights_per_cluster`, `index_list_cap` only); it guards on
            // `fi >= cluster_count` where `cluster_count` comes from `load_cluster_params
            // (LightBuf)` — the LIVE light-table header, re-read every dispatch — so the base
            // arm's write bound is whatever `sync_cluster_light_gate` (`light.rs:875`) last
            // wrote there, NOT the capacity `ClusterGrid` was sized from. This is the
            // PRE-EXISTING VB-P1k skew: the base arm's shader token stream and dispatch shape
            // are UNCHANGED by this rung (D11), so this `match` neither introduces nor closes
            // that exposure — it merely routes to whichever arm's shader enforces its own bound
            // the way it always did.
            // SCOPE: this bounds THIS dispatch's writes only, and only in the sense above (HIER:
            // boot-fixed; BASE: live-header, unclosed VB-P1k). The `ClusterGrid` *readers*
            // (vb_resolve/vb_shade/deferred_pbr/forward_opaque) also index with the live dims
            // and carry the SAME pre-existing skew exposure tracked as VB-P1k.
            // `&cull_set.descriptor_set` is a single-element local alive for the call
            // (first_set 0, count 1).
            unsafe {
                (self.fns.cmd_bind_pipeline)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    cull_pipeline.pipeline,
                );
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
                    push_len,
                    push_ptr.cast(),
                );
                (self.fns.cmd_dispatch)(cmd, cull_groups, 1, 1);
            }

            // (L1-2) The cull's ClusterGrid + LightIndexList writes are made available +
            // visible to the resolve's reads by the graph: it derives these
            // COMPUTE→COMPUTE, SHADER_WRITE→SHADER_READ buffer barriers at the resolve
            // (the grid/index reader), so `record_pass(resolve)` emits them before the
            // resolve dispatch (still after this cull dispatch's writes) — NOT here.
        }

        // === CSM Increment 1b (Rung A): the cascade DEPTH pass (W5 — a NEW recorder bracket,
        // NOT record_gbuffer's main raster). Recorded ONLY when the scene wires the depth
        // activation (`scene.csm.is_some()`); otherwise NO rendering is recorded and the
        // cascade map/sampler/UBO stay bound-but-unread — the graph's UNCONDITIONAL resolve
        // read still derives the discard-legal UNDEFINED→SHADER_READ_ONLY transition that
        // keeps the always-bound descriptor's layout valid (VUID-...-09600; PIXELS stay
        // byte-identical — the resolve's `csm_mode == 0` gate never samples it). Renders the
        // SAME caster batches (`scene.mesh_draw` + `scene.instance_bind_group`) from the SUN's
        // POV into cascade layer 0, so the resolve can `min`-combine the exact hard shadow.
        // RUN BEFORE the resolve dispatch (5b) so the cascade depth is SHADER_READ-visible. ===
        if let Some(csm) = &scene.csm {
            let cascade = scene.csm_cascade_texture;
            // CSM Increment 3 (Rung B): the number of cascade LAYERS to render — clamped to the
            // backend cap so an out-of-range `active_count` cannot drive `layer_render_view` /
            // the barrier range past the array bounds. `1` reproduces the Rung-A single-cascade path.
            let active = (csm.active_count as usize).clamp(1, MAX_CASCADES) as u32;
            // (CSM-0) Barrier-in: the cascade image UNDEFINED → DEPTH_ATTACHMENT_OPTIMAL (the
            // depth-write access, DEPTH aspect) over the FULL `MAX_CASCADES` array — the resolve
            // samples through a whole-array 2D_ARRAY view, so the `[active..MAX)` tail must ride
            // the same layout cycle (09600; discard-legal garbage the shader's `active_count`
            // gate never samples). Each layer is re-`UNDEFINED`'d (the prior frame's content is
            // discarded before this frame's depth pass); the rendering loop below still touches
            // only `[0..active)`.
            // The graph's "csm" pass (declaring the cascade layered DEPTH_WRITE over
            // `depth_layers(MAX_CASCADES)`) DRIVES this barrier-in, recorded HERE, before the
            // cascade depth loop. Its barrier-OUT (→SHADER_READ_ONLY) is derived at the
            // resolve (the cascade reader), so `record_pass(resolve)` emits it — NOT here.
            // SAFETY: recording is open; `record_graph_pass` records the graph's derived
            // UNDEFINED→DEPTH barrier-in for the "csm" pass into `cmd`.
            let csm_pass = self
                .gbuffer_pass_plan
                .as_ref()
                .expect("invariant: declare_frame_graph ran before record_gbuffer")
                .csm
                .expect("invariant: scene.csm.is_some() ⇒ csm pass declared");
            // HW-RT rung R0: open the CsmDepth bracket BEFORE the pass's barrier-in +
            // cascade depth loop (the reset MUST have run before `begin_rendering`, so the
            // begin write is still legal here — it is outside the per-cascade rendering scope,
            // which opens below inside the loop). GATED — `None` records nothing.
            if let Some(tc) = scene.gpu_timing {
                // SAFETY: recording is open; `self.fns` is the live device fn-table; the pool was
                // reset at the frame top; this write is outside any `begin_rendering` scope; `fi`
                // is this present's in-flight slot.
                unsafe { tc.write_begin(self.fns, cmd, fi, TimedPass::CsmDepth) };
            }
            self.record_graph_pass(csm_pass, cmd, targets, scene, fi);

            // (CSM-1) Depth-only dynamic rendering, LOOPED over the `[0..active)` cascades (Rung B).
            // The render area / viewport / scissor are cascade-INDEPENDENT (the square shadow-map
            // resolution — NOT the swapchain/composite extent), so they are built ONCE; only the
            // per-layer render view + the pushed `view_proj` change per cascade. NO color attachment
            // (`color_attachment_count == 0`), one depth attachment (CLEAR to far / STORE).
            let cascade_extent = VkExtent2D {
                width: csm.shadow_dim,
                height: csm.shadow_dim,
            };
            let csm_area = VkRect2D {
                offset: VkOffset2D { x: 0, y: 0 },
                extent: cascade_extent,
            };
            let csm_viewport = VkViewport {
                x: 0.0,
                y: 0.0,
                width: cascade_extent.width as f32,
                height: cascade_extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            let mut csm_push = csm.push;
            // BUILD-ONCE-CONSUME-N-VIEWS: the SAME caster batches + instance SSBO are rendered into
            // each cascade layer; only cascade `c`'s `view_proj` differs. Loop the active cascades.
            for c in 0..active {
                // Stamp cascade `c`'s COLUMN-MAJOR `view_proj` (64 B) into the push's leading
                // matrix bytes (the O1 single-matrix pin — byte-equal to the resolve UBO's
                // `gCascades[c].view_proj`). The trailing words (`use_model_matrix @84`) are
                // unchanged from the template; `base_instance @80` is re-pushed per batch below.
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
                        depth_stencil: VkClearDepthStencilValue {
                            depth: 1.0,
                            stencil: 0,
                        },
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
                    p_depth_attachment: (&csm_depth_attachment as *const VkRenderingAttachmentInfo)
                        .cast(),
                    p_stencil_attachment: ptr::null(),
                };
                // SAFETY: recording is open; `csm_rendering` is fully initialized — its depth
                // attachment names the live cascade layer-`c` render view (now
                // DEPTH_ATTACHMENT_OPTIMAL; `c < active <= MAX_CASCADES` so `layer_render_view(c)`
                // is in bounds), NO color attachment (depth-only); the depth-only pipeline
                // (declaring an EMPTY `color_formats` + `depth_format = D32Sfloat` + `cull_mode:
                // Front` + a depth bias + the set-0 instance layout) belongs to this device (caller
                // contract). The SAME instance SSBO (`scene.instance_bind_group`) the main pass
                // binds is bound at set 0 to satisfy the depth VS's static `instances` reference;
                // the 88-byte push carries cascade `c`'s `view_proj` (`@0`) + `use_model_matrix ==
                // 1` (`@84`), and per caster batch the recorder re-pushes its `base_instance` (4
                // bytes @80, in-range of the 88-byte VERTEX push) then `draw_indexed` reads that
                // batch's bound vertex+index buffers (created on this device with VERTEX/INDEX
                // usage). The locals outlive the bracketed calls. Begin/End bracket each cascade.
                unsafe {
                    (self.fns.cmd_begin_rendering)(cmd, &csm_rendering);
                    (self.fns.cmd_bind_pipeline)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_GRAPHICS,
                        csm.pipeline.pipeline,
                    );
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
                    // The caster batches: the instanced mesh draws the main pass rasterizes,
                    // FILTERED to `casts_shadow` (the `With<ShadowCaster>` subset). A RECEIVER-only
                    // mesh (room floor/wall) is skipped so it does not stamp itself into the cascade
                    // and cast a spurious shadow over the scene. An EMPTY list (or all-receivers)
                    // records the depth scope with no draw (a cleared cascade — every receiver fully
                    // lit, the `min` a no-op).
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
                        (self.fns.cmd_bind_vertex_buffers)(
                            cmd,
                            0,
                            1,
                            &batch.vertex_buffer.buffer,
                            &vertex_offset,
                        );
                        (self.fns.cmd_bind_index_buffer)(
                            cmd,
                            batch.index_buffer.buffer,
                            0,
                            batch.index_type,
                        );
                        (self.fns.cmd_draw_indexed)(
                            cmd,
                            batch.index_count,
                            batch.instance_count,
                            0,
                            0,
                            0,
                        );
                    }
                    (self.fns.cmd_end_rendering)(cmd);
                }
            }
            // HW-RT rung R0: close the CsmDepth bracket AFTER the cascade depth loop (all
            // `end_rendering`s recorded — this write is outside any rendering scope). GATED.
            if let Some(tc) = scene.gpu_timing {
                // SAFETY: recording is open; the pool was reset this frame; the per-cascade
                // rendering scopes are all closed; `fi` is this slot.
                unsafe { tc.write_end(self.fns, cmd, fi, TimedPass::CsmDepth) };
            }

            // (CSM-2) The dual-use depth barrier: DEPTH_ATTACHMENT_OPTIMAL →
            // SHADER_READ_ONLY_OPTIMAL (reusing the marcher's depth-barrier shape) over ALL
            // `[0..active)` cascade layers in ONE barrier. Depth WRITES happen at
            // LATE_FRAGMENT_TESTS; the resolve PCF-SAMPLES at COMPUTE_SHADER. This barrier (DEPTH
            // aspect, the full-array range) makes every rendered cascade's depth available +
            // visible to the resolve's `SampleCmpLevelZero(float3(uv, c))` and transitions the
            // layout for sampling.
            // The graph derives this →SHADER_READ_ONLY barrier-out at the resolve (the
            // cascade reader), so `record_pass(resolve)` emits it before the resolve
            // dispatch (still after this cascade depth loop) — NOT here.
        }

        // === Shadow Phase 5 Inc-1-GPU: the sparse SPOT atlas DEPTH pass (a NEW recorder bracket,
        // a CLONE of the CSM depth pass above). Recorded ONLY when the scene wires the activation
        // (`scene.atlas_punctual.is_some()`); otherwise NO rendering is recorded and the atlas
        // map/sampler/UBO stay bound-but-unread — the graph's UNCONDITIONAL resolve read still
        // derives the discard-legal UNDEFINED→SHADER_READ_ONLY transition that keeps the
        // always-bound descriptor's layout valid (09600, the CSM-pass mirror above). Renders the
        // SAME caster batches (`scene.mesh_draw` + `scene.instance_bind_group`) from each SPOT's
        // POV into atlas layer `s`, so the resolve can multiply the exact hard shadow into that
        // spot's contribution. RUN BEFORE the resolve dispatch (5b) so the atlas depth is
        // SHADER_READ-visible to the resolve. ===
        if let Some(atlas_act) = &scene.atlas_punctual {
            let atlas = scene.shadow_atlas_texture;
            // The number of atlas LAYERS to render — clamped to the backend cap so an out-of-range
            // `active_layers` cannot drive `layer_render_view` / the barrier range past the array
            // bounds. `1` reproduces the single-spot path.
            let active = (atlas_act.active_layers as usize).clamp(1, MAX_TEXTURE_LAYERS) as u32;
            // Barrier-in: the atlas image UNDEFINED → DEPTH_ATTACHMENT_OPTIMAL (depth-write access,
            // DEPTH aspect) over the FULL `MAX_TEXTURE_LAYERS` array — the resolve samples through
            // a whole-array 2D_ARRAY view, so the `[active..MAX)` tail must ride the same layout
            // cycle (09600, the CSM barrier-in mirror above). Each layer is re-`UNDEFINED`'d; the
            // rendering loop below still touches only `[0..active)`.
            // The graph's "atlas_depth" pass (declaring the atlas layered DEPTH_WRITE over
            // `depth_layers(MAX_TEXTURE_LAYERS)`) DRIVES this barrier-in, recorded HERE, before
            // the atlas depth loop. Its barrier-OUT (→SHADER_READ_ONLY) is derived at the
            // resolve (the atlas reader) — NOT here.
            // SAFETY: recording is open; `record_graph_pass` records the graph's derived
            // UNDEFINED→DEPTH barrier-in for the "atlas_depth" pass into `cmd`.
            let atlas_pass = self
                .gbuffer_pass_plan
                .as_ref()
                .expect("invariant: declare_frame_graph ran before record_gbuffer")
                .atlas
                .expect("invariant: scene.atlas_punctual.is_some() ⇒ atlas pass declared");
            // HW-RT rung R0: open the PunctualDepth bracket BEFORE the pass's barrier-in +
            // atlas depth loop (outside the per-slot rendering scope, which opens below inside
            // the loop). GATED — `None` records nothing.
            if let Some(tc) = scene.gpu_timing {
                // SAFETY: recording is open; `self.fns` is the live device fn-table; the pool was
                // reset at the frame top; this write is outside any `begin_rendering` scope; `fi`
                // is this present's in-flight slot.
                unsafe { tc.write_begin(self.fns, cmd, fi, TimedPass::PunctualDepth) };
            }
            self.record_graph_pass(atlas_pass, cmd, targets, scene, fi);

            // Depth-only dynamic rendering, LOOPED over the `[0..active)` atlas slots. The render
            // area / viewport / scissor are slot-INDEPENDENT (the square shadow-map resolution), so
            // they are built ONCE; only the per-slot render view + the pushed `view_proj` change.
            let atlas_extent = VkExtent2D {
                width: atlas_act.shadow_dim,
                height: atlas_act.shadow_dim,
            };
            let atlas_area = VkRect2D {
                offset: VkOffset2D { x: 0, y: 0 },
                extent: atlas_extent,
            };
            let atlas_viewport = VkViewport {
                x: 0.0,
                y: 0.0,
                width: atlas_extent.width as f32,
                height: atlas_extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            let mut atlas_push = atlas_act.push;
            // BUILD-ONCE-CONSUME-N-VIEWS: the SAME caster batches + instance SSBO are rendered into
            // each atlas layer; only slot `s`'s `view_proj` (+ the POINT `cam_eye` lane) differs. The
            // active layers are GROUPED by type in the activation (spot-faces then point-faces, or
            // vice versa), so binding the per-slot pipeline only when it CHANGES costs at most TWO
            // pipeline binds. `bound_point` tracks which pipeline is currently bound (`None` = none
            // yet).
            let mut bound_point: Option<bool> = None;
            for s in 0..active {
                // Shadow Phase 5 Inc-2: select this layer's pipeline by its TYPE. A SPOT face uses
                // the `csm_depth` NDC-z pipeline; a POINT cube face uses the `punctual_depth`
                // linear-distance pipeline (a depth-WRITE FS). Both share the SAME pipeline LAYOUT
                // (the set-0 instance SSBO + the 88-byte push), so the descriptor set + push stamps
                // are identical; only the bound pipeline object differs.
                let is_point = atlas_act.face_is_point[s as usize];
                let face_pipeline = if is_point {
                    atlas_act.point_pipeline
                } else {
                    atlas_act.pipeline
                };
                // Stamp slot `s`'s COLUMN-MAJOR `view_proj` (64 B) into the push's leading matrix
                // bytes (byte-equal to the resolve UBO's `gFaces[s].view_proj`). The trailing words
                // are unchanged; `base_instance @80` is re-pushed per batch below.
                atlas_push[0..64].copy_from_slice(&atlas_act.face_view_proj[s as usize]);
                // Shadow Phase 5 Inc-2: for a POINT face, stamp the `cam_eye@64` lane (16 B) =
                // `light_pos.xyz` + `inv_range` so the FS computes `length(world - light_pos) *
                // inv_range`. For a SPOT face this lane is unused (the empty NDC-z FS), so it is left
                // as the template default.
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
                        depth_stencil: VkClearDepthStencilValue {
                            depth: 1.0,
                            stencil: 0,
                        },
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
                    p_depth_attachment: (&atlas_depth_attachment as *const VkRenderingAttachmentInfo)
                        .cast(),
                    p_stencil_attachment: ptr::null(),
                };
                // SAFETY: recording is open; `atlas_rendering` is fully initialized — its depth
                // attachment names the live atlas layer-`s` render view (now DEPTH_ATTACHMENT_OPTIMAL;
                // `s < active <= MAX_TEXTURE_LAYERS` so `layer_render_view(s)` is in bounds), NO color
                // attachment (depth-only); the selected depth-only pipeline (the SPOT `csm_depth` or
                // the POINT `punctual_depth` pipeline — both EMPTY `color_formats` + `depth_format =
                // D32Sfloat` + `cull_mode: Front` + the set-0 instance layout) belongs to this device
                // (caller contract) and shares the SAME pipeline layout. The SAME instance SSBO
                // (`scene.instance_bind_group`) the main pass binds is bound at set 0 to satisfy the
                // depth VS's static `instances` reference; the 88-byte push carries slot `s`'s
                // `view_proj` (`@0`) + (POINT only) the `cam_eye@64` `light_pos`/`inv_range` lane +
                // `use_model_matrix == 1` (`@84`), pushed for `VERTEX | FRAGMENT` (the POINT FS reads
                // `cam_eye`; the layout declares that range), and per caster batch the recorder
                // re-pushes its `base_instance` (4 bytes @80, in-range of the 88-byte push) then
                // `draw_indexed` reads that batch's bound vertex+index buffers (created on this device
                // with VERTEX/INDEX usage). The pipeline + descriptor set are (re)bound only when the
                // face TYPE changes (the layers are grouped), so at most two binds occur; the push is
                // re-stamped every slot (the per-slot `view_proj` differs). The locals outlive the
                // bracketed calls. Begin/End bracket each slot.
                unsafe {
                    (self.fns.cmd_begin_rendering)(cmd, &atlas_rendering);
                    if bound_point != Some(is_point) {
                        (self.fns.cmd_bind_pipeline)(
                            cmd,
                            VK_PIPELINE_BIND_POINT_GRAPHICS,
                            face_pipeline.pipeline,
                        );
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
                    // The caster batches: the instanced mesh draws the main pass rasterizes,
                    // FILTERED to `casts_shadow` (the `With<ShadowCaster>` subset). A RECEIVER-only
                    // mesh (room floor/wall) is skipped so it does not stamp itself into this slot and
                    // cast a spurious omni/cone shadow. An EMPTY list (or all-receivers) records the
                    // depth scope with no draw (a cleared slot — every receiver in that cone fully lit).
                    for batch in scene.mesh_draw {
                        if !batch.casts_shadow {
                            continue;
                        }
                        let base = batch.base_instance;
                        atlas_push[GBUFFER_PUSH_BASE_INSTANCE_OFFSET as usize
                            ..GBUFFER_PUSH_BASE_INSTANCE_OFFSET as usize + 4]
                            .copy_from_slice(&base.to_le_bytes());
                        // The `base_instance` lane (@80) is read only by the VS, but the push
                        // MUST still name the layout range's FULL `VERTEX | FRAGMENT` stage set:
                        // VUID-vkCmdPushConstants-offset-01796 requires the call's stageFlags to
                        // include ALL stages of every overlapping range — a subset is invalid.
                        // Both pipelines share the SAME layout, so `face_pipeline.layout` is correct
                        // for either face type.
                        (self.fns.cmd_push_constants)(
                            cmd,
                            face_pipeline.layout,
                            VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
                            GBUFFER_PUSH_BASE_INSTANCE_OFFSET,
                            4,
                            (&base as *const u32).cast(),
                        );
                        (self.fns.cmd_bind_vertex_buffers)(
                            cmd,
                            0,
                            1,
                            &batch.vertex_buffer.buffer,
                            &vertex_offset,
                        );
                        (self.fns.cmd_bind_index_buffer)(
                            cmd,
                            batch.index_buffer.buffer,
                            0,
                            batch.index_type,
                        );
                        (self.fns.cmd_draw_indexed)(
                            cmd,
                            batch.index_count,
                            batch.instance_count,
                            0,
                            0,
                            0,
                        );
                    }
                    (self.fns.cmd_end_rendering)(cmd);
                }
            }
            // HW-RT rung R0: close the PunctualDepth bracket AFTER the atlas depth loop (all
            // `end_rendering`s recorded — outside any rendering scope). GATED.
            if let Some(tc) = scene.gpu_timing {
                // SAFETY: recording is open; the pool was reset this frame; the per-slot
                // rendering scopes are all closed; `fi` is this slot.
                unsafe { tc.write_end(self.fns, cmd, fi, TimedPass::PunctualDepth) };
            }

            // The graph derives the dual-use depth barrier-out
            // (DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL over ALL `[0..active)`
            // atlas layers) at the resolve (the atlas reader), so `record_pass(resolve)`
            // emits it before the resolve dispatch (still after this atlas depth loop) —
            // NOT here.
        }

        // === HW-RT rung 3a: the spatial RT soft-shadow DENOISE stack — the VIS pre-pass + the
        // `levels` à-trous filter passes. Recorded ONLY when the scene wires `scene.shadow` (the
        // step-7 gate; the host keeps it a LITERAL `None` this rung, so this whole block is skipped
        // on EVERY current frame — NO bind, NO dispatch, NO barrier — and the resolve stays
        // RESOLVE_INLINE-hwrt ⇒ BYTE-IDENTICAL). When `Some`:
        //   (a) the VIS pass re-runs the resolve front-matter + traces the TLAS, WRITING
        //       `gShadowVis` (@21 of its 22-binding set = `shadow_vis[fi]`); dispatched at the
        //       resolve's 1D group count.
        //   (b) `levels` à-trous passes ping-pong `shadow_vis` ⇄ `shadow_vis2`, each pushing
        //       `step = 1 << level` (a 4-byte `{ uint step }`); dispatched at the SAME grid.
        // The resolve then binds the DENOISED pipeline (selected in the `(pipeline, layout, set)`
        // triple below), reading the FILTERED `gShadowVis`. All input/RAW barriers are graph-derived
        // (the "shadow_vis" + "shadow_atrous" passes recorded here). ===
        //
        // Belt-and-suspenders: the record REQUIRES the pre-built VIS + à-trous sets
        // ([`GBufferTargets::build_shadow_denoise_sets`]). They are built decoupled from this
        // per-frame gate (on the STABLE boot signals), so `scene.shadow.is_some()` normally implies
        // both are `Some`. If a future gate mismatch ever leaves them `None` while `scene.shadow` is
        // `Some`, we DEGRADE GRACEFULLY — skip the whole VIS/à-trous stack (no bind, no dispatch, no
        // barrier) exactly as if `scene.shadow` were `None`. The `denoised_triple` selection below is
        // ALSO `None`-set-guarded (it `.map`s over `shadow_denoised_resolve_set`), so the resolve
        // falls back to the RESOLVE_INLINE-hwrt triple — never a DENOISED bind with no `gShadowVis`
        // data. This removes the `None`-set panic as a failure mode; the primary fix is that the sets
        // ARE built at create.
        #[cfg(feature = "hwrt")]
        if let (Some(sh), Some(vis_ring), Some(atrous_sets)) = (
            scene.shadow.as_ref(),
            targets.shadow_vis_resolve_set.as_ref(),
            targets.shadow_atrous_sets.as_ref(),
        ) {
            let plan = self
                .gbuffer_pass_plan
                .as_ref()
                .expect("invariant: declare_frame_graph ran before record_gbuffer");
            // (a) The VIS pre-pass. Its input barriers (gNormal/gViewT store→load already visible,
            // the build→VIS AS barrier, the `shadow_vis` first-touch UNDEFINED→GENERAL) are DRIVEN
            // by the graph's "shadow_vis" pass, recorded here.
            let vis_pass = plan
                .shadow_vis
                .expect("invariant: scene.shadow.is_some() ⇒ shadow_vis pass declared");
            // HW-RT Rung 3b step 5b: select the SDF motion-vector VIS-variant pipeline + its
            // 24-binding set when temporal is active (`sdf_mv_active()`). That variant writes
            // `gShadowVis` @21 (bit-identical to the base VIS) AND each SDF pixel's camera-only `Δuv`
            // to `motion_vec` @23. `sdf_mv_active()` is the SINGLE source shared with
            // `declare_deferred_graph` (the `motion_vec` STORAGE write is declared under the SAME
            // predicate — W1: the barrier declaration and this write must never disagree). `Some`
            // implies the boot MV pipeline exists (⇒ RT + storage), a strict superset of the VIS-MV
            // set-build gate, so both `expect`s hold (they trip loudly on a future gate loosening,
            // matching the step-5a `expect` discipline). When false ⇒ the base VIS pipeline + its
            // 22-binding set (byte-identical).
            let (vis_pipeline, vis_set) = if scene.sdf_mv_active() {
                let p = scene
                    .vis_mv_pipeline
                    .expect("invariant: sdf_mv_active implies vis_mv_pipeline is Some");
                let ring = targets.shadow_vis_mv_resolve_set.as_ref().expect(
                    "invariant: sdf_mv_active + shadow.is_some implies the VIS-MV resolve set was built",
                );
                (p, &ring[self.frame_index])
            } else {
                (sh.vis_pipeline, &vis_ring[self.frame_index])
            };
            // SAFETY: recording is open; `record_graph_pass` records the graph's derived input
            // barriers for the "shadow_vis" pass into `cmd`.
            self.record_graph_pass(vis_pass, cmd, targets, scene, fi);
            // SAFETY: recording is open; the selected VIS pipeline + its layout (22-binding base or
            // 24-binding VIS-MV) are live on this device (caller contract); `vis_set` binds the
            // resolve inputs + `gShadowVis` @21 = `shadow_vis[fi]` (the write target) [+ the
            // `MotionCam` UBO @22 + `motion_vec[fi]` @23 on the VIS-MV path]; `dispatch_group_count_x`
            // covers the pixel count (the resolve grid); `&vis_set.descriptor_set` is a
            // single-element local alive for the call. The VIS shader reads its camera/params from
            // the bound UBOs; the resolve's 80-byte push range is declared-but-unread here (no push
            // recorded).
            unsafe {
                (self.fns.cmd_bind_pipeline)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    vis_pipeline.pipeline,
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    vis_pipeline.layout,
                    0,
                    1,
                    &vis_set.descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
            }
            // (b) The `levels` à-trous passes. Each pushes `step = 1 << level`, binds the level's
            // per-FIF set (ping-ponging `shadow_vis`/`shadow_vis2`), and dispatches at the resolve
            // grid. The per-level RAW barriers on the ping-pong pair are graph-derived (the
            // "shadow_atrous" passes recorded here).
            //
            // W1: the SAME `.clamp(1, MAX_ATROUS_LEVELS)` the graph-declare site
            // (`declare_deferred_graph`) and the host `clamped_levels()` use — all three agree by
            // construction (floor at 1 so it can never be an empty ping-pong, ceiling at the
            // per-level array bound). A prior `.min(atrous_sets.len())` here dropped the floor, so a
            // `levels == 0` author config would have recorded ZERO à-trous passes while the graph
            // declared / the DENOISED bind expected the floored count — a divergence.
            // Rung 3b: `atrous_levels` may be `0` (Temporal-only mode) ⇒ NO à-trous pass, the raw VIS
            // feeds the temporal reproject. Only the CEILING is clamped (`.min`, not `.clamp(1, ..)`) —
            // the same floor-0/ceiling-MAX the graph-declare site uses (W1). The graph declared exactly
            // this many à-trous passes.
            let atrous_levels =
                (sh.atrous_levels as usize).min(crate::present::MAX_ATROUS_LEVELS as usize);
            // The per-level set array is sized `MAX_ATROUS_LEVELS`, so the ceiling already guarantees
            // `atrous_levels <= atrous_sets.len()`; assert it so an array-size change can never
            // silently let the `take(atrous_levels)` index past the built sets.
            debug_assert!(
                atrous_sets.len() >= atrous_levels,
                "invariant: the à-trous set array must hold at least `atrous_levels` levels"
            );
            // The DENOISED resolve set binds `gShadowVis` @21 to the FINAL à-trous ring (or, on the
            // temporal path, `gVisIn` @0 of the temporal set), chosen by `final_is_vis2` (odd count ⇒
            // `shadow_vis2`, even/`0` ⇒ `shadow_vis` = the raw VIS). Assert the record parity matches so
            // the bind target can never diverge from the à-trous chain's last write (a divergence would
            // read the wrong ring — a stale/uninitialized shadow).
            debug_assert_eq!(
                sh.final_is_vis2,
                atrous_levels % 2 == 1,
                "denoised/temporal bind ring must match the last à-trous parity"
            );
            for (level, level_ring) in atrous_sets.iter().enumerate().take(atrous_levels) {
                let atrous_pass = plan
                    .shadow_atrous[level]
                    .expect("invariant: level < scene.shadow.atrous_levels ⇒ shadow_atrous[level] declared");
                let step: u32 = 1u32 << level;
                // SAFETY: recording is open; `record_graph_pass` records the "shadow_atrous" pass's
                // derived RAW barriers on the ping-pong pair into `cmd`.
                self.record_graph_pass(atrous_pass, cmd, targets, scene, fi);
                let atrous_set = &level_ring[self.frame_index];
                // SAFETY: recording is open; the à-trous pipeline + its 6-binding layout are live on
                // this device (caller contract); `atrous_set` binds `gVisIn`/`gVisOut` (the
                // ping-pong pair) + gNormal/gViewT + the ResolvedShadowDenoise UBO + the camera UBO;
                // the 4-byte `{ uint step }` push covers the pipeline's declared COMPUTE range;
                // `dispatch_group_count_x` covers the pixel count; `&atrous_set.descriptor_set` is a
                // single-element local alive for the call.
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

        // === HW-RT Rung 3b step 6: the TEMPORAL reproject+accumulate pass. ===
        // Runs AFTER the à-trous chain, BEFORE the resolve, when the author's mode is temporal
        // (`scene.temporal_active()`) AND the pre-built temporal sets exist. Belt-and-suspenders (the
        // à-trous precedent): the temporal sets are built decoupled on the STABLE boot signals, so
        // `temporal_active()` normally implies them; a future gate mismatch DEGRADES GRACEFULLY — no
        // temporal dispatch, and the `denoised_triple` below falls back to the à-trous DENOISED set
        // (never a temporal-DENOISED bind reading an unwritten `temporal_out`). The pass reads the
        // à-trous FINAL output (or the raw VIS when `atrous_levels == 0`) via `gVisIn`, `motion_vec`,
        // `viewt`, and the cross-frame history `[1-fi]` (all bound in the set — the `[1-fi]` read is
        // NOT framegraph-tracked, covered by the ResId-14 GENERAL seed), and writes the history `[fi]`
        // + `temporal_out`. Its input/RAW barriers are graph-derived (the "shadow_temporal" pass).
        #[cfg(feature = "hwrt")]
        if scene.temporal_active()
            && let Some(temporal_sets) = targets.shadow_temporal_set.as_ref()
        {
            let sh = scene
                .shadow
                .as_ref()
                .expect("invariant: temporal_active() implies scene.shadow.is_some()");
            let temporal_pipeline = sh.temporal_pipeline.expect(
                "invariant: temporal_active() + the temporal set built implies the temporal pipeline",
            );
            let plan = self
                .gbuffer_pass_plan
                .as_ref()
                .expect("invariant: declare_frame_graph ran before record_gbuffer");
            let temporal_pass = plan
                .shadow_temporal
                .expect("invariant: scene.temporal_active() ⇒ shadow_temporal pass declared");
            // SAFETY: recording is open; `record_graph_pass` records the "shadow_temporal" pass's
            // derived input/RAW barriers (final-vis/motion_vec/viewt → read, hist[fi]/temporal_out
            // first-touch/RAW) into `cmd`.
            self.record_graph_pass(temporal_pass, cmd, targets, scene, fi);
            let temporal_set = &temporal_sets[self.frame_index];
            // SAFETY: recording is open; the temporal pipeline + its 8-binding layout are live on this
            // device (caller contract); `temporal_set` binds gVisIn/gMotionVec/gViewT/gHistIn/gHistOut/
            // gTemporalOut + the ResolvedTemporalShadow UBO + the camera UBO for `frame_index`;
            // `dispatch_group_count_x` covers the pixel count (`numthreads(64,1,1)`, the resolve grid);
            // `&temporal_set.descriptor_set` is a single-element local alive for the call. The temporal
            // shader reads NO push (its params ride the b6 UBO); the pipeline's declared 4-byte COMPUTE
            // range is bound-but-unread (no push recorded).
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

        // The RESOLVE's INPUT barriers — the albedo store→load, the ssao store→load, the
        // L1 grid/index cull→resolve, the layered cascade/atlas →sampled, and the lit
        // first-touch UNDEFINED→GENERAL — are all DRIVEN by the graph's "resolve" pass,
        // recorded HERE, immediately before the resolve dispatch. The graph derives each
        // at its true first-use (the resolve is the first reader of albedo / ssao / grid /
        // index / cascade / atlas, and the writer of lit), so this single `record_pass`
        // emits the barriers those producers deferred to their consumers.
        // SAFETY: recording is open; `record_graph_pass` records the graph's derived input
        // barriers for the "resolve" pass into `cmd` against the live G-buffer targets.
        let resolve = self
            .gbuffer_pass_plan
            .as_ref()
            .expect("invariant: declare_frame_graph ran before record_gbuffer")
            .resolve;
        // HW-RT rung R0: open the DeferredResolve bracket BEFORE the resolve's input barriers
        // + dispatch. This spans the WHOLE resolve dispatch, INCLUDING the inline SDF
        // soft-shadow march (R0 brackets passes, not shader sections). GATED — `None` records
        // nothing.
        if let Some(tc) = scene.gpu_timing {
            // SAFETY: recording is open; `self.fns` is the live device fn-table; the pool was
            // reset at the frame top; this write is outside any rendering scope (the resolve is a
            // compute dispatch); `fi` is this present's in-flight slot.
            unsafe { tc.write_begin(self.fns, cmd, fi, TimedPass::DeferredResolve) };
        }
        self.record_graph_pass(resolve, cmd, targets, scene, fi);

        // (5b) Deferred RESOLVE pass: bind the resolve pipeline + the resolve set (gAlbedo
        // @0, gMaterial @1, lit @2 — all STORAGE in GENERAL), dispatch at the SAME grid the
        // marcher used (1:1 the marched pixels). It composites `lit = mask ? base*vis : base`.
        //
        // R2a-4b: select the `(pipeline, layout, set)` TRIPLE together. Route to the HWRT `rayQuery`
        // variant (its mesh-shadow term traces the TLAS at binding 19) ONLY when BOTH hold:
        //   (1) the scene wires the HWRT resolve pipeline+layout AND the targets carry the 20-binding
        //       HWRT resolve set (i.e. `feature = "hwrt"` + `ctx.ray_query_enabled()` + config
        //       HardwareTri — all boot-gated, present on EVERY RT+hwrt frame); AND
        //   (2) a TLAS was (re)built + build→trace-barriered THIS frame — i.e. `scene.tlas.is_some()`
        //       (the per-frame `TlasBuildActivation`, `None` when the drawable count is 0).
        // Condition (2) is the P0 correctness gate: the TLAS build + the AS-write→read barrier
        // (gbuffer.rs:311-326) run ONLY under `scene.tlas.is_some()`, so on a zero-mesh-instance frame
        // (warm-up / gather-lag / all-SDF / count-drops-to-0) NO build + NO barrier ran — tracing
        // `tlas[fi].accel` there would read an UNBUILT (device-lost UB) or stale, un-barriered AS.
        // When (2) is false we fall back to the SOFTWARE triple (which needs no TLAS — a zero-mesh
        // frame casts no mesh shadows anyway), so the HWRT resolve is selected ⟺ a TLAS was built +
        // barriered this frame. The showcase (count > 0 every frame) is behaviorally unchanged.
        // The layout, pipeline, and set MUST swap in LOCK-STEP — the HWRT layout has 20 bindings vs
        // the software 19; a mismatch is a device-lost.
        #[cfg(feature = "hwrt")]
        let hwrt_triple = scene
            .tlas
            .as_ref()
            .and(scene.resolve_pipeline_hwrt.zip(scene.resolve_layout_hwrt))
            .and_then(|(pipe, layout)| {
                targets
                    .resolve_set_hwrt
                    .as_ref()
                    .map(|sets| (pipe.pipeline, pipe.layout, &sets[self.frame_index], layout))
            });
        // FORWARD-SEAM INVARIANT (hardening; NO functional change): `scene.tlas.is_some()`
        // is the SOLE predicate that arms the HW shadow resolve — the host folds the
        // shadow-backend decision (`RayBackendConfig`'s HardwareTri mesh-shadow cell + the
        // owner's force-software knob) INTO `tlas_enabled`, so a disarmed TLAS ⇒ the
        // software resolve here. Selecting `hwrt_triple` therefore implies `scene.tlas`.
        // RISK: a FUTURE workload that consumes the TLAS for something OTHER than mesh
        // shadows (AO / GI / reflections on hardware) must SPLIT the TLAS-arm predicate
        // from the shadow-backend decision — the TLAS may then be armed while the shadow
        // cell is software (or vice versa), and folding both into one `tlas_enabled` bit
        // would mis-route this selection. This is the riskiest forward seam.
        #[cfg(feature = "hwrt")]
        debug_assert!(
            hwrt_triple.is_none() || scene.tlas.is_some(),
            "invariant: the HW shadow resolve is armed ⟺ scene.tlas.is_some() (the sole predicate)"
        );
        // HW-RT rung 3a: the DENOISED resolve triple (the à-trous ON path). When the scene wires
        // `scene.shadow` (the step-7 gate; kept `None` this rung), the resolve binds the DENOISED
        // pipeline (`deferred_pbr_hwrt_denoised.comp`, reading the FILTERED `gShadowVis` @21) + its
        // 22-binding layout + the DENOISED resolve set — REPLACING the RESOLVE_INLINE-hwrt triple. It
        // takes priority over `hwrt_triple` (both need `scene.tlas`, but `scene.shadow.is_some()`
        // implies the à-trous stack ran this frame). `None` ⇒ fall through to `hwrt_triple`
        // (RESOLVE_INLINE) or the software triple ⇒ byte-identical.
        #[cfg(feature = "hwrt")]
        let denoised_triple = scene.shadow.as_ref().and_then(|sh| {
            // Rung 3b step 6 (S1): on a TEMPORAL frame the DENOISED resolve reads `temporal_out` via the
            // sibling `shadow_temporal_denoised_resolve_set` (@21 = `temporal_out[fi]`); otherwise it
            // reads the à-trous FINAL ring via `shadow_denoised_resolve_set` (the Rung-3a path,
            // byte-identical). `temporal_active()` is the SINGLE source shared with the graph's temporal
            // read + the temporal dispatch above (W1). If the selected set is absent (a gate mismatch),
            // the `.map` yields `None` ⇒ `denoised_triple` falls through to the RESOLVE_INLINE
            // `hwrt_triple` — matching the temporal-dispatch degrade above (never a temporal-DENOISED
            // bind reading an unwritten `temporal_out`).
            let sets = if scene.temporal_active() {
                targets.shadow_temporal_denoised_resolve_set.as_ref()
            } else {
                targets.shadow_denoised_resolve_set.as_ref()
            };
            sets.map(|sets| {
                (
                    sh.denoised_pipeline.pipeline,
                    sh.denoised_pipeline.layout,
                    &sets[self.frame_index],
                )
            })
        });
        // The software triple (the default / byte-identical path).
        let (resolve_pipeline_h, resolve_layout_h, resolve_set_h) = {
            #[cfg(feature = "hwrt")]
            if let Some((p, l, s)) = denoised_triple {
                (p, l, s)
            } else if let Some((p, l, s, _layout)) = hwrt_triple {
                (p, l, s)
            } else {
                (
                    scene.resolve_pipeline.pipeline,
                    scene.resolve_pipeline.layout,
                    &targets.resolve_set[self.frame_index],
                )
            }
            #[cfg(not(feature = "hwrt"))]
            {
                (
                    scene.resolve_pipeline.pipeline,
                    scene.resolve_pipeline.layout,
                    &targets.resolve_set[self.frame_index],
                )
            }
        };
        // SAFETY: recording is open; the selected resolve pipeline + its layout (the software
        // `resolve_layout` at set 0, or the HWRT 20-binding layout when routing is Hardware — chosen
        // as one triple so the pipeline/layout/set never mismatch) are live on this device (caller
        // contract); the selected set binds the now-stored (GENERAL) albedo/material + the lit
        // (GENERAL) images (+ the binding-19 TLAS on the HWRT set); `dispatch_group_count_x` covers
        // `present_extent`'s pixel count (the same grid the marcher dispatched); `&...descriptor_set`
        // is a single-element local alive for the call (first_set 0, count 1, zero dynamic offsets).
        // The resolve pushes NO constants.
        unsafe {
            (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, resolve_pipeline_h);
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                resolve_layout_h,
                0,
                1,
                &resolve_set_h.descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
        }
        // HW-RT rung R0: close the DeferredResolve bracket AFTER the resolve dispatch. GATED.
        if let Some(tc) = scene.gpu_timing {
            // SAFETY: recording is open; the pool was reset this frame; the resolve dispatch is
            // recorded; `fi` is this slot.
            unsafe { tc.write_end(self.fns, cmd, fi, TimedPass::DeferredResolve) };
        }

        // Anti-aliasing Stage 4 (TAA W5): the temporal-resolve pass — recorded HERE, BEFORE
        // `present_sample`'s `lit` GENERAL→SHADER_READ_ONLY_OPTIMAL transition below. TAA is a
        // COMPUTE dispatch that reads `lit` at `GENERAL`, straight out of the resolve's write
        // (the framegraph's `taa_resolve` pass is declared in that exact position — see
        // `graph_bridge.rs`), the OPPOSITE ordering FXAA/SMAA/SSAA use (FRAGMENT graphics
        // pipelines reading `lit` AFTER the SHADER_READ_ONLY_OPTIMAL transition, below). Gated on
        // `scene.taa.is_some()` AND `targets.taa_resolve_set.is_some()` (kept in lockstep by
        // `GBufferTargets::create`) — `None` on every other `AaMode` records nothing here.
        if let Some(taa) = scene.taa.as_ref()
            && targets.taa_resolve_set.is_some()
        {
            let taa_pass = self
                .gbuffer_pass_plan
                .as_ref()
                .expect("invariant: declare_frame_graph ran before record_gbuffer")
                .taa_resolve
                .expect("invariant: scene.taa.is_some() ⇒ the taa_resolve pass was declared");
            // SAFETY: recording is open; `aa_out`/`taa_hist`/`taa_resolve_set` were built by
            // `create()` under the same `scene.taa` that gates this branch; `taa_pass` was
            // declared this frame under the same gate (the invariant above).
            unsafe { self.record_taa(cmd, targets, taa, taa_pass, scene, fi) };

            // TAA rung T3: the post-resolve RCAS sharpen pass — recorded IMMEDIATELY after
            // `record_taa` (resolve THEN rcas), still BEFORE `present_sample` below. Gated on
            // `scene.rcas.is_some()` AND `targets.rcas_set.is_some()` (kept in lockstep by
            // `GBufferTargets::create`, which itself only arms `taa_resolved`/`rcas_set` when
            // `scene.rcas.is_some()`, which in turn requires `scene.taa.is_some()` — the SAME
            // lockstep discipline `scene.taa`/`targets.taa_resolve_set` use above). `None` (the
            // 0%-gate, `SharpenMode::None`) records nothing here — byte-identical to the
            // pre-RCAS resolve.
            if let Some(rcas) = scene.rcas.as_ref()
                && targets.rcas_set.is_some()
            {
                // SAFETY: recording is open; `record_taa` (just above) already wrote
                // `taa_resolved[fi]`, leaving it in GENERAL; `taa_resolved`/`aa_out`/`rcas_set`
                // were built by `create()` under the same `scene.rcas` that gates this branch;
                // `present_extent` sizes both `taa_resolved` and `aa_out` (the SAME extent the
                // resolve dispatched over).
                unsafe { self.record_rcas(cmd, targets, rcas, present_extent, scene, fi) };
            }
        }

        // (5c) LIT: GENERAL → SHADER_READ_ONLY_OPTIMAL for the present-blit sample. The
        // present now samples LIT (the resolve's output), NOT albedo (the deletion target
        // of the old step-6 albedo→SHADER_READ_ONLY barrier — albedo stays GENERAL,
        // consumed only by the resolve as a STORAGE-in-GENERAL load).
        // The graph's "present_sample" pass (declaring `lit`
        // FRAGMENT/SHADER_READ/SHADER_READ_ONLY) DRIVES this transition. The SWAPCHAIN WSI
        // barriers below (sites 7/9) stay HAND-recorded — the acquired presentable image
        // is not a graph resource here.
        // SAFETY: recording is open; `record_graph_pass` records the graph's derived
        // GENERAL→SHADER_READ_ONLY barrier for the "present_sample" pass into `cmd`,
        // making the resolve's lit store available + visible to the present-blit's sample.
        let present_sample = self
            .gbuffer_pass_plan
            .as_ref()
            .expect("invariant: declare_frame_graph ran before record_gbuffer")
            .present_sample;
        self.record_graph_pass(present_sample, cmd, targets, scene, fi);

        // Anti-aliasing Stage 1 (FXAA) / Stage 2 (SMAA) / Stage 3 (SSAA). Stage 4 (TAA) was
        // ALREADY recorded above (before `present_sample` — see that block's ordering comment).
        // `sync_gbuffer` keeps `targets.aa_out.is_some() == (scene.aa.is_some() ||
        // scene.smaa.is_some() || scene.ssaa.is_some() || scene.taa.is_some())` in lockstep (an
        // arm-state change forces a fence-safe resync, exactly like an extent change), so these
        // always agree within a frame. Gate on `aa_out` (what `present_set` follows) so any
        // transient mismatch degrades to "present samples lit, no AA pass" — never a panic. RAW
        // barriers on `aa_out` only (the DDGI-update/TLAS-build precedent); `lit` needs none
        // (already SHADER_READ_ONLY_OPTIMAL from `present_sample` above). Consumes `lit` after
        // the framegraph's last declared use — safe until a transient-aliasing allocator lands;
        // exempt this site then. OFF (`aa_out` is `None`) records nothing. FXAA is checked
        // FIRST (byte-identical to the committed Stage-1 dispatch); `scene.aa`/`scene.smaa`/
        // `scene.ssaa`/`scene.taa` are mutually exclusive by construction (`debug_assert!` in
        // `GBufferTargets::create`). SSAA uses `aa_extent` (the BOOT-FIXED native extent
        // `aa_out` was actually allocated at), NOT `extent` (live, tracks window resizes) or
        // `present_extent` (2× under SSAA) — the crux difference from FXAA/SMAA.
        if targets.aa_out.is_some() {
            if let Some(fxaa) = scene.aa.as_ref() {
                // SAFETY: recording is open; `present_sample` above left `lit` in
                // SHADER_READ_ONLY_OPTIMAL; `aa_out`/`fxaa_set` were built by `create()`
                // under the same `scene.aa` that gates this branch; `present_extent` sizes
                // `aa_out`.
                unsafe { self.record_fxaa(cmd, targets, fxaa, present_extent, fi) };
            } else if let Some(smaa) = scene.smaa.as_ref() {
                // SAFETY: recording is open; `present_sample` above left `lit` in
                // SHADER_READ_ONLY_OPTIMAL; `aa_out`/`smaa_edges`/`smaa_weights`/the three
                // `smaa_*_set` rings were built by `create()` under the same `scene.smaa`
                // that gates this branch; `present_extent` sizes every SMAA target.
                unsafe { self.record_smaa(cmd, targets, smaa, present_extent, fi) };
            } else if let Some(ssaa) = scene.ssaa.as_ref() {
                debug_assert!(targets.aa_out.is_some() && targets.downsample_set.is_some());
                // SAFETY: recording is open; `present_sample` above left `lit` (the 2× ring)
                // in SHADER_READ_ONLY_OPTIMAL; `aa_out`/`downsample_set` were built by
                // `create()` under the same `scene.ssaa` that gates this branch, sized to
                // `aa_extent` (the BOOT-FIXED native size, NOT `present_extent`, which is 2×
                // under SSAA, and NOT the live `extent`, which tracks window resizes).
                unsafe { self.record_ssaa(cmd, targets, ssaa, aa_extent, fi) };
            } else {
                // `aa_out.is_some()` with none of aa/smaa/ssaa matched ⇒ TAA is the reason
                // (the four arms are mutually exclusive by construction); `record_taa` already
                // ran above — nothing left to do here.
                debug_assert!(
                    scene.taa.is_some(),
                    "invariant: aa_out is armed but none of aa/smaa/ssaa/taa matched"
                );
            }
        }

        // === Pass C: present-blit the LIT image (the resolve's output, or `aa_out` when
        // AA is armed) into the swapchain. ===

        // (7) Barrier (swapchain color): UNDEFINED → COLOR_ATTACHMENT_OPTIMAL.
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

        // (8) Dynamic rendering: the swapchain image (CLEAR/STORE), no depth. The
        // present pipeline's declared color format equals the swapchain format (W2-b).
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
        let present_rendering = VkRenderingInfo {
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
        // Present the composite at its NATIVE size in the swapchain image's TOP-LEFT,
        // NOT stretched to the (possibly WSI-clamped wider) swapchain extent. The
        // viewport/scissor are clamped to `min(swapchain_extent, present_extent)` at
        // origin: the fullscreen triangle writes exactly the composite's pixels 1:1, and
        // a wider swapchain image's remainder keeps the clear color. A 1:1 top-left
        // mapping makes a per-texel golden exact regardless of any WSI clamp.
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
        let blit_scissor = VkRect2D {
            offset: VkOffset2D { x: 0, y: 0 },
            extent: blit_extent,
        };
        // SAFETY: recording is open; `present_rendering` is fully initialized — its color
        // attachment names the live swapchain `view` (now COLOR_ATTACHMENT_OPTIMAL);
        // dynamic rendering is enabled. The present pipeline + its bind-group layout
        // belong to this device (caller contract) and its declared color format equals
        // the swapchain's (W2-b). The present set @fi binds `lit[fi]` (now
        // SHADER_READ_ONLY_OPTIMAL — the SAME slot the resolve just wrote) + sampler at set 0;
        // `blit_viewport`/`blit_scissor` outlive the bracketed calls; `draw(3, 1, 0, 0)`
        // is the `SV_VertexID` fullscreen triangle (no vertex buffer). Begin/End bracket
        // pass C exactly.
        unsafe {
            (self.fns.cmd_begin_rendering)(cmd, &present_rendering);
            (self.fns.cmd_bind_pipeline)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                scene.present_pipeline.pipeline,
            );
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

        // (9) The post-draw swapchain transition: steady → PRESENT, or the readback
        // path → TRANSFER_SRC, copy-to-buffer, → PRESENT (identical to
        // `record_present_sampled`'s branch — the swapchain still presents after the
        // copy).
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
                    image_extent: VkExtent3D {
                        width: extent.width,
                        height: extent.height,
                        depth: 1,
                    },
                };
                // SAFETY: recording is open; the swapchain image is TRANSFER_SRC_OPTIMAL
                // per the barrier above; one full-image tightly-packed color region
                // copies into the live host-visible `staging.buffer` (≥ the image's byte
                // size per this fn's contract); `&region` outlives the call. This copies
                // the SWAPCHAIN image (the on-screen golden) — NOT the depth (the depth
                // copy is the deletion target this path proves absent).
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
                // TRANSFER_SRC→PRESENT releases the image to the present engine after the
                // readback copy; `&to_present` outlives the call.
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
