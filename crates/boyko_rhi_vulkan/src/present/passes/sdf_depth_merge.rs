//! Render wave RD1 (defect R0b) — the SDF depth merge's recorder.
//!
//! One full-screen triangle in a depth-only dynamic-rendering scope over the path's depth image:
//! the fragment shader (`sdf_depth_merge.fs`) reads the composite `t` the SDF marcher left in
//! `gViewT` and writes the path's encode of it through `SV_Depth`, depth-tested with the path's
//! compare op. Recorded IMMEDIATELY before `particle_draw` on all three paths — the position the
//! declarators declared it in (`graph_bridge.rs`'s `declare_sdf_depth_merge`).
//!
//! # Every barrier is derived
//!
//! Like `record_particle_draw`, this fn takes the path's barrier callback and invokes it once, at
//! the pass's position, so it never learns which path it is on: Deferred's `depth` arrives
//! `SHADER_READ_ONLY` (a transition), Forward's / VB's depth arrives on a pending attachment write
//! (an availability barrier), and on VisibilityBuffer × Sdf `vb_depth` arrives UNDEFINED — the
//! merge is its first touch, so it CLEARs on load. The load op is tied to that derivation by a
//! debug check (`transparent_depth::debug_check_merge_clear`, critique W1).

use core::ptr;

use boyko_rhi::TimestampStage;

use crate::device::DeviceFns;
use crate::ffi::*;
use crate::framegraph::PassId;
use crate::rhi_impl::VulkanBindGroup;

#[cfg(feature = "profiling-census")]
use super::super::command_witness::CommandWitness;
use super::super::frame_driver::Renderer;
use super::super::gpu_zone::{GpuZoneRecorder, ZONE_SDF_DEPTH_MERGE, zone_begin_stage};
use super::super::scene_types::{GBufferScene, SdfDepthMergeActivation};
use super::super::transparent_depth::SdfDepthMerge;

/// What the merge renders into, resolved by the caller from ITS OWN per-path targets.
#[derive(Clone, Copy)]
pub(crate) struct MergeTargets<'a> {
    /// The path's depth image view, entered at `DEPTH_ATTACHMENT_OPTIMAL`.
    pub(crate) depth_view: VkImageView,
    /// This frame slot's merge set: `gViewT[fi]` @0 (`SampledImageAtGeneral`), camera ring @1.
    pub(crate) set: &'a VulkanBindGroup,
    /// The path's FAR depth — the CLEAR value of a first-touch merge (reverse-Z `0.0`).
    pub(crate) clear_depth: f32,
    /// The composite extent: the render area, the viewport, and the extent the marcher's
    /// `generate_ray` used (the push's `img_w` / `img_h`).
    pub(crate) extent: VkExtent2D,
}

/// The zone-recorder handle a path recorder hands the merge recorder — `ParticleZoneArm`'s shape
/// (`passes/particles.rs`'s doc: arming is a claim about the CALLER, whose witness reset this
/// frame's pool and will seal the slot after the last bracket).
#[derive(Clone, Copy)]
pub(crate) struct MergeZoneArm<'a> {
    /// The frame's zone recorder and the ring slot it opened, or `None` when this frame records no
    /// zones.
    zr: Option<(&'a GpuZoneRecorder, usize)>,
    /// The command census, when a gate armed one.
    #[cfg(feature = "profiling-census")]
    cw: Option<&'a CommandWitness>,
}

impl<'a> MergeZoneArm<'a> {
    /// The arm for a path whose own witness reset this frame's pool and seals the slot after the
    /// merge — every path since render wave PX0 gave Forward its `FrameWitness` (the
    /// `disarmed()` constructor Forward needed was deleted with its last caller).
    #[inline]
    pub(crate) fn from_scene(scene: &GBufferScene<'a>) -> MergeZoneArm<'a> {
        MergeZoneArm {
            zr: scene.gpu_zone,
            #[cfg(feature = "profiling-census")]
            cw: scene.vb_cmd_witness,
        }
    }

    /// One witnessed record site. Compiles to nothing without `profiling-census`.
    #[inline]
    fn cmd(&self) {
        #[cfg(feature = "profiling-census")]
        if let Some(w) = self.cw {
            w.command();
        }
    }

    /// Opens [`ZONE_SDF_DEPTH_MERGE`]'s bracket; returns its pair, or `None` (no command) when
    /// unarmed or the ring slot is full.
    ///
    /// # Safety
    /// Recording is open on `cmd`, outside a rendering scope; `fns` is the live device fn-table;
    /// this frame's pool was reset by the caller's witness (the arm's contract).
    #[inline]
    unsafe fn begin(&self, fns: &DeviceFns, cmd: VkCommandBuffer) -> Option<u16> {
        let (rec, ring) = self.zr?;
        let pair = rec.alloc_pair(ring, ZONE_SDF_DEPTH_MERGE)?;
        // SAFETY: caller contract; `pair` came from `alloc_pair` on this slot immediately above.
        let stage = unsafe {
            rec.record_begin(fns, cmd, ring, pair, zone_begin_stage(ZONE_SDF_DEPTH_MERGE))
        };
        self.mark(stage, true);
        Some(pair)
    }

    /// Closes the bracket [`Self::begin`] opened (no-op without a pair).
    ///
    /// # Safety
    /// As [`Self::begin`], for this zone's end query.
    #[inline]
    unsafe fn end(&self, fns: &DeviceFns, cmd: VkCommandBuffer, pair: Option<u16>) {
        if let (Some((rec, ring)), Some(pair)) = (self.zr, pair) {
            // SAFETY: caller contract; `pair` is the one `begin` allocated for this zone.
            let stage = unsafe { rec.record_end(fns, cmd, ring, pair) };
            self.mark(stage, false);
        }
    }

    /// The census half of a BEGIN (`open`) or an END.
    #[inline]
    fn mark(&self, stage: TimestampStage, open: bool) {
        #[cfg(feature = "profiling-census")]
        if let Some(w) = self.cw {
            if open {
                w.open_pair(ZONE_SDF_DEPTH_MERGE);
                w.timestamp(stage);
            } else {
                w.timestamp(stage);
                w.close_pair(ZONE_SDF_DEPTH_MERGE);
            }
        }
        #[cfg(not(feature = "profiling-census"))]
        let _ = (stage, open);
    }
}

impl Renderer<'_> {
    /// Render wave RD1: records the SDF depth merge — `pass`'s derived barriers, a depth-only
    /// rendering scope over `targets.depth_view` (`loadOp = CLEAR` on a first touch, else `LOAD`;
    /// `storeOp = STORE`), and, unless `shape` is [`SdfDepthMerge::ClearOnly`], one full-screen
    /// triangle through `act.pipeline`.
    ///
    /// `barriers` is the path's own `record_*_pass`, invoked once before the scope.
    ///
    /// # Safety
    ///
    /// * `cmd` is an OPEN command buffer recording OUTSIDE any dynamic-rendering scope.
    /// * `targets.depth_view` is a live view of the path depth `pass` declares (its first access),
    ///   which the derived barrier leaves at `DEPTH_ATTACHMENT_OPTIMAL`; `targets.set` binds this
    ///   frame slot's `gViewT` (left at `GENERAL` by its producer's declared write, read as
    ///   `SampledImageAtGeneral`) and camera ring slot, written against `act.layout`.
    /// * `act.pipeline` was built against `act.layout` at set 0 with a FRAGMENT push range of
    ///   `SDF_DEPTH_MERGE_PUSH_BYTES` at offset 0 and the D32 depth format `depth_view` has.
    /// * If `zones` is armed, this frame's query pool was reset by the caller's witness and that
    ///   witness seals the ring slot after this call.
    #[allow(clippy::too_many_arguments)] // each argument is a distinct record-site input
    pub(crate) unsafe fn record_sdf_depth_merge<F: FnMut(PassId)>(
        &self,
        cmd: VkCommandBuffer,
        act: &SdfDepthMergeActivation<'_>,
        pass: PassId,
        shape: SdfDepthMerge,
        clears: bool,
        targets: MergeTargets<'_>,
        zones: MergeZoneArm<'_>,
        mut barriers: F,
    ) {
        debug_assert!(shape != SdfDepthMerge::Absent, "invariant: an Absent merge records nothing");
        // SAFETY: recording is open and outside a rendering scope (this fn's contract); the pool
        // was reset this frame (the arm's contract) and this is the only site stamping this id.
        let pair = unsafe { zones.begin(self.fns, cmd) };
        zones.cmd();
        barriers(pass);
        // Critique W1: the load op this recorder chose must be the one the graph's derivation
        // implies — CLEAR exactly on a first touch (an UNDEFINED old layout in this pass's
        // barriers). A LOAD of a first-touched depth reads undefined memory; a CLEAR of a written
        // one erases the opaque depth. Neither is visible to validation or to a static image.
        #[cfg(debug_assertions)]
        {
            // The path depth is the merge's FIRST declared access (`declare_sdf_depth_merge`), so
            // the check names the resource the graph derived from, not one this site restates.
            let depth_res = self
                .frame_graph
                .pass_accesses(pass)
                .next()
                .expect("invariant: the merge declares the path depth first")
                .res;
            super::super::transparent_depth::debug_check_merge_clear(
                &self.frame_graph,
                pass,
                depth_res,
                clears,
            );
        }

        let area = VkRect2D { offset: VkOffset2D { x: 0, y: 0 }, extent: targets.extent };
        let viewport = VkViewport {
            x: 0.0,
            y: 0.0,
            width: targets.extent.width as f32,
            height: targets.extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let depth_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: targets.depth_view,
            image_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: if clears { VK_ATTACHMENT_LOAD_OP_CLEAR } else { VK_ATTACHMENT_LOAD_OP_LOAD },
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue {
                depth_stencil: VkClearDepthStencilValue { depth: targets.clear_depth, stencil: 0 },
            },
        };
        let rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: area,
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: 0,
            p_color_attachments: ptr::null(),
            p_depth_attachment: (&depth_attachment as *const VkRenderingAttachmentInfo).cast(),
            p_stencil_attachment: ptr::null(),
        };
        let push = crate::compute::SdfDepthMergePush::new(
            act.encode,
            targets.extent.width,
            targets.extent.height,
        );
        let push_bytes = push.as_bytes();

        // SAFETY: every precondition is this fn's contract — recording is open and outside a
        // rendering scope; the depth view is live and in the layout the derived barrier left it in
        // (or UNDEFINED → cleared by this scope's CLEAR on a first touch, the W1 check above);
        // the pipeline's set-0 layout is the set's, its FRAGMENT push range is exactly
        // `push_bytes` (20 B) at offset 0, and its depth format is the view's (D32). The three `&`
        // temporaries (`depth_attachment`, `rendering`, `viewport`/`area`) outlive the calls.
        // Begin/End bracket the scope exactly; a ClearOnly merge records the empty scope alone.
        unsafe {
            zones.cmd();
            (self.fns.cmd_begin_rendering)(cmd, &rendering);
            if shape == SdfDepthMerge::Draw {
                zones.cmd();
                (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, act.pipeline.pipeline);
                zones.cmd();
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_GRAPHICS,
                    act.pipeline.layout,
                    0,
                    1,
                    &targets.set.descriptor_set,
                    0,
                    ptr::null(),
                );
                zones.cmd();
                (self.fns.cmd_push_constants)(
                    cmd,
                    act.pipeline.layout,
                    act.pipeline.push_stages,
                    0,
                    push_bytes.len() as u32,
                    push_bytes.as_ptr().cast(),
                );
                zones.cmd();
                (self.fns.cmd_set_viewport)(cmd, 0, 1, &viewport);
                zones.cmd();
                (self.fns.cmd_set_scissor)(cmd, 0, 1, &area);
                zones.cmd();
                (self.fns.cmd_draw)(cmd, 3, 1, 0, 0);
            }
            zones.cmd();
            (self.fns.cmd_end_rendering)(cmd);
        }
        // SAFETY: recording is open and the rendering scope closed immediately above; the matching
        // begin was recorded at the top of this fn (or none was, and `pair` is `None`).
        unsafe { zones.end(self.fns, cmd, pair) };
    }
}
