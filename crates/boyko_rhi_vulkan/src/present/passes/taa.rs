//! `Renderer::record_taa`: the anti-aliasing Stage 4 (TAA) temporal-resolve pass body,
//! recorded right after the frame's last `lit` producer and BEFORE the post seam — see
//! [`gbuffer`](super::gbuffer)'s record-site ordering comment and [`TaaActivation`]'s "Compute,
//! not graphics" doc.
//!
//! Render wave PX1a (D-S5 step 1): the resolve writes only `taa_hist[fi]`; `post_final` reads it
//! as the scene input (render wave PX1b: under the sharpen bit, its RCAS stage reads the five-tap
//! cross of the same image), so this body records no hand barrier — every image it touches is
//! frame-graph tracked.

use core::ptr;

use crate::ffi::*;

use super::super::frame_driver::Renderer;
use super::super::gpu_zone::ZONE_POST_TAA_RESOLVE;
use super::super::scene_types::{GBufferScene, TaaActivation};
use super::super::targets::GBufferTargets;

impl Renderer<'_> {
    /// Records the TAA temporal-resolve compute dispatch into `cmd`:
    ///
    /// 1. `record_graph_pass` emits the framegraph's derived barriers for the "taa_resolve" pass
    ///    (`lit`/`viewt`/`taa_hist_read` reads, `taa_hist[fi]` write — all framegraph-tracked).
    /// 2. Bind the resolve pipeline + `taa_resolve_set[fi]` + push the 4-byte `{ uint reset; }`
    ///    constant (`activation.reset`), dispatch `scene.dispatch_group_count_x` groups
    ///    (`numthreads(64,1,1)`, the same 1D pixel grid the resolve/marcher use).
    ///
    /// # Safety
    ///
    /// `cmd` must be recordable (recording open, within the caller's begin/end bracket);
    /// `targets.taa_hist` / `targets.taa_resolve_set` must be `Some` (the caller gates on the
    /// declared `taa_resolve` pass, which the declarator declares only under live TAA —
    /// `scene.taa` armed AND [`GBufferTargets::taa_history_built`] of these targets, review r1
    /// W1); `activation.resolve_pipeline`'s layout matches `taa_resolve_set`'s (7 bindings + a
    /// 4-byte COMPUTE push range); `fi` is this frame's in-flight slot; `taa_pass` is the
    /// "taa_resolve" pass declared this frame in [`Renderer::frame_graph`].
    pub(crate) unsafe fn record_taa(
        &self,
        cmd: VkCommandBuffer,
        targets: &GBufferTargets,
        activation: &TaaActivation<'_>,
        taa_pass: crate::framegraph::PassId,
        scene: &GBufferScene<'_>,
        fi: usize,
    ) {
        // `record_graph_pass` records the "taa_resolve" pass's derived barriers
        // (lit/viewt/taa_hist_read reads, taa_hist[fi] write) into `cmd` against the live
        // G-buffer targets — a safe fn; recording is open per this fn's caller contract.
        self.record_graph_pass(taa_pass, cmd, targets, scene, fi);
        // SAFETY: forwarded caller contract (recording open, taa targets/sets armed in
        // lockstep, `fi` this frame's slot); the pass barriers were just emitted above.
        unsafe { self.record_taa_body(cmd, targets, activation, scene, fi) };
    }

    /// TAA-under-VB seam: the graph-emit-free resolve body — everything [`Self::record_taa`]
    /// does AFTER emitting the framegraph barriers. The VB recorder emits its `taa_resolve`
    /// pass's barriers through its OWN [`VbBarrierSink`](super::super::graph_bridge::VbBarrierSink)
    /// (`record_vb_pass` — a different ResId table than the deferred sink) and then calls this
    /// directly; the Deferred path keeps calling [`Self::record_taa`], whose emit + this body
    /// are byte-identical to the pre-split fn.
    ///
    /// # Safety
    ///
    /// Same contract as [`Self::record_taa`], PLUS: the frame's `taa_resolve` graph pass
    /// barriers (lit→SHADER_READ_ONLY, viewt/hist reads, `taa_hist[fi]` write) must have
    /// ALREADY been emitted into `cmd` by the caller's own sink.
    pub(crate) unsafe fn record_taa_body(
        &self,
        cmd: VkCommandBuffer,
        targets: &GBufferTargets,
        activation: &TaaActivation<'_>,
        scene: &GBufferScene<'_>,
        fi: usize,
    ) {
        // Render wave PX0: `ZONE_POST_TAA_RESOLVE` brackets this body — its own barriers, bind,
        // push and dispatch; the graph-derived barriers the caller emitted are outside it.
        let zone = super::zone_arm::ZoneArm::from_scene(scene);
        // SAFETY: recording is open (this fn's contract), outside any rendering scope; the path
        // recorder's witness reset this frame's pool and seals the slot after this body.
        let zone_pair = unsafe { zone.begin(self.fns, cmd, ZONE_POST_TAA_RESOLVE) };
        let set = targets
            .taa_resolve_set
            .as_ref()
            .expect("invariant: armed scene.taa implies taa_resolve_set was built alongside it");

        let reset_push: u32 = activation.reset as u32;
        // SAFETY: recording is open; `activation.resolve_pipeline` + its 8-binding layout are
        // live on this device (caller contract); `set[fi]` binds `gLit`/`gViewT`/`gHistIn`/
        // `gHistOut` + the `ResolvedTaa`/camera/`MotionCam` UBOs for `fi`;
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

        // SAFETY: as the begin above; `zone_pair` is the pair it returned for this zone.
        unsafe { zone.end(self.fns, cmd, ZONE_POST_TAA_RESOLVE, zone_pair) };
    }
}
