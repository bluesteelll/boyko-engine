//! The Render Dependency Graph bridge for the whole G-buffer frame: the
//! [`GbufferPassPlan`] (per-frame `PassId` map), the [`GbufferBarrierSink`] that
//! resolves each derived barrier to a physical `VkImage`/`VkBuffer` and records it,
//! and the `Renderer` methods that (re)declare the graph + drive one pass's derived
//! barriers. Split out of the former monolithic `swapchain.rs` (audit W4).

use core::ptr;

use crate::device::DeviceFns;
use crate::ffi::*;
use crate::texture::{MAX_CASCADES, MAX_TEXTURE_LAYERS};

use super::frame_driver::Renderer;
use super::scene_types::GBufferScene;
use super::targets::GBufferTargets;
use super::COLOR_SUBRESOURCE_RANGE;

/// The per-frame [`PassId`](crate::framegraph::PassId) of each G-buffer pass declared
/// into [`Renderer::frame_graph`] by [`Renderer::render_gbuffer_frame`] (Steps 1d/1e).
///
/// `record_gbuffer` reads this (via `as_ref().expect(...)`) at each barrier site to
/// drive that pass's derived barriers when the framegraph flag is ON. The optional
/// members mirror `record_gbuffer`'s config gates EXACTLY: a member is `Some` iff its
/// pass body is recorded this frame, so a graph barrier is never emitted for a pass
/// whose GPU work was skipped (and vice versa — no double-barrier, no missing one).
#[derive(Clone, Copy)]
pub(crate) struct GbufferPassPlan {
    /// Pillar B B3: the per-instance TRS interpolation compute PRE-PASS (`scene.interp.
    /// is_some()`). Recorded BEFORE the raster pass; its `record_pass` is a no-op (the interp
    /// pass reads the FIF-private pair SSBO + writes the FIF-private draw SSBO — both frame-
    /// private, so the graph derives NO input barrier). The COMPUTE→VERTEX RAW barrier on the
    /// draw SSBO is derived at the raster pass (the draw-buffer READER) and emitted by
    /// `record_pass(raster)` — after this pass's dispatch wrote the draw columns.
    pub(crate) interp: Option<crate::framegraph::PassId>,
    /// The 3-MRT + depth mesh raster pass (sites 0/1). Multi-paradigm render-path plan, rung
    /// R2 (Decision 2 / O1): `Some` iff [`GBufferScene::path_has_raster`] holds. Under Deferred
    /// (the only reachable path today), the resolver's R2 guard
    /// (`DEFERRED_LEG_DISABLE_IMPLEMENTED == false`) keeps `GeometryLegs` pinned to `Both`, so
    /// this is `Some` on every current frame — byte-identical to the pre-R2 always-present pass.
    pub(crate) raster: Option<crate::framegraph::PassId>,
    /// The async light-table re-upload (`scene.light_dirty && light_upload_bytes>0`).
    pub(crate) light_upload: Option<crate::framegraph::PassId>,
    /// The P0 coarse tile-cull (`scene.coarse.is_some()`).
    pub(crate) coarse: Option<crate::framegraph::PassId>,
    /// The SDF marcher pass. Its `record_pass` emits the collapsed input transitions
    /// (depth→sampled, color→general, lit/viewt first-touch — sites 3/3b/4). Multi-paradigm
    /// render-path plan, rung R2 (Decision 2 / O1): `Some` iff
    /// [`GBufferScene::path_has_marcher`] holds — see [`Self::raster`]'s doc for the R2 guard
    /// that keeps this `Some` on every currently reachable frame.
    pub(crate) marcher: Option<crate::framegraph::PassId>,
    /// The SSAO pass (`scene.ssao.is_some()`).
    pub(crate) ssao: Option<crate::framegraph::PassId>,
    /// The SSAO edge-avoiding à-trous denoise chain: one pass per dispatch level (`scene.ssao`
    /// carries the per-frame level count `SsaoActivation::atrous_levels`, `0` or
    /// `2..=MAX_SSAO_ATROUS_LEVELS`). UNCONDITIONAL (both feature legs — SOFTWARE, NOT
    /// `hwrt`-gated, unlike [`Self::shadow_atrous`]). Exactly `atrous_levels` are populated; the
    /// unused tail slots stay `None`. Each level's role (read the R8 `gSsao` endpoint / an
    /// interior R16 ring / write BACK into `gSsao`) is [`crate::present::ssao_atrous_step`]'s
    /// [`crate::present::AtrousStepRole`].
    pub(crate) ssao_atrous:
        [Option<crate::framegraph::PassId>; crate::present::MAX_SSAO_ATROUS_LEVELS as usize],
    /// SDFDDGI I2: the probe-update pass (`scene.ddgi_update.is_some()`). Writes the two atlas
    /// storage images + the classification buffer; the update→resolve barrier is DERIVED at the
    /// resolve (the atlas reader), NOT hand-written.
    pub(crate) ddgi_update: Option<crate::framegraph::PassId>,
    /// The L1 clustered light-cull pass (`scene.cluster_cull.is_some()` + the buffers).
    pub(crate) light_cull: Option<crate::framegraph::PassId>,
    /// The CSM cascade depth pass (`scene.csm.is_some()`).
    pub(crate) csm: Option<crate::framegraph::PassId>,
    /// The sparse spot/point atlas depth pass (`scene.atlas_punctual.is_some()`).
    pub(crate) atlas: Option<crate::framegraph::PassId>,
    /// HW-RT rung R2a-3: the TLAS-instance PACK compute pre-pass (`scene.tlas.is_some()`). Runs
    /// after interp, before the raster `begin_rendering`; writes the `tlas_instances` array
    /// (COMPUTE/SHADER_WRITE) and, when interp ran, reads the shared ring (COMPUTE/SHADER_READ).
    /// `Some` iff `scene.tlas.is_some()` (the "member is Some iff its body is recorded" invariant).
    #[cfg(feature = "hwrt")]
    pub(crate) tlas_pack: Option<crate::framegraph::PassId>,
    /// HW-RT rung R2a-3: the per-frame TLAS BUILD pass (`scene.tlas.is_some()`), right after
    /// `tlas_pack`. Reads the `tlas_instances` array at the AS-build stage
    /// (AS_BUILD/SHADER_READ), deriving the pack-write → build-read barrier; the AS write into
    /// the UNTRACKED backing/scratch is invisible to the graph.
    #[cfg(feature = "hwrt")]
    pub(crate) tlas_build: Option<crate::framegraph::PassId>,
    /// HW-RT rung 3a: the VIS pre-pass (`scene.shadow.is_some()`) — the resolve front-matter re-run
    /// that traces the TLAS + WRITES `shadow_vis`. Reads gNormal/gViewT + the tlas buffer at COMPUTE;
    /// `Some` iff `scene.shadow.is_some()` (the "member is Some iff its body is recorded" invariant).
    /// Recorded BEFORE the à-trous passes + the resolve.
    #[cfg(feature = "hwrt")]
    pub(crate) shadow_vis: Option<crate::framegraph::PassId>,
    /// HW-RT rung 3a: the per-level à-trous denoise passes (`scene.shadow.is_some()`), ping-ponging
    /// `shadow_vis` / `shadow_vis2`. Exactly `levels` (`1..=MAX_ATROUS_LEVELS`) are populated; the
    /// unused tail slots stay `None`. Each reads the in-ResId + gNormal/gViewT, writes the out-ResId.
    /// The last one's write → resolve-read barrier is derived at the resolve.
    #[cfg(feature = "hwrt")]
    pub(crate) shadow_atrous:
        [Option<crate::framegraph::PassId>; crate::present::MAX_ATROUS_LEVELS as usize],
    /// HW-RT Rung 3b step 6: the temporal reproject+accumulate pass (`scene.temporal_active()`).
    /// Recorded AFTER the à-trous chain, BEFORE the resolve; reads the à-trous FINAL output
    /// (`final_vis_res`) + `motion_vec` + `viewt`, writes `shadow_temporal_hist[fi]` + `temporal_out`.
    /// The cross-frame `shadow_temporal_hist[1-fi]` READ is bound DIRECTLY in the set (seeded GENERAL,
    /// not framegraph-tracked). `Some` iff `scene.temporal_active()` (the "member is Some iff its body
    /// is recorded" invariant). The write → resolve-read barrier on `temporal_out` is derived at the
    /// resolve.
    #[cfg(feature = "hwrt")]
    pub(crate) shadow_temporal: Option<crate::framegraph::PassId>,
    /// The always-present deferred resolve pass.
    pub(crate) resolve: crate::framegraph::PassId,
    /// Anti-aliasing Stage 4 (TAA W5): the temporal-resolve pass (`scene.taa.is_some()`).
    /// Recorded AFTER the main resolve dispatch, BEFORE `present_sample` (it reads `lit` at
    /// `GENERAL`, straight out of the resolve's write — see `TaaActivation`'s "Compute, not
    /// graphics" doc). Reads `lit`/`viewt`/`taa_hist_read`, writes `taa_hist[fi]`; `aa_out`'s
    /// UNDEFINED→GENERAL→SHADER_READ_ONLY_OPTIMAL barriers are hand-recorded (untracked, like
    /// every other AA mode's `aa_out`). `Some` iff `scene.taa.is_some()` (the "member is Some
    /// iff its body is recorded" invariant).
    pub(crate) taa_resolve: Option<crate::framegraph::PassId>,
    /// The always-present present-sample pass: only the `lit` GENERAL→SHADER_READ_ONLY
    /// transition (site 5c). The swapchain WSI barriers stay hand-recorded.
    pub(crate) present_sample: crate::framegraph::PassId,
}

/// The number of IMAGE resources the whole-frame graph declares (Steps 1d/1e), in the
/// FIXED ResId order the sink resolves by: albedo=0, normal=1, material=2, depth=3,
/// viewt=4, lit=5, ssao=6, cascade=7, atlas=8, ddgi_irr=9, ddgi_depth=10. Buffer ResIds
/// follow, offset by this on a `not(hwrt)` build. SDFDDGI I2 appended the two DDGI atlas storage
/// images (9/10) — declared UNCONDITIONALLY (seeded with the boot `SHADER_READ_ONLY_OPTIMAL`
/// layout) but only ACCESSED on the `ddgi_update`/`resolve` passes that name them, so the OFF-path
/// barrier set (which never routes a barrier at ResId 9/10) is byte-unchanged.
///
/// `feature = "hwrt"`: the count is cfg-selected `12 → 18`. The SIX `hwrt`-only images
/// (`shadow_vis` (ResId 11) .. `shadow_temporal_hist_read` (ResId 16)) are declared LAST in the
/// image block, AFTER `ddgi_depth`, BEFORE the first `add_buffer` — their ResIds 11..16 are
/// UNCHANGED by T6a's addition below (byte-unchanged doc/comments throughout that block):
/// - Rung 3a: `shadow_vis` (ResId 11) + `shadow_vis2` (ResId 12) — the à-trous ping-pong.
/// - Rung 3b: `motion_vec` (ResId 13, RG16F) + `shadow_temporal_hist` (ResId 14, RGBA16, seeded
///   GENERAL cross-frame) + `temporal_out` (ResId 15, RG16) — the temporal reproject targets.
/// - Rung 3b C1/H2: `shadow_temporal_hist_read` (ResId 16) — the cross-frame sibling/READ image.
///
/// Textured-PBR T6a appends `pbr` (the `gPbr` deferred-resolve MRT lane) as the LAST image, AFTER
/// every `hwrt`-only image (or right after `ddgi_depth` on a `not(hwrt)` build, where there is no
/// `hwrt` block) — so it lands at ResId 11 (`not(hwrt)`) / 17 (`hwrt`), i.e. exactly the OLD
/// `FRAMEGRAPH_IMAGE_COUNT` value on each leg, and the buffers still begin at the NEW
/// `FRAMEGRAPH_IMAGE_COUNT` on both builds by construction (the three `- FRAMEGRAPH_IMAGE_COUNT`
/// buffer re-base sites re-base by the SAME const, so a buffer's LOGICAL sink slot is unchanged).
/// `pbr` is declared UNCONDITIONALLY (both feature legs) via `add_image_seeded` with
/// `ResSync::undefined()` — a fresh, discard-legal UNDEFINED→GENERAL transition is derived EVERY
/// frame at the `resolve` pass's unconditional `image_access` (no producer pass writes `pbr` this
/// rung; the `seeded` flag exempts it from `compile`'s unwritten-transient-read authoring guard,
/// which would otherwise correctly flag an always-read-never-written transient image).
///
/// No pass accesses ResId 11..16 (the `hwrt`-only images) on the current (non-temporal) frame, so
/// the derived barrier set on THOSE resources is unchanged (byte-identical render); the VIS/à-
/// trous/temporal passes (gated on `scene.shadow`) add the accesses when armed.
///
/// Anti-aliasing Stage 4 (TAA W4) appends TWO more images — AFTER `pbr`, UNCONDITIONALLY
/// (both feature legs): `taa_hist` (the write ResId) + `taa_hist_read` (the C1-fix cross-frame
/// read-sibling, mirroring `shadow_temporal_hist`/`shadow_temporal_hist_read`). They land at
/// ResId 12/13 (`not(hwrt)`) or 18/19 (`hwrt`) — exactly the OLD `FRAMEGRAPH_IMAGE_COUNT`/
/// `+1` on each leg — so every EARLIER ResId (0..old-count-1) is byte-unchanged and the buffers
/// still begin at the NEW `FRAMEGRAPH_IMAGE_COUNT`. No pass accesses them unless `scene.taa` is
/// `Some` (`AaMode::Taa` armed — W5's `taa_resolve` pass) ⇒ byte-identical on every other mode.
///
/// The SSAO à-trous denoise chain's RHI DISPATCH WIRING follow-up appends TWO more images, LAST —
/// AFTER `taa_hist_read`, UNCONDITIONALLY (both feature legs — SOFTWARE, NOT `hwrt`-gated):
/// `ssao_ring_a` + `ssao_ring_b` (the two `R16_UNORM` interior ping-pong rings). They land at
/// ResId 14/15 (`not(hwrt)`) or 20/21 (`hwrt`) — exactly the OLD `FRAMEGRAPH_IMAGE_COUNT`/`+1` on
/// each leg (mirroring the TAA-append precedent immediately above), so every EARLIER ResId is
/// byte-unchanged. No pass accesses them unless the SSAO à-trous chain's per-frame level count
/// (`SsaoActivation::atrous_levels`) is `> 0` ⇒ byte-identical on the `N == 0` OFF path.
#[cfg(feature = "hwrt")]
pub(crate) const FRAMEGRAPH_IMAGE_COUNT: usize = 22;
/// See the `hwrt` variant: a `not(hwrt)` build keeps the count at 16 (11 base + `pbr` + TAA's
/// `taa_hist`/`taa_hist_read` + the SSAO à-trous `ssao_ring_a`/`ssao_ring_b`, no shadow-vis
/// targets, byte-unchanged on every ResId 0..10).
#[cfg(not(feature = "hwrt"))]
pub(crate) const FRAMEGRAPH_IMAGE_COUNT: usize = 16;

/// The REAL [`BarrierSink`](crate::framegraph::BarrierSink) for the whole G-buffer
/// frame (Steps 1c–1e): it resolves each derived barrier's logical `res` → the current
/// physical `VkImage`/`VkBuffer` (images indexed by the ResId `0..9`, buffers by
/// `res.index() - FRAMEGRAPH_IMAGE_COUNT` — the graph's fixed declaration order) and
/// records ONE batched sync1 `vkCmdPipelineBarrier` per barrier group, exactly as the
/// hand path did.
///
/// Lives only for the duration of one `record_pass` call; borrows the device fn-table
/// and the open command buffer.
pub(crate) struct GbufferBarrierSink<'a> {
    pub(crate) fns: &'a DeviceFns,
    pub(crate) cmd: VkCommandBuffer,
    /// The physical images resolved by image `ResId` index `0..FRAMEGRAPH_IMAGE_COUNT`
    /// — `[albedo, normal, material, depth, viewt, lit, ssao, cascade, atlas, ddgi_irr,
    /// ddgi_depth, ..(hwrt-only).., pbr, taa_hist, taa_hist_read, ssao_ring_a, ssao_ring_b]` for
    /// the current frame slot (`ddgi_irr`/`ddgi_depth` are
    /// SDFDDGI I2 single-instance world-fixed atlases — NOT ringed; `pbr` — textured-PBR T6a's
    /// `gPbr` — IS ringed, like albedo/normal/etc., and is declared/bound LAST, AFTER every
    /// `hwrt`-only image, so it never perturbs their ResIds; `ssao_ring_a`/`ssao_ring_b` — the
    /// SSAO à-trous denoise chain's interior ping-pong rings — are appended LAST of all, mirroring
    /// `pbr`'s "append at the end" discipline). MUST match the graph's declaration order. A pass
    /// that does NOT declare an optional image (e.g. cascade when CSM is off, or the DDGI atlases
    /// when the update pass is off) never routes a barrier naming that ResId, so its slot may hold
    /// [`VkImage::NULL`] harmlessly.
    ///
    /// Rung 3a (`hwrt`): the array grows by SIX — `shadow_vis` (ResId 11) + `shadow_vis2` (ResId
    /// 12), the ringed RT soft-shadow-visibility targets, declared right AFTER `ddgi_depth`
    /// (`pbr` is appended LAST, past this whole block, so these ResIds are UNCHANGED by T6a). Each
    /// slot carries the current frame slot's handle when the targets are allocated, or
    /// [`VkImage::NULL`] when the device lacks `RG8`/`RG16` UNORM storage (the DDGI-degrade mirror
    /// — the targets are `Option`-guarded on the boot probe). In THIS step no pass names ResId
    /// 11/12, so their slots are never handed to the driver either way; steps 4-6 add the passes
    /// that read them (gated on the same `shadow_denoise_storage_ok()` predicate).
    pub(crate) images: [VkImage; FRAMEGRAPH_IMAGE_COUNT],
    /// The physical buffers resolved by `res.index() - FRAMEGRAPH_IMAGE_COUNT` (the graph's
    /// FIXED buffer declaration order): `[light_table, tiles, grid, index, alloc,
    /// ddgi_classification, ddgi_ray_table, interp_pairs, interp_out_slot, interp_model_out]`.
    /// The two SDFDDGI I2 buffers (classification + Fibonacci ray-table) are single-instance,
    /// named by the `ddgi_update` pass ONLY when `scene.ddgi_update.is_some()`. The interp trio
    /// (Pillar B B3, refined-B) is the CURRENT frame slot's FIF-ringed interpolation SSBOs,
    /// bound ONLY when `scene.interp.is_some()`.
    ///
    /// Under `hwrt` the array grows by ONE: `tlas_instances` (HW-RT rung R2a-3) is declared
    /// UNCONDITIONALLY right AFTER the DDGI buffers (so its ResId is FIXED regardless of the
    /// conditional interp trio, which then shifts one slot later), landing at index 7 — the
    /// order becomes `[.., ddgi_ray_table, tlas_instances, interp_pairs, interp_out_slot,
    /// interp_model_out]`. On a `not(hwrt)` build the array is exactly `[VkBuffer; 10]` with
    /// unchanged indices (RISK-2: cfg-gated ⇒ no OFF-path ResId shift). On any OFF path an
    /// ungated slot is never named by a derived barrier, so its [`VkBuffer::NULL`] is inert
    /// (same NULL-when-ungated rule as [`Self::images`]).
    #[cfg(not(feature = "hwrt"))]
    pub(crate) buffers: [VkBuffer; 10],
    /// See the `not(hwrt)` variant's doc: `hwrt` grows this by one (`tlas_instances` at index 7).
    #[cfg(feature = "hwrt")]
    pub(crate) buffers: [VkBuffer; 11],
}

/// Compile-time guard that [`GbufferBarrierSink::images`] is exactly [`FRAMEGRAPH_IMAGE_COUNT`]
/// long on BOTH builds (`22` under `hwrt`, `16` otherwise). The field's `[VkImage;
/// FRAMEGRAPH_IMAGE_COUNT]` type already ties the two; this pins the concrete count so an
/// accidental const edit (e.g. adding a third shadow target without growing the const) trips
/// here, and the `record_graph_pass` array literal — whose element count the compiler checks
/// against this same field type — cannot silently drift.
const _: () = {
    #[cfg(feature = "hwrt")]
    assert!(
        FRAMEGRAPH_IMAGE_COUNT == 22,
        "hwrt: 11 base + pbr (textured-PBR T6a) + shadow_vis + shadow_vis2 + motion_vec + shadow_temporal_hist + temporal_out + shadow_temporal_hist_read + taa_hist + taa_hist_read + ssao_ring_a + ssao_ring_b"
    );
    #[cfg(not(feature = "hwrt"))]
    assert!(
        FRAMEGRAPH_IMAGE_COUNT == 16,
        "not(hwrt): the 11 base images + pbr (textured-PBR T6a) + taa_hist + taa_hist_read + ssao_ring_a + ssao_ring_b, no shadow-vis targets"
    );
};

impl crate::framegraph::BarrierSink for GbufferBarrierSink<'_> {
    fn image_barriers(
        &mut self,
        src_stage: u32,
        dst_stage: u32,
        group: &[crate::framegraph::ImgBarrier],
    ) {
        debug_assert!(
            group.len() <= crate::framegraph::MAX_PASS_BARRIERS,
            "image barrier group ({}) exceeds MAX_PASS_BARRIERS",
            group.len()
        );
        // Build the `VkImageMemoryBarrier` array on the stack (alloc-free) from the
        // logical group, resolving each `res` → the bound physical image. Field style
        // mirrors the hand `to_color`/`to_depth` barriers this replaces. (`VkImageMemoryBarrier`
        // is not `Copy` — it carries a `p_next` pointer — so `from_fn` fills each slot;
        // the tail `[n..]` slots are inert placeholders never handed to the driver.)
        let n = group.len();
        let arr: [VkImageMemoryBarrier; crate::framegraph::MAX_PASS_BARRIERS] =
            core::array::from_fn(|i| {
                if i < n {
                    let b = group[i];
                    VkImageMemoryBarrier {
                        s_type: VkStructureType::ImageMemoryBarrier,
                        p_next: ptr::null(),
                        src_access_mask: b.src_access,
                        dst_access_mask: b.dst_access,
                        old_layout: b.old_layout,
                        new_layout: b.new_layout,
                        src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                        dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                        image: self.images[b.res.index()],
                        subresource_range: VkImageSubresourceRange {
                            aspect_mask: b.subresource.aspect,
                            base_mip_level: b.subresource.base_mip,
                            level_count: b.subresource.mip_count,
                            base_array_layer: b.subresource.base_layer,
                            layer_count: b.subresource.layer_count,
                        },
                    }
                } else {
                    VkImageMemoryBarrier {
                        s_type: VkStructureType::ImageMemoryBarrier,
                        p_next: ptr::null(),
                        src_access_mask: 0,
                        dst_access_mask: 0,
                        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                        new_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                        src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                        dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                        image: VkImage::NULL,
                        subresource_range: COLOR_SUBRESOURCE_RANGE,
                    }
                }
            });
        // SAFETY: the command buffer is open (this sink is only driven inside the open
        // `record_gbuffer` recording). Every `arr[i].image` was resolved from the
        // `images[res.index()]` slot (a live G-buffer image for the current frame);
        // `res.index()` is in `0..FRAMEGRAPH_IMAGE_COUNT` for every image barrier the
        // whole-frame graph derives (images are ResId `0..FRAMEGRAPH_IMAGE_COUNT` — `0..12`,
        // or `0..18` under `hwrt`). The masks/layouts/
        // subresource are the graph-derived Vk values that reproduce the hand path's
        // transitions. `arr[..n]` (a stack array) outlives the call; the count == `n`.
        // No memory or buffer barriers (`0, ptr::null(), 0, ptr::null()`), matching the
        // hand image calls.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                self.cmd,
                src_stage,
                dst_stage,
                0,
                0,
                ptr::null(),
                0,
                ptr::null(),
                n as u32,
                arr.as_ptr().cast(),
            );
        }
    }

    fn buffer_barriers(
        &mut self,
        src_stage: u32,
        dst_stage: u32,
        group: &[crate::framegraph::BufBarrier],
    ) {
        debug_assert!(
            group.len() <= crate::framegraph::MAX_PASS_BARRIERS,
            "buffer barrier group ({}) exceeds MAX_PASS_BARRIERS",
            group.len()
        );
        // Build the `VkBufferMemoryBarrier` array on the stack (alloc-free), resolving
        // each `res` → the bound physical buffer via `res.index() - FRAMEGRAPH_IMAGE_COUNT`
        // (buffers are declared AFTER the images, so a buffer ResId is offset by the
        // image count). Field style mirrors the hand `to_shader_read`/`tiles_barrier`/
        // `cull_to_resolve` barriers this replaces. Whole-buffer range (`offset: 0`,
        // `size: VK_WHOLE_SIZE`) matches every hand buffer barrier. `VkBufferMemoryBarrier`
        // is `Copy` (no `p_next` payload), but `from_fn` keeps the tail slots inert
        // placeholders never handed to the driver (count == `n`).
        let n = group.len();
        let arr: [VkBufferMemoryBarrier; crate::framegraph::MAX_PASS_BARRIERS] =
            core::array::from_fn(|i| {
                if i < n {
                    let b = group[i];
                    VkBufferMemoryBarrier {
                        s_type: VkStructureType::BufferMemoryBarrier,
                        p_next: ptr::null(),
                        src_access_mask: b.src_access,
                        dst_access_mask: b.dst_access,
                        src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                        dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                        buffer: self.buffers[b.res.index() - FRAMEGRAPH_IMAGE_COUNT],
                        offset: 0,
                        size: VK_WHOLE_SIZE,
                    }
                } else {
                    VkBufferMemoryBarrier {
                        s_type: VkStructureType::BufferMemoryBarrier,
                        p_next: ptr::null(),
                        src_access_mask: 0,
                        dst_access_mask: 0,
                        src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                        dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                        buffer: VkBuffer::NULL,
                        offset: 0,
                        size: VK_WHOLE_SIZE,
                    }
                }
            });
        // SAFETY: the command buffer is open (this sink is only driven inside the open
        // `record_gbuffer` recording). Every `arr[i].buffer` was resolved from the
        // `buffers[res.index() - FRAMEGRAPH_IMAGE_COUNT]` slot (a live scene buffer for
        // this frame); a buffer barrier's `res.index()` is always `>= FRAMEGRAPH_IMAGE_COUNT`
        // (buffers are declared after the images — 11, or 13 under `hwrt`) and `< FRAMEGRAPH_IMAGE_COUNT +
        // buffers.len()` (the 5 core + 2 DDGI + 3 interp buffer ResIds, plus the 1 R2a-3
        // `tlas_instances` ResId under `hwrt`).
        // The masks are the graph-derived Vk values that reproduce the hand path's
        // TRANSFER→COMPUTE / COMPUTE→COMPUTE flush/visibility hazards; the whole-buffer
        // range matches every hand buffer barrier. `arr[..n]` (a stack array) outlives
        // the call; the count == `n`. No memory or image barriers, matching the hand
        // buffer calls.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                self.cmd,
                src_stage,
                dst_stage,
                0,
                0,
                ptr::null(),
                n as u32,
                arr.as_ptr().cast(),
                0,
                ptr::null(),
            );
        }
    }
}

impl Renderer<'_> {
    /// Multi-paradigm render-path plan, rung R2 (§B "Per-path framegraph", Decision 2): the
    /// SINGLE dispatch point `render_gbuffer_frame` calls to (re)declare the whole-frame graph,
    /// selecting the per-path declarator from `scene.resolved_render_path.path`. Every current
    /// call site goes through this fn — never `declare_deferred_graph` (or a future sibling)
    /// directly — so a future path addition only ever ADDS a match arm here.
    ///
    /// The `Deferred` arm is the ONLY implemented one today (`declare_deferred_graph`,
    /// unchanged, byte-identical to the pre-R2 `declare_gbuffer_graph`); the R1 resolver
    /// (`boyko_render::render_path_config::resolve_render_path` — a crate this one sits BELOW
    /// in the dependency graph, hence the plain-text reference rather than an intra-doc link)
    /// degrades every other `RenderPath` request to `Deferred` at boot
    /// (`FORWARD_IMPLEMENTED`/`FORWARD_PLUS_IMPLEMENTED`/`VB_IMPLEMENTED` are all `false`), so
    /// the other three arms are structurally unreachable and `unreachable!` rather than
    /// stubbed — each names the rung that will fill it in.
    pub(crate) fn declare_frame_graph(&mut self, scene: &GBufferScene<'_>) {
        match scene.resolved_render_path.path {
            // RenderPath::Deferred == 0 (render_path_config.rs) — the only rung-landed path.
            0 => self.declare_deferred_graph(scene),
            // RenderPath::Forward == 1 — lands at rung R4.
            1 => unreachable!(
                "resolver degrades unimplemented paths to Deferred (R1 degrade ladder); \
                 RenderPath::Forward's declarator lands at rung R4"
            ),
            // RenderPath::ForwardPlus == 2 — lands at rung R5.
            2 => unreachable!(
                "resolver degrades unimplemented paths to Deferred (R1 degrade ladder); \
                 RenderPath::ForwardPlus's declarator lands at rung R5"
            ),
            // RenderPath::VisibilityBuffer == 3 — lands at rung R8.
            3 => unreachable!(
                "resolver degrades unimplemented paths to Deferred (R1 degrade ladder); \
                 RenderPath::VisibilityBuffer's declarator lands at rung R8"
            ),
            other => unreachable!(
                "invariant: ResolvedRenderPathGpu::path is a RenderPath discriminant in 0..=3, got {other}"
            ),
        }
    }

    /// Steps 1d/1e: re-declare the WHOLE G-buffer frame into `self.frame_graph`
    /// (`reset` + declare + `compile`), config-gated from `scene`, and store the
    /// per-pass [`GbufferPassPlan`] in `self.gbuffer_pass_plan`. Called ONLY by
    /// [`Self::declare_frame_graph`]'s `Deferred` arm, immediately before the `&self`
    /// `record_gbuffer`, which drives each pass's derived barriers through it.
    ///
    /// The declared accesses MUST mirror `record_gbuffer`'s real `(stage, access,
    /// layout, subresource)` for the MAXIMAL permutation — this is the reference
    /// `tests/framegraph_gbuffer_equiv.rs::build_maximal_frame` (minus the swapchain
    /// image, whose WSI barriers stay hand-recorded). Resources are declared in a FIXED
    /// order that pins the ResIds the [`GbufferBarrierSink`] resolves by: images
    /// albedo=0..atlas=8, then SDFDDGI I2 ddgi_irr=9/ddgi_depth=10 (then, under `hwrt`, the SIX
    /// Rung 3a/3b shadow-vis + temporal targets shadow_vis=11..shadow_temporal_hist_read=16),
    /// then textured-PBR T6a's `pbr`=11 (`not(hwrt)`) / =17 (`hwrt`) — declared LAST in the image
    /// block, past every `hwrt`-only image, so it never perturbs their ResIds — then buffers
    /// light_table..alloc, ddgi_classification/ddgi_ray_table, then the (conditional) interp trio —
    /// each buffer at `FRAMEGRAPH_IMAGE_COUNT + slot`, so a buffer's LOGICAL sink slot
    /// (`ResId - FRAMEGRAPH_IMAGE_COUNT`) is cfg-invariant even though its numeric ResId shifts
    /// under `hwrt` (absorbed by the const).
    ///
    /// Zero heap allocation (the arenas keep capacity across `reset`); the per-frame
    /// `compile` walks a ~11-pass line (cheap). Multi-paradigm render-path plan, rung R2: the
    /// `raster`/`marcher` passes are now `Option`-gated on
    /// [`GBufferScene::path_has_raster`]/[`GBufferScene::path_has_marcher`] (O1 single
    /// predicate) — every OTHER declaration in this fn is byte-for-byte unchanged from the
    /// pre-R2 `declare_gbuffer_graph`.
    pub(crate) fn declare_deferred_graph(&mut self, scene: &GBufferScene<'_>) {
        use crate::framegraph::{ResSync, SubRange};

        // The (EARLY|LATE)_FRAGMENT_TESTS stage pair the depth-write barriers use.
        const FRAG: u32 =
            VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT;
        // The marcher/SSAO storage-image read|write access on the G-buffer attributes.
        const RW: u32 = VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT;

        let g = &mut self.frame_graph;
        g.reset();

        // --- Images (FIXED ResId order: albedo=0..atlas=8). The G-buffer attributes +
        // depth + lit + ssao are RINGED per frame-in-flight (`ring[fi]`), so each slot's
        // reuse is already fence-ordered two frames back — they start `undefined()`.
        // The CSM cascade + shadow atlas are SINGLE instances shared by BOTH in-flight
        // frames: their depth passes re-render them every armed frame, so the re-render
        // must ORDER after the sibling frame's still-pipelined resolve reads — the
        // cross-frame seed supplies that WAR src (audit B-003; the world-fixed viewer
        // masked it because identical content made the torn read benign).
        let albedo = g.add_image("albedo");
        let normal = g.add_image("normal");
        let material = g.add_image("material");
        let depth = g.add_image("depth");
        let viewt = g.add_image("viewt");
        let lit = g.add_image("lit");
        let ssao = g.add_image("ssao");
        let cascade = g.add_image_seeded(
            "cascade",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let atlas = g.add_image_seeded(
            "atlas",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        // SDFDDGI I2 (ResIds 9/10): the two DDGI probe atlases (irradiance + depth). SINGLE
        // world-fixed instances (Decision D1/D2 — probe i is the same world point every frame, one
        // persistent atlas, NO ring, NO reprojection). Boot-initialized to SHADER_READ_ONLY_OPTIMAL
        // in `DdgiAtlas::create` (unlike SSAO's UNDEFINED first-touch), so SEED them with that layout
        // (the light_table `add_buffer_seeded` cross-frame-seed pattern) — the RDG then derives the
        // correct SHADER_READ_ONLY_OPTIMAL → GENERAL transition for the update's storage WRITE, and
        // the update-write → resolve-read GENERAL→GENERAL barrier at the resolve (the atlas reader).
        // Declared UNCONDITIONALLY (fixed ResId order) but ACCESSED only by the `ddgi_update` /
        // `resolve` passes that name them, so the OFF path routes no barrier here (byte-identical).
        // Seed the REAL boot layout (SHADER_READ_ONLY_OPTIMAL), NOT the discard-legal UNDEFINED the
        // cascade/atlas use: the DDGI atlases are PERSISTENT accumulators (Decision D2 — round-robin
        // writes 1/N tiles/frame, the rest MUST survive), so the first storage write each frame needs
        // a CONTENT-PRESERVING SHADER_READ_ONLY_OPTIMAL → GENERAL transition. A UNDEFINED oldLayout
        // would let the driver DISCARD the un-updated tiles (plan §2.5/§7).
        let ddgi_irr = g.add_image_seeded(
            "ddgi_irr",
            ResSync::seeded_readers_at_layout(
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            ),
        );
        let ddgi_depth = g.add_image_seeded(
            "ddgi_depth",
            ResSync::seeded_readers_at_layout(
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            ),
        );
        // Rung 3a (`hwrt`, ResIds 11/12): the two RT soft-shadow-visibility targets, declared LAST
        // in the image block — AFTER `ddgi_depth`, BEFORE the first `add_buffer` (light_table).
        // This keeps ResId 0..10 byte-unchanged AND makes `light_table` land at ResId
        // `FRAMEGRAPH_IMAGE_COUNT` on BOTH builds by construction (13 under hwrt, 11 otherwise), so
        // the three `- FRAMEGRAPH_IMAGE_COUNT` buffer re-base sites keep every buffer's LOGICAL sink
        // slot fixed. `shadow_vis` (RG8, R=mesh_vis/G=validity) + `shadow_vis2` (RG16, the à-trous
        // ping-pong) are ringed per-FIF STORAGE targets. Plain `add_image` (undefined first-touch,
        // like the ringed G-buffer images). In THIS step (targets + decls + sink ONLY) NO pass names
        // them via `image_access`, so the graph routes ZERO barriers at ResId 11/12 → the derived
        // barrier set is unchanged → byte-identical render. The VIS / à-trous / RESOLVE_DENOISED
        // passes below access them, but ONLY when `scene.shadow.is_some()` — the host keeps that
        // gate a literal `None` (rung-3a step 7 flips it), so on EVERY current frame NO pass names
        // ResId 11/12 → byte-identical.
        #[cfg(feature = "hwrt")]
        let shadow_vis = g.add_image("shadow_vis"); // ResId 11
        #[cfg(feature = "hwrt")]
        let shadow_vis2 = g.add_image("shadow_vis2"); // ResId 12
        // Rung 3b (`hwrt`, ResIds 13/14/15): the temporal reproject targets, declared LAST in the
        // image block (AFTER `shadow_vis2`, BEFORE the first `add_buffer`), so ResId 0..12 stay
        // byte-unchanged and the buffers still begin at `FRAMEGRAPH_IMAGE_COUNT` (16 under hwrt).
        // `motion_vec` (RG16F Δuv) + `temporal_out` (RG16 accumulated vis) are FRAME-PRIVATE (ringed,
        // written+read within one frame in steps 5-6) ⇒ plain `add_image` (undefined first-touch,
        // like the G-buffer ring images). `shadow_temporal_hist` (RGBA16) is CROSS-FRAME (frame `fi`
        // reads `[1-fi]`, writes `[fi]`) ⇒ `add_image_seeded` at GENERAL — the DDGI-precedent
        // content-preserving seed (I3), so the first temporal frame's read of the sibling slot orders
        // after a real GENERAL layout, never a discard. In THIS step NO pass names ResId 13/14/15
        // (`image_access`), so the graph routes ZERO barriers on them ⇒ the seed is inert and the
        // render is byte-identical; steps 5-6 add the producers + the temporal pass.
        #[cfg(feature = "hwrt")]
        let motion_vec = g.add_image("motion_vec"); // ResId 13
        #[cfg(feature = "hwrt")]
        let shadow_temporal_hist = g.add_image_seeded(
            "shadow_temporal_hist",
            ResSync::seeded_readers_at_layout(
                VK_IMAGE_LAYOUT_GENERAL,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            ),
        ); // ResId 14
        #[cfg(feature = "hwrt")]
        let temporal_out = g.add_image("temporal_out"); // ResId 15
        // Rung 3b step 6b (C1 cross-frame RACE fix): `shadow_temporal_hist` is a CROSS-FRAME
        // PERSISTENT parity ping-pong POOL (frame `fi` WRITES `pool[fi]`, READS `pool[fi^1]` = the
        // sibling in-flight frame's write). BOTH physical images the temporal pass touches must be
        // framegraph-tracked so the graph derives their cross-frame barriers (single-queue submission
        // order reaches the sibling's prior submit — the shipped `seeded_*` cross-frame precedent):
        //   * ResId 14 `shadow_temporal_hist` = the `[fi]` WRITE — `seeded_readers_at_layout` (WAR:
        //     order frame N's write after the sibling's still-pipelined read of the same image).
        //   * ResId 16 `shadow_temporal_hist_read` = the `[fi^1]` READ — `seeded_writer_at_layout`
        //     (content-preserving RAW: `transition()` emits `COMPUTE/SHADER_WRITE → COMPUTE/SHADER_
        //     READ` before frame N's read, ordering it after — and making visible — the sibling
        //     frame N-1's write of that same physical image). WITHOUT this the read was direct-bound
        //     and UNSYNCHRONIZED against the sibling's write — the C1 "wrong only in motion" race.
        // The sink binds ResId 16 to the SIBLING slot `hist[fi^1]` (the ONE non-`[fi]` sink entry).
        // On a non-temporal frame no pass names ResId 14/15/16 ⇒ zero derived barriers ⇒ byte-identical.
        #[cfg(feature = "hwrt")]
        let shadow_temporal_hist_read = g.add_image_seeded(
            "shadow_temporal_hist_read",
            ResSync::seeded_writer_at_layout(
                VK_IMAGE_LAYOUT_GENERAL,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            ),
        ); // ResId 16
        // Textured-PBR T6a: `pbr` (the `gPbr` deferred-resolve MRT lane), declared LAST in the
        // image block — AFTER every `hwrt`-only image, BEFORE the first `add_buffer` — so it lands
        // at ResId 11 (`not(hwrt)`) / 17 (`hwrt`), i.e. exactly the OLD `FRAMEGRAPH_IMAGE_COUNT` on
        // each leg, WITHOUT perturbing the `hwrt`-only images' ResIds 11..16 above. UNCONDITIONAL
        // (both feature legs). RINGED (per-FIF, like albedo/normal/etc.) ⇒ plain `add_image`-style
        // sync would be correct too, but T6a's resolve `image_access` below is UNCONDITIONAL (no
        // producer pass ever writes `pbr` this rung — the flag-gated `.Load` never dynamically
        // fires), which `compile`'s DEBUG-ONLY authoring guard would otherwise flag as "reads a
        // transient image with no prior producer" (a genuine, but here INTENTIONAL, pattern: the
        // read exists purely to keep the STORAGE_IMAGE descriptor's bound layout valid, not for its
        // data). `add_image_seeded` with `ResSync::undefined()` marks it "seeded" (exempting the
        // guard) while keeping the SAME `layout = UNDEFINED` starting state a plain `add_image`
        // would use — so the resolve's first (and only) access this frame still derives a REAL,
        // discard-legal `UNDEFINED → GENERAL` transition, EVERY frame (the T6a first-touch design).
        let pbr = g.add_image_seeded("pbr", ResSync::undefined());
        // Anti-aliasing Stage 4 (TAA W4): `taa_hist` (write) + `taa_hist_read` (the cross-frame
        // read-sibling), declared LAST in the image block — AFTER `pbr`, BEFORE the first
        // `add_buffer` — so ResId 0..(old FRAMEGRAPH_IMAGE_COUNT-1) stay byte-unchanged and the
        // buffers still begin at the NEW `FRAMEGRAPH_IMAGE_COUNT` (the three `- FRAMEGRAPH_IMAGE_
        // COUNT` buffer re-base sites re-base by the SAME const, so a buffer's LOGICAL sink slot is
        // unchanged). UNCONDITIONAL (both feature legs — TAA is not `hwrt`-only, unlike
        // `shadow_temporal_hist`). Declared exactly like the Rung-3b `shadow_temporal_hist` /
        // `shadow_temporal_hist_read` pair (C1 fix precedent — see the comment above `pbr`'s
        // sibling declaration): `taa_hist` is a CROSS-FRAME PERSISTENT parity ping-pong pool (frame
        // `fi` WRITES `pool[fi]`, READS `pool[fi^1]`), so BOTH physical images must be
        // framegraph-tracked for the graph to derive their cross-frame barriers:
        //   * `taa_hist` (write ResId) = the `[fi]` WRITE — `seeded_readers_at_layout` (WAR: order
        //     frame N's write after the sibling's still-pipelined read of the same image).
        //   * `taa_hist_read` (read-sibling ResId) = the `[fi^1]` READ — `seeded_writer_at_layout`
        //     (content-preserving RAW: orders frame N's read after — and makes visible — the
        //     sibling frame N-1's write of that same physical image).
        // The sink binds the read-sibling ResId to `taa_hist[fi^1]` (the ONE non-`[fi]` entry). NO
        // pass names either ResId this rung (the resolve dispatch is a follow-up), so the graph
        // routes ZERO barriers on them ⇒ the seed is inert and the render is byte-identical —
        // EXACTLY the same "declared but not yet accessed" discipline `shadow_vis`/`motion_vec`
        // used between their declaration rung and their first consuming pass.
        let taa_hist = g.add_image_seeded(
            "taa_hist",
            ResSync::seeded_readers_at_layout(
                VK_IMAGE_LAYOUT_GENERAL,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            ),
        );
        let taa_hist_read = g.add_image_seeded(
            "taa_hist_read",
            ResSync::seeded_writer_at_layout(
                VK_IMAGE_LAYOUT_GENERAL,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            ),
        );
        // The SSAO à-trous denoise chain's two interior ping-pong rings (`R16_UNORM`), declared
        // LAST in the image block — AFTER `taa_hist_read`, BEFORE the first `add_buffer` — so
        // every EARLIER ResId (0..old-count-1, incl. every `hwrt`-only + `pbr` + TAA image) stays
        // byte-unchanged and the buffers still begin at the NEW `FRAMEGRAPH_IMAGE_COUNT` (mirrors
        // `pbr`'s / `taa_hist`'s "append at the end" append discipline exactly — see their
        // declarations above). UNCONDITIONAL (both feature legs — SOFTWARE, NOT `hwrt`-gated,
        // unlike `shadow_vis`/`shadow_vis2`). FRAME-PRIVATE (ringed, written+read within one
        // frame, like the G-buffer ring images) ⇒ plain `add_image` (undefined first-touch) — NOT
        // a cross-frame seed like `taa_hist`/`shadow_temporal_hist` (the à-trous chain never reads
        // a sibling in-flight frame's slot). No pass names either ResId this call (the ssao_atrous
        // pass declarations below add the accesses when `N > 0`), so an `N == 0` frame routes ZERO
        // barriers on them ⇒ inert, byte-identical.
        let ssao_ring_a = g.add_image("ssao_ring_a");
        let ssao_ring_b = g.add_image("ssao_ring_b");
        // --- Buffers (ResId FRAMEGRAPH_IMAGE_COUNT..+4) — ALL single instances shared by both in-flight
        // frames (audit B-002). light_table/tiles/grid/index end their frame consumed
        // by a COMPUTE read (resolve / marcher), so a dirty-frame re-write must order
        // after those sibling reads (WAR seed). `alloc` ends its frame on the cull's
        // atomic WRITES with no draining read, so its per-frame TRANSFER reset needs
        // the full memory dependency (writer seed).
        let light_table = g.add_buffer_seeded(
            "light_table",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let tiles = g.add_buffer_seeded(
            "tiles",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let grid = g.add_buffer_seeded(
            "grid",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let index = g.add_buffer_seeded(
            "index",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let alloc = g.add_buffer_seeded(
            "alloc",
            ResSync::seeded_writer(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT),
        );
        // --- SDFDDGI I2 buffers (ResIds 16/17 — sink slots 8/9). Declared UNCONDITIONALLY (so the
        // interp trio below always follows at ResIds 18/19/20 → its fixed sink slots 5/6/7) but
        // ACCESSED only by the `ddgi_update` pass that names them, so the OFF path routes no barrier
        // here. Both are single device-local instances (the classification buffer 1 u32/probe and the
        // boot-static Fibonacci ray table): the update pass reads the ray table, read/writes the
        // classification. Seeded like the other single-instance buffers so a cross-frame re-touch
        // orders after the sibling frame's still-pipelined update reads (WAR seed).
        let ddgi_classification = g.add_buffer_seeded(
            "ddgi_classification",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let ddgi_ray_table = g.add_buffer_seeded(
            "ddgi_ray_table",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        // --- HW-RT rung R2a-3 `tlas_instances` (ResId 18 under `hwrt`; RISK-2) — the compute-
        // written `VkAccelerationStructureInstanceKHR[]` array, the ONLY new framegraph-tracked
        // resource. Declared UNCONDITIONALLY (so its ResId is FIXED regardless of the conditional
        // interp trio, which then shifts to 19/20/21 → sink slots 8/9/10). Per-FIF frame-private
        // (`add_buffer`, undefined seed — the pack fully overwrites `[0..count)` each frame, and
        // the sibling in-flight frame touches the OTHER slot; no cross-frame WAR/WAW hazard).
        // Because the whole declaration is `#[cfg(feature = "hwrt")]`-gated, a `not(hwrt)` build
        // leaves every existing ResId unchanged. On a tlas-off frame no pass declares a
        // `buffer_access` on it, so the graph routes zero barriers naming it (byte-identical OFF).
        #[cfg(feature = "hwrt")]
        let tlas_instances = g.add_buffer("tlas_instances");
        // --- Pillar B B3 interp SSBOs (ResIds 18/19/20; 19/20/21 under `hwrt`, refined-B) —
        // declared ONLY when the
        // interp pass is wired, so the OFF path's ResId + barrier counts are byte-unchanged (the
        // equiv pins). All three are FIF-RINGED (frame-private, like the G-buffer ring): the host
        // writes this frame's slot of `interp_pairs` + `interp_out_slot`, the interp compute
        // writes the DYNAMIC slots of the SHARED `interp_model_out` (the instance ring), and the
        // raster/shadow VS read the SAME `interp_model_out` slot — a sibling in-flight frame
        // touches a DIFFERENT slot. So they start `undefined()` (plain `add_buffer`, NOT seeded):
        // no cross-frame WAR/WAW hazard, only the intra-frame COMPUTE→VERTEX RAW the graph derives
        // at the raster (the model_out reader).
        let (interp_pairs, interp_out_slot, interp_model_out) = if scene.interp.is_some() {
            (
                Some(g.add_buffer("interp_pairs")),
                Some(g.add_buffer("interp_out_slot")),
                Some(g.add_buffer("interp_model_out")),
            )
        } else {
            (None, None, None)
        };

        // Pass `interp` (Pillar B B3, refined-B) — gated `scene.interp.is_some()`. Runs FIRST
        // (before raster): reads the pair + out-slot SSBOs (COMPUTE/SHADER_READ — first touch, no
        // barrier needed on fresh frame-private slots) + writes the SHARED model-out ring
        // (COMPUTE/SHADER_WRITE — the dynamic slots). The COMPUTE→VERTEX barrier ordering this
        // write before the raster VS read is derived at the raster pass (the model_out reader),
        // NOT here.
        let interp = if let (Some(pairs), Some(out_slot), Some(model_out)) =
            (interp_pairs, interp_out_slot, interp_model_out)
        {
            let p = g.add_pass("interp");
            g.buffer_access(
                pairs,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
            g.buffer_access(
                out_slot,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
            g.buffer_access(
                model_out,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            );
            Some(p)
        } else {
            None
        };

        // HW-RT rung R2a-3: the TLAS pack + build passes — declared ONLY when `scene.tlas.is_some()`
        // (armed under hwrt + ray_query + count > 0), so the OFF path's ResId + barrier counts are
        // byte-unchanged. Run after interp, BEFORE the raster `begin_rendering` (the pack reads the
        // shared instance ring; the build reads the pack output).
        #[cfg(feature = "hwrt")]
        let (tlas_pack, tlas_build) = if scene.tlas.is_some() {
            // Pass `tlas_pack`: writes the `tlas_instances` array (COMPUTE/SHADER_WRITE); when the
            // interp pass ran it ALSO reads the shared `interp_model_out` ring the interp compute
            // wrote (COMPUTE/SHADER_READ — deriving the interp-WRITE → pack-READ RAW on the ring),
            // mirroring the raster pass's conditional ring read. When interp is OFF the ring is
            // host-CPU-scattered into host-coherent memory and the submit's host-write → device
            // domain dependency orders it (exactly as the raster VS reads the host-scattered ring),
            // so the pack declares ONLY its `tlas_instances` write.
            let pack = g.add_pass("tlas_pack");
            if let Some(model_out) = interp_model_out {
                g.buffer_access(
                    model_out,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                );
            }
            g.buffer_access(
                tlas_instances,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            );
            // Pass `tlas_build`: reads the `tlas_instances` array at the AS-build stage — the graph
            // derives the pack(COMPUTE/SHADER_WRITE) → build(AS_BUILD/SHADER_READ) barrier. The build
            // writes the AS into the UNTRACKED backing/scratch (invisible to the graph), so ONLY this
            // instance-array read is declared. `VK_ACCESS_SHADER_READ_BIT & WRITE_ACCESS_MASK == 0`,
            // so `sync.rs` classifies it a READ with no `WRITE_ACCESS_MASK`/sync-engine change.
            let build = g.add_pass("tlas_build");
            g.buffer_access(
                tlas_instances,
                VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR,
                VK_ACCESS_SHADER_READ_BIT,
            );
            (Some(pack), Some(build))
        } else {
            (None, None)
        };

        // Pass `raster` (sites 0/1): the 3-MRT G-buffer + depth. Multi-paradigm render-path
        // plan, rung R2 (Decision 2 / O1): gated on `scene.path_has_raster()` — the SAME
        // predicate `record_gbuffer`'s raster begin/end-rendering block checks (a
        // `debug_assert_eq!` there guards the two never diverging). Under the R2 resolver
        // guard (`DEFERRED_LEG_DISABLE_IMPLEMENTED == false`) `GeometryLegs` stays pinned to
        // `Both` under `Deferred`, so this is `true` on every currently reachable frame — the
        // declaration order and every `image_access`/`buffer_access` call below are BYTE-FOR-
        // BYTE unchanged from the pre-R2 unconditional form, just nested one level deeper.
        let raster = if scene.path_has_raster() {
            let p = g.add_pass("raster");
            // Pillar B B3 (refined-B): when the interp pass ran, the raster VS READS the SHARED
            // model-out ring the compute wrote — the graph derives the COMPUTE(WRITE)→VERTEX(READ)
            // RAW barrier here (the reader). The ring is consumed at the VERTEX stage (the raster +
            // shadow VS index `instances[...]`), so declare a VERTEX_SHADER/SHADER_READ access.
            // Declared ONLY when the interp pass exists, so the OFF path derives nothing.
            if let Some(model_out) = interp_model_out {
                g.buffer_access(
                    model_out,
                    VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                );
            }
            for &c in &[albedo, normal, material] {
                g.image_access(
                    c,
                    VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                    VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                    VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    SubRange::COLOR,
                );
            }
            // HW-RT Rung 3b step 5a: when the mesh-MV path is active, the raster pass ALSO writes the
            // `motion_vec` 4th MRT (the mesh Δuv). Declare its COLOR_ATTACHMENT_WRITE so the graph
            // transitions it UNDEFINED → COLOR_ATTACHMENT_OPTIMAL (first-touch, like the other ring
            // color targets) and orders the later temporal-pass read after it. The gate is
            // `mesh_mv_active()` — the SAME predicate the recorder binds the MV pipeline under (NOT
            // `temporal_enabled` alone), so the graph never declares a write the recorder won't emit
            // (W1: gate-divergence on a storage-ok-but-no-ray-query device). OFF (default / non-hwrt)
            // ⇒ no access ⇒ the graph routes ZERO barriers on `motion_vec` ⇒ byte-identical.
            #[cfg(feature = "hwrt")]
            if scene.mesh_mv_active() {
                g.image_access(
                    motion_vec,
                    VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                    VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                    VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    SubRange::COLOR,
                );
            }
            // Textured-PBR T6c: when the TEXTURED path is active, the raster pass ALSO writes the
            // `pbr` 4th MRT (metallic/roughness/AO-modulation/emissive-modulation). Declare its
            // COLOR_ATTACHMENT_WRITE so the graph orders it before the resolve's UNCONDITIONAL
            // `pbr` read (declared below, T6a) — the SAME `mesh_tex_active()` predicate the
            // recorder binds the TEXTURED pipeline under (W1: gate-divergence risk, mirrors
            // `mesh_mv_active` above). TEXTURED and MOTION_VECTORS are mutually exclusive (T6c
            // plan Decision D4), so this and the `motion_vec` write above never both fire the same
            // frame. OFF (the default / non-textured scene) ⇒ no access ⇒ the graph routes ZERO
            // EXTRA barriers on `pbr` beyond the resolve's own first-touch UNDEFINED → GENERAL ⇒
            // byte-identical (T6a's gate).
            if scene.mesh_tex_active() {
                g.image_access(
                    pbr,
                    VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                    VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                    VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    SubRange::COLOR,
                );
            }
            g.image_access(
                depth,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::DEPTH,
            );
            Some(p)
        } else {
            None
        };

        // Pass `light_upload` (async light-table re-upload) — gated exactly as the hand
        // site: `light_dirty && light_upload_bytes > 0`.
        let light_upload = if scene.light_dirty && scene.light_upload_bytes > 0 {
            let p = g.add_pass("light_upload");
            g.buffer_access(
                light_table,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_ACCESS_TRANSFER_WRITE_BIT,
            );
            Some(p)
        } else {
            None
        };

        // Pass `coarse` (P0 coarse tile-cull) — gated `scene.coarse.is_some()`. Samples
        // depth (transitions it to SHADER_READ_ONLY) + writes the tiles buffer.
        let coarse = if scene.coarse.is_some() {
            let p = g.add_pass("coarse");
            g.image_access(
                depth,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                SubRange::DEPTH,
            );
            g.buffer_access(
                tiles,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            );
            Some(p)
        } else {
            None
        };

        // Pass `marcher` (sites 3/3b/4 collapsed): reads depth (→sampled if coarse did
        // not already) + tiles, read|writes the 3 attributes + gViewT (→GENERAL). Its
        // `record_pass` emits depth→sampled, color→general, and viewt's first-touch
        // UNDEFINED→GENERAL. (lit/ssao first-touch UNDEFINED→GENERAL are placed by the
        // graph at their own true first-use — resolve / ssao — a sound-superset re-order
        // of the hand path's eager site-(4) batch; see the module docs + equiv tests.)
        // Multi-paradigm render-path plan, rung R2 (Decision 2 / O1): gated on
        // `scene.path_has_marcher()` — the SAME predicate `record_gbuffer`'s marcher dispatch
        // checks (a `debug_assert_eq!` there guards parity). See [`Self::declare_deferred_graph`]'s
        // `raster` doc for why this is `true` on every currently reachable frame; every
        // `image_access`/`buffer_access` call below is unchanged from the pre-R2 unconditional
        // form, just nested one level deeper.
        let marcher = if scene.path_has_marcher() {
            let p = g.add_pass("marcher");
            g.image_access(
                depth,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                SubRange::DEPTH,
            );
            // The marcher READS the coarse tile bounds ONLY when the coarse pass ran (its
            // push gates the read off otherwise, and the hand path emits NO tiles barrier
            // when coarse is off). Declaring it unconditionally would derive a spurious
            // first-touch tiles barrier the hand path never records.
            if coarse.is_some() {
                g.buffer_access(
                    tiles,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                );
            }
            for &c in &[albedo, normal, material, viewt] {
                g.image_access(
                    c,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    RW,
                    VK_IMAGE_LAYOUT_GENERAL,
                    SubRange::COLOR,
                );
            }
            Some(p)
        } else {
            None
        };

        // Pass `ssao` — gated `scene.ssao.is_some()`. Reads normal/material/viewt (all
        // already SHADER_READ-visible from the marcher store→load), writes ssao.
        let ssao_pass = if scene.ssao.is_some() {
            let p = g.add_pass("ssao");
            for &c in &[normal, material, viewt] {
                g.image_access(
                    c,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_GENERAL,
                    SubRange::COLOR,
                );
            }
            g.image_access(
                ssao,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
            Some(p)
        } else {
            None
        };

        // The SSAO edge-avoiding à-trous denoise chain: `atrous_levels` passes (`0` or
        // `2..=MAX_SSAO_ATROUS_LEVELS`), declared ONLY when `scene.ssao.is_some()` (mirrors the
        // gather pass's gate — à-trous cannot run without a fresh gather). UNCONDITIONAL (both
        // feature legs — SOFTWARE, NOT `hwrt`-gated, unlike the shadow-visibility à-trous below).
        // Level `k`'s (in, out) ResId pair is [`crate::present::ssao_atrous_step`]'s
        // [`crate::present::AtrousStepRole`], folding the R8 `ssao` (`gSsao`) endpoint in as the
        // virtual "ring -1" / "ring N" slot: `Read8` reads `ssao`/writes `ssao_ring_a`; `Interior`
        // ping-pongs `ssao_ring_a`/`ssao_ring_b`; `Write8` reads a ring/writes BACK into `ssao` —
        // so the chain orders `ssao`: gather-write → level-0-read → .. → last-level-write →
        // resolve-read (the resolve's conditional `ssao` read below derives the FINAL barrier).
        let ssao_atrous_levels = scene
            .ssao
            .as_ref()
            .map_or(0u32, |a| a.atrous_levels.min(crate::present::MAX_SSAO_ATROUS_LEVELS));
        debug_assert!(
            ssao_atrous_levels == 0 || ssao_pass.is_some(),
            "invariant: ssao_atrous_levels > 0 requires the gather pass (scene.ssao.is_some())"
        );
        // W2 (degrade-path gate coupling): this declarator gates the pass count on `scene.ssao` +
        // levels ONLY; the RECORDER (`record_gbuffer`) additionally requires the 5 role-keyed sets,
        // which are `None` on a device lacking `R16_UNORM` STORAGE (`ssao_atrous_storage_ok()` false).
        // On such a device declared = N but recorded = 0. This is SAFE by DIRECTION (declared > recorded
        // is inert — a phantom pass's derived barriers are simply never emitted, and the NULL ring
        // images are never named by a recorded barrier) AND by CONSTRUCTION: the resolve's `ssao` RAW
        // barrier is derived from the declared `Write8` `ssao`-write, whose COMPUTE/SHADER_WRITE stage/
        // access masks are IDENTICAL to the gather's `ssao`-write — so the gather-write → resolve-read
        // ordering holds even when zero à-trous passes actually run. INVARIANT TO PRESERVE: `Write8`'s
        // `ssao`-write stage/access mask MUST stay == the gather's, or this degrade-path barrier is
        // silently lost. (The recorded > declared direction — which WOULD trip `plan.ssao_atrous[level]
        // .expect(..)` — cannot occur: both sides clamp the SAME `scene.ssao.atrous_levels`.)
        // O1: the contract is `0 || 2..=MAX` (the host's `clamped_atrous_levels`), asserted where the
        // RHI first consumes it so a future raw-`1` never records a lone `Read8` that never writes back.
        debug_assert!(
            ssao_atrous_levels == 0
                || (2..=crate::present::MAX_SSAO_ATROUS_LEVELS).contains(&ssao_atrous_levels),
            "invariant: ssao_atrous_levels is 0 or 2..=MAX (clamped_atrous_levels); got {ssao_atrous_levels}"
        );
        let mut ssao_atrous: [Option<crate::framegraph::PassId>;
            crate::present::MAX_SSAO_ATROUS_LEVELS as usize] =
            [None; crate::present::MAX_SSAO_ATROUS_LEVELS as usize];
        for (level, slot) in ssao_atrous
            .iter_mut()
            .enumerate()
            .take(ssao_atrous_levels as usize)
        {
            let level = level as u32;
            let rings = [ssao_ring_a, ssao_ring_b];
            let (in_res, out_res) = match crate::present::ssao_atrous_step(level, ssao_atrous_levels) {
                crate::present::AtrousStepRole::Read8 => (ssao, rings[0]),
                crate::present::AtrousStepRole::Interior { in_ring } => {
                    (rings[in_ring as usize], rings[1 - in_ring as usize])
                }
                crate::present::AtrousStepRole::Write8 { in_ring } => (rings[in_ring as usize], ssao),
            };
            let p = g.add_pass("ssao_atrous");
            g.image_access(
                viewt,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
            g.image_access(
                in_res,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
            g.image_access(
                out_res,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
            *slot = Some(p);
        }

        // HW-RT rung 3a: the RT soft-shadow VIS pre-pass + the `levels` à-trous passes — declared
        // ONLY when `scene.shadow.is_some()` (the step-7 gate; the host keeps it a literal `None`
        // this rung, so these are dead on EVERY current frame → the OFF path's ResId + barrier
        // counts are byte-unchanged). The VIS pass RE-RUNS the resolve front-matter + traces the
        // TLAS, so it reads gNormal/gViewT (GENERAL, already SHADER_READ-visible from the marcher
        // store→load) + the `tlas_instances` array (COMPUTE/SHADER_READ — the graph derives the
        // build→VIS AS-visibility barrier), and WRITES `shadow_vis` (COMPUTE/SHADER_WRITE, first
        // touch UNDEFINED→GENERAL). Each à-trous level then reads the in-ResId + gNormal/gViewT
        // and writes the out-ResId (ping-pong `shadow_vis` ⇄ `shadow_vis2`), the graph deriving
        // each level's RAW on the ping-pong pair.
        #[cfg(feature = "hwrt")]
        let (shadow_vis_pass, shadow_atrous_passes, final_vis_res, shadow_temporal_pass) =
            if let Some(sh) = scene.shadow.as_ref()
        {
            // Pass `shadow_vis`: reads gNormal/gViewT (GENERAL) + the tlas buffer (COMPUTE read),
            // writes `shadow_vis` (GENERAL). NOTE: the `tlas_instances` buffer_access derives the
            // build(AS_BUILD/SHADER_WRITE) → VIS(COMPUTE/SHADER_READ) barrier only when the tlas
            // pass ran (`scene.tlas.is_some()`); the step-7 gate implies both, so declare it.
            let vis = g.add_pass("shadow_vis");
            for &c in &[normal, viewt] {
                g.image_access(
                    c,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_GENERAL,
                    SubRange::COLOR,
                );
            }
            g.buffer_access(
                tlas_instances,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
            g.image_access(
                shadow_vis,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
            // HW-RT Rung 3b step 5b: when the SDF-MV path is active, the VIS pass ALSO writes each
            // SDF pixel's camera-only `Δuv` to `motion_vec` (STORAGE, GENERAL). Declaring this WRITE
            // makes the graph order the raster pass's earlier COLOR_ATTACHMENT write of the MESH
            // pixels (step 5a) BEFORE this STORAGE write — the required COLOR_ATTACHMENT_OPTIMAL →
            // GENERAL transition + WAW barrier (the two passes cover DISJOINT pixels). The gate is
            // `sdf_mv_active()` — the SAME predicate the recorder binds the VIS-MV pipeline under
            // (W1: the barrier declaration and the write must never disagree). OFF (temporal off /
            // non-storage / non-hwrt) ⇒ no access ⇒ the graph routes ZERO barriers on `motion_vec`
            // for this pass ⇒ byte-identical. (`mode == Both` this rung — the VIS pass runs only when
            // spatial is on; step 6 extends it to pure Temporal.)
            if scene.sdf_mv_active() {
                g.image_access(
                    motion_vec,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_WRITE_BIT,
                    VK_IMAGE_LAYOUT_GENERAL,
                    SubRange::COLOR,
                );
            }

            // The `atrous_levels` à-trous passes (ping-pong). Level `i` reads `i`-even ? `shadow_vis` :
            // `shadow_vis2` and writes the other; the FINAL write lands in `shadow_vis2` for odd
            // `atrous_levels`, `shadow_vis` for even (== the input of the last level). Rung 3b allows
            // `0` (Temporal-only mode: the raw VIS feeds the temporal pass ⇒ NO à-trous pass, so
            // `final_res == shadow_vis` = the raw VIS). Only the CEILING is clamped (the per-level
            // array bound) — the floor stays 0 (`.min`, not `.clamp(1, ..)`).
            let atrous_levels =
                (sh.atrous_levels as usize).min(crate::present::MAX_ATROUS_LEVELS as usize);
            let mut atrous: [Option<crate::framegraph::PassId>;
                crate::present::MAX_ATROUS_LEVELS as usize] =
                [None; crate::present::MAX_ATROUS_LEVELS as usize];
            for (i, slot) in atrous.iter_mut().enumerate().take(atrous_levels) {
                let (in_res, out_res) = if i % 2 == 0 {
                    (shadow_vis, shadow_vis2)
                } else {
                    (shadow_vis2, shadow_vis)
                };
                let p = g.add_pass("shadow_atrous");
                for &c in &[normal, viewt] {
                    g.image_access(
                        c,
                        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                        VK_ACCESS_SHADER_READ_BIT,
                        VK_IMAGE_LAYOUT_GENERAL,
                        SubRange::COLOR,
                    );
                }
                g.image_access(
                    in_res,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_GENERAL,
                    SubRange::COLOR,
                );
                g.image_access(
                    out_res,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_WRITE_BIT,
                    VK_IMAGE_LAYOUT_GENERAL,
                    SubRange::COLOR,
                );
                *slot = Some(p);
            }
            // The final filtered-vis target: `shadow_vis2` for odd `atrous_levels` (last write landed
            // there), `shadow_vis` for even (incl. `0` ⇒ the raw VIS). Consumed by the temporal pass's
            // `gVisIn` (below) and/or the resolve read.
            let final_res = if atrous_levels % 2 == 1 { shadow_vis2 } else { shadow_vis };
            // HW-RT Rung 3b step 6: the temporal reproject+accumulate pass, declared AFTER the à-trous
            // chain when the author's mode is temporal (`sh.temporal`). Reads `final_res` (the à-trous
            // FINAL / the raw VIS), `motion_vec` (ResId 13), and `viewt` — all COMPUTE/SHADER_READ at
            // GENERAL. Writes `shadow_temporal_hist` (ResId 14, the `[fi]` slot) + `temporal_out`
            // (ResId 15) — COMPUTE/SHADER_WRITE at GENERAL. The cross-frame `shadow_temporal_hist[fi^1]`
            // READ is declared as ResId 16 `shadow_temporal_hist_read` (C1 fix): its `seeded_writer_at_
            // layout` seed makes `transition()` emit the RAW ordering frame N's read after — and visible
            // to — the sibling frame N-1's write of that same physical image (was direct-bound +
            // unsynchronized = the race). The write → resolve-read barrier on `temporal_out` is derived
            // at the resolve (the reader). OFF (non-temporal) ⇒ `None` ⇒ zero derived barriers on
            // ResId 14/15/16 ⇒ byte-identical.
            let temporal = if sh.temporal {
                let p = g.add_pass("shadow_temporal");
                for &c in &[final_res, motion_vec, viewt, shadow_temporal_hist_read] {
                    g.image_access(
                        c,
                        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                        VK_ACCESS_SHADER_READ_BIT,
                        VK_IMAGE_LAYOUT_GENERAL,
                        SubRange::COLOR,
                    );
                }
                for &w in &[shadow_temporal_hist, temporal_out] {
                    g.image_access(
                        w,
                        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                        VK_ACCESS_SHADER_WRITE_BIT,
                        VK_IMAGE_LAYOUT_GENERAL,
                        SubRange::COLOR,
                    );
                }
                Some(p)
            } else {
                None
            };
            (Some(vis), atrous, final_res, temporal)
        } else {
            (
                None,
                [None; crate::present::MAX_ATROUS_LEVELS as usize],
                shadow_vis,
                None,
            )
        };

        // Pass `ddgi_update` (SDFDDGI I2 probe update) — gated `scene.ddgi_update.is_some()`, i.e.
        // ONLY when `ResolvedDdgi::enabled()`. Recorded AFTER the marcher (edit-list warm) + AFTER
        // the L0 light-table copy (`LightBuf` COMPUTE-read-visible), BEFORE the resolve. Reads the
        // light table + the boot-static ray table (COMPUTE/SHADER_READ), read/writes the
        // classification (COMPUTE/RW), and WRITES the two atlas storage images (COMPUTE/SHADER_WRITE,
        // GENERAL) — the RDG derives the SHADER_READ_ONLY_OPTIMAL → GENERAL transition here (the
        // content-preserving seed layout, so the round-robin's un-updated tiles survive) and the
        // update-write → resolve-read GENERAL → SHADER_READ_ONLY_OPTIMAL barrier at the resolve (the
        // atlas reader), so NO `cmd_pipeline_barrier` is hand-written. The edit-list SSBO (`Buf` @0)
        // is NOT a graph resource (host-seeded once, like the marcher's read — no per-frame barrier),
        // so it is not named here.
        let ddgi_update = if scene.ddgi_update.is_some() {
            let p = g.add_pass("ddgi_update");
            g.buffer_access(
                light_table,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
            g.buffer_access(
                ddgi_ray_table,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
            g.buffer_access(ddgi_classification, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW);
            for &img in &[ddgi_irr, ddgi_depth] {
                g.image_access(
                    img,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_WRITE_BIT,
                    VK_IMAGE_LAYOUT_GENERAL,
                    SubRange::color_layers(crate::ddgi::DDGI_ATLAS_LAYERS),
                );
            }
            Some(p)
        } else {
            None
        };

        // Pass `light_cull` (L1 clustered cull) — gated exactly as the hand site: the
        // pipeline AND all three cluster buffers are `Some`. Resets alloc (transfer),
        // reads the table, writes grid/index.
        let light_cull = if scene.cluster_cull.is_some()
            && scene.cluster_grid.is_some()
            && scene.light_index.is_some()
            && scene.light_index_alloc.is_some()
        {
            let p = g.add_pass("light_cull");
            g.buffer_access(
                alloc,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_ACCESS_TRANSFER_WRITE_BIT,
            );
            g.buffer_access(alloc, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW);
            g.buffer_access(
                light_table,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
            g.buffer_access(
                grid,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            );
            g.buffer_access(
                index,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            );
            Some(p)
        } else {
            None
        };

        // Pass `csm` (cascade depth) — gated `scene.csm.is_some()`. Layered depth over
        // `[0..active_count)` cascades, clamped `[1, MAX_CASCADES]` exactly as the hand
        // site. The barrier-out (→SHADER_READ_ONLY) is placed by the graph at the
        // resolve's cascade read (a sound-superset re-order; matches build_maximal_frame).
        let csm = if let Some(csm_act) = &scene.csm {
            let active = (csm_act.active_count as usize).clamp(1, MAX_CASCADES) as u32;
            let p = g.add_pass("csm_depth");
            g.image_access(
                cascade,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::depth_layers(active),
            );
            Some(p)
        } else {
            None
        };

        // Pass `atlas` (spot/point atlas depth) — gated `scene.atlas_punctual.is_some()`.
        // Layered depth over `[0..active_layers)`, clamped `[1, MAX_TEXTURE_LAYERS]`.
        let atlas_pass = if let Some(atlas_act) = &scene.atlas_punctual {
            let active =
                (atlas_act.active_layers as usize).clamp(1, MAX_TEXTURE_LAYERS) as u32;
            let p = g.add_pass("atlas_depth");
            g.image_access(
                atlas,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::depth_layers(active),
            );
            Some(p)
        } else {
            None
        };

        // Pass `resolve`: reads albedo/normal/material/viewt/ssao (GENERAL) + the L1
        // buffers + the layered shadow maps (→sampled), writes lit (first-touch
        // UNDEFINED→GENERAL). The optional reads are declared ONLY when their producer
        // pass ran, so the graph derives their →sampled / →read barriers exactly when the
        // hand path does. Declaring them conditionally keeps a resource the frame never
        // touched (e.g. cascade when CSM is off) out of the compiled barrier set.
        let resolve = g.add_pass("resolve");
        for &c in &[albedo, normal, material, viewt] {
            g.image_access(
                c,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
        }
        // Textured-PBR T6a: the resolve ALWAYS declares this read (UNLIKE `ssao` below, which is
        // gated on its producer pass having run) — `pbr` has NO producer this rung, and the plan
        // wants its layout ACTIVELY GENERAL (not left `UNDEFINED`, unlike the `ssao`-off precedent)
        // so the statically-referenced `gPbr` STORAGE_IMAGE descriptor is always in a valid layout.
        // `pbr` is `add_image_seeded` (see its declaration above), so this derives a real, discard-
        // legal `UNDEFINED → GENERAL` transition every frame rather than tripping the unwritten-
        // transient-read authoring guard. The SPIR-V `.Load` behind this binding is inside the
        // flag-gated branch, so a flag=0 material (every current one) never dynamically reads the
        // discarded content — pixel output is byte-identical.
        g.image_access(
            pbr,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            SubRange::COLOR,
        );
        if ssao_pass.is_some() {
            g.image_access(
                ssao,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
        }
        if light_cull.is_some() {
            g.buffer_access(
                grid,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
            g.buffer_access(
                index,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
        }
        // The resolve reads the light table whenever it was uploaded this frame (the
        // hand path's marcher/resolve read the table after the L0-r0 copy). The graph
        // only needs a barrier here if a producer left a pending flush; declaring the
        // read is harmless (free when already visible) but we mirror the hand site: the
        // table's producer is the light_upload copy (TRANSFER_WRITE), consumed at COMPUTE.
        if light_upload.is_some() {
            g.buffer_access(
                light_table,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
        }
        if csm.is_some() {
            let active =
                (scene.csm.as_ref().map(|c| c.active_count).unwrap_or(1) as usize)
                    .clamp(1, MAX_CASCADES) as u32;
            g.image_access(
                cascade,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                SubRange::depth_layers(active),
            );
        }
        if atlas_pass.is_some() {
            let active = (scene
                .atlas_punctual
                .as_ref()
                .map(|a| a.active_layers)
                .unwrap_or(1) as usize)
                .clamp(1, MAX_TEXTURE_LAYERS) as u32;
            g.image_access(
                atlas,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                SubRange::depth_layers(active),
            );
        }
        // SDFDDGI I2: when the update pass ran, the resolve READS the two atlas storage images the
        // update WROTE. Declaring the read here (the atlas READER) is what makes the RDG DERIVE the
        // update-write → resolve-read barrier. The resolve SAMPLES the atlases through a
        // COMBINED_IMAGE_SAMPLER, so it reads them at SHADER_READ_ONLY_OPTIMAL — deriving a
        // GENERAL → SHADER_READ_ONLY_OPTIMAL, COMPUTE→COMPUTE, SHADER_WRITE→SHADER_READ transition
        // over all `DDGI_ATLAS_LAYERS` layers. This is the CONTENT-PRESERVING CYCLE that keeps the
        // persistent accumulator consistent across frames: boot SHADER_READ_ONLY_OPTIMAL → update
        // GENERAL (write) → resolve SHADER_READ_ONLY_OPTIMAL (sample), so the image ENDS each frame
        // at SHADER_READ_ONLY_OPTIMAL == the cross-frame seed layout (`seeded_readers_at_layout`
        // above) — no layout desync on the next frame's update write. Declared ONLY when
        // `ddgi_update.is_some()`, so the OFF path derives nothing (byte-identical).
        if ddgi_update.is_some() {
            for &img in &[ddgi_irr, ddgi_depth] {
                g.image_access(
                    img,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                    SubRange::color_layers(crate::ddgi::DDGI_ATLAS_LAYERS),
                );
            }
        }
        // HW-RT rung 3a/3b: when the denoise stack ran (`scene.shadow.is_some()`), the DENOISED
        // resolve READS the filtered visibility — declaring the read here (the vis READER) is what
        // makes the RDG DERIVE the last-write → resolve-read barrier. Rung 3b: on a TEMPORAL frame the
        // resolve reads `temporal_out` (ResId 15, the temporal-accumulate OUTPUT), deriving the
        // temporal-write → resolve-read barrier; otherwise it reads the à-trous FINAL `final_vis_res`
        // (the Rung-3a path, byte-identical). Declared ONLY on the ON path, so the OFF path derives
        // nothing (byte-identical). `debug_assert` the à-trous parity (the ping-pong invariant) on the
        // non-temporal path: `final_vis_res == shadow_vis2` iff `atrous_levels` is odd.
        #[cfg(feature = "hwrt")]
        if shadow_vis_pass.is_some() {
            let temporal = scene.temporal_active();
            debug_assert!(
                temporal || {
                    let atrous_levels = scene
                        .shadow
                        .as_ref()
                        .map(|s| (s.atrous_levels as usize).min(crate::present::MAX_ATROUS_LEVELS as usize))
                        .unwrap_or(0);
                    let last_write = if atrous_levels % 2 == 1 { shadow_vis2 } else { shadow_vis };
                    final_vis_res.index() == last_write.index()
                },
                "invariant: the resolve reads the ResId the last à-trous pass wrote (ping-pong parity)"
            );
            let vis_read = if temporal { temporal_out } else { final_vis_res };
            g.image_access(
                vis_read,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
        }
        g.image_access(
            lit,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            SubRange::COLOR,
        );

        // Anti-aliasing Stage 4 (TAA W5): the temporal-resolve pass declaration, positioned at the
        // resolve→present seam — AFTER the deferred resolve's `lit` write above, BEFORE
        // `present_sample`'s GENERAL→SHADER_READ_ONLY_OPTIMAL read. UNLIKE FXAA/SMAA/SSAA (which
        // read `lit` AFTER `present_sample`'s transition, at `SHADER_READ_ONLY_OPTIMAL`, FRAGMENT
        // stage), TAA is a COMPUTE dispatch that reads `lit` at `GENERAL` straight out of the
        // resolve's write — so `gbuffer.rs::record_taa` MUST be recorded (in this exact
        // declaration order) BEFORE `present_sample`, not after (see `TaaActivation`'s "Compute,
        // not graphics" doc). Reads `lit` (this frame's shaded color) + `viewt` (the depth proxy
        // the MV reconstruction needs) + `taa_hist_read` (the cross-frame history sibling, C1-fix
        // shape — see the `taa_hist`/`taa_hist_read` declaration above); writes `taa_hist` (this
        // frame's history slot). Gated on `scene.taa.is_some()` — `None` (OFF/FXAA/SMAA/SSAA)
        // declares no pass ⇒ the graph routes ZERO barriers on `taa_hist`/`taa_hist_read` ⇒
        // byte-identical (the same "declared ahead of its first consumer" discipline
        // `shadow_vis`/`motion_vec` used between their ResId declaration rung and their first
        // consuming pass). `aa_out`'s own barriers are hand-recorded in `record_taa` (it is not a
        // framegraph-tracked resource — see [`FRAMEGRAPH_IMAGE_COUNT`]'s doc).
        let taa_resolve_pass = scene.taa.is_some().then(|| {
            let taa_resolve = g.add_pass("taa_resolve");
            // C2 fix: `lit` @0 is bound as a COMBINED_IMAGE_SAMPLER, whose descriptor records
            // SHADER_READ_ONLY_OPTIMAL (rhi_impl/device.rs) — the graph MUST leave `lit` in THAT
            // layout at the dispatch, not GENERAL (a recorded-vs-actual layout divergence is
            // spec-UB / a validation error). The deferred resolve wrote `lit` in GENERAL, so this
            // read derives the GENERAL->SHADER_READ_ONLY transition BEFORE the taa_resolve dispatch;
            // `present_sample`'s later `lit`->SHADER_READ read then finds it already in layout and
            // derives no barrier (matching FXAA/SMAA/SSAA, which read `lit` at SHADER_READ_ONLY).
            g.image_access(
                lit,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                SubRange::COLOR,
            );
            // `viewt` (gViewT r32f) + `taa_hist_read` (gHistIn rgba16f) are bound as STORAGE images
            // (RWTexture2D), read in GENERAL — unchanged.
            for &c in &[viewt, taa_hist_read] {
                g.image_access(
                    c,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_GENERAL,
                    SubRange::COLOR,
                );
            }
            g.image_access(
                taa_hist,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
            taa_resolve
        });

        // Pass `present_sample` (site 5c): lit GENERAL→SHADER_READ_ONLY for the
        // present-blit's FRAGMENT sample. The swapchain WSI barriers (sites 7/9) stay
        // hand-recorded, so the swapchain image is NOT a graph resource here.
        let present_sample = g.add_pass("present_sample");
        g.image_access(
            lit,
            VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::COLOR,
        );

        g.compile();

        self.gbuffer_pass_plan = Some(GbufferPassPlan {
            interp,
            raster,
            light_upload,
            coarse,
            marcher,
            ssao: ssao_pass,
            ssao_atrous,
            ddgi_update,
            light_cull,
            csm,
            atlas: atlas_pass,
            #[cfg(feature = "hwrt")]
            tlas_pack,
            #[cfg(feature = "hwrt")]
            tlas_build,
            #[cfg(feature = "hwrt")]
            shadow_vis: shadow_vis_pass,
            #[cfg(feature = "hwrt")]
            shadow_atrous: shadow_atrous_passes,
            #[cfg(feature = "hwrt")]
            shadow_temporal: shadow_temporal_pass,
            resolve,
            taa_resolve: taa_resolve_pass,
            present_sample,
        });
    }

    /// Steps 1d/1e: drive one declared pass's derived barriers (Step 1c's
    /// [`GbufferBarrierSink`], now whole-frame) into the open `cmd`. Builds the sink
    /// resolving the graph's FIXED ResIds → the current frame slot's physical
    /// `VkImage`/`VkBuffer` handles, then calls `record_pass`, which emits the minimum
    /// number of batched sync1 `vkCmdPipelineBarrier` calls for `pass`.
    ///
    /// An optional buffer/image the frame did not wire resolves to a NULL handle: the
    /// declaration never routed a barrier naming its ResId, so the NULL is never handed
    /// to the driver (a pass records ONLY the barriers derived from ITS declared
    /// accesses).
    pub(crate) fn record_graph_pass(
        &self,
        pass: crate::framegraph::PassId,
        cmd: VkCommandBuffer,
        targets: &GBufferTargets,
        scene: &GBufferScene<'_>,
        fi: usize,
    ) {
        // C1 guard (the critic's H1): ResId 16 (the temporal history READ) MUST bind the SIBLING
        // parity slot `hist[fi^1]`, distinct from ResId 14 (the WRITE, `hist[fi]`) — else the
        // cross-frame RAW barrier lands on the wrong image (a false-green: passes static, shimmers
        // only in motion). The pool is 2 distinct textures, so the parity slots always differ when
        // present; a future edit that collapses them (or copies the uniform `r[fi]` pattern here)
        // trips this in debug.
        #[cfg(feature = "hwrt")]
        debug_assert!(
            targets
                .shadow_temporal_hist
                .as_ref()
                .is_none_or(|r| r[fi].image != r[fi ^ 1].image),
            "invariant: the temporal history pool's [fi] write slot and [fi^1] read slot must be distinct images"
        );
        let mut sink = GbufferBarrierSink {
            fns: self.fns,
            cmd,
            images: [
                targets.albedo[fi].image,
                targets.normal[fi].image,
                targets.material[fi].image,
                targets.depth[fi].image,
                targets.viewt[fi].image,
                targets.lit[fi].image,
                targets.ssao[fi].image,
                // cascade / atlas are single-instance (NOT ringed) world-fixed maps.
                scene.csm_cascade_texture.image,
                scene.shadow_atlas_texture.image,
                // SDFDDGI I2 (ResIds 9/10): the two DDGI atlas storage images — single-instance
                // (NOT ringed) world-fixed atlases, the SAME textures the resolve set samples. Only
                // named by a derived barrier on the `ddgi_update` ON path; on the OFF path they are
                // unreferenced (their slots inert, like cascade/atlas when those maps are off).
                scene.ddgi_irr_texture.image,
                scene.ddgi_depth_texture.image,
                // Rung 3a (`hwrt`, ResIds 11/12): the two RT soft-shadow-visibility targets — ringed
                // per-FIF STORAGE images (BOTH R16G16_UNORM — the uniform-RG16 ping-pong), so bind
                // the CURRENT frame slot's handle (like the G-buffer ring slots above). `Option`-
                // guarded (the DDGI-degrade mirror): `None` — resolving to [`VkImage::NULL`] — when
                // the device lacks `RG16` UNORM storage
                // (`shadow_denoise_storage_ok() == false`), the target is not allocated and the
                // denoise stays disabled. In THIS step no pass names ResId 11/12, so these slots are
                // never handed to the driver either way (a NULL there is inert, like cascade/atlas
                // when those maps are off); steps 4-6 add the passes, gated on the same predicate.
                #[cfg(feature = "hwrt")]
                targets
                    .shadow_vis
                    .as_ref()
                    .map_or(VkImage::NULL, |r| r[fi].image),
                #[cfg(feature = "hwrt")]
                targets
                    .shadow_vis2
                    .as_ref()
                    .map_or(VkImage::NULL, |r| r[fi].image),
                // Rung 3b (`hwrt`, ResIds 13/14/15): the temporal reproject target rings — bind the
                // CURRENT frame slot's handle (like the G-buffer / shadow-vis ring slots above).
                // `Option`-guarded (degrade-to-`NULL` when the device lacks the storage format). NO
                // pass names ResId 13/14/15 this step, so these slots are never handed to the driver
                // (a NULL there is inert, like the shadow-vis slots when the denoise is off).
                #[cfg(feature = "hwrt")]
                targets
                    .motion_vec
                    .as_ref()
                    .map_or(VkImage::NULL, |r| r[fi].image),
                #[cfg(feature = "hwrt")]
                targets
                    .shadow_temporal_hist
                    .as_ref()
                    .map_or(VkImage::NULL, |r| r[fi].image),
                #[cfg(feature = "hwrt")]
                targets
                    .temporal_out
                    .as_ref()
                    .map_or(VkImage::NULL, |r| r[fi].image),
                // C1 fix — ResId 16 `shadow_temporal_hist_read`: the CROSS-FRAME READ image = the
                // SIBLING parity slot `hist[fi ^ 1]` (the image frame N-1 wrote). THIS IS THE ONE SINK
                // ENTRY THAT IS NOT `r[fi]`. Binding `r[fi]` here (the natural copy-paste mistake)
                // would land ResId 16's RAW barrier on the WRITE image, NOT the read image — the
                // barrier count would look right and a static scene would pass, but the cross-frame
                // read would stay unsynchronized and shimmer in motion (the exact false-green that
                // shipped twice). The temporal set @3 (`gHistIn`) binds this SAME sibling slot.
                #[cfg(feature = "hwrt")]
                targets
                    .shadow_temporal_hist
                    .as_ref()
                    .map_or(VkImage::NULL, |r| r[fi ^ 1].image),
                // Textured-PBR T6a: `pbr` (the `gPbr` deferred-resolve MRT lane) — declared LAST in
                // the image block (ResId 11 `not(hwrt)` / 17 `hwrt`, past every `hwrt`-only image
                // above), so it is bound here LAST too, UNCONDITIONALLY (both feature legs). RINGED
                // (per-FIF), like albedo/normal/etc. above — bind the CURRENT frame slot's handle.
                targets.pbr[fi].image,
                // Anti-aliasing Stage 4 (TAA W4/W5): `taa_hist` — the `[fi]` WRITE slot. `Option`-
                // guarded (`None` when `AaMode::Taa` is off, or on an allocation-degraded device —
                // resolves to [`VkImage::NULL`], inert since no pass names this ResId then).
                // UNCONDITIONAL (both feature legs — TAA is not `hwrt`-only).
                targets.taa_hist.as_ref().map_or(VkImage::NULL, |r| r[fi].image),
                // TAA W4 (C1-fix shape): `taa_hist_read` — the CROSS-FRAME READ image = the SIBLING
                // parity slot `taa_hist[fi ^ 1]` (the image frame N-1 wrote). Mirrors
                // `shadow_temporal_hist_read`'s `[fi ^ 1]` bind EXACTLY — the one sink entry that is
                // NOT `r[fi]` (binding `r[fi]` here would land the cross-frame RAW barrier on the
                // WRITE image, not the read image — the exact false-green the C1 fix closed for the
                // shadow-temporal precedent).
                targets.taa_hist.as_ref().map_or(VkImage::NULL, |r| r[fi ^ 1].image),
                // The SSAO à-trous denoise chain's two interior ping-pong rings — declared LAST in
                // the image block (ResId 14/15 `not(hwrt)` / 20/21 `hwrt`, past every earlier
                // image), bound here LAST too, UNCONDITIONALLY (both feature legs). RINGED
                // (per-FIF) — bind the CURRENT frame slot's handle. `Option`-guarded (`None` on a
                // device lacking `R16_UNORM` storage — resolves to [`VkImage::NULL`], inert since
                // no pass names either ResId when `atrous_levels == 0`).
                targets.ssao_ring_a.as_ref().map_or(VkImage::NULL, |r| r[fi].image),
                targets.ssao_ring_b.as_ref().map_or(VkImage::NULL, |r| r[fi].image),
            ],
            #[cfg(not(feature = "hwrt"))]
            buffers: [
                scene.light_table.buffer,
                scene.tiles_buffer.buffer,
                scene.cluster_grid.map_or(VkBuffer::NULL, |b| b.buffer),
                scene.light_index.map_or(VkBuffer::NULL, |b| b.buffer),
                scene.light_index_alloc.map_or(VkBuffer::NULL, |b| b.buffer),
                // SDFDDGI I2 (ResIds 16/17 → slots 5/6): the single-instance classification +
                // Fibonacci ray-table buffers, ALWAYS resolved (declared unconditionally in the
                // graph). Named by a derived barrier ONLY on the `ddgi_update` ON path.
                scene.ddgi_classification.buffer,
                scene.ddgi_ray_table.buffer,
                // Pillar B B3 (ResIds 18/19/20 → slots 7/8/9, refined-B): the CURRENT frame slot's
                // FIF-ringed interp SSBOs, NULL on the interp-OFF path (never named by a derived
                // barrier there). On the ON path the ONLY derived barrier is the COMPUTE→VERTEX RAW on
                // `interp_model_out` (the shared instance ring) at the raster pass; `interp_pairs`
                // and `interp_out_slot` are declared but never barriered (first-touch reads).
                scene.interp.map_or(VkBuffer::NULL, |a| a.pair_buffer.buffer),
                scene.interp.map_or(VkBuffer::NULL, |a| a.out_slot_buffer.buffer),
                scene.interp.map_or(VkBuffer::NULL, |a| a.model_out_buffer.buffer),
            ],
            // HW-RT rung R2a-3 (RISK-2): `tlas_instances` is declared UNCONDITIONALLY right after
            // the DDGI buffers (ResId 18 → slot 7), so its slot is FIXED regardless of the
            // conditional interp trio (which shifts to ResIds 19/20/21 → slots 8/9/10). NULL on the
            // tlas-OFF path (never named by a derived barrier there); on the ON path the pack write
            // → build read barrier is derived on this slot.
            #[cfg(feature = "hwrt")]
            buffers: [
                scene.light_table.buffer,
                scene.tiles_buffer.buffer,
                scene.cluster_grid.map_or(VkBuffer::NULL, |b| b.buffer),
                scene.light_index.map_or(VkBuffer::NULL, |b| b.buffer),
                scene.light_index_alloc.map_or(VkBuffer::NULL, |b| b.buffer),
                scene.ddgi_classification.buffer,
                scene.ddgi_ray_table.buffer,
                scene.tlas.map_or(VkBuffer::NULL, |t| t.instance_array.buffer),
                scene.interp.map_or(VkBuffer::NULL, |a| a.pair_buffer.buffer),
                scene.interp.map_or(VkBuffer::NULL, |a| a.out_slot_buffer.buffer),
                scene.interp.map_or(VkBuffer::NULL, |a| a.model_out_buffer.buffer),
            ],
        };
        self.frame_graph.record_pass(pass, &mut sink);
    }
}
