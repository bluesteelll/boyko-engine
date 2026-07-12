//! `Renderer::record_taa`: the anti-aliasing Stage 4 (TAA) temporal-resolve pass body,
//! recorded right after the main deferred-resolve dispatch, BEFORE `present_sample`'s
//! `lit` GENERAL→SHADER_READ_ONLY_OPTIMAL transition — see [`gbuffer`](super::gbuffer)'s
//! record-site ordering comment and [`TaaActivation`]'s "Compute, not graphics" doc for why
//! this differs from FXAA/SMAA/SSAA's post-`present_sample` ordering.

use core::ptr;

use crate::ffi::*;

use super::super::frame_driver::Renderer;
use super::super::scene_types::{GBufferScene, TaaActivation};
use super::super::targets::GBufferTargets;
use super::super::COLOR_SUBRESOURCE_RANGE;

impl Renderer<'_> {
    /// Records the TAA temporal-resolve compute dispatch into `cmd`:
    ///
    /// 1. `record_graph_pass` emits the framegraph's derived barriers for the "taa_resolve" pass
    ///    (`lit`/`viewt`/`taa_hist_read` reads, `taa_hist[fi]` write — all framegraph-tracked).
    /// 2. A HAND-RECORDED barrier `aa_out[fi]` UNDEFINED → GENERAL (`aa_out` is NOT a
    ///    framegraph-tracked resource — see [`crate::present::graph_bridge::FRAMEGRAPH_IMAGE_COUNT`]'s
    ///    doc — mirrors FXAA's own hand-recorded `aa_out` barrier, adapted to a STORAGE-image
    ///    write instead of a color-attachment write). `UNDEFINED` is valid on EVERY frame (not
    ///    just the first): the shader writes every dispatched pixel of `gAaOut` unconditionally
    ///    (both its `reset_now` and normal branches), so the prior contents are always fully
    ///    discarded, exactly like FXAA's fullscreen-triangle overwrite.
    /// 3. Bind the resolve pipeline + `taa_resolve_set[fi]` + push the 4-byte `{ uint reset; }`
    ///    constant (`activation.reset`), dispatch `scene.dispatch_group_count_x` groups
    ///    (`numthreads(64,1,1)`, the same 1D pixel grid the resolve/marcher use).
    /// 4. A HAND-RECORDED barrier `aa_out[fi]` GENERAL → SHADER_READ_ONLY_OPTIMAL, so the
    ///    present-blit's sample (pass C, recorded after this call via the repointed
    ///    `present_set`) sees a valid, ordered read.
    ///
    /// # Safety
    ///
    /// `cmd` must be recordable (recording open, within the caller's begin/end bracket);
    /// `targets.aa_out` / `targets.taa_hist` / `targets.taa_resolve_set` must be `Some` (the
    /// caller gates on `scene.taa.is_some() && targets.taa_resolve_set.is_some()`, kept in
    /// lockstep by [`GBufferTargets::create`]); `activation.resolve_pipeline`'s layout matches
    /// `taa_resolve_set`'s (8 bindings + a 4-byte COMPUTE push range); `fi` is this frame's
    /// in-flight slot; the "taa_resolve" pass must be declared in [`Renderer::frame_graph`]
    /// (the caller's `scene.taa.is_some()` gate implies `declare_gbuffer_graph` declared it —
    /// see `graph_bridge.rs`).
    pub(crate) unsafe fn record_taa(
        &self,
        cmd: VkCommandBuffer,
        targets: &GBufferTargets,
        activation: &TaaActivation<'_>,
        taa_pass: crate::framegraph::PassId,
        scene: &GBufferScene<'_>,
        fi: usize,
    ) {
        let aa = targets
            .aa_out
            .as_ref()
            .expect("invariant: record gate (scene.taa.is_some()) implies aa_out Some");
        let set = targets
            .taa_resolve_set
            .as_ref()
            .expect("invariant: armed scene.taa implies taa_resolve_set was built alongside it");

        // `record_graph_pass` records the "taa_resolve" pass's derived barriers
        // (lit/viewt/taa_hist_read reads, taa_hist[fi] write) into `cmd` against the live
        // G-buffer targets — a safe fn; recording is open per this fn's caller contract.
        self.record_graph_pass(taa_pass, cmd, targets, scene, fi);

        // --- Barrier 1: aa_out[fi] UNDEFINED → GENERAL (the resolve's STORAGE-image write). ---
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
        // SAFETY: recording is open (caller contract); one image barrier on the live `aa[fi]`
        // texture (built by `create()` when `scene.taa` was armed); TOP_OF_PIPE→COMPUTE_SHADER
        // with UNDEFINED→GENERAL is the superset-correct pre-dispatch transition (the shader
        // writes every dispatched pixel unconditionally, so UNDEFINED-discard is always valid,
        // not just on the first frame); `&to_general` outlives the call.
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

        let reset_push: u32 = activation.reset as u32;
        // SAFETY: recording is open; `activation.resolve_pipeline` + its 8-binding layout are
        // live on this device (caller contract); `set[fi]` binds `gLit`/`gViewT`/`gHistIn`/
        // `gHistOut`/`gAaOut` + the `ResolvedTaa`/camera/`MotionCam` UBOs for `fi`;
        // `scene.dispatch_group_count_x` covers the pixel count (`numthreads(64,1,1)`, the
        // resolve/marcher grid); `&set[fi].descriptor_set` is a single-element local alive for
        // the call; the 4-byte push covers the pipeline's declared COMPUTE range exactly.
        unsafe {
            (self.fns.cmd_bind_pipeline)(
                cmd,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                activation.resolve_pipeline.pipeline,
            );
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                activation.resolve_pipeline.layout,
                0,
                1,
                &set[fi].descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_push_constants)(
                cmd,
                activation.resolve_pipeline.layout,
                VK_SHADER_STAGE_COMPUTE_BIT,
                0,
                4,
                (&reset_push as *const u32).cast(),
            );
            (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
        }

        // --- Barrier 2: aa_out[fi] GENERAL → SHADER_READ_ONLY_OPTIMAL (the present-blit read). --
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
    }
}
