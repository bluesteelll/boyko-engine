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
    /// R2 (Decision 2 / O1): `Some` iff [`GBufferScene::path_has_raster`] holds. Rung R3 lifted
    /// the SDF-only leg-disable, so `Deferred × Sdf` reaches here with this `None` — see
    /// [`Self::mesh_depth_neutral_clear`] for the pass that replaces this one's depth-clear
    /// producer on that leg. Both `Deferred` legs are landed as of rung R3b (see
    /// [`Self::viewt_from_depth`]), so this is `Some` on every reachable `Deferred` frame EXCEPT
    /// `Deferred × Sdf`.
    pub(crate) raster: Option<crate::framegraph::PassId>,
    /// Multi-paradigm render-path plan, rung R3 (§E leg-disable / the O2 audit finding) — the
    /// mesh-depth NEUTRAL CLEAR pass: `Deferred × Sdf`'s replacement for [`Self::raster`]'s
    /// depth-clear producer. `Some` iff [`GBufferScene::path_has_mesh_depth_neutral_clear`]
    /// holds (`== sdf_leg && !mesh_leg`, mutually exclusive with [`Self::raster`] by
    /// construction). A depth-ONLY dynamic-rendering scope (no color attachments, no draw) that
    /// CLEARs the shared depth image to the far-plane sentinel, giving the marcher's
    /// (byte-UNCHANGED) `gDepth.Load` a deterministic "no mesh in the scene" reading every
    /// pixel — the SAME code path an entirely mesh-less scene already exercises byte-identically
    /// today. See [`GBufferScene::path_has_mesh_depth_neutral_clear`]'s doc for the full
    /// rationale (incl. why this reuses the existing marcher `.spv` unchanged rather than a new
    /// compiled `HAS_MESH` variant).
    pub(crate) mesh_depth_neutral_clear: Option<crate::framegraph::PassId>,
    /// Multi-paradigm render-path plan, rung R3b (`Deferred × Mesh` — the SDF leg fully off): the
    /// `viewt_from_depth` `gViewT`-producer pass, [`Self::mesh_depth_neutral_clear`]'s
    /// mirror-image. `Some` iff [`GBufferScene::viewt_from_depth`] is armed (capability =
    /// component presence — mirrors [`Self::ssao`]'s gate on `scene.ssao.is_some()`, NOT a
    /// leg-predicate fn: the declare site here and the record site both read the SAME `scene`
    /// field, so the two cannot diverge by construction). The caller MUST arm it exactly under
    /// `mesh_leg && !sdf_leg` (mutually exclusive with [`Self::mesh_depth_neutral_clear`] by
    /// construction); the belt-and-braces `debug_assert!` in [`Renderer::declare_deferred_graph`]
    /// guards that seam (mirrors the R3 mesh-shadow-producer invariant checks). Under `Deferred ×
    /// Mesh` the marcher ([`Self::marcher`]) is `None`, so nothing writes `gViewT`; this
    /// full-screen compute pass reproduces JUST the marcher's mesh-depth → `gViewT` conversion
    /// (`sdf_gbuffer_composite.hlsl`'s `mesh_norm`/`t_mesh` logic) for every pixel. Declared here
    /// (after the depth producer, before the marcher) so its `record_pass` derives the
    /// raster/mesh_depth_neutral_clear → viewt_from_depth depth-read barrier the SAME way the
    /// marcher's own depth-read barrier is derived under every other leg.
    pub(crate) viewt_from_depth: Option<crate::framegraph::PassId>,
    /// The async light-table re-upload (`scene.light_dirty && light_upload_bytes>0`).
    pub(crate) light_upload: Option<crate::framegraph::PassId>,
    /// The P0 coarse tile-cull (`scene.coarse.is_some()`).
    pub(crate) coarse: Option<crate::framegraph::PassId>,
    /// The SDF marcher pass. Its `record_pass` emits the collapsed input transitions
    /// (depth→sampled, color→general, lit/viewt first-touch — sites 3/3b/4). Multi-paradigm
    /// render-path plan, rung R2 (Decision 2 / O1): `Some` iff
    /// [`GBufferScene::path_has_marcher`] holds — see [`Self::raster`]'s doc for the R3 guard
    /// state.
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

/// Multi-paradigm render-path plan, rung R4b-b — the per-frame [`PassId`](crate::framegraph::PassId)
/// map [`Renderer::declare_forward_graph`] produces, driving [`Renderer::record_forward`]'s
/// derived barriers (the [`GbufferPassPlan`] sibling for the `Forward` v1 declarator).
///
/// Forward v1 declares its OWN small, PRIVATE resource/ResId space on `self.frame_graph`
/// (`interp?`/`light_upload?`/`csm?`/`atlas?`/`forward_opaque`/`present_sample` only — no
/// SSAO/DDGI/shadow-denoise/motion this v1 rung, the `forward_opaque.fs.hlsl` scope cut) —
/// DECOUPLED from [`GbufferBarrierSink`]/[`FRAMEGRAPH_IMAGE_COUNT`], the Deferred declarator's
/// fixed shared ResId space. This is a DEVIATION from the plan's literal "new images append
/// LAST in the fixed ResId order" text (see this rung's developer report for the full
/// trade-off): `self.frame_graph` is fully `reset()` + re-declared every frame regardless of
/// path, so a private, per-frame ResId space costs nothing and — critically — needs ZERO edits
/// to `declare_deferred_graph`/`record_graph_pass`/`GbufferBarrierSink`/`FRAMEGRAPH_IMAGE_COUNT`,
/// which the orchestrator's golden gate treats as reachable Deferred code. [`record_forward`]
/// builds its OWN small [`ForwardBarrierSink`] resolving THIS plan's ResIds.
#[derive(Clone, Copy)]
pub(crate) struct ForwardPassPlan {
    /// Pillar B B3: the per-instance TRS interpolation compute PRE-PASS — the SAME activation
    /// [`GbufferPassPlan::interp`] gates, since Forward's raster reads the SAME shared
    /// `instance_rings[fi]` model ring the interp pass writes into
    /// ([`crate::present::scene_types::GBufferScene::forward_instance_ring`] is that SAME ring,
    /// exposed raw). `Some` iff `scene.interp.is_some()`.
    pub(crate) interp: Option<crate::framegraph::PassId>,
    /// Multi-paradigm render-path plan, rung R5 (ForwardPlus): the depth-only PRE-PASS
    /// (Decision 4's EQUAL-depth early-Z contract) — writes `forward_depth` FIRST among the
    /// Forward pass sequence, before `forward_opaque` tests `EQUAL` against it. `Some` iff
    /// [`GBufferScene::path_needs_depth_prepass`] holds (`ForwardPlus` this rung — the O1
    /// single-predicate rule, mirrored at both this declare site and `record_forward`).
    pub(crate) depth_prepass: Option<crate::framegraph::PassId>,
    /// The async light-table re-upload (`scene.light_dirty && scene.light_upload_bytes > 0`) —
    /// the SAME gate [`GbufferPassPlan::light_upload`] uses.
    pub(crate) light_upload: Option<crate::framegraph::PassId>,
    /// The CSM cascade depth pass (`scene.csm.is_some()`) — declared BEFORE `forward_opaque`
    /// (unlike the plan's literal pass-order text, which lists `forward_opaque` before
    /// `light_upload?/csm?/atlas?`): `forward_opaque.fs.hlsl` samples the cascade/atlas maps
    /// INLINE (Set 1), so their depth-write producers MUST precede the fragment shader's read —
    /// the SAME dependency order `declare_deferred_graph` observes (`raster → csm/atlas → resolve`,
    /// where `resolve` is the shading pass, mirroring `forward_opaque` here). See this rung's
    /// developer report for the full escalation.
    pub(crate) csm: Option<crate::framegraph::PassId>,
    /// The sparse spot/point atlas depth pass (`scene.atlas_punctual.is_some()`) — see
    /// [`Self::csm`]'s doc for the ordering rationale.
    pub(crate) atlas: Option<crate::framegraph::PassId>,
    /// Multi-paradigm render-path plan, rung R5 (ForwardPlus): the L1 clustered froxel
    /// light-cull pass (`cluster_cull.hlsl`) — the SAME "4-buffers-Some" gate
    /// [`GbufferPassPlan::light_cull`] uses (`scene.cluster_cull`/`cluster_grid`/`light_index`/
    /// `light_index_alloc` all `Some`), duplicated into Forward's own small graph (the
    /// established `forward.rs` duplication discipline — see [`ForwardPassPlan`]'s own doc for
    /// why this is a SEPARATE, decoupled ResId space). Declared between `light_upload` and
    /// `forward_opaque`; `forward_opaque` under `ForwardPlus` reads its `ClusterGrid`/
    /// `LightIndexList` writes.
    pub(crate) light_cull: Option<crate::framegraph::PassId>,
    /// The always-present Forward mesh raster + inline-shade pass: writes `lit`
    /// (`ColorAttachmentWrite`/`COLOR_ATTACHMENT_OPTIMAL`, Decision 2's C5 per-path `lit`-producer
    /// access) + the Forward-only reverse-Z `forward_depth` — under plain `Forward`
    /// `DepthStencilAttachmentWrite`/`DEPTH_ATTACHMENT_OPTIMAL` (first-touch, `GREATER`); under
    /// `ForwardPlus` (rung R5) `DepthStencilAttachmentRead` (`EQUAL`, depth-write OFF —
    /// `depth_prepass` already committed the value) since [`Self::depth_prepass`] is `Some` and
    /// wrote it first. Reads `cascade`/`atlas` inline (FRAGMENT) when armed, reads the shared
    /// instance-model ring (VERTEX) when interp armed, reads `ClusterGrid`/`LightIndexList`
    /// (FRAGMENT) when [`Self::light_cull`] ran this frame.
    pub(crate) forward_opaque: crate::framegraph::PassId,
    /// Multi-paradigm render-path plan, rung R-SDFFWD: the fused SDF march-then-shade compute
    /// pass — writes `lit` (STORAGE, extending `forward_opaque`'s COLOR write, C5) and, when the
    /// resolved legs also carry the mesh leg, reads `forward_depth` to bound the march at the
    /// mesh surface (Decision 4). `Some` iff [`GBufferScene::path_has_sdf_forward`] holds
    /// (`ForwardMesh` profile with the SDF leg present — the O1 single-predicate rule, mirrored
    /// at both this declare site and `record_forward`).
    pub(crate) sdf_forward_march: Option<crate::framegraph::PassId>,
    /// The always-present present-sample pass: the `lit` `COLOR_ATTACHMENT_OPTIMAL` (or, when
    /// [`Self::sdf_forward_march`] ran, `GENERAL`) → `SHADER_READ_ONLY_OPTIMAL` transition (C5:
    /// the framegraph derives this from the producer/consumer access pair, not a hardcoded
    /// source layout). The swapchain WSI barriers stay hand-recorded, exactly like
    /// [`GbufferPassPlan::present_sample`].
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

/// Multi-paradigm render-path plan, rung R4b-b (buffers grown at rung R5): the
/// [`BarrierSink`](crate::framegraph::BarrierSink) for Forward v1's small, PRIVATE per-frame
/// graph (see [`ForwardPassPlan`]'s doc for why this is a SEPARATE sink type from
/// [`GbufferBarrierSink`], not a shared/extended one). Resolves the FIXED local ResId order
/// [`Renderer::declare_forward_graph`] declares — images `[lit=0, forward_depth=1, cascade=2,
/// atlas=3]` (UNCHANGED at rung R5: `depth_prepass` writes the SAME `forward_depth` ResId, no
/// new image), buffers `[light_table=0, instance_model_ring=1, light_index_alloc=2,
/// cluster_grid=3, light_index=4]` (buffer ResIds offset by `FORWARD_IMAGE_COUNT`; the 3 L1
/// cluster-cull buffers appended at rung R5, mirroring `GbufferBarrierSink`'s OWN
/// `[grid, index, alloc]` ordering for the SAME resources) — to the current frame's physical
/// handles. Lives only for the duration of one `record_pass` call inside
/// [`Renderer::record_forward`].
pub(crate) struct ForwardBarrierSink<'a> {
    pub(crate) fns: &'a DeviceFns,
    pub(crate) cmd: VkCommandBuffer,
    /// `[lit, forward_depth, cascade, atlas]` — see this type's doc for the fixed order. `lit`
    /// and `forward_depth` are the current frame slot's [`ForwardTargets`](super::targets::ForwardTargets)
    /// images; `cascade`/`atlas` are the SAME single-instance, world-fixed textures the deferred
    /// resolve's Set-2-equivalent bindings reference (`scene.csm_cascade_texture`/
    /// `scene.shadow_atlas_texture`).
    pub(crate) images: [VkImage; FORWARD_IMAGE_COUNT],
    /// `[light_table, instance_model_ring, light_index_alloc, cluster_grid, light_index]`. A
    /// pass that does not declare an access on an unarmed resource (e.g. `instance_model_ring`
    /// when `scene.interp` is `None`, or the L1 trio when [`ForwardPassPlan::light_cull`] is
    /// `None`) never routes a barrier naming it, so an inert `VkBuffer::NULL` there is harmless
    /// (the same "ungated slot may hold NULL" rule [`GbufferBarrierSink`] documents).
    pub(crate) buffers: [VkBuffer; 5],
}

/// The number of IMAGE resources [`Renderer::declare_forward_graph`] declares — see
/// [`ForwardBarrierSink::images`]'s doc for the fixed order. A PRIVATE, per-frame ResId space
/// (Forward's `self.frame_graph` is fully `reset()` + re-declared every frame, so this constant
/// has no relationship to [`FRAMEGRAPH_IMAGE_COUNT`] and never grows it).
const FORWARD_IMAGE_COUNT: usize = 4;

impl crate::framegraph::BarrierSink for ForwardBarrierSink<'_> {
    fn image_barriers(&mut self, src_stage: u32, dst_stage: u32, group: &[crate::framegraph::ImgBarrier]) {
        debug_assert!(
            group.len() <= crate::framegraph::MAX_PASS_BARRIERS,
            "image barrier group ({}) exceeds MAX_PASS_BARRIERS",
            group.len()
        );
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
        // `record_forward` recording). Every `arr[i].image` was resolved from the
        // `images[res.index()]` slot (a live Forward target for the current frame);
        // `res.index()` is in `0..FORWARD_IMAGE_COUNT` for every image barrier this small graph
        // derives. `arr[..n]` (a stack array) outlives the call; the count == `n`. No memory or
        // buffer barriers, matching [`GbufferBarrierSink::image_barriers`].
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

    fn buffer_barriers(&mut self, src_stage: u32, dst_stage: u32, group: &[crate::framegraph::BufBarrier]) {
        debug_assert!(
            group.len() <= crate::framegraph::MAX_PASS_BARRIERS,
            "buffer barrier group ({}) exceeds MAX_PASS_BARRIERS",
            group.len()
        );
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
                        buffer: self.buffers[b.res.index() - FORWARD_IMAGE_COUNT],
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
        // `record_forward` recording). Every `arr[i].buffer` was resolved from the
        // `buffers[res.index() - FORWARD_IMAGE_COUNT]` slot (a live scene buffer for this
        // frame); a buffer barrier's `res.index()` is always `>= FORWARD_IMAGE_COUNT` and
        // `< FORWARD_IMAGE_COUNT + buffers.len()`. `arr[..n]` outlives the call; the count == `n`.
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
    /// The `Deferred` arm (`declare_deferred_graph`, unchanged, byte-identical to the pre-R2
    /// `declare_gbuffer_graph`), the `Forward` FAMILY arm (`declare_forward_graph`, shared
    /// verbatim by `Forward` and `ForwardPlus` — Decision 2's one-declarator-per-path-FAMILY,
    /// `ForwardPlus` landing at rung R5), and, as of rung R8, the `VisibilityBuffer` arm
    /// (`declare_vb_graph`, the FUSED v1 declarator) are all implemented — every arm of the R1
    /// resolver's `RenderPath` enum (`boyko_render::render_path_config::RenderPath` — a crate
    /// this one sits BELOW in the dependency graph, hence the plain-text reference rather than
    /// an intra-doc link) now reaches a real declarator.
    pub(crate) fn declare_frame_graph(&mut self, scene: &GBufferScene<'_>) {
        match scene.resolved_render_path.path {
            // RenderPath::Deferred == 0 (render_path_config.rs) — the only rung-landed path.
            0 => self.declare_deferred_graph(scene),
            // RenderPath::Forward == 1 (rung R4b-b) / RenderPath::ForwardPlus == 2 (rung R5) —
            // ONE shared declarator (Decision 2); `declare_forward_graph` internally branches on
            // `GBufferScene::path_needs_depth_prepass`/`path_is_forward_plus` where the two paths
            // diverge (the depth prepass + EQUAL-depth + froxel-Set0 growth).
            1 | 2 => self.declare_forward_graph(scene),
            // RenderPath::VisibilityBuffer == 3 (rung R8): the FUSED v1 declarator
            // (`mesh_geo_shade_split == false` — SSAO/DDGI/shadow-denoise/TAA all structurally
            // capped off this rung, `cap_vb_v1_consumers`).
            3 => self.declare_vb_graph(scene),
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

        // Multi-paradigm render-path plan, rung R3 (P1 fix invariant guard — orchestrator
        // architecture decision): mesh-shadow producers (CSM cascade depth, the punctual
        // spot/point atlas depth, and under `hwrt` the per-frame TLAS pack/build + the
        // shadow_vis/à-trous/temporal denoise chain) are MESH-LEG-OWNED — they rasterize/trace
        // MESH casters only, never SDF ones (the SDF leg's shadow is the marcher's baked soft
        // march). `GpuSceneBundles::scene()` (`boyko_app::gpu_scene`) is the single
        // scene-assembly seam that suppresses them to `None` under `!mesh_leg`; this
        // `debug_assert!` is the framegraph-side belt-and-braces check that the seam was not
        // missed (a scene fixture assembled elsewhere — e.g. a hand-built test harness — that
        // forgets the gate trips here instead of silently rasterizing invisible mesh shadows
        // into unused cascade/atlas targets).
        debug_assert!(
            scene.resolved_render_path.mesh_leg || (scene.csm.is_none() && scene.atlas_punctual.is_none()),
            "invariant: mesh-shadow activation without mesh leg (scene-assembly gate missed)"
        );
        #[cfg(feature = "hwrt")]
        debug_assert!(
            scene.resolved_render_path.mesh_leg || (scene.tlas.is_none() && scene.shadow.is_none()),
            "invariant: mesh-shadow activation without mesh leg (scene-assembly gate missed)"
        );

        // Multi-paradigm render-path plan, rung R3b (§E leg-disable / the R3b audit finding): the
        // `viewt_from_depth` belt-and-braces check, mirroring the mesh-shadow-producer guards
        // above — the scene-assembly seam (`GpuSceneBundles::scene()`) MUST arm
        // `scene.viewt_from_depth` exactly when the resolved legs are `GeometryLegs::Mesh`
        // (`mesh_leg && !sdf_leg`), never more (an armed activation under `Both`/`Sdf` would
        // dispatch a redundant/wrong-owner producer) and never less (an unarmed one under
        // `Deferred × Mesh` leaves `gViewT` wholly unwritten — the R3b bug this pass exists to
        // close). A hand-built test fixture that forgets the gate trips here instead of silently
        // shipping a stale/undefined `gViewT` lane.
        debug_assert_eq!(
            scene.viewt_from_depth.is_some(),
            scene.resolved_render_path.mesh_leg && !scene.resolved_render_path.sdf_leg,
            "invariant: viewt_from_depth activation must be armed iff mesh_leg && !sdf_leg \
             (scene-assembly gate missed)"
        );

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
        // `ssao` is ALWAYS-BOUND at resolve @11 (STORAGE, GENERAL) but only WRITTEN when the SSAO
        // pass runs — seeded like `pbr` (T6a) so the SSAO-off frame's unconditional resolve read
        // (below) derives a real, discard-legal UNDEFINED→GENERAL first-touch transition instead
        // of tripping `compile`'s unwritten-transient-read guard. With SSAO on, the SSAO pass's
        // write is the first touch and the seed is inert — barriers identical to before.
        let ssao = g.add_image_seeded("ssao", ResSync::undefined());
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
        // touches a DIFFERENT slot. All three therefore START `undefined()`: no cross-frame
        // WAR/WAW hazard, only the intra-frame COMPUTE→VERTEX RAW the graph derives at the raster
        // (the model_out reader).
        //
        // VG R3 P2-8 — WHY THE FIRST TWO ARE `add_buffer_seeded(.., undefined())` AND THE THIRD IS
        // NOT. The seed is now the framegraph's PROVENANCE declaration, not just a cross-frame
        // start state (`graph.rs::add_buffer`'s contract): a bare `add_buffer` promises this graph
        // WRITES the resource before reading it, and `compile`'s debug guard enforces that for
        // buffers as of P2-8. `interp_pairs`/`interp_out_slot` are filled by the HOST into
        // host-coherent memory and only READ in this graph (the `interp` pass below is their sole
        // access, and it is a read) — the submit's host-write → device-domain dependency is what
        // orders them, so they have no in-graph producer BY DESIGN and must say so. That is the
        // exact case the blanket "drop the kind test" experiment reddened, which is what proved the
        // exemption load-bearing. `interp_model_out` keeps the bare `add_buffer`: the `interp`
        // compute WRITES it here, before every reader. The seed VALUE is `undefined()` in both
        // spellings, so not one derived barrier field moves.
        let (interp_pairs, interp_out_slot, interp_model_out) = if scene.interp.is_some() {
            (
                Some(g.add_buffer_seeded("interp_pairs", ResSync::undefined())),
                Some(g.add_buffer_seeded("interp_out_slot", ResSync::undefined())),
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
        // `debug_assert_eq!` there guards the two never diverging). Since rung R3 landed
        // the SDF-only leg, `Deferred × Sdf` reaches here `false` — see
        // `mesh_depth_neutral_clear` below for its depth-clear replacement. Every other
        // currently reachable `Deferred` config still has `mesh_leg == true`, so the
        // declaration order and every `image_access`/`buffer_access` call below stay BYTE-FOR-
        // BYTE unchanged from the pre-R2 unconditional form there.
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

        // Pass `mesh_depth_neutral_clear` (§E leg-disable / the R3 O2 audit finding): under
        // `Deferred × Sdf` the raster pass above is skipped, so NOTHING transitions/clears the
        // shared depth image the marcher samples at binding 1 (`gDepth`) — without a
        // replacement the marcher's `.Load` would read UNDEFINED (garbage) depth, making
        // `has_mesh` effectively random per pixel instead of universally `false` (the correct
        // "no mesh in the scene" classification the marcher already handles byte-identically,
        // see `sdf_gbuffer_composite.hlsl`'s own 0%-gate doc). This depth-ONLY clear pass (no
        // color attachments, no draw) reproduces EXACTLY the depth half of the raster pass's own
        // clear (CLEAR to the far-plane sentinel, `DEPTH_ATTACHMENT_OPTIMAL`), so the marcher
        // deterministically reads "no mesh" for every pixel — ZERO shader changes. `Some` iff
        // `scene.path_has_mesh_depth_neutral_clear()` — the SAME predicate `record_gbuffer`'s
        // depth-only begin/end-rendering block checks (W1 parity, mirroring `raster`/`marcher`).
        // Mutually exclusive with `raster` by construction (raster iff mesh_leg; this iff
        // sdf_leg && !mesh_leg), so the two never both fire and depth has exactly one producer
        // every frame.
        let mesh_depth_neutral_clear = if scene.path_has_mesh_depth_neutral_clear() {
            let p = g.add_pass("mesh_depth_neutral_clear");
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

        // Pass `viewt_from_depth` (§E leg-disable / the R3b audit finding): `Deferred × Mesh`'s
        // `gViewT`-producer replacement for the (undispatched) marcher. `Some` iff
        // `scene.viewt_from_depth.is_some()` — the SAME field `record_gbuffer`'s dispatch block
        // reads (capability = component presence, mirroring `scene.ssao.is_some()`'s gate — the
        // two sites read the SAME `scene` field, so they cannot diverge by construction; no W1
        // predicate fn is needed the way `mesh_depth_neutral_clear` above needs one, since that
        // pass has NO caller-supplied Option to key off). Declared right after the depth producer
        // (`raster` or `mesh_depth_neutral_clear`, whichever ran) and before `marcher` — the SAME
        // slot the marcher itself would occupy under every other leg, so its depth-read barrier
        // derives identically. Reads depth (the SAME SHADER_READ_ONLY access the marcher declares
        // below) + WRITES gViewT (every dispatched pixel is written exactly once — no prior-frame
        // value survives, unlike the marcher's RW batch which shares one access across four
        // attribute images).
        let viewt_from_depth = if scene.viewt_from_depth.is_some() {
            let p = g.add_pass("viewt_from_depth");
            g.image_access(
                depth,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                SubRange::DEPTH,
            );
            g.image_access(
                viewt,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
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

        // Pass `csm` (cascade depth) — gated `scene.csm.is_some()`. The write is declared over
        // the FULL `MAX_CASCADES` array (NOT just `[0..active_count)`): the resolve samples the
        // cascade map through a 2D_ARRAY view spanning EVERY layer, and
        // VUID-vkCmdDraw/Dispatch-None-09600 requires every layer of a statically-bound
        // descriptor to sit in the descriptor's layout — so the `[active..MAX)` tail must ride
        // the same UNDEFINED→DEPTH_ATTACHMENT→SHADER_READ_ONLY cycle (discard-legal garbage the
        // shader's `active_count` gate never dynamically samples). The rendering loop still
        // touches only `[0..active)`. The barrier-out (→SHADER_READ_ONLY) is placed by the graph
        // at the resolve's cascade read (a sound-superset re-order; matches build_maximal_frame).
        let csm = if scene.csm.is_some() {
            let p = g.add_pass("csm_depth");
            g.image_access(
                cascade,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::depth_layers(MAX_CASCADES as u32),
            );
            Some(p)
        } else {
            None
        };

        // Pass `atlas` (spot/point atlas depth) — gated `scene.atlas_punctual.is_some()`.
        // Full `MAX_TEXTURE_LAYERS` array for the SAME 09600 whole-view reason as `csm` above.
        let atlas_pass = if scene.atlas_punctual.is_some() {
            let p = g.add_pass("atlas_depth");
            g.image_access(
                atlas,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::depth_layers(MAX_TEXTURE_LAYERS as u32),
            );
            Some(p)
        } else {
            None
        };

        // Pass `resolve`: reads albedo/normal/material/viewt/ssao (GENERAL) + the L1
        // buffers + the layered shadow maps (→sampled), writes lit (first-touch
        // UNDEFINED→GENERAL). Buffer reads stay gated on their producer having run (a
        // buffer descriptor has no layout to keep valid). IMAGE reads of always-bound
        // descriptors (cascade/atlas — like `pbr` below) are declared UNCONDITIONALLY:
        // VUID-vkCmdDraw/Dispatch-None-09600 requires a statically-referenced descriptor's
        // image to sit in the descriptor layout even when the shader's mode gate never
        // dynamically samples it, so the OFF path derives a discard-legal
        // UNDEFINED→SHADER_READ_ONLY transition (the T6a `pbr` first-touch pattern; the
        // `seeded_readers` seed keeps layout UNDEFINED, so the transition is real).
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
        // UNCONDITIONAL (09600, the T6a `pbr` pattern): `gSsao` @11 is statically referenced by
        // the resolve `.spv` regardless of `ssao_mode`, so the SSAO-off frame must still leave
        // the image in GENERAL — the seeded first-touch read derives the discard-legal
        // UNDEFINED→GENERAL transition; the SSAO-on frame's read is unchanged (the SSAO pass
        // wrote first, so this derives the same write→read barrier it always did).
        g.image_access(
            ssao,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            SubRange::COLOR,
        );
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
        // UNCONDITIONAL + FULL-ARRAY (09600, see the pass comment above): with the depth pass ON
        // this derives the whole-array DEPTH_ATTACHMENT→SHADER_READ_ONLY barrier-out; with it OFF
        // (bound-but-unread) it derives the discard-legal UNDEFINED→SHADER_READ_ONLY transition
        // that keeps the always-bound descriptor's layout valid.
        g.image_access(
            cascade,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::depth_layers(MAX_CASCADES as u32),
        );
        g.image_access(
            atlas,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::depth_layers(MAX_TEXTURE_LAYERS as u32),
        );
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
            mesh_depth_neutral_clear,
            viewt_from_depth,
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

    /// Multi-paradigm render-path plan, rung R4b-b (§B "Forward / ForwardPlus", Decision 2): the
    /// `Forward` v1 declarator, called ONLY by [`Self::declare_frame_graph`]'s `Forward` arm.
    /// Re-declares [`Self::frame_graph`] from scratch (`reset` + declare + `compile`) into its
    /// OWN small, PRIVATE resource/ResId space (see [`ForwardPassPlan`]'s doc for why this is
    /// decoupled from [`GbufferBarrierSink`]/`FRAMEGRAPH_IMAGE_COUNT`) and stores the resulting
    /// [`ForwardPassPlan`] in [`Self::forward_pass_plan`] — [`Self::gbuffer_pass_plan`] is reset
    /// to `None` for hygiene (a `Forward` frame never reads it; `record_forward` dispatches on
    /// `scene.path_is_forward()`, the SAME predicate this fn's caller used, not on which plan is
    /// `Some`).
    ///
    /// v1 scope cut (`forward_opaque.fs.hlsl`'s own doc, `cap_forward_v1_consumers`): mesh-only,
    /// all-lights (no froxel), NO SSAO/DDGI/shadow-denoise/motion/TAA — so this declarator is
    /// `interp? → light_upload? → csm? → atlas? → forward_opaque → present_sample`, matching
    /// [`Renderer::record_forward`]'s ACTUAL record order EXACTLY (`compile()` derives barriers in
    /// DECLARATION order — a pass declared after its reader emits the transition backwards,
    /// code-review P1-2). Dependency order (NOT the plan's literal pass-listing order, which
    /// places `light_upload?/csm?/atlas?` AFTER `forward_opaque`): `forward_opaque.fs.hlsl`
    /// samples the cascade/atlas depth maps AND the light table INLINE, so `csm`/`atlas`/
    /// `light_upload`'s producers must all precede it — see [`ForwardPassPlan::csm`]'s doc.
    /// `light_upload`'s EXACT position relative to `csm`/`atlas` is immaterial (all three only
    /// need to precede `forward_opaque`, the shared reader); it is declared FIRST among the three
    /// here, mirroring the Deferred declarator's own placement (`raster → light_upload? → coarse?
    /// → marcher`).
    pub(crate) fn declare_forward_graph(&mut self, scene: &GBufferScene<'_>) {
        use crate::framegraph::{ResSync, SubRange};

        const FRAG: u32 =
            VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT;

        self.gbuffer_pass_plan = None;
        let g = &mut self.frame_graph;
        g.reset();

        // --- Images (FIXED local ResId order: lit=0, forward_depth=1, cascade=2, atlas=3 —
        // see `ForwardBarrierSink::images`'s doc). `lit` is a FRESH `add_image` (undefined first
        // touch) here — Forward's OWN producer access (`ColorAttachmentWrite`), never the
        // Deferred `StorageWrite` this same physical image carries on a Deferred boot (C5: the
        // two paths are boot-mutually-exclusive). `cascade`/`atlas` mirror
        // `declare_deferred_graph`'s cross-frame seed (audit B-003: the re-render this frame must
        // order after the sibling in-flight frame's still-pipelined FRAGMENT read).
        let lit = g.add_image("lit");
        let forward_depth = g.add_image("forward_depth");
        let cascade = g.add_image_seeded(
            "cascade",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let atlas = g.add_image_seeded(
            "atlas",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        // Code-review P2-2: pins the declared image count against `FORWARD_IMAGE_COUNT`
        // (`ForwardBarrierSink`'s doc) — a silent divergence here would offset every buffer ResId
        // (`res.index() - FORWARD_IMAGE_COUNT` in `ForwardBarrierSink::buffer_barriers`) without
        // any other compile-time signal.
        debug_assert_eq!(
            atlas.index() + 1,
            FORWARD_IMAGE_COUNT,
            "invariant: declare_forward_graph's image declarations must match FORWARD_IMAGE_COUNT"
        );
        // --- Buffers (light_table=0, instance_model_ring=1 — logical sink slots, offset by
        // `FORWARD_IMAGE_COUNT` in the ResId space). `light_table` mirrors the Deferred cross-frame
        // seed (a dirty-frame re-write orders after the sibling frame's still-pipelined FRAGMENT
        // read, this rung's equivalent of the resolve's COMPUTE read). `instance_model_ring`
        // represents the SAME shared `instance_rings[fi]` buffer — VERIFIED (code review open
        // question) against `boyko_app::gpu_scene`: `InterpGpuProd::activation`'s `model_out`
        // param is passed `&self.instance_rings[slot]` at its ONE call site (`gpu_scene/mod.rs`'s
        // `scene()`), and `GBufferScene::forward_instance_ring` is ALSO `&self.instance_rings`
        // (the SAME field) at its OWN assignment in `scene()` — so `interp`'s write target and
        // `forward_opaque`'s VS-bound Set-0 binding-0 buffer are PROVABLY the same physical
        // buffer, not two distinct rings (unlike a hypothetical design with a separate model_out
        // ring — this codebase has none; `interp.rs`'s own doc: "model_out_buffer is the SHARED
        // instance ring slot"). Frame-private (a sibling in-flight frame touches a DIFFERENT ring
        // slot), so `add_buffer` (undefined) — and it stays a BARE `add_buffer` under VG R3 P2-8's
        // provenance contract because this graph does produce it: `interp`'s COMPUTE write below is
        // declared under `scene.interp.is_some()`, the SAME predicate that gates every declared
        // read of it (`depth_prepass`, `forward_opaque`), so no frame reads it unproduced.
        let light_table = g.add_buffer_seeded(
            "light_table",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let instance_model_ring = g.add_buffer("instance_model_ring");
        // Multi-paradigm render-path plan, rung R5 (ForwardPlus): the L1 cluster-cull trio —
        // the SAME single-instance, cross-frame-shared buffers `declare_deferred_graph` declares
        // (`grid`/`index`/`alloc` there), seeded identically: `light_index_alloc` ends its frame
        // on the cull's atomic WRITES with no draining read (writer seed); `cluster_grid`/
        // `light_index` end their frame consumed by `forward_opaque`'s FRAGMENT read (not
        // Deferred's COMPUTE resolve — Forward's own reader stage), so a dirty-frame re-cull
        // must order after the sibling in-flight frame's still-pipelined FRAGMENT read.
        let light_index_alloc = g.add_buffer_seeded(
            "light_index_alloc",
            ResSync::seeded_writer(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT),
        );
        let cluster_grid = g.add_buffer_seeded(
            "cluster_grid",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let light_index = g.add_buffer_seeded(
            "light_index",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );

        // Pass `interp` — gated `scene.interp.is_some()`, the SAME activation
        // `declare_deferred_graph`'s own `interp` pass reads. Writes `instance_model_ring`
        // (COMPUTE/SHADER_WRITE); the COMPUTE→VERTEX barrier ordering this write before
        // `forward_opaque`'s VS read is derived at `forward_opaque` (the reader), not here.
        let interp = if scene.interp.is_some() {
            let p = g.add_pass("interp");
            g.buffer_access(
                instance_model_ring,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            );
            Some(p)
        } else {
            None
        };

        // Pass `depth_prepass` — Multi-paradigm render-path plan, rung R5 (ForwardPlus,
        // Decision 4's EQUAL-depth early-Z contract). Gated by [`GBufferScene::
        // path_needs_depth_prepass`] (the O1 single-predicate rule shared with `record_forward`).
        // Declared right after `interp` (the earliest point `instance_model_ring` is valid to
        // read) and BEFORE every other Forward pass — the FIRST writer of `forward_depth` this
        // frame under `ForwardPlus` (first-touch UNDEFINED→DEPTH_ATTACHMENT_OPTIMAL, `GREATER`,
        // write ON); `forward_opaque`'s later `EQUAL` test then only READS the value this pass
        // committed (declared at the `forward_opaque` site below).
        let depth_prepass = if scene.path_needs_depth_prepass() {
            let p = g.add_pass("depth_prepass");
            g.image_access(
                forward_depth,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::DEPTH,
            );
            if interp.is_some() {
                g.buffer_access(
                    instance_model_ring,
                    VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                );
            }
            Some(p)
        } else {
            None
        };

        // Pass `light_upload` (async light-table re-upload) — the SAME gate
        // `declare_deferred_graph`'s own `light_upload` pass uses. Code-review P1-2: declared
        // BEFORE `csm`/`atlas`/`forward_opaque` (matching `record_forward`'s ACTUAL record order
        // — `interp? → depth_prepass? → light_upload? → light_cull? → csm? → atlas? →
        // forward_opaque → present_sample`, rung R5 widened — this pass just needs to precede
        // `forward_opaque`, the reader; `compile()` derives barriers in DECLARATION order, so a
        // pass declared AFTER its reader would emit the TRANSFER_WRITE → SHADER_READ barrier the
        // wrong way around — the exact bug this fix closes).
        let light_upload = if scene.light_dirty && scene.light_upload_bytes > 0 {
            let p = g.add_pass("light_upload");
            g.buffer_access(light_table, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT);
            Some(p)
        } else {
            None
        };

        // Pass `light_cull` (L1 clustered froxel cull) — Multi-paradigm render-path plan, rung
        // R5 (ForwardPlus). Gated EXACTLY as `declare_deferred_graph`'s own `light_cull` pass
        // (the "4-buffers-Some" predicate: the cull pipeline AND all three cluster buffers are
        // `Some`) AND `ForwardPlus` — the base `Forward` pipeline's Set 0 declares no
        // `ClusterGrid`/`LightIndexList` bindings at all, so this pass would be meaningless (and
        // unusable) under plain `Forward`. Resets `light_index_alloc` (transfer), reads the light
        // table, writes `cluster_grid`/`light_index` — byte-for-byte the SAME access shape
        // `GbufferPassPlan::light_cull`'s declaration site uses.
        let light_cull = if scene.path_is_forward_plus()
            && scene.cluster_cull.is_some()
            && scene.cluster_grid.is_some()
            && scene.light_index.is_some()
            && scene.light_index_alloc.is_some()
        {
            const RW: u32 = VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT;
            let p = g.add_pass("light_cull");
            g.buffer_access(
                light_index_alloc,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_ACCESS_TRANSFER_WRITE_BIT,
            );
            g.buffer_access(light_index_alloc, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW);
            g.buffer_access(
                light_table,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
            g.buffer_access(
                cluster_grid,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            );
            g.buffer_access(
                light_index,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            );
            Some(p)
        } else {
            None
        };

        // Pass `csm` (cascade depth) — gated `scene.csm.is_some()`. FULL `MAX_CASCADES` array
        // (NOT `[0..active_count)`), the SAME 09600 whole-view shape `declare_deferred_graph`'s
        // `csm` pass declares (see that site's comment for the tail-layer rationale).
        let csm = if scene.csm.is_some() {
            let p = g.add_pass("csm_depth");
            g.image_access(
                cascade,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::depth_layers(MAX_CASCADES as u32),
            );
            Some(p)
        } else {
            None
        };

        // Pass `atlas` (spot/point atlas depth) — gated `scene.atlas_punctual.is_some()`. Full
        // `MAX_TEXTURE_LAYERS` array, the SAME shape `declare_deferred_graph`'s `atlas` pass
        // declares.
        let atlas_pass = if scene.atlas_punctual.is_some() {
            let p = g.add_pass("atlas_depth");
            g.image_access(
                atlas,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::depth_layers(MAX_TEXTURE_LAYERS as u32),
            );
            Some(p)
        } else {
            None
        };

        // Pass `forward_opaque` (always present): writes `lit` (COLOR, first-touch
        // UNDEFINED→COLOR_ATTACHMENT_OPTIMAL — Decision 2's C5 per-path producer access); the
        // `forward_depth` access depends on `depth_prepass` (rung R5, Decision 4) — see below.
        // Reads `cascade`/`atlas` inline at FRAGMENT (→SHADER_READ_ONLY_OPTIMAL) when their depth
        // pass ran this frame — gated on the SAME `scene.csm`/`scene.atlas_punctual` predicate the
        // producer used, so an unarmed shadow source routes zero barriers on it, exactly like the
        // Deferred resolve's conditional reads; reads `instance_model_ring` (VERTEX) when interp
        // armed (the COMPUTE→VERTEX barrier this read derives is what orders `interp`'s write
        // before it); reads `light_table` (FRAGMENT) when `light_upload` ran this frame (the
        // TRANSFER→FRAGMENT barrier this derives orders the copy before the FS's light-table
        // load — code-review P1-2: this read was MISSING entirely before this fix); reads
        // `cluster_grid`/`light_index` (FRAGMENT) when `light_cull` ran this frame (rung R5).
        let forward_opaque = g.add_pass("forward_opaque");
        g.image_access(
            lit,
            VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
            VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            SubRange::COLOR,
        );
        if depth_prepass.is_some() {
            // ForwardPlus (rung R5): `depth_prepass` already wrote `forward_depth` this frame
            // (first-touch, `GREATER`, write ON) — `forward_opaque`'s `EQUAL` test (depth-write
            // OFF, `create_graphics_pipeline_forward_plus`) only READS it, so the graph derives
            // the WRITE→READ barrier between the two passes, not a second first-touch write.
            g.image_access(
                forward_depth,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::DEPTH,
            );
        } else {
            // Plain `Forward` (no prepass this rung either — `cap_forward_v1_consumers` still
            // caps pre-light consumers off): the SOLE, first-touch writer of `forward_depth` —
            // byte-identical to rung R4b-b.
            g.image_access(
                forward_depth,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::DEPTH,
            );
        }
        // Code-review P1-1: `cascade`/`atlas` are `Format::D32Sfloat` DEPTH images — the read
        // MUST declare `SubRange::depth_layers(..)` (DEPTH aspect, the SAME layered range the
        // producer wrote), not `SubRange::COLOR` (a spec violation: the derived
        // DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL transition would carry a COLOR
        // aspect mask, so the DEPTH aspect never actually transitions — broken/undefined shadow
        // sampling). UNCONDITIONAL + FULL-ARRAY (09600): `forward_opaque.fs` statically
        // references both maps, so on the OFF path this derives the discard-legal
        // UNDEFINED→SHADER_READ_ONLY transition that keeps the always-bound Set-1 descriptors'
        // layout valid (the SAME shape `declare_deferred_graph`'s resolve reads use).
        g.image_access(
            cascade,
            VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::depth_layers(MAX_CASCADES as u32),
        );
        g.image_access(
            atlas,
            VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::depth_layers(MAX_TEXTURE_LAYERS as u32),
        );
        if interp.is_some() {
            g.buffer_access(
                instance_model_ring,
                VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
        }
        // Code-review P1-2: the light-table READ `forward_opaque.fs.hlsl` performs every frame
        // (`LightBuf` @3, `load_light_header`/`load_light`) was never declared — a dirty-frame
        // `light_upload` copy (TRANSFER_WRITE) had NO derived ordering against this FRAGMENT
        // read, mirroring `declare_deferred_graph`'s OWN `if light_upload.is_some()` resolve-read
        // gate (only need the barrier when a write happened THIS frame; the cross-frame seed
        // already covers the steady-state read).
        if light_upload.is_some() {
            g.buffer_access(
                light_table,
                VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
        }
        // Multi-paradigm render-path plan, rung R5 (ForwardPlus): the froxel `ClusterGrid`/
        // `LightIndexList` reads `forward_opaque_froxel.fs.hlsl` performs every frame, gated on
        // `light_cull.is_some()` — the SAME "only need the barrier when a write happened THIS
        // frame" discipline `light_upload`'s read gate above uses (the cross-frame seed already
        // covers the steady-state read).
        if light_cull.is_some() {
            g.buffer_access(
                cluster_grid,
                VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
            g.buffer_access(
                light_index,
                VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
        }

        // Pass `sdf_forward_march` — Multi-paradigm render-path plan, rung R-SDFFWD: the fused
        // SDF march-then-shade COMPUTE pass. Gated on [`GBufferScene::path_has_sdf_forward`] (`==
        // resolved_render_path.sdf_forward_marched` — Forward-family with the SDF leg present,
        // the O1 single-predicate rule shared with `record_forward`). Declared AFTER
        // `forward_opaque` (whose raster COLOR write it extends, C5) and BEFORE `present_sample`
        // (its reader): writes `lit` at COMPUTE/STORAGE_WRITE/GENERAL — the graph derives
        // `forward_opaque`'s COLOR_ATTACHMENT_OPTIMAL → GENERAL transition here, then
        // `present_sample`'s GENERAL → SHADER_READ_ONLY_OPTIMAL below (the SAME 3-state chain the
        // deferred resolve's own `lit` write establishes, `declare_deferred_graph`'s doc). Reads
        // `forward_depth` at COMPUTE/SHADER_READ/SHADER_READ_ONLY_OPTIMAL ONLY when the resolved
        // legs also carry the mesh leg (`resolved_render_path.mesh_leg` — the `HAS_MESH` compute
        // variant samples it to bound the march at the mesh surface; the mesh-less variant never
        // references the binding, so declaring the read under `!mesh_leg` would derive a spurious
        // barrier the recorder never needs). No buffer/vocab-resource accesses are declared here
        // (the SDF field/material/brick buffers are boot-seeded/untracked, the SAME precedent the
        // deferred `marcher` pass's own declaration establishes — this file's `marcher` pass doc).
        //
        // TAA-under-VB tripwire: the Forward family has NO AA seam, so the VIEWT-variant marcher
        // (whose `viewt` write only `declare_vb_graph` declares) must never arm here — a future
        // Forward-TAA rung must extend THIS declarator with the `viewt` access first.
        debug_assert!(
            !scene.path_sdf_forward_writes_viewt(),
            "invariant: path_sdf_forward_writes_viewt() under a Forward-family declarator — \
             the Forward graph declares no viewt write for the marcher"
        );
        let sdf_forward_march = if scene.path_has_sdf_forward() {
            let p = g.add_pass("sdf_forward_march");
            g.image_access(
                lit,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
            if scene.resolved_render_path.mesh_leg {
                g.image_access(
                    forward_depth,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                    SubRange::DEPTH,
                );
            }
            Some(p)
        } else {
            None
        };

        // Pass `present_sample` (site 5c): `lit` COLOR_ATTACHMENT_OPTIMAL→SHADER_READ_ONLY_OPTIMAL
        // for the present-blit's FRAGMENT sample (C5: derived from the producer/consumer access
        // pair, not a hardcoded source layout — the SAME `present_sample` shape
        // `declare_deferred_graph` declares, just against `forward_opaque`'s COLOR write instead
        // of the resolve's STORAGE write). Rung R-SDFFWD: when `sdf_forward_march` ran this frame,
        // `lit` is already `GENERAL` from that pass's write, so this derives GENERAL →
        // SHADER_READ_ONLY_OPTIMAL (the deferred resolve's own transition shape); when it did not
        // run, this is UNCHANGED from rung R4b-b (`forward_opaque`'s COLOR_ATTACHMENT_OPTIMAL →
        // SHADER_READ_ONLY_OPTIMAL). The swapchain WSI barriers stay hand-recorded.
        let present_sample = g.add_pass("present_sample");
        g.image_access(
            lit,
            VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::COLOR,
        );

        g.compile();

        self.forward_pass_plan = Some(ForwardPassPlan {
            interp,
            depth_prepass,
            light_upload,
            csm,
            atlas: atlas_pass,
            light_cull,
            forward_opaque,
            sdf_forward_march,
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

    /// Multi-paradigm render-path plan, rung R4b-b (buffers grown at rung R5): the
    /// [`ForwardBarrierSink`] sibling of [`Self::record_graph_pass`] — drives one
    /// [`ForwardPassPlan`] pass's derived barriers (declared by [`Self::declare_forward_graph`])
    /// into `cmd`, resolving Forward's OWN small ResId space (`[lit, forward_depth, cascade,
    /// atlas]` images; `[light_table, instance_model_ring, light_index_alloc, cluster_grid,
    /// light_index]` buffers — [`ForwardBarrierSink`]'s doc) to the current frame's physical
    /// handles. `lit` is read from `targets` (the SAME [`GBufferTargets::lit`] ring Option 2's
    /// full deferred allocation always builds — Forward reuses it verbatim, C5); `forward_depth`/
    /// the Forward-only descriptor sets are read from `forward` (the current frame's
    /// [`ForwardTargets`], which the caller has already `.expect()`-ed present, mirroring how
    /// [`Self::record_graph_pass`]'s caller unwraps `self.gbuffer_pass_plan`). The L1 trio falls
    /// back to `scene.light_table` when unarmed (`GBufferScene::cluster_grid`/`light_index` are
    /// `None`) — the SAME bound-but-unread placeholder idiom `ForwardTargets::build`'s Set-0
    /// entries use; an unarmed frame never routes a barrier naming ResId 2/3/4 anyway
    /// (`declare_forward_graph`'s `light_cull.is_some()` gate), so the placeholder is inert.
    pub(crate) fn record_forward_pass(
        &self,
        pass: crate::framegraph::PassId,
        cmd: VkCommandBuffer,
        targets: &GBufferTargets,
        forward: &super::targets::ForwardTargets,
        scene: &GBufferScene<'_>,
        fi: usize,
    ) {
        let mut sink = ForwardBarrierSink {
            fns: self.fns,
            cmd,
            images: [
                targets.lit[fi].image,
                forward.depth[fi].image,
                scene.csm_cascade_texture.image,
                scene.shadow_atlas_texture.image,
            ],
            buffers: [
                scene.light_table.buffer,
                // The SAME physical buffer `forward_opaque`'s VS reads at Set-0 binding 0
                // (`scene.forward_instance_ring[fi]`) AND (when armed) the interp compute pass
                // writes into (`GBufferScene::forward_instance_ring`'s doc — the "SAME shared
                // model-out ring" the Deferred raster/interp precedent already establishes).
                // This fn is called ONLY on a `Forward`-resolved frame (the caller's `forward:
                // &ForwardTargets` param already required unwrapping `TargetsProfile::ForwardMesh`),
                // so `Some(...)` is a production invariant here (`GBufferScene::forward_pipeline`'s
                // doc).
                scene
                    .forward_instance_ring
                    .expect("invariant: a Forward-resolved scene always carries forward_instance_ring")
                    [fi]
                    .buffer,
                // Multi-paradigm render-path plan, rung R5 (ForwardPlus): the L1 cluster-cull
                // trio, single-instance (NOT ringed) — the SAME physical buffers
                // `record_graph_pass`'s own `[cluster_grid, light_index, light_index_alloc]`
                // slots resolve for Deferred.
                scene.light_index_alloc.map_or(scene.light_table.buffer, |b| b.buffer),
                scene.cluster_grid.map_or(scene.light_table.buffer, |b| b.buffer),
                scene.light_index.map_or(scene.light_table.buffer, |b| b.buffer),
            ],
        };
        self.frame_graph.record_pass(pass, &mut sink);
    }
}

/// Multi-paradigm render-path plan, rung R8: the FUSED VisibilityBuffer v1 path's per-frame
/// `PassId` map (`declare_vb_graph`) — a private, per-frame ResId space (`self.frame_graph` is
/// fully `reset()` + re-declared every frame regardless of path, the SAME trade-off
/// [`ForwardPassPlan`]'s doc explains). `record_vb` reads this to drive each pass's derived
/// barriers through [`VbBarrierSink`].
///
/// V1 scope cut (mirrors `cap_vb_v1_consumers`): no `interp` (the VB instance ring
/// [`GBufferScene::vb_instance_ring`] is a plain CPU `bytemuck` upload,
/// `boyko_render::upload::upload_vb_instance_rows` — no GPU-side interpolation this rung, a
/// documented v1 simplification), no depth prepass (`mesh_geo_shade_split == false`, fused
/// only), no froxel/light-cull (VB v1 shades ALL-LIGHTS, mirrors plain `Forward`'s own base
/// compile).
#[derive(Clone, Copy)]
pub(crate) struct VbPassPlan {
    /// The async light-table re-upload (`scene.light_dirty && scene.light_upload_bytes > 0`) —
    /// the SAME gate [`ForwardPassPlan::light_upload`] uses.
    pub(crate) light_upload: Option<crate::framegraph::PassId>,
    /// VB-P1a ("dark infra"): the L1 clustered froxel light-cull pass — the SAME "4-buffers-Some"
    /// gate [`ForwardPassPlan::light_cull`] uses (`scene.cluster_cull`/`cluster_grid`/
    /// `light_index`/`light_index_alloc` all `Some`). ⚠️ The arm is **default-OFF, not hardcoded
    /// off**: `froxel_light_cull = clusters_wanted && path == VisibilityBuffer`, and
    /// `clusters_wanted` threads from the owner-set `LightingConfig::clusters_enabled`, whose
    /// default is `false`. So this is `None` on every DEFAULT boot — which is all the 0%-gate
    /// byte-identity argument needs — but NOT on every boot: `vb_mesh_froxel` and
    /// `vb_mesh_tex_froxel` set it `true` and are golden-pinned.
    /// Resets `light_index_alloc` (transfer), reads the light table, writes
    /// `cluster_grid`/`light_index`. Declared BEFORE `csm`/`atlas`/`vb_resolve`/`vb_shade` (which
    /// read the cull's writes) — the SAME declaration-order-parity discipline every pass in this
    /// plan follows.
    pub(crate) light_cull: Option<crate::framegraph::PassId>,
    /// The CSM cascade depth pass (`scene.csm.is_some()`) — declared BEFORE `vb_resolve` (which
    /// samples the cascade inline via `shadow_apply.hlsli`), the SAME dependency order
    /// [`ForwardPassPlan::csm`] observes.
    pub(crate) csm: Option<crate::framegraph::PassId>,
    /// The sparse spot/point atlas depth pass (`scene.atlas_punctual.is_some()`) — see
    /// [`Self::csm`]'s doc.
    pub(crate) atlas: Option<crate::framegraph::PassId>,
    /// The always-present sky BACKGROUND pass — REUSES `forward_sky.{vs,fs}.hlsl`'s compiled
    /// SPIR-V verbatim against a NEW pipeline object (`GBufferScene::vb_sky_pipeline`'s doc):
    /// writes `lit` (`ColorAttachmentWrite`/`COLOR_ATTACHMENT_OPTIMAL`, first touch, Decision 2's
    /// C5 per-path `lit`-producer access) for every pixel; `vb_resolve` (below) then overwrites
    /// only the pixels its own geometry fetch covers, leaving the sky color standing elsewhere
    /// (the SAME "misses write nothing" contract `sdf_forward_march` documents).
    pub(crate) vb_sky: crate::framegraph::PassId,
    /// The mesh id-raster pass (`vb_raster.{vs,fs}.hlsl`, Decision 9): writes `vb_id`
    /// (`ColorAttachmentWrite`/`COLOR_ATTACHMENT_OPTIMAL`, R32G32_UINT) + the VB-only reverse-Z
    /// `vb_depth` (`DepthStencilAttachmentWrite`/`DEPTH_ATTACHMENT_OPTIMAL`, first-touch, `GREATER`,
    /// Decision 4). Early-Z-clean (no `SV_Depth`/`discard`/UAV in the FS).
    ///
    /// VG rung R2d-4: it also READS `vb_visible_instance` (`SHADER_READ` at `VERTEX_SHADER`, Set-0
    /// @11) when the cull is armed — the consumer half of the `COMPUTE → VERTEX_SHADER` RAW the
    /// rung-R2d-3 region write exists to produce.
    ///
    /// Rung R10: `Some` only when `resolved_render_path.mesh_leg` — a `VisibilityBuffer × Sdf`
    /// (mesh-less) frame gates this OFF entirely (it needs the Decision-0 geometry table, which
    /// carries no slot with no mesh leg) and leaves `lit` for `vb_sky` + [`Self::sdf_forward_march`]
    /// alone. `record_vb` reads the SAME `mesh_leg` predicate, so a `None` here is never recorded.
    pub(crate) vb_raster: Option<crate::framegraph::PassId>,
    /// VG R3 piece 2 step P2-5 (docs/VG-R3-P2-CAPABILITY-SPLIT-PLAN.md, decision D4): the LATE
    /// raster scope — a SECOND `vkCmdBeginRendering`/`EndRendering` bracket over the SAME two
    /// attachments and the SAME `renderArea`, `LOAD_OP_LOAD`/`STORE_OP_STORE` on both, fed by its
    /// own [`GBufferScene::vb_indirect_late`](super::scene_types::GBufferScene::vb_indirect_late)
    /// record array.
    ///
    /// `Some` iff [`GBufferScene::path_vb_occlusion_split`](super::scene_types::GBufferScene::path_vb_occlusion_split)
    /// — the SAME single predicate `record_vb` reads, so declare and record cannot disagree.
    ///
    /// It shipped INERT through piece 2 and through steps P3-1..P3-5: every record it fetched
    /// carried `instanceCount = 0`, so `LOAD_OP_LOAD` yielded exactly what the early scope stored,
    /// no fragment was produced, and `STORE_OP_STORE` wrote the loaded contents back — final
    /// contents identical, by an argument that needs no numerics. That is the shipped-inert
    /// discipline `vb_batch_cull.comp.hlsl` documents for its own two rungs ("EACH LEVEL SHIPPED
    /// INERT ONE RUNG BEFORE IT WAS ARMED, on purpose… Neither was a placeholder"): the scope, the
    /// per-batch loop and the record array were real, and only the one word a later step produces
    /// was the inert constant.
    ///
    /// VG R3 piece 3 step P3-6 changed that word's PRODUCER (host `0` → the late cull's survivor
    /// count `n_keep`), bound `vb_set0_late` and set the survivor-indirection bit — adding two
    /// VERTEX reads to this pass's access list and no structure. ⚠️ `n_keep` is correctly ZERO on
    /// every converged static frame (plan D12's fixed point), so the scope still draws nothing
    /// there; that is the theorem, not residual inertness.
    ///
    /// A dedicated array rather than a rewrite of [`GBufferScene::vb_indirect`](super::scene_types::GBufferScene::vb_indirect)
    /// between the scopes: the early scope needs `instanceCount = early_k` and the late one
    /// `instanceCount = late_k` in ONE command buffer, so sharing would put a transfer against the
    /// early scope's still-in-flight indirect fetches, every frame, forever, to save 20 KiB × FIF.
    pub(crate) vb_raster_late: Option<crate::framegraph::PassId>,
    /// Rung R2a': the inline `vkCmdUpdateBuffer` that fills this frame's indirect draw records —
    /// and, since rung R2c0, the [`VbBatchDesc`](super::scene_types::VbBatchDesc) array the batch
    /// cull reads. `Some` iff the mesh leg records a raster (the same gate `vb_raster` itself
    /// carries) AND `scene.vb_indirect` is armed; `None` leaves the recorder on its direct-draw
    /// path.
    pub(crate) vb_indirect_upload: Option<crate::framegraph::PassId>,
    /// VG R3 piece 2 step P2-5 (plan D4): the inline `vkCmdUpdateBuffer` that fills this frame's
    /// LATE indirect draw records. `Some` iff
    /// [`GBufferScene::path_vb_occlusion_split`](super::scene_types::GBufferScene::path_vb_occlusion_split),
    /// declared BEFORE [`Self::vb_raster_late`] fetches from them.
    ///
    /// ⚠️ ITS OWN PASS, deliberately NOT folded into [`Self::vb_indirect_upload`], whose gate is
    /// `scene.vb_indirect.is_some()` — reconciling two different predicates on one pass is exactly
    /// how the single-predicate discipline gets broken.
    ///
    /// ⚠️ AND IT MUST EXIST. With no declared writer the graph takes the first-touch arm and
    /// derives `(TOP_OF_PIPE, 0)` for the late raster's indirect fetch — an execution-only edge
    /// that makes the fill neither available nor visible. On frame 1, over freshly allocated
    /// DEVICE_LOCAL memory, `instanceCount` is then arbitrary, `firstInstance` may be nonzero with
    /// `drawIndirectFirstInstance` VK_FALSE and `robustBufferAccess` off, and the scope this whole
    /// piece claims draws nothing, draws. A barrier COUNT cannot see that — the defective shape
    /// derives the SAME three barriers and differs only in `src_stage`/`src_access` — which is why
    /// G4 asserts fields. Since VG R3 P2-8 `graph.rs`'s unwritten-read backstop catches it too: it
    /// discriminates by declared PROVENANCE rather than resource kind, and `vb_indirect_late` is a
    /// bare `add_buffer`, so dropping this pass's write fires a `debug_assert!` in every
    /// dev-profile run. (It waved every BUFFER read through until then — P2-7 executed the
    /// deletion and measured all four gates green.)
    pub(crate) vb_indirect_late_upload: Option<crate::framegraph::PassId>,
    /// VG rung R2c0: the per-BATCH draw-record cull compute pass (`vb_batch_cull.comp.hlsl`).
    /// Declared BETWEEN [`Self::vb_indirect_upload`] and [`Self::vb_raster`], which is what makes
    /// the graph derive both halves of the seam this rung exists to de-risk: `TRANSFER → COMPUTE`
    /// against the upload, and `COMPUTE → DRAW_INDIRECT` against the raster's indirect fetch
    /// (which until this rung was derived against the TRANSFER directly).
    ///
    /// `Some` iff [`Self::vb_indirect_upload`] is AND the whole R2c0 arm is wired (pipeline,
    /// layout and all three cull buffers). Since rung R2c the shader makes a real decision; the
    /// nine golden pins stay byte-identical anyway because every pinned scene is entirely
    /// on-screen, which is exactly why `vb_cull_offscreen.rs` — not a golden — is what proves the
    /// cull rejects.
    pub(crate) vb_batch_cull: Option<crate::framegraph::PassId>,
    /// VG rung R2c-tail / R2d-5: the cull READBACK copy — `Some` only when `scene.vb_cull_readback`
    /// is armed (the `BOYKO_VB_CULL_READBACK` probe). Declares `TRANSFER_READ` on all FOUR buffers
    /// the probe copies (`vb_cull_count`, `vb_cull_visible`, `vb_indirect`, `vb_visible_instance`),
    /// so the graph derives the `COMPUTE -> TRANSFER` dependency that makes the cull's writes
    /// available to the copies. Unarmed on every golden/interactive boot ⇒ no pass, no commands.
    ///
    /// VG R3 piece 3 step P3-3: on a SPLIT frame it also declares `vb_late_visible` and
    /// `vb_late_count` — the PRE-late snapshot, the only place the early phase's candidate set is
    /// observable, because the late phase compacts that list in place.
    pub(crate) vb_cull_readback: Option<crate::framegraph::PassId>,
    /// VG R3 piece 3 step P3-3 (docs/VG-R3-P3-CULL-INTEGRATION-PLAN.md, decisions D4/D5/D8): the
    /// LATE cull compute pass — the SECOND dispatch of `vb_batch_cull.comp.hlsl`, selected by the
    /// `pc.phase` push word, declared between the last [`Self::hzb_build`] and
    /// [`Self::vb_raster_late`].
    ///
    /// `Some` iff [`GBufferScene::path_vb_occlusion_split`](super::scene_types::GBufferScene::path_vb_occlusion_split)
    /// — the SAME single predicate `record_vb` reads.
    ///
    /// It is the declared writer of `vb_indirect_late`'s `instanceCount`, which is piece 2's
    /// obligation 1 discharged: that array's chain becomes
    /// `vb_indirect_late_upload (TRANSFER_WRITE) → vb_cull_late (SHADER_WRITE) → vb_raster_late
    /// (INDIRECT_COMMAND_READ)`, plus `vb_cull_readback_late (TRANSFER_READ)` on probe frames.
    ///
    /// ⚠️ ITS PHASE-1 BODY IS REAL SINCE STEP P3-4 AND STILL WRITES ONLY ZEROS. The loop bound is
    /// `VbLateCount[i]`, which the early phase writes only under `VB_CULL_OCC_ARMED` and which the
    /// late phase reads as `0` without it — so the compaction runs zero iterations and stores the
    /// `instanceCount = 0` the host upload already seeded. The fork and this dispatch were ONE
    /// commit deliberately: without the fork, phase 1 would re-run phase 0's body and rewrite the
    /// early lists after the early raster had fetched them.
    pub(crate) vb_cull_late: Option<crate::framegraph::PassId>,
    /// VG R3 piece 3 step P3-3 (plan D8): the POST-late readback snapshot — `vb_late_visible`'s
    /// compacted prefix, `vb_late_count` again (the no-clobber clause) and `vb_indirect_late`'s
    /// GPU-written `instanceCount`.
    ///
    /// `Some` iff the split is armed AND `scene.vb_cull_readback` is (the `BOYKO_VB_CULL_READBACK`
    /// probe). Declared AFTER [`Self::vb_raster_late`] so the shipping chain it observes is
    /// field-identical with and without the probe — sited between the late cull and the late raster
    /// it would re-source that scope's indirect fetch.
    pub(crate) vb_cull_readback_late: Option<crate::framegraph::PassId>,
    /// VG R3 piece 1 step P1-5 (docs/VG-R3-P1-PYRAMID-PLAN.md): the HZB depth-pyramid BUILD
    /// chain — `hzb_build_p` writes pyramid mips `[6p, 6p + n)` and, for `p > 0`, reads mip
    /// `6p - 1`. Slot `p` is `Some` iff `p < plan.levels.div_ceil(HZB_LEVELS_PER_PASS)`, the
    /// SAME arithmetic `HzbTargets::sets` is sized by, so a recorded dispatch always has both a
    /// declared pass and a built descriptor set.
    ///
    /// `[None; MAX_HZB_PASSES]` on the `HzbMode::Off` 0%-gate AND on a mesh-less
    /// (`VisibilityBuffer × Sdf`) frame: the pyramid reduces the depth THIS FRAME'S RASTER
    /// WROTE, and without the mesh leg `vb_depth` has no producer at all this frame.
    pub(crate) hzb_build: [Option<crate::framegraph::PassId>; crate::compute::MAX_HZB_PASSES],
    /// VG R3 piece 1 step P1-8 (plan §5/§13, gate G8): the pyramid POISON clear — one
    /// `vkCmdClearColorImage` filling mips `[0, plan.levels)` with
    /// [`HZB_PYRAMID_POISON`](super::scene_types::HZB_PYRAMID_POISON), declared BEFORE the first
    /// [`Self::hzb_build`] pass and gated on EXACTLY [`Self::hzb_dump`]'s predicate.
    ///
    /// It is what makes G8 non-vacuous WITHOUT depending on the fixture's screen coverage. Step
    /// P1-6 measured the `vb_mesh` dump: the scene covers ~11% of the framebuffer, the rest is the
    /// reverse-Z far plane `0.0`, a `min` footprint containing any `0.0` is `0.0` — so 89.3% of
    /// the pyramid is `0.0` and levels 6..9 are ENTIRELY so. A pyramid a driver zero-filled and
    /// NOBODY WROTE matches the oracle at every one of those texels, and levels 6..9 are precisely
    /// what the SECOND build pass writes. Poisoned first, an unwritten texel reads `-1.0`, which
    /// the reduce cannot produce at any coverage.
    ///
    /// `None` on every frame that is not a `BOYKO_HZB_DUMP` frame ⇒ no pass, no barrier, no
    /// command, and every golden pin stays byte-identical.
    pub(crate) hzb_poison: Option<crate::framegraph::PassId>,
    /// VG R3 piece 1 step P1-6 (plan §5, gate G8): the pyramid DUMP copy — `Some` only when
    /// `scene.hzb_dump` is armed (the `BOYKO_HZB_DUMP` probe) on a frame that also builds a
    /// pyramid. Declares `TRANSFER_READ` on `vb_depth` (`TRANSFER_SRC_OPTIMAL`, DEPTH aspect) and
    /// on `hzb_pyramid` (`GENERAL`, mips `[0, plan.levels)`), so the graph derives both the
    /// layout transition out of the depth's last reader and the `COMPUTE → TRANSFER` flush of the
    /// build's stores. Declared LAST in the whole graph, so it observes the FINISHED pyramid.
    /// Unarmed on every golden/interactive boot ⇒ no pass, no barrier, no command.
    pub(crate) hzb_dump: Option<crate::framegraph::PassId>,
    /// VG R3 piece 3 step P3-7 (plan D10, gate G-P3-E): the EARLY-DEPTH dump copy — one
    /// `TRANSFER_READ` of `vb_depth` at `TRANSFER_SRC_OPTIMAL` (DEPTH aspect), declared between the
    /// last [`Self::hzb_build`] pass and [`Self::vb_cull_late`].
    ///
    /// `Some` iff the frame arms the occlusion split AND `scene.hzb_dump` is armed — the ONLY
    /// configuration in which the two depths can differ, and the only one in which the question is
    /// asked. `path_vb_occlusion_split()` already carries `mesh_leg` and `hzb.is_some()`, so this
    /// pass's presence implies [`Self::hzb_dump`]'s (asserted below, not assumed).
    ///
    /// # Why the copy is a SECOND pass and the end-of-frame one did not simply move
    ///
    /// Moving it would give ONE depth and therefore only a one-sided claim: "the pyramid agrees
    /// with a rebuild from this depth". Dumping BOTH lets the gate also state the negative — where
    /// the two depths differ, a rebuild from the FINAL one disagrees with the pyramid — which is
    /// what turns G5 from "the build is correct" into "the build ran at the right point in the
    /// frame". Piece 2's own `hzb_dump` comment records that it could not make that claim.
    ///
    /// The declared position is the whole of its correctness: after the builds, the depth is still
    /// exactly what they reduced; before `vb_raster_late`, nothing has drawn into it again. The
    /// graph derives the round trip out of `hzb_build_0`'s `SHADER_READ_ONLY_OPTIMAL` and back into
    /// `vb_raster_late`'s `DEPTH_ATTACHMENT_OPTIMAL` — both preserving, neither a first touch.
    pub(crate) hzb_dump_depth_early: Option<crate::framegraph::PassId>,
    /// VB-P2 classification plan (docs/VB-P2-CLASSIFICATION-PLAN.md), rung P2b (gate widened at
    /// rung P2c): the `fill` pass (two `vkCmdFillBuffer`s — zeros `counts[MAX]`, sentinels
    /// `group_to_mat[G+MAX]` with `0xFFFFFFFF`, critic P1-1) declared as the FIRST producer on
    /// the `gclassify` ResId (`TRANSFER_WRITE`). `Some` iff `mesh_leg &&
    /// scene.vb_use_classified` (rung P2c narrowed this from the P2b-era `mesh_leg`-only gate,
    /// plan P1-4: a `!vb_use_classified` frame pays ZERO classify tax — the chain is not even
    /// declared). The classify chain's output feeds [`Self::vb_shade`] (below) when armed; when
    /// unarmed, [`Self::vb_resolve`] alone shades every pixel.
    pub(crate) vb_classify_fill: Option<crate::framegraph::PassId>,
    /// The `count` classify compute pass (`vb_classify_count.comp.hlsl`): reads `vb_id` (COMPUTE,
    /// `SHADER_READ_ONLY_OPTIMAL`) — the SAME image [`Self::vb_resolve`]/[`Self::vb_shade`] read;
    /// this pass runs FIRST so it derives the `COLOR_ATTACHMENT_OPTIMAL`→`SHADER_READ_ONLY_OPTIMAL`
    /// barrier, and the `lit`-producer's later same-layout read needs none — and reads+writes
    /// `gclassify` (COMPUTE, RW: `InterlockedAdd(counts[mat], 1)` per non-sentinel pixel). `Some`
    /// under the SAME gate as [`Self::vb_classify_fill`].
    pub(crate) vb_classify_count: Option<crate::framegraph::PassId>,
    /// The `scan` classify compute pass (`vb_classify_scan.comp.hlsl`, a SINGLE workgroup):
    /// reads+writes `gclassify` only (COMPUTE, RW — the two chained exclusive-prefix-sum phases
    /// over `counts`/`offsets`/`cursors`/`gbase`/`group_to_mat`). `Some` under the SAME gate as
    /// [`Self::vb_classify_fill`].
    pub(crate) vb_classify_scan: Option<crate::framegraph::PassId>,
    /// The `scatter` classify compute pass (`vb_classify_scatter.comp.hlsl`): the SAME `vb_id` +
    /// `gclassify` access shape as [`Self::vb_classify_count`] (`InterlockedAdd(cursors[mat], 1)`
    /// then `pixel_list[slot] = py*w+px`). `Some` under the SAME gate as
    /// [`Self::vb_classify_fill`].
    pub(crate) vb_classify_scatter: Option<crate::framegraph::PassId>,
    /// The FUSED resolve compute pass (`vb_resolve.comp.hlsl`, Decision 5): reads `vb_id`
    /// (`ShaderRead`/`SHADER_READ_ONLY_OPTIMAL`, COMPUTE) and writes `lit` (`ShaderWrite`/`GENERAL`,
    /// extending `vb_sky`'s COLOR write, C5). Reads `cascade`/`atlas` inline (COMPUTE) when armed,
    /// reads `light_table` (COMPUTE) when [`Self::light_upload`] ran this frame. Rung P2c: `Some`
    /// iff `mesh_leg && !scene.vb_use_classified` — mutually exclusive with [`Self::vb_shade`] by
    /// construction (exactly one of the two is the frame's `lit` producer; the fused-vs-classified
    /// selector, plan P1-4).
    pub(crate) vb_resolve: Option<crate::framegraph::PassId>,
    /// VB-P2 classification plan, rung P2c: the material-classified shading compute pass
    /// (`vb_shade.comp.hlsl`) — the [`Self::vb_resolve`] SIBLING `lit` producer, selected instead
    /// of the fused resolve when `scene.vb_use_classified` holds. Reads `vb_id` (COMPUTE,
    /// `SHADER_READ_ONLY_OPTIMAL` — already transitioned by [`Self::vb_classify_count`], the
    /// FIRST reader this frame when this pass is armed, so this read derives no further barrier)
    /// and `gclassify` (COMPUTE, `SHADER_READ` only — `vb_shade` never writes it, unlike the
    /// classify chain's own RW access) and writes `lit` (COMPUTE, `SHADER_WRITE`/`GENERAL`,
    /// extending `vb_sky`'s COLOR write, C5 — the SAME transition [`Self::vb_resolve`] derives).
    /// Reads `vb_instance_ring` (COMPUTE, the geometry-fetch instance-row lookup — the SAME read
    /// [`Self::vb_resolve`] declares) and `cascade`/`atlas`/`light_table` inline when armed (the
    /// SAME conditional reads [`Self::vb_resolve`] declares — `vb_shade`'s shading tail is
    /// character-identical, plan D3). `Some` iff `mesh_leg && scene.vb_use_classified`
    /// `&& !path_vb_split()` (rung R9b: the split displaces the classification chain).
    pub(crate) vb_shade: Option<crate::framegraph::PassId>,
    /// Rung R9b: the split's thin-aux GEOMETRY producer (`vb_geo.comp` — the first `vb_id`
    /// reader under split, first-touch `thin_normal` writer). `Some` iff
    /// [`GBufferScene::path_vb_split`](super::scene_types::GBufferScene::path_vb_split).
    pub(crate) vb_geo: Option<crate::framegraph::PassId>,
    /// Rung R9b: the VB SSAO gather (`sdf_ssao` `-D VB_THIN` variant — reads
    /// `thin_normal`+`viewt`, writes `ssao`). `Some` iff
    /// [`GBufferScene::path_vb_ssao`](super::scene_types::GBufferScene::path_vb_ssao).
    pub(crate) vb_ssao: Option<crate::framegraph::PassId>,
    /// Rung R9b: the VB à-trous denoise chain (the DEFERRED `ssao_atrous` role loop mirrored
    /// verbatim) — `[None; MAX]` unless the gather armed with `atrous_levels >= 2`.
    pub(crate) ssao_atrous:
        [Option<crate::framegraph::PassId>; crate::present::MAX_SSAO_ATROUS_LEVELS as usize],
    /// Rung R9c: the DDGI probe-update pass under VB (the deferred `ddgi_update` mirrored).
    /// `Some` iff [`GBufferScene::path_vb_ddgi`](super::scene_types::GBufferScene::path_vb_ddgi)
    /// — reachable only on `VB × Both`.
    pub(crate) ddgi_update: Option<crate::framegraph::PassId>,
    /// Rung R9b: the split's `lit` producer (`vb_shade_split.comp` — RE-fetch + shade +
    /// unconditional gSsao consumption; rung R9c adds the CONDITIONAL DDGI atlas reads).
    /// `Some` iff
    /// [`GBufferScene::path_vb_split`](super::scene_types::GBufferScene::path_vb_split).
    pub(crate) vb_shade_split: Option<crate::framegraph::PassId>,
    /// HW-RT rung R9d: the TLAS-instance PACK compute pre-pass (`scene.tlas.is_some()`) — the VB
    /// sibling of [`GbufferPassPlan::tlas_pack`], declared inside the split arm (VB's TLAS
    /// exists only to feed this chain).
    #[cfg(feature = "hwrt")]
    pub(crate) tlas_pack: Option<crate::framegraph::PassId>,
    /// HW-RT rung R9d: the per-frame TLAS BUILD pass — the VB sibling of
    /// [`GbufferPassPlan::tlas_build`].
    #[cfg(feature = "hwrt")]
    pub(crate) tlas_build: Option<crate::framegraph::PassId>,
    /// HW-RT rung R9d: the RT soft-shadow VIS pre-pass (`GBufferScene::path_vb_hwrt_shadow()`)
    /// — the VB sibling of [`GbufferPassPlan::shadow_vis`], reading `thin_normal`/`viewt`
    /// instead of the fat `gNormal`/`gViewT`.
    #[cfg(feature = "hwrt")]
    pub(crate) shadow_vis: Option<crate::framegraph::PassId>,
    /// HW-RT rung R9d: the per-level à-trous denoise passes — the VB sibling of
    /// [`GbufferPassPlan::shadow_atrous`].
    #[cfg(feature = "hwrt")]
    pub(crate) shadow_atrous:
        [Option<crate::framegraph::PassId>; crate::present::MAX_ATROUS_LEVELS as usize],
    /// HW-RT rung R9d: the temporal reproject+accumulate pass — the VB sibling of
    /// [`GbufferPassPlan::shadow_temporal`].
    #[cfg(feature = "hwrt")]
    pub(crate) shadow_temporal: Option<crate::framegraph::PassId>,
    /// Rung R10: the fused `sdf_forward_march` COMPUTE pass (`shaders/sdf_forward_march.comp.hlsl`,
    /// the SAME pass the Forward family declares — [`ForwardPassPlan::sdf_forward_march`]). `Some`
    /// iff [`GBufferScene::path_has_sdf_forward`](super::scene_types::GBufferScene::path_has_sdf_forward)
    /// (`== resolved_render_path.sdf_forward_marched`). Writes `lit` (`ShaderWrite`/`GENERAL`,
    /// extending `vb_resolve`'s STORAGE write under `Both`, or `vb_sky`'s COLOR write under `Sdf`,
    /// C5); reads `vb_depth` (COMPUTE/`SHADER_READ_ONLY_OPTIMAL`) ONLY under `mesh_leg` (the
    /// `HAS_MESH` variant samples the mesh surface to bound the march — the SAME conditional read
    /// `declare_forward_graph`'s own `sdf_forward_march` arm declares).
    pub(crate) sdf_forward_march: Option<crate::framegraph::PassId>,
    /// The always-present present-sample pass: the `lit` `GENERAL` → `SHADER_READ_ONLY_OPTIMAL`
    /// transition (C5: derived from the LAST `lit` producer's access — `vb_resolve` under `Mesh`,
    /// `sdf_forward_march` when it ran, else `vb_sky`). The swapchain WSI barriers stay
    /// hand-recorded, exactly like [`ForwardPassPlan::present_sample`].
    pub(crate) present_sample: crate::framegraph::PassId,
    /// TAA-under-VB: the `vb_viewt` `gViewT`-producer compute pass
    /// (`viewt_from_depth_rz.comp.hlsl`) — reads `vb_depth` (COMPUTE/SHADER_READ_ONLY, the
    /// raster barrier-out), first-touch writes `viewt` (UNDEFINED→GENERAL). `Some` iff
    /// [`GBufferScene::viewt_from_vb_depth`](super::scene_types::GBufferScene::viewt_from_vb_depth)
    /// is armed (`VisibilityBuffer × Mesh` with TAA on).
    pub(crate) viewt_from_depth: Option<crate::framegraph::PassId>,
    /// TAA-under-VB: the TAA history-resolve compute pass — the VB sibling of
    /// [`GbufferPassPlan::taa_resolve`], identical access list (`lit` CIS SHADER_READ_ONLY +
    /// `viewt`/`taa_hist_read` GENERAL reads + `taa_hist` GENERAL write; `aa_out`/`taa_resolved`
    /// hand-recorded in `record_taa`/`record_rcas`). `Some` iff `scene.taa.is_some()`.
    pub(crate) taa_resolve: Option<crate::framegraph::PassId>,
}

/// Multi-paradigm render-path plan, rung R8: the [`BarrierSink`](crate::framegraph::BarrierSink)
/// for VB v1's small, PRIVATE per-frame graph — the SAME "own decoupled ResId space" discipline
/// [`ForwardBarrierSink`]'s doc explains for the Forward family. Resolves the FIXED local ResId
/// order [`Renderer::declare_vb_graph`] declares — images `[lit=0, vb_id=1, vb_depth=2,
/// cascade=3, atlas=4, viewt=5, taa_hist=6, taa_hist_read=7]` (the TAA-under-VB trio appended
/// after `atlas` so 0..4 stayed byte-unchanged), buffers `[light_table=0, vb_instance_ring=1,
/// gclassify=2]` — to the current frame's physical handles. Lives only for the duration of one
/// `record_pass` call inside [`Renderer::record_vb`].
pub(crate) struct VbBarrierSink<'a> {
    pub(crate) fns: &'a DeviceFns,
    pub(crate) cmd: VkCommandBuffer,
    /// `[lit, vb_id, vb_depth, cascade, atlas, viewt, taa_hist, taa_hist_read]` — see this
    /// type's doc for the fixed order. `lit` is the current frame slot's
    /// [`GBufferTargets::lit`] image (C5-reused, the SAME physical image every path's `lit`
    /// producer writes); `vb_id`/`vb_depth` are the current frame slot's
    /// [`VbTargets`](super::targets::VbTargets)/[`ForwardTargets`](super::targets::ForwardTargets)
    /// images (VB reuses `ForwardTargets::depth` verbatim for `vb_depth` — `VbTargets`'s doc);
    /// `cascade`/`atlas` are the SAME single-instance, world-fixed textures every other path's
    /// shadow chain references. TAA-under-VB appendix: `viewt` = `targets.viewt[fi]` (always
    /// allocated — VbMesh runs the DeferredFull-shaped body); `taa_hist` = the `[fi]` WRITE
    /// slot, `taa_hist_read` = the SIBLING `taa_hist[fi ^ 1]` (the ONE non-`[fi]` entry — the
    /// deferred sink's own C1-fix shape), both `VkImage::NULL`-bound when TAA is off (inert: no
    /// pass names their ResIds then). Rung R9b/c append `[thin_normal, ssao, ssao_ring_a,
    /// ssao_ring_b, ddgi_irr, ddgi_depth]` (ResIds 8..13); rung R9d appends the SAME hwrt
    /// `[shadow_vis, shadow_vis2, motion_vec, shadow_temporal_hist, temporal_out,
    /// shadow_temporal_hist_read]` tail the deferred sink's own [`FRAMEGRAPH_IMAGE_COUNT`] doc
    /// documents (ResIds 14..19) — same NULL-when-ungated rule.
    ///
    /// VG R3 piece 1 step P1-5 appends `hzb_pyramid` LAST (ResId 14, or 20 under `hwrt`) — the
    /// `HzbTargets::pyramid` image, `VkImage::NULL` on the `HzbMode::Off` 0%-gate, where no pass
    /// names its ResId (the `taa_hist`/`thin_normal`/`ssao_ring_a` precedent, NOT `viewt`'s
    /// always-allocated one). It is the ONE entry whose barriers carry a non-trivial mip span.
    pub(crate) images: [VkImage; VB_IMAGE_COUNT],
    /// `[light_table, vb_instance_ring, gclassify, ddgi_classification, ddgi_ray_table]` (+ rung
    /// R9d's `tlas_instances` under `hwrt`). A pass that does not declare an access on an unarmed
    /// resource never routes a barrier naming it, so an inert `VkBuffer::NULL` there is harmless
    /// (the SAME "ungated slot may hold NULL" rule [`ForwardBarrierSink`] documents).
    /// `gclassify` — VB-P2 classification plan rung P2b: the current frame slot's
    /// [`VbClassifyTargets::gclassify`](super::targets::VbClassifyTargets::gclassify) buffer.
    /// The two DDGI buffers (rung R9c) are the deferred sink's own single-instance sources; the
    /// rung R9d `tlas_instances` source mirrors [`GbufferBarrierSink`]'s own
    /// `scene.tlas.map_or(VkBuffer::NULL, |t| t.instance_array.buffer)`. VB-P1a ("dark infra"):
    /// `cluster_grid`/`light_index`/`light_index_alloc` — the L1 froxel trio, falling back to
    /// the light-table placeholder when unarmed (`scene.cluster_grid`/`light_index`/
    /// `light_index_alloc` are `None` — hardcoded today), the SAME bound-but-unread idiom
    /// [`ForwardBarrierSink`]'s own trio uses; a `light_cull.is_none()` frame never routes a
    /// barrier naming these ResIds anyway, so the placeholder is inert.
    /// Rung R2a' appended `vb_indirect`; rung R2c0 appends the batch-cull trio
    /// `[vb_batch_desc, vb_cull_visible, vb_cull_count]` after it — placeholder-backed when the
    /// arm is unwired, and inert then for the same "no pass names the ResId" reason. Rung R2d-3
    /// appends `vb_visible_instance` (the per-INSTANCE survivor list the cull writes) LAST, and it
    /// is MANDATORY rather than placeholder-backed — `GpuSceneBundles` wires it unconditionally,
    /// exactly like `vb_instance_ring` at index 1. VG R3 piece 2 step P2-3 appends
    /// `vb_indirect_late` after IT, LAST in both `cfg` arms and `.expect()`ed for the same
    /// mandatory-on-every-VB-boot reason — see [`VB_BUFFER_COUNT`]. VG R3 piece 3 step P3-3 appends
    /// the occlusion split's trio `[vb_late_visible, vb_late_count, vb_cull_uniform]` after THAT,
    /// on the same rule and for the same reason: `GpuSceneBundles::boot` mints all three
    /// unconditionally, so a placeholder would be strictly worse than a `.expect()` — it resolves to
    /// a LIVE WRONG buffer with no VUID and no validation message.
    pub(crate) buffers: [VkBuffer; VB_BUFFER_COUNT],
}

/// The number of BUFFER resources [`Renderer::declare_vb_graph`] declares — see
/// [`VbBarrierSink::buffers`]'s doc for the fixed order. The buffer-side sibling of
/// [`VB_IMAGE_COUNT`], and it is the ARRAY LENGTH rather than a number standing beside one: a
/// constant that merely equals a hand-written literal is dead, and dead is what the two literals
/// it replaces (13 / 14) were.
///
/// ⚠️ The buffer side needs this MORE than the image side does. A `res.index()` that lands on the
/// wrong buffer slot names a **live wrong buffer** — every slot resolves to a real handle, since
/// the unarmed ones fall back to `scene.light_table.buffer` — so a mis-indexed buffer barrier
/// synchronises the wrong allocation with no VUID and no validation message, where the image array
/// would hold `VkImage::NULL` and fault loudly. Appending is the one edit shape that cannot
/// re-key an existing barrier, and `declare_vb_graph`'s `debug_assert_eq!` on the LAST buffer
/// ResId is what keeps this count and the declaration order from drifting apart.
///
/// The `hwrt` arm's one extra slot is rung R9d's `tlas_instances` at index 5, which shifts the
/// VB-P1a froxel trio to 6/7/8 and every slot after it by one — the two arms share no index past
/// 4, which is why the count is `cfg`-split rather than derived.
///
/// VG R3 piece 2 step P2-3: 13 → 14 (`vb_indirect_late`), 14 → 15 under `hwrt`. VG R3 piece 3 step
/// P3-3 appends the occlusion split's THREE buffers after it — `vb_late_visible`, `vb_late_count`,
/// `vb_cull_uniform` — so 14 → 17 and 15 → 18.
#[cfg(feature = "hwrt")]
const VB_BUFFER_COUNT: usize = 18;
/// See the `hwrt` variant's doc: a `not(hwrt)` build has no `tlas_instances` slot, so 18 - 1 = 17.
#[cfg(not(feature = "hwrt"))]
const VB_BUFFER_COUNT: usize = 17;

/// The number of IMAGE resources [`Renderer::declare_vb_graph`] declares — see
/// [`VbBarrierSink::images`]'s doc for the fixed order. A PRIVATE, per-frame ResId space (mirrors
/// [`FORWARD_IMAGE_COUNT`]'s own doc — never related to [`FRAMEGRAPH_IMAGE_COUNT`]).
///
/// Rung R9d: under `hwrt` the array grows by SIX — the same `shadow_vis`/`shadow_vis2`/
/// `motion_vec`/`shadow_temporal_hist`/`temporal_out`/`shadow_temporal_hist_read` tail the
/// deferred declarator appends after `ddgi_depth` ([`FRAMEGRAPH_IMAGE_COUNT`]'s own doc) — VB
/// appends the SAME six after ITS OWN `ddgi_depth` (ResId 13), landing at 14..19.
///
/// VG R3 piece 1 step P1-5 appends the HZB depth pyramid LAST in BOTH `cfg` arms — ResId 20
/// under `hwrt`, ResId 14 without it — so every existing ResId is byte-unchanged.
#[cfg(feature = "hwrt")]
const VB_IMAGE_COUNT: usize = 21;
/// See the `hwrt` variant's doc: a `not(hwrt)` build keeps the count at 14 + the pyramid = 15.
#[cfg(not(feature = "hwrt"))]
const VB_IMAGE_COUNT: usize = 15;

/// VG R3 piece 1 step P1-5: the HZB build passes' framegraph names, indexed by pass number.
/// [`FrameGraph::add_pass`](crate::framegraph::FrameGraph::add_pass) takes a `&'static str`, so a
/// per-pass name cannot be formatted at declare time — one literal per slot IS the mechanism.
/// Sized by [`MAX_HZB_PASSES`](crate::compute::MAX_HZB_PASSES), so a capacity change is a compile
/// error here rather than an index panic first reached at a 4096-wide render extent.
const HZB_BUILD_PASS_NAMES: [&str; crate::compute::MAX_HZB_PASSES] =
    ["hzb_build_0", "hzb_build_1", "hzb_build_2"];

/// VG R3 piece 1 step P1-5: a single-layer COLOR [`SubRange`](crate::framegraph::SubRange) over
/// mips `[base, base + count)` — the HZB build chain's per-pass span.
///
/// [`SubRange::color_mips`](crate::framegraph::SubRange::color_mips) cannot express it (it pins
/// `base_mip: 0`, while a reduce pass reads mip `d - 1` and writes from mip `d`), and it is also
/// the spelling that invites the `MAX_HZB_LEVELS` mistake. Every span this helper is called with
/// is DERIVED from `HzbPlan::levels`; the capacity constant is never a span.
#[inline]
const fn hzb_mips(base: u32, count: u32) -> crate::framegraph::SubRange {
    crate::framegraph::SubRange {
        aspect: VK_IMAGE_ASPECT_COLOR_BIT,
        base_mip: base,
        mip_count: count,
        base_layer: 0,
        layer_count: 1,
    }
}

/// VG R3 piece 2 step P2-5 (docs/VG-R3-P2-CAPABILITY-SPLIT-PLAN.md, decision D6): declares the
/// `[hzb_poison, hzb_build_0 .. hzb_build_{n-1}]` BLOCK, which
/// [`Renderer::declare_vb_graph`] places in ONE of two slots per frame — today's (after the `lit`
/// producer) on an unsplit frame, and immediately after the EARLY `vb_raster` on an armed-split
/// one. Returns `(hzb_poison, hzb_build)` for [`VbPassPlan`].
///
/// # Why the block is ONE unit, and why that is a function rather than a convention
///
/// `hzb_poison` is asserted to precede EVERY `hzb_build_*` (the `debug_assert!` beside the plan
/// construction in `declare_vb_graph`), [`PassId`](crate::framegraph::PassId) is strictly
/// monotonic in declare order and `compile()` does not reorder. So moving the builds without the
/// poison puts `build.index() < poison.index()`: a clear that ERASES the levels the dispatches
/// just wrote. A dev-profile build fires the assert; a release binary compiles it out, the dump
/// then reads `HZB_PYRAMID_POISON` everywhere and gate G8 reds claiming "the build never ran" —
/// a gate reporting the wrong defect. One function is what makes "the block moves whole" a
/// property of the code instead of a property of a reviewer.
///
/// # The arguments carry the gates, so the two call sites cannot disagree
///
/// `hzb_levels` is `scene.hzb.filter(|_| mesh_leg).map(|p| p.levels)` — the pyramid's LIVE level
/// count with the `mesh_leg` conjunct already folded in. That conjunct is load-bearing: without a
/// mesh leg `vb_raster` is not declared, nothing writes `vb_depth` this frame, and `hzb_build_0`'s
/// read would take `compile`'s first-touch arm on an unwritten transient. Every span below is
/// derived from that number and NEVER from `MAX_HZB_LEVELS`, which is a capacity (17) and out of
/// range at every real render extent — since P1-5a `image_access` rejects it in RELEASE.
///
/// `dump_armed` is `scene.hzb_dump.is_some()`. The poison arms on EXACTLY
/// `hzb_levels.is_some() && dump_armed`, which is the dump pass's own predicate verbatim: a frame
/// that is poisoned is always a frame that is dumped, and vice versa. Anything narrower poisons a
/// pyramid nobody reads back; anything wider puts a transfer write on a frame that ships.
fn declare_hzb_poison_build(
    g: &mut crate::framegraph::FrameGraph,
    hzb_levels: Option<u32>,
    dump_armed: bool,
    hzb_pyramid: crate::framegraph::ResId,
    vb_depth: crate::framegraph::ResId,
) -> (
    Option<crate::framegraph::PassId>,
    [Option<crate::framegraph::PassId>; crate::compute::MAX_HZB_PASSES],
) {
    use crate::framegraph::SubRange;

    // ==== VG R3 piece 1 step P1-8 (plan §5/§13, gate G8): the pyramid POISON clear. ====
    //
    // Declared FIRST — before every `hzb_build_p`, which is the whole point of it and is asserted
    // at the plan construction in the shape step P1-6 used to pin the dump's own position. On a
    // dump frame the pyramid IMAGE is filled with `HZB_PYRAMID_POISON` so that an unwritten texel
    // holds a value the reduce can never produce; the host half then reads "no texel is the
    // poison" as "every level was WRITTEN", at any scene coverage.
    //
    // ⚠️ THE IMAGE, NOT THE STAGING. The host driver already prefills the staging with NaN, and
    // that catches a failed COPY — it cannot see a level the BUILD never wrote, because copying an
    // unwritten level succeeds and faithfully transfers whatever the image holds. Step P1-6
    // measured what that costs: 89.3% of the `vb_mesh` pyramid is the far plane `0.0`, levels 6..9
    // entirely so, and a zero-filled image agrees with the oracle there.
    //
    // The write is `TRANSFER(TRANSFER_WRITE)` at `GENERAL`. ⚠️ Since VG R3 piece 3 step P3-0 it is
    // no longer a first TOUCH: the pyramid is seeded
    // `seeded_writer_at_layout(GENERAL, COMPUTE_SHADER, SHADER_WRITE)` (the cross-frame seed D2
    // argues for), so on an UNSPLIT frame the clear derives
    // `COMPUTE(SHADER_WRITE) → TRANSFER(TRANSFER_WRITE)` at `GENERAL → GENERAL` — a WAW against the
    // PREVIOUS frame's build, rather than an `UNDEFINED → GENERAL` transition that would have
    // licensed discarding what that build wrote.
    //
    // ⚠️ AND SINCE STEP P3-3 IT IS NO LONGER THE FRAME'S FIRST ACCESS EITHER, on a SPLIT frame:
    // `vb_batch_cull` reads the pyramid — as the PREVIOUS frame left it, which is the whole of the
    // early predicate — before this block runs. The clear then derives a WAR against THAT read
    // (`COMPUTE → TRANSFER`, `src_access` 0, an execution-only edge) instead of the WAW against the
    // seed. Strictly weaker in form and exactly right in substance: what must be ordered is the
    // early cull's read finishing before the clear overwrites what it read.
    // `hzb_build_0` still derives a real WAW flush (`TRANSFER_WRITE → SHADER_WRITE`) off this
    // clear; on an UNDUMPED frame the frame's first access is `hzb_build_0`'s own write and it is
    // sourced from the seed instead. `GENERAL` is one of the two layouts `vkCmdClearColorImage`
    // accepts, and it is the layout the pyramid holds for life — from `boot_clear_hzb_pyramid`
    // onward — so no extra transition appears anywhere.
    let hzb_poison = match (hzb_levels, dump_armed) {
        (Some(levels), true) => {
            let p = g.add_pass("hzb_poison");
            g.image_access(
                hzb_pyramid,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_ACCESS_TRANSFER_WRITE_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                hzb_mips(0, levels),
            );
            Some(p)
        }
        _ => None,
    };

    // ==== VG R3 piece 1 step P1-5: the HZB depth-pyramid BUILD chain. ====
    //
    // Pass `p` writes mips `[d, d + n)` where `d = p * HZB_LEVELS_PER_PASS` and
    // `n = min(HZB_LEVELS_PER_PASS, levels - d)`, and (for `p > 0`) reads mip `d - 1`. That is TWO
    // spans on ONE ResId inside ONE pass, which is exactly what step P1-5a re-keyed the sync state
    // `(ResId, mip)` to admit; `tests/framegraph_gbuffer_equiv.rs`'s
    // `compile_derives_the_hzb_build_chain_at_a_real_extent` pins the three barriers this
    // declaration derives at `levels = 10`.
    //
    // `hzb_build_0`'s `vb_depth` read is what derives that image's
    // `DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL` transition out of the raster; every
    // later same-layout read then needs none. ⚠️ VG R3 piece 2 step P2-5: at the ARMED-SPLIT slot
    // "later" no longer means "for the rest of the frame" — `vb_raster_late` writes the depth
    // again immediately after this block, so every downstream reader is re-sourced from a real RAW
    // flush plus a preserving layout transition (strictly stronger than the execution-only edge it
    // replaces, and the reason `hzb_dump`'s own comment had to be corrected).
    let mut hzb_build: [Option<crate::framegraph::PassId>; crate::compute::MAX_HZB_PASSES] =
        [None; crate::compute::MAX_HZB_PASSES];
    if let Some(levels) = hzb_levels {
        let pass_count = levels.div_ceil(crate::compute::HZB_LEVELS_PER_PASS) as usize;
        debug_assert!(
            pass_count <= crate::compute::MAX_HZB_PASSES,
            "invariant: the plan's pass count fits MAX_HZB_PASSES (levels <= MAX_HZB_LEVELS)"
        );
        for (p, slot) in hzb_build.iter_mut().enumerate().take(pass_count) {
            let d = p as u32 * crate::compute::HZB_LEVELS_PER_PASS;
            let n = (levels - d).min(crate::compute::HZB_LEVELS_PER_PASS);
            let pass = g.add_pass(HZB_BUILD_PASS_NAMES[p]);
            if p == 0 {
                // The SOURCE depth, at the SAME (stage, access, layout, aspect) shape
                // `vb_viewt`/`sdf_forward_march` already declare for this image.
                g.image_access(
                    vb_depth,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                    SubRange::DEPTH,
                );
            } else {
                // The FINE level this pass reduces from — mip `d - 1`, written by pass `p - 1`.
                // One mip, so the derived barrier is a RAW flush over that mip ALONE, leaving the
                // rest of the chain untouched.
                g.image_access(
                    hzb_pyramid,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_GENERAL,
                    hzb_mips(d - 1, 1),
                );
            }
            g.image_access(
                hzb_pyramid,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                hzb_mips(d, n),
            );
            *slot = Some(pass);
        }
    }

    (hzb_poison, hzb_build)
}

impl crate::framegraph::BarrierSink for VbBarrierSink<'_> {
    fn image_barriers(&mut self, src_stage: u32, dst_stage: u32, group: &[crate::framegraph::ImgBarrier]) {
        debug_assert!(
            group.len() <= crate::framegraph::MAX_PASS_BARRIERS,
            "image barrier group ({}) exceeds MAX_PASS_BARRIERS",
            group.len()
        );
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
        // `record_vb` recording). Every `arr[i].image` was resolved from the
        // `images[res.index()]` slot (a live VB target for the current frame); `res.index()` is
        // in `0..VB_IMAGE_COUNT` for every image barrier this small graph derives. `arr[..n]`
        // (a stack array) outlives the call; the count == `n`. No memory or buffer barriers,
        // matching [`ForwardBarrierSink::image_barriers`].
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

    fn buffer_barriers(&mut self, src_stage: u32, dst_stage: u32, group: &[crate::framegraph::BufBarrier]) {
        debug_assert!(
            group.len() <= crate::framegraph::MAX_PASS_BARRIERS,
            "buffer barrier group ({}) exceeds MAX_PASS_BARRIERS",
            group.len()
        );
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
                        buffer: self.buffers[b.res.index() - VB_IMAGE_COUNT],
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
        // `record_vb` recording). Every `arr[i].buffer` was resolved from the
        // `buffers[res.index() - VB_IMAGE_COUNT]` slot (a live scene buffer for this frame); a
        // buffer barrier's `res.index()` is always `>= VB_IMAGE_COUNT` and `< VB_IMAGE_COUNT +
        // buffers.len()`. `arr[..n]` outlives the call; the count == `n`.
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
    /// Multi-paradigm render-path plan, rung R8 (the `lit`-producer branch widened at VB-P2
    /// classification plan rung P2c): (re)declares the VisibilityBuffer v1 frame graph into
    /// `self.frame_graph` — mirrors [`Self::declare_forward_graph`]'s shape, trimmed to VB v1's
    /// scope cut (see [`VbPassPlan`]'s doc). The classify chain
    /// (`fill`/`count`/`scan`/`scatter`) and the `vb_shade` vs `vb_resolve` `lit`-producer choice
    /// both key off `scene.vb_use_classified` (plan P1-4); exactly one of `vb_shade`/`vb_resolve`
    /// is ever `Some`. Stores the result in [`Self::vb_pass_plan`]; [`Self::record_vb`] then
    /// drives each pass's derived barriers through it. Called ONLY by
    /// [`Self::declare_frame_graph`]'s `VisibilityBuffer` arm.
    ///
    /// # The pass chain, and the ONE predicate that picks between its two shapes
    ///
    /// VG R3 piece 2 step P2-5 (docs/VG-R3-P2-CAPABILITY-SPLIT-PLAN.md, decisions D4/D6): the
    /// `[hzb_poison, hzb_build_*]` block and the raster scopes are ordered by
    /// [`GBufferScene::path_vb_occlusion_split`](super::scene_types::GBufferScene::path_vb_occlusion_split)
    /// alone, read ONCE into a local so this declarator and `record_vb` cannot disagree about
    /// which shape the frame has:
    ///
    /// * **unsplit** (every scene in the tree today) — `light_upload? → light_cull? → csm? →
    ///   atlas? → vb_sky → vb_indirect_upload? → vb_batch_cull? → vb_raster → (classify?) →
    ///   vb_shade | vb_resolve → hzb_poison? → hzb_build_*? → (split arm) → … → present_sample →
    ///   hzb_dump?`, i.e. byte-for-byte the chain that shipped before the split existed;
    /// * **armed split** — `… → vb_indirect_upload? → vb_indirect_late_upload → vb_batch_cull? →
    ///   vb_cull_readback? → vb_raster → hzb_poison? → hzb_build_*? → vb_cull_late →
    ///   vb_raster_late → vb_cull_readback_late? → (classify?) → vb_shade | vb_resolve → …`
    ///   (VG R3 piece 3 step P3-3 inserted the two `vb_cull_*late` passes; the four asserts beside
    ///   the plan construction below pin their neighbours).
    ///
    /// The block moves EARLIER, never later, so the `hzb_poison < hzb_build_* < hzb_dump` chain of
    /// asserts below is preserved (the dump is declared last in the whole graph). The reorder lands
    /// in the one step where it is provably neutral — while the late scope still draws nothing —
    /// rather than in the step that arms a decision.
    pub(crate) fn declare_vb_graph(&mut self, scene: &GBufferScene<'_>) {
        use crate::framegraph::{ResSync, SubRange};

        const FRAG: u32 =
            VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT;

        self.gbuffer_pass_plan = None;
        self.forward_pass_plan = None;
        let g = &mut self.frame_graph;
        g.reset();

        // --- Images (FIXED local ResId order — see `VbBarrierSink::images`'s doc). `lit` is a
        // FRESH `add_image` (undefined first touch) — VB's OWN producer access
        // (`ColorAttachmentWrite` via `vb_sky`, then `ShaderWrite` via `vb_resolve`), never the
        // Deferred/Forward access this same physical image carries on another boot (C5: every
        // path is boot-mutually-exclusive).
        //
        // Code review (P1-1): the seed stage MUST track the path's actual SHADING-PASS stage,
        // not be copy-pasted from a sibling path's declarator. `declare_forward_graph` seeds
        // `cascade`/`atlas`/`light_table` at FRAGMENT_SHADER because `forward_opaque` (the
        // reader) is a raster FRAGMENT shader; VB's shading consumer is `vb_resolve`, a COMPUTE
        // pass — seeding at FRAGMENT_SHADER here would under-order a dirty-frame re-write against
        // the SIBLING in-flight frame's still-pipelined COMPUTE read (a real WAR hazard: torn
        // shadows/lights under a dynamically-changing light/CSM/atlas frame), the SAME class of
        // bug `declare_deferred_graph`'s own compute-seeded `cascade`/`atlas`/`light_table`
        // avoid (its `resolve` reader is ALSO compute — graph_bridge.rs's deferred declarator,
        // the precedent this fn now matches instead of Forward's).
        let lit = g.add_image("lit");
        let vb_id = g.add_image("vb_id");
        let vb_depth = g.add_image("vb_depth");
        let cascade = g.add_image_seeded(
            "cascade",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let atlas = g.add_image_seeded(
            "atlas",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        // TAA-under-VB (appended AFTER `atlas` so ResIds 0..4 stay byte-unchanged): `viewt` is a
        // FRESH `add_image` — when armed, the `vb_viewt` pass is its sole, every-frame first-touch
        // producer (UNDEFINED→GENERAL write), exactly like the G-buffer ring images. The
        // `taa_hist`/`taa_hist_read` CROSS-FRAME parity pair copies the deferred declarator's
        // shapes VERBATIM (frame `fi` WRITES `pool[fi]`, READS `pool[fi^1]`):
        //   * `taa_hist` (write ResId) — `seeded_readers_at_layout` (WAR: order frame N's write
        //     after the sibling's still-pipelined read of the same physical image);
        //   * `taa_hist_read` (read-sibling ResId) — `seeded_writer_at_layout`
        //     (content-preserving RAW: order + make visible frame N-1's write).
        // Both seeds are COMPUTE (the TAA resolve is a compute pass — the P1-1 seed-stage rule
        // above). With TAA off NO pass names these three ResIds ⇒ the graph routes ZERO barriers
        // on them ⇒ byte-identical (the deferred "declared ahead of its first consumer"
        // discipline).
        let viewt = g.add_image("viewt");
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
        // Rung R9b (appended AFTER `taa_hist_read` so ResIds 0..7 stay byte-unchanged — the
        // Track-A append discipline):
        //   * `thin_normal` — plain `add_image`: `vb_geo` is its sole, every-frame first-touch
        //     producer under split (UNDEFINED→GENERAL); never read without it.
        //   * `ssao` — `add_image_seeded(ResSync::undefined())`, the DEFERRED declarator's own
        //     `ssao` seed VERBATIM and for the same reason: `vb_shade_split`'s gSsao read is
        //     UNCONDITIONAL under split (the 09600 stable-descriptor discipline), so a
        //     split-without-SSAO frame (DDGI-only at R9c, Temporal-only at R9d) needs the
        //     discard-legal UNDEFINED→GENERAL first-touch the seed licenses; with SSAO armed the
        //     gather's write is the first touch and the seed is inert.
        //   * `ssao_ring_a`/`ssao_ring_b` — plain `add_image` (the à-trous interior ping-pong;
        //     never read without the chain).
        // With the split off NO pass names these four ⇒ zero barriers ⇒ every existing pin
        // byte-identical by construction.
        let thin_normal = g.add_image("thin_normal");
        let ssao_img = g.add_image_seeded("ssao", ResSync::undefined());
        let ssao_ring_a = g.add_image("ssao_ring_a");
        let ssao_ring_b = g.add_image("ssao_ring_b");
        // Rung R9c (ResIds 12/13): the DDGI probe atlases — the DEFERRED declarator's seeds
        // VERBATIM (persistent round-robin accumulators living in SHADER_READ_ONLY_OPTIMAL
        // between updates; a CONTENT-PRESERVING SRO→GENERAL transition — an UNDEFINED oldLayout
        // would license discarding the un-updated tiles).
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

        // Rung R9d (ResIds 14..19): the VB hardware shadow chain's own image tail, appended AFTER
        // `ddgi_depth` in the SAME DEFERRED order/seeds — see `declare_deferred_graph`'s own
        // `shadow_vis`/`shadow_vis2`/`motion_vec`/`shadow_temporal_hist`/`temporal_out`/
        // `shadow_temporal_hist_read` declarations (this module's doc) for the full seed
        // rationale; VB reuses the identical shapes verbatim against its OWN local ResId space.
        // NO pass names these ResIds unless `path_vb_hwrt_shadow()` arms this frame, so the OFF
        // path (every current pin) routes ZERO barriers here — byte-identical.
        #[cfg(feature = "hwrt")]
        let shadow_vis = g.add_image("shadow_vis"); // ResId 14
        #[cfg(feature = "hwrt")]
        let shadow_vis2 = g.add_image("shadow_vis2"); // ResId 15
        #[cfg(feature = "hwrt")]
        let motion_vec = g.add_image("motion_vec"); // ResId 16
        #[cfg(feature = "hwrt")]
        let shadow_temporal_hist = g.add_image_seeded(
            "shadow_temporal_hist",
            ResSync::seeded_readers_at_layout(
                VK_IMAGE_LAYOUT_GENERAL,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            ),
        ); // ResId 17
        #[cfg(feature = "hwrt")]
        let temporal_out = g.add_image("temporal_out"); // ResId 18
        #[cfg(feature = "hwrt")]
        let shadow_temporal_hist_read = g.add_image_seeded(
            "shadow_temporal_hist_read",
            ResSync::seeded_writer_at_layout(
                VK_IMAGE_LAYOUT_GENERAL,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            ),
        ); // ResId 19

        // VG R3 piece 1 step P1-5 (docs/VG-R3-P1-PYRAMID-PLAN.md): the hierarchical-Z depth
        // pyramid, appended LAST in BOTH `cfg` arms — ResId 14 without `hwrt`, 20 with it — so
        // every ResId above is byte-unchanged (the Track-A append discipline every addition in
        // this block follows).
        //
        // DECLARED UNCONDITIONALLY, armed or not, exactly like `viewt`/`taa_hist` above: with the
        // pyramid disarmed NO pass names this ResId, so the graph routes ZERO barriers on it and
        // an unarmed boot's recorded command stream is byte-identical. The sink's slot then holds
        // `VkImage::NULL` and is never read (`VbBarrierSink::images`'s own NULL-when-ungated rule).
        //
        // ⚠️ THE DECLARED MIP COUNT. `image_access` range-checks every span against this number IN
        // RELEASE, so it is not a formality: the live chain has `plan.levels` mips, and a DISARMED
        // boot has no image at all. `1` is the disarmed value — the minimum `add_image_mipped`
        // accepts (`0` panics: a zero-mip resource owns no sync entries and the NEXT resource
        // would alias its `state_base`) — and it is sound precisely because no access names the
        // ResId then. When the image DOES exist the declared shape and the real image agree
        // EXACTLY: `HzbTargets::build` creates it with `mip_levels: plan.levels` from the same
        // `scene.hzb` this line reads.
        //
        // ⚠️ THE SEED, which is a decision and not a default. VG R3 piece 3 step P3-0
        // (`docs/VG-R3-P3-CULL-INTEGRATION-PLAN.md`, D2). The pyramid is NON-RINGED (one image
        // shared by both frames in flight) and is written EVERY armed frame, so the cross-frame
        // question is real, and piece 3 gives it a READER.
        //
        // ⚠️ WHAT THIS COMMENT SAID UNTIL P3-0, AND WHY IT WAS WRONG. It prescribed
        // `seeded_readers_at_layout(GENERAL, COMPUTE, SHADER_READ)` for the moment a reader
        // appears. That is the WRONG HALF. A seed describes the state a frame ENDS in, and piece 3
        // produces TWO different endings:
        //
        //   * ARMED-SPLIT frame — the pyramid's last access is `vb_cull_late`'s COMPUTE
        //     `SHADER_READ` (or, on a dump frame, `hzb_dump`'s TRANSFER_READ). Next frame's first
        //     WRITE must WAR against it.
        //   * HZB-ARMED, SPLIT OFF (today's `[vb_mesh_hzb]` pin, the ONLY committed pin that
        //     builds a pyramid) — the last access is `hzb_build_{n-1}`'s COMPUTE `SHADER_WRITE`.
        //     Next frame's first READ must RAW against it.
        //
        // A READER seed makes the first case right and the second SILENTLY wrong:
        // `ResSync::seeded_readers_at_layout`'s sibling doc names that failure in the engine's own
        // words — *"the reader WAR seed would leave the read FREE/already-visible, which is
        // exactly the race"*. A WRITER seed makes the second case exactly right and the first
        // merely CONSERVATIVE (next frame's first write derives a WAW where a WAR would have
        // sufficed). Only the writer form is conservative for BOTH residuals, and both residuals
        // are reachable in the shipped pin set. This is `shadow_temporal_hist_read`'s shape,
        // already used above for the same cross-frame reason.
        //
        // ⚠️ AND `ResSync::undefined()` becomes actively WRONG the moment a reader exists: a first
        // touch derives `oldLayout = UNDEFINED`, which LICENSES THE DRIVER TO DISCARD the image
        // contents. Frame N+1 would read an image the graph just told the driver it may throw
        // away — content- and motion-dependent, i.e. verbatim the engine's recorded "wrong only in
        // motion, stable when stopped" fingerprint.
        //
        // ⚠️ WHAT MAKES THE `GENERAL` CLAIM TRUE. `HzbTargets::build` only ALLOCATED — no encoder,
        // no barrier, no submit — so until P3-0 the framegraph's first touch WAS the pyramid's
        // only layout producer. P3-0 added `HzbTargets::boot_clear_hzb_pyramid`, a real encoder +
        // submit + fence wait that clears every mip to `0.0` and lands the image in `GENERAL`
        // before any frame is recorded — once per targets generation, i.e. after every resize,
        // not once at boot. Without it this seed would emit `oldLayout = GENERAL` against an image
        // genuinely in `UNDEFINED` (`VUID-VkImageMemoryBarrier-oldLayout-01197`) for the life of
        // the generation. The seed and the clear are ONE change; neither is sound alone.
        //
        // ⚠️ ONE RESIDUAL, STATED RATHER THAN LEFT IMPLICIT. On a DUMP frame the pyramid's last
        // access is `hzb_dump`'s TRANSFER_READ, and no derived `srcStageMask` can ever name
        // TRANSFER for the next frame's first write — `seeded_writer_at_layout` sets
        // `visible_stages = 0` and the last-writer branch in `sync::transition` sources from
        // there. That one-frame exposure is on a diagnostic path and is strictly IMPROVED by this
        // seed: the previous `undefined()` gave `TOP_OF_PIPE` *plus* a licensed content discard.
        // A both-halves seed would close it and is deliberately NOT taken — the two-residual
        // argument selects the writer-only form and a hybrid seed has no second consumer.
        //
        // ⚠️ THE PREMISE, named so it can be re-derived when it stops holding: this cross-frame
        // ordering rides on SUBMISSION ORDER (ONE queue, ONE queue family — no async compute
        // exists) and on every one of these barriers being recorded OUTSIDE a render-pass
        // instance, which is what makes a `vkCmdPipelineBarrier`'s first synchronization scope
        // reach the PREVIOUS frame's commands. The day async compute lands, submission order stops
        // being a total order and the pyramid must be re-examined or ringed.
        let hzb_pyramid = g.add_image_mipped(
            "hzb_pyramid",
            scene.hzb.map_or(1, |p| p.levels),
            ResSync::seeded_writer_at_layout(
                VK_IMAGE_LAYOUT_GENERAL,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            ),
        );
        // ONE assert rather than the two `cfg`-split ones this step replaced: the pyramid is the
        // last image in BOTH arms, so `ddgi_depth`'s and `shadow_temporal_hist_read`'s former
        // invariants collapse into this single statement.
        debug_assert_eq!(
            hzb_pyramid.index() + 1,
            VB_IMAGE_COUNT,
            "invariant: declare_vb_graph's image declarations must match VB_IMAGE_COUNT"
        );

        // --- Buffers (light_table=0, vb_instance_ring=1, gclassify=2). `light_table`'s seed
        // stage is COMPUTE for the SAME P1-1 reason as `cascade`/`atlas` above (`vb_resolve` is
        // its reader, not a fragment shader). `vb_instance_ring`/`gclassify` are both
        // frame-private (a sibling in-flight frame touches a DIFFERENT ring slot), so both START
        // `undefined()` — no cross-frame WAR to seed against.
        //
        // VG R3 P2-8 — WHY THE RING IS `add_buffer_seeded(.., undefined())` AND `gclassify` IS A
        // BARE `add_buffer`. The two spellings now declare PROVENANCE (`graph.rs::add_buffer`'s
        // contract), and the two resources differ on exactly that: VB v1 has NO `interp` pass (the
        // V1 scope cut, `VbPassPlan`'s doc), so `vb_instance_ring` is HOST-CPU-scattered into
        // host-coherent memory and every one of its declared accesses in this graph is a READ —
        // it has no in-graph producer by design, and the submit's host-write → device-domain
        // dependency is what orders it. `gclassify`'s own FIRST producer this frame IS in the
        // graph: the `vb_classify_fill` pass's TRANSFER_WRITE (VB-P2 classification plan rung
        // P2b), declared under the same predicate as every reader of it. The seed VALUE is
        // `undefined()`, i.e. what the bare declarator already used, so the derived stream —
        // including the ring's `TOP_OF_PIPE → COMPUTE` first touch at the cull — is unmoved.
        let light_table = g.add_buffer_seeded(
            "light_table",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let vb_instance_ring = g.add_buffer_seeded("vb_instance_ring", ResSync::undefined());
        let gclassify = g.add_buffer("gclassify");
        // Rung R9c (buffer ResIds 3/4): the DDGI classification + Fibonacci ray table — the
        // DEFERRED declarator's WAR seeds verbatim (single device-local instances; a cross-frame
        // re-touch orders after the sibling frame's still-pipelined update reads).
        let ddgi_classification = g.add_buffer_seeded(
            "ddgi_classification",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let ddgi_ray_table = g.add_buffer_seeded(
            "ddgi_ray_table",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        // Rung R9d (buffer ResId 5): the compute-written `VkAccelerationStructureInstanceKHR[]`
        // array the VB hwrt shadow chain's TLAS pack/build passes touch — the DEFERRED
        // declarator's `tlas_instances` shape verbatim (frame-private, undefined seed).
        #[cfg(feature = "hwrt")]
        let tlas_instances = g.add_buffer("tlas_instances");
        // VB-P1a/P1b: the L1 froxel cull trio. These are NOT frame-private — they are SINGLE
        // device-local instances shared across frames in flight (no per-FIF ring; only the
        // descriptor set is per-FIF), exactly like the `grid`/`index`/`alloc` trio
        // `declare_deferred_graph` and `declare_forward_graph` declare. VB-P1a could declare them
        // undefined because the arm was hardcoded OFF (no pass named them); VB-P1b ARMED the cull,
        // so they must carry the same cross-frame seeds their siblings do, or a dirty-frame re-cull
        // races the sibling in-flight frame's still-pipelined read (the engine's cross-frame WAR
        // fingerprint). `cluster_grid`/`light_index` end their frame consumed by
        // `vb_resolve`/`vb_shade`'s COMPUTE read (VB's reader stage — Deferred's, not Forward's
        // FRAGMENT); `light_index_alloc` ends on the cull's atomic WRITES with no draining read
        // (writer seed). Declaration ORDER is unchanged (grid, index, alloc) so the
        // `VbBarrierSink` ResId slots are untouched.
        let cluster_grid = g.add_buffer_seeded(
            "cluster_grid",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let light_index = g.add_buffer_seeded(
            "light_index",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let light_index_alloc = g.add_buffer_seeded(
            "light_index_alloc",
            ResSync::seeded_writer(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT),
        );
        // Rung R2a': the indirect-draw record buffer. Declared LAST so every existing sink slot
        // keeps its index -- appending is the one edit shape that cannot silently re-key a barrier.
        // Frame-private (per-FIF), so `add_buffer` (undefined seed) exactly like `vb_instance_ring`:
        // a sibling in-flight frame touches a DIFFERENT slot, so there is no cross-frame WAR to
        // seed against, and this frame's own TRANSFER write is its first touch.
        let vb_indirect = g.add_buffer("vb_indirect");
        // Rung R2c0: the batch-cull's three buffers, appended after `vb_indirect` for the SAME
        // reason it was appended after the froxel trio. All three are frame-private (per-FIF), so
        // `add_buffer`'s undefined seed is right: this frame's own TRANSFER/COMPUTE write is the
        // first touch of its own slot.
        let vb_batch_desc = g.add_buffer("vb_batch_desc");
        let vb_cull_visible = g.add_buffer("vb_cull_visible");
        let vb_cull_count = g.add_buffer("vb_cull_count");
        // Rung R2d-3: the per-INSTANCE survivor list the cull now WRITES. Appended LAST, the same
        // edit shape every addition above used — no existing sink slot moves. Frame-private
        // (per-FIF) like the trio above, so `add_buffer`'s undefined seed is right: this frame's
        // own COMPUTE write is the first touch of its own slot.
        //
        // The per-mesh bounds buffer (`gMeshBounds`, bound at `vb_cull_layout` @5) gets NO ResId,
        // and that asymmetry is deliberate rather than an omission: it is the same class as
        // `gMeshMeta[]` — a single host-coherent allocation written at mesh REGISTRATION, read by
        // the GPU, never written by it — so there is no producer for the graph to order a reader
        // against, and `gMeshMeta` (bound on every VB shade/resolve pass) carries none either.
        let vb_visible_instance = g.add_buffer("vb_visible_instance");
        // VG R3 piece 2 step P2-3 (docs/VG-R3-P2-CAPABILITY-SPLIT-PLAN.md, decision D4): the
        // occlusion split's LATE indirect record array. Appended LAST — the same edit shape every
        // addition above used, and the only one that cannot re-key an existing barrier.
        // Frame-private (per-FIF) like `vb_indirect` itself, so `add_buffer`'s undefined seed is
        // right: this frame's own write is the first touch of its own slot.
        //
        // NAMED BY NO PASS unless the occlusion split arms this frame — the `hzb_pyramid` shape
        // one screen up. A resource no access names routes ZERO barriers, so every UNSPLIT frame's
        // recorded command stream is byte-identical to the one that shipped before the split
        // existed. Step P2-5 added `vb_indirect_late_upload` (the TRANSFER write) and
        // `vb_raster_late` (the DRAW_INDIRECT read) TOGETHER, in one commit, because a declared
        // indirect read with no declared writer derives `(TOP_OF_PIPE, 0)` — a missing barrier, not
        // a wasted one, and at that time `graph.rs`'s unwritten-read backstop waved buffers through
        // by construction. Since VG R3 P2-8 it does not: the BARE `add_buffer` here is now the
        // declaration that this graph produces the buffer, and separating the two halves again
        // fires `compile`'s `debug_assert!` instead of shipping silently.
        let vb_indirect_late = g.add_buffer("vb_indirect_late");
        // VG R3 piece 3 step P3-3 (docs/VG-R3-P3-CULL-INTEGRATION-PLAN.md, decisions D3/D6): the
        // occlusion split's three buffers, appended LAST in that order — the same edit shape every
        // addition above used, and the only one that cannot re-key an existing barrier.
        //
        // ALL THREE ARE BARE `add_buffer`, never `add_buffer_seeded`, and after P2-8 that spelling
        // IS the provenance claim: each has an IN-GRAPH producer on every frame it is read.
        //   * `vb_late_visible` — the early phase's candidate list and the late phase's survivor
        //     list, one region per batch (INVARIANT VG-P3-LATE-REGION: batch `b` owns
        //     `[base_instance_b, base_instance_b + instance_count_b)` and writes nowhere else — the
        //     SAME host-established disjointness `vb_visible_instance` has, from the same
        //     `VbBatchDesc` fields). First touch is `vb_batch_cull`'s COMPUTE write; since step
        //     P3-6 its LAST reader on a split frame is `vb_raster_late`'s VERTEX fetch, which is
        //     the RAW that orders the late compaction against the draws that index through it.
        //   * `vb_late_count` — per-batch `n_defer` plus ONE reserved tail slot for the frame index
        //     the GPU observed. Its first touch is ALSO `vb_batch_cull`'s COMPUTE write, which is
        //     what makes the P2-8 provenance guard LIVE on it: deleting that declaration turns
        //     `vb_cull_late`'s read below into a first-touch read of a bare `add_buffer` and fires
        //     `compile`'s `debug_assert!` in every dev-profile run — and every golden run is one.
        //     It is the ONE new buffer in this piece the guard can protect (`vb_indirect_late`'s
        //     first touch is the upload's TRANSFER write, and a write is never tested).
        //   * `vb_cull_uniform` — the cull's non-push inputs. Its producer is the
        //     `vkCmdUpdateBuffer` declared INSIDE `vb_batch_cull` as a TRANSFER write, the same
        //     intra-pass shape `vb_cull_count`'s own fill uses.
        //
        // NAMED BY NO PASS on an unsplit frame — except `vb_cull_uniform`, whose pair is declared on
        // every frame the cull runs (D6: the shader's `level >= levels ⇒ Keep` early-out reads
        // `levels` out of this buffer, so a gated fill would leave that read on unwritten allocation
        // contents on a disarmed boot, with `robustBufferAccess` off).
        let vb_late_visible = g.add_buffer("vb_late_visible");
        let vb_late_count = g.add_buffer("vb_late_count");
        let vb_cull_uniform = g.add_buffer("vb_cull_uniform");
        // The buffer-side sibling of the `hzb_pyramid` assert above, and the buffer side is where
        // it matters more: a mis-keyed buffer barrier names a LIVE WRONG buffer (every sink slot
        // resolves to a real handle) instead of faulting on a NULL. `- VB_IMAGE_COUNT` is exactly
        // the mapping `VbBarrierSink::buffer_barriers` performs.
        debug_assert_eq!(
            vb_cull_uniform.index() + 1 - VB_IMAGE_COUNT,
            VB_BUFFER_COUNT,
            "invariant: vb_cull_uniform is the LAST buffer ResId declare_vb_graph declares, and \
             the buffer ResIds must exactly fill VbBarrierSink::buffers"
        );

        // Pass `light_upload` (async light-table re-upload) — the SAME gate
        // `declare_forward_graph`'s own `light_upload` pass uses.
        let light_upload = if scene.light_dirty && scene.light_upload_bytes > 0 {
            let p = g.add_pass("light_upload");
            g.buffer_access(light_table, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT);
            Some(p)
        } else {
            None
        };

        // Pass `light_cull` (L1 clustered froxel cull) — VB-P1a ("dark infra"). Gated EXACTLY as
        // `declare_forward_graph`'s own `light_cull` pass (the "4-buffers-Some" predicate: the
        // cull pipeline AND all three cluster buffers are `Some`) — VB-ONLY, no separate path
        // check needed (this declarator IS the VB one). ⚠️ Default-OFF, not hardcoded off — the
        // arm is the owner's `LightingConfig::clusters_enabled` (default `false`), so on an
        // UNARMED boot `scene.cluster_cull` is `None` ⇒ this is `None` ⇒ zero declared accesses ⇒
        // byte-identical. That is what the 0%-gate rests on, and it is unaffected by the armed
        // boots existing (`vb_mesh_froxel`, golden-pinned).
        // Resets `light_index_alloc` (transfer), reads the light table, writes
        // `cluster_grid`/`light_index` — byte-for-byte the SAME access shape
        // `ForwardPassPlan::light_cull`'s declaration site uses.
        let light_cull = if scene.cluster_cull.is_some()
            && scene.cluster_grid.is_some()
            && scene.light_index.is_some()
            && scene.light_index_alloc.is_some()
        {
            const RW: u32 = VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT;
            let p = g.add_pass("light_cull");
            g.buffer_access(
                light_index_alloc,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_ACCESS_TRANSFER_WRITE_BIT,
            );
            g.buffer_access(light_index_alloc, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW);
            g.buffer_access(
                light_table,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
            g.buffer_access(
                cluster_grid,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            );
            g.buffer_access(
                light_index,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            );
            Some(p)
        } else {
            None
        };

        // Pass `csm` (cascade depth) — gated `scene.csm.is_some()`, declared BEFORE `vb_resolve`
        // (which samples the cascade inline). FULL `MAX_CASCADES` array, the SAME 09600
        // whole-view shape `declare_forward_graph`'s `csm` pass declares.
        let csm = if scene.csm.is_some() {
            let p = g.add_pass("csm_depth");
            g.image_access(
                cascade,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::depth_layers(MAX_CASCADES as u32),
            );
            Some(p)
        } else {
            None
        };

        // Pass `atlas` (spot/point atlas depth) — gated `scene.atlas_punctual.is_some()`. Full
        // `MAX_TEXTURE_LAYERS` array, the SAME shape the other declarators use.
        let atlas_pass = if scene.atlas_punctual.is_some() {
            let p = g.add_pass("atlas_depth");
            g.image_access(
                atlas,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::depth_layers(MAX_TEXTURE_LAYERS as u32),
            );
            Some(p)
        } else {
            None
        };

        // Pass `vb_sky` (always present): writes `lit` (COLOR, first-touch
        // UNDEFINED→COLOR_ATTACHMENT_OPTIMAL — Decision 2's C5 per-path producer access).
        let vb_sky = g.add_pass("vb_sky");
        g.image_access(
            lit,
            VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
            VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            SubRange::COLOR,
        );

        // Passes `vb_raster` + `vb_resolve` — rung R10: `mesh_leg`-gated as a PAIR (`vb_resolve`
        // re-fetches through the Decision-0 geometry table `vb_raster` populates `vb_id` for). A
        // `VisibilityBuffer × Sdf` (mesh-less) frame skips BOTH — the table carries no slot with
        // no registered mesh, so `vb_sky` + `sdf_forward_march` become the sole `lit` producers
        // (see `sdf_forward_march` below). `record_vb` reads the SAME `mesh_leg` predicate, so a
        // `None` here is never recorded (and `vb_id`/`vb_depth` — always `add_image`d above for a
        // fixed ResId order — simply stay untouched, sampled by nothing).
        let mesh_leg = scene.resolved_render_path.mesh_leg;
        // VB-P2 classification plan, rung P2c (the P1-4 owner-decided selector,
        // `GBufferScene::vb_use_classified`'s own doc): the classified-vs-fused `lit`-producer
        // choice, read ONCE here so the classify-chain gate and the `vb_shade`/`vb_resolve`
        // branch below can never disagree (the W1 single-predicate discipline).
        let use_classified = scene.vb_use_classified;
        // Rung R9b: the split DISPLACES the classification chain and the fused lit producer
        // (docs/R9-VB-SPLIT-PLAN.md §0) — but SURGICALLY: `vb_raster` stays gated on bare
        // `mesh_leg` (BOTH arms consume the `vb_id`/`vb_depth` it produces; the split's own
        // producers/consumer are declared AFTER this block, before `sdf_forward_march`).
        let split = scene.path_vb_split();
        // VG rung R2c0: the batch-cull arm, read ONCE so the pass gate, the upload's extra
        // `vb_batch_desc` access and `record_vb`'s own recording gate cannot disagree (the same
        // W1 single-predicate discipline `use_classified` above follows). The R2c0 `Option`s are
        // wired together or not at all by `GpuSceneBundles::scene`, so this is an all-or-nothing
        // arm, never a partial one.
        //
        // VG rung R2d-2 added `vb_mesh_bounds` — spelled VERBATIM as in `record_vb`, and for the
        // reason stated there: it is the one conjunct that is false on a boot without a geometry
        // table, and it is exactly the condition under which `GBufferTargets::sync` leaves
        // `vb_cull_set` `None`. Declaring a cull pass the recorder then skips would strand
        // `vb_indirect`'s declared writer on a COMPUTE that never ran.
        let batch_cull_armed = scene.vb_indirect.is_some()
            && scene.vb_batch_desc.is_some()
            && scene.vb_cull_visible.is_some()
            && scene.vb_cull_count.is_some()
            && scene.vb_mesh_bounds.is_some()
            && scene.vb_batch_cull_pipeline.is_some();
        // VG R3 piece 2 step P2-5 (plan D3/D4/D6): THE single source of "this frame has two raster
        // scopes", read ONCE here because it gates FOUR things that must agree — the
        // `vb_indirect_late_upload` pass, the `vb_raster_late` pass, which of the two slots the
        // `[hzb_poison, hzb_build_*]` block occupies, and (through the same method on the same
        // scene) every one of `record_vb`'s matching gates. Two spellings of one decision is how
        // declare/record parity breaks.
        let occlusion_split = scene.path_vb_occlusion_split();
        // VG R3 piece 2 step P2-5 (plan D6): the poison+build block's two INPUTS, computed once,
        // above BOTH slots. `mesh_leg` is folded in here (never re-derived at a slot) so the block
        // is armed by one expression no matter where it lands — see
        // `declare_hzb_poison_build`'s own doc for why each conjunct is load-bearing.
        let hzb_levels = scene.hzb.filter(|_| mesh_leg).map(|p| p.levels);
        let hzb_dump_armed = scene.hzb_dump.is_some();
        // The moving unit's outputs: ONE binding with TWO possible assignment sites, exactly one of
        // which runs. `mut` locals rather than more elements of the tuple below, because the armed
        // slot is INSIDE that tuple's `if mesh_leg` block and the unsplit one is after it.
        let mut hzb_poison: Option<crate::framegraph::PassId> = None;
        let mut hzb_build: [Option<crate::framegraph::PassId>; crate::compute::MAX_HZB_PASSES] =
            [None; crate::compute::MAX_HZB_PASSES];
        // The split's own two passes, likewise: both are declared inside the `if mesh_leg` block,
        // and `path_vb_occlusion_split()` already carries `mesh_leg`, so a `None` on a mesh-less
        // frame is not a special case — it is the predicate being false.
        let mut vb_indirect_late_upload: Option<crate::framegraph::PassId> = None;
        let mut vb_raster_late: Option<crate::framegraph::PassId> = None;
        // VG R3 piece 3 step P3-3 (plan D8): the LATE cull and the POST-late readback snapshot,
        // both declared inside the same `if mesh_leg` block — `path_vb_occlusion_split()` already
        // carries `mesh_leg`, so a `None` on a mesh-less frame is the predicate being false rather
        // than a special case, exactly as for the two bindings above.
        let mut vb_cull_late: Option<crate::framegraph::PassId> = None;
        let mut vb_cull_readback_late: Option<crate::framegraph::PassId> = None;
        // VG R3 piece 3 step P3-7 (plan D10): the early-depth dump copy, for the same reason — its
        // slot is between the poison+build block's ARMED position and `vb_cull_late`, both of which
        // live inside the `if mesh_leg` block below.
        let mut hzb_dump_depth_early: Option<crate::framegraph::PassId> = None;
        let (
            vb_classify_fill,
            vb_classify_count,
            vb_classify_scan,
            vb_classify_scatter,
            vb_indirect_upload,
            vb_batch_cull,
            vb_cull_readback,
            vb_raster,
            vb_resolve,
            vb_shade,
        ) = if mesh_leg {
            // Pass `vb_raster`: writes `vb_id` (COLOR, first-touch) + `vb_depth` (DEPTH,
            // first-touch, `GREATER`, write ON — Decision 4). Reads `vb_instance_ring` (VERTEX).
            // Declared UNCONDITIONALLY under `mesh_leg` — BOTH the fused (`vb_resolve`) and
            // classified (`vb_shade`) `lit`-producer arms re-fetch geometry through the `vb_id`
            // this pass writes.
            // Rung R2a': the inline `vkCmdUpdateBuffer` that fills this frame's draw records --
            // and, since rung R2c0, the `VbBatchDesc` array the cull reads. Its own pass, so the
            // graph DERIVES the TRANSFER -> COMPUTE / TRANSFER -> DRAW_INDIRECT dependency against
            // the consumers below instead of anyone hand-writing it.
            //
            // ⚠️ This gate and `record_vb`'s must be IDENTICAL, not nested. A frame that declared
            // the cull pass but skipped recording it would leave `vb_indirect`'s last declared
            // writer as the COMPUTE that never ran, and the TRANSFER -> DRAW_INDIRECT dependency
            // the upload actually needs would be derived nowhere -- a missing barrier, not a
            // wasted one. Rung R2a' declared the upload unconditionally under `mesh_leg`; that
            // also put a spurious TRANSFER -> DRAW_INDIRECT barrier on the placeholder buffer of a
            // `vb_indirect: None` (direct-draw) boot, which this narrowing removes.
            let indirect_armed = scene.vb_indirect.is_some();
            let vb_indirect_upload = indirect_armed.then(|| {
                let p = g.add_pass("vb_indirect_upload");
                g.buffer_access(vb_indirect, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT);
                if batch_cull_armed {
                    g.buffer_access(
                        vb_batch_desc,
                        VK_PIPELINE_STAGE_TRANSFER_BIT,
                        VK_ACCESS_TRANSFER_WRITE_BIT,
                    );
                }
                p
            });

            // ==== VG R3 piece 2 step P2-5 (plan D4): the LATE indirect record array's WRITE. ====
            //
            // The mirror of the upload directly above, and it is a SEPARATE pass on purpose: its
            // gate is `path_vb_occlusion_split()` while that one's is `scene.vb_indirect.is_some()`,
            // and folding two different predicates onto one pass is how a pass ends up declaring an
            // access the recorder does not perform (or the reverse).
            //
            // ⚠️ THIS DECLARATION IS THE DIFFERENCE BETWEEN "DRAWS NOTHING" AND "DRAWS WHATEVER WAS
            // IN FRESHLY ALLOCATED DEVICE MEMORY". The fill is a `vkCmdUpdateBuffer` — the buffer is
            // DEVICE_LOCAL | TRANSFER_DST, so it can only be a transfer op — and `vb_raster_late`
            // fetches from it in the SAME command buffer. Omit this half and the graph finds no
            // writer, takes the first-touch arm, and derives `(TOP_OF_PIPE, 0)`: an execution-only
            // edge that makes the update neither available nor visible. That is a MISSING barrier,
            // not a wasted one, and until VG R3 P2-8 nothing in this repository would have reported
            // it — a buffer hazard is invisible to the goldens, to the validation layers, and to
            // `robustBufferAccess` (off on this device), and `graph.rs`'s unwritten-read backstop
            // waved every buffer through by construction (`!is_image || ..`). P2-7 EXECUTED that
            // deletion and measured all four gates GREEN, which is why P2-8 re-cut the backstop on
            // declared PROVENANCE instead of kind: `vb_indirect_late` is a bare `add_buffer`, so
            // deleting this access now trips `compile`'s debug_assert in every dev-profile run —
            // and every golden run is one.
            //
            // ⚠️ AND THE BARRIER COUNT CANNOT SEE IT EITHER. Dropping this access leaves the late
            // scope's boundary at the SAME three barriers, differing only in `src_stage`/
            // `src_access` — which is why gate G4 asserts the derived barrier's FIELDS.
            vb_indirect_late_upload = occlusion_split.then(|| {
                let p = g.add_pass("vb_indirect_late_upload");
                g.buffer_access(
                    vb_indirect_late,
                    VK_PIPELINE_STAGE_TRANSFER_BIT,
                    VK_ACCESS_TRANSFER_WRITE_BIT,
                );
                p
            });

            // Rungs R2c0/R2c: the batch cull. Reads the descriptors, tests each batch's world AABB
            // against the six pushed frustum planes, rewrites that record's `instanceCount`, and
            // atomic-appends survivors into the compacted list. Declared on EVERY VB frame, never
            // behind an opt-in: R2c0 shipped it INERT (the decision was the literal `true`) because
            // the null control `docs/VG-DECIDABILITY-FLOOR.md` demands has to be present in the
            // MEASURED configuration to be a control at all, and the same reasoning keeps the armed
            // version unconditional.
            let vb_batch_cull = batch_cull_armed.then(|| {
                const RW: u32 = VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT;
                let p = g.add_pass("vb_batch_cull");
                // The counter's own `vkCmdFillBuffer` reset and the atomics that follow it, in ONE
                // pass -- the SAME intra-pass TRANSFER -> COMPUTE shape `light_cull` uses for
                // `light_index_alloc`.
                g.buffer_access(vb_cull_count, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT);
                g.buffer_access(vb_cull_count, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW);
                g.buffer_access(vb_batch_desc, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
                g.buffer_access(vb_indirect, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT);
                g.buffer_access(vb_cull_visible, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT);
                // Rung R2d-3: the two per-INSTANCE accesses. `vb_instance_ring` was bound to this
                // pass at rung R2d-2 (`vb_cull_layout` @4) and DECLARED nowhere — its only declared
                // access in this whole graph was `vb_raster`'s VERTEX read below. Declare/record
                // parity is the invariant this file and the recorder both treat as load-bearing:
                // an access the recorder performs but the declarator omits is a barrier derived
                // nowhere, and a buffer hazard is invisible to goldens, to the validation layers
                // and to `robustBufferAccess` (off on this device). The COMPUTE read is declared
                // whether or not `keep` is hardwired this rung — the descriptor is bound and the
                // module may load from it the moment the arming rung edits one expression.
                //
                // TWO CONCRETE BARRIER CONSEQUENCES, recorded so a future reader diffing barrier
                // counts is not surprised. Both are execution-only edges (`src_access` 0) and
                // neither can move a pixel, but the recorded stream is NOT byte-identical here —
                // this is the one place in R2d-3 where it changes:
                //   1. The cull acquires a TOP_OF_PIPE -> COMPUTE edge on the ring, which is that
                //      resource's first touch of the frame.
                //   2. `vb_raster`'s ring read is re-sourced from TOP_OF_PIPE -> VERTEX to
                //      COMPUTE -> VERTEX, because the cull now precedes it as the ring's reader.
                g.buffer_access(vb_instance_ring, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
                g.buffer_access(
                    vb_visible_instance,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_WRITE_BIT,
                );
                // ==== VG R3 piece 3 step P3-3 (plan D6/D8): the cull's UNIFORM, declared on EVERY
                // frame the cull runs — the ONE new access here that is NOT gated on the split. ====
                //
                // A `vkCmdUpdateBuffer` (TRANSFER) followed by the dispatch's read (COMPUTE), both
                // inside THIS pass — verbatim the intra-pass shape the counter fill two screens up
                // uses, and `light_cull`'s `light_index_alloc` before it. No new pass.
                //
                // ⚠️ UNCONDITIONAL, and the reason is a per-field validity question the alternative
                // would have to answer: the module's `level >= levels ⇒ Keep` early-out — the guard
                // that makes the disarmed `hzb_null` load safe — reads `levels` OUT OF THIS BUFFER.
                // Gating the fill on the split would leave that read on unwritten allocation
                // contents on a disarmed boot, and `robustBufferAccess` is off on this device. The
                // fill is 96 bytes in a pass that already records one.
                //
                // ⚠️ AND IT MOVES THE UNSPLIT STREAM. This is the one declaration in P3-3 that a
                // frame WITHOUT the occlusion split derives a barrier from: every VB frame that
                // records the cull gains one `TRANSFER(TRANSFER_WRITE) → COMPUTE(SHADER_READ)`
                // buffer barrier inside this pass. It moved no pixel when it landed — the buffer
                // was read by no shader until P3-4, and the values it carries reach a verdict only
                // under `VB_CULL_OCC_ARMED`, which is clear on every UNSPLIT frame — so every
                // golden pin is byte-identical; what it moves is
                // `tests/vb_barrier_stream_baseline.rs`'s U-rows as well as its S-rows.
                g.buffer_access(
                    vb_cull_uniform,
                    VK_PIPELINE_STAGE_TRANSFER_BIT,
                    VK_ACCESS_TRANSFER_WRITE_BIT,
                );
                g.buffer_access(
                    vb_cull_uniform,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                );
                // ==== VG R3 piece 3 step P3-3 (plan D8): the EARLY phase's three split-only
                // accesses, gated on `path_vb_occlusion_split()` VERBATIM so an unsplit frame's
                // declared set is bit-unchanged. ====
                //
                // The pyramid read is the EARLY predicate's input, and it is the pyramid AS THE
                // PREVIOUS FRAME LEFT IT — this frame's build has not run yet. That cross-frame RAW
                // is what `hzb_pyramid`'s `seeded_writer_at_layout(GENERAL, COMPUTE, SHADER_WRITE)`
                // seed (P3-0, plan D2) exists to order, and `GENERAL` is the layout the image holds
                // for life, so no transition appears.
                //
                // ⚠️ The pyramid access carries the EXTRA `hzb_levels.is_some()` conjunct. Until
                // step P3-6 it was load-bearing: `path_vb_occlusion_split()` did not imply
                // `scene.hzb.is_some()`, so a split-armed HZB-off frame was reachable, and on it
                // the sink's pyramid slot holds `VkImage::NULL` while the declared image has the
                // disarmed 1-mip shape — a barrier naming a NULL image is a VUID, not a wasted
                // edge. P3-6's D9 conjunct makes the state unreachable, so this is now REDUNDANT
                // rather than wrong; it is kept because `levels` has to be unwrapped for
                // `hzb_mips` either way, which makes the `if let` the natural spelling and not a
                // second predicate.
                //
                // The two list writes are declared unconditionally on a split frame while the
                // module performs them only under `VB_CULL_OCC_ARMED` (P3-4's stores are gated on
                // that bit). DECLARED is therefore a superset of PERFORMED, which is the safe
                // direction — a barrier nobody needed, never a write nobody barriered — and it is
                // what makes `vb_late_count`'s first touch a write, the whole of the P2-8 provenance
                // coverage this piece gains, and what keeps `vb_cull_late`'s reads below from being
                // first-touch reads of a bare `add_buffer`. ⚠️ Step P3-6 PAID the containment's
                // other half: `GpuSceneBundles::scene` sets `VB_CULL_OCC_ARMED` by CALLING
                // `path_vb_occlusion_split()` on the assembled scene, so ARMED implies this
                // predicate structurally — one predicate read twice, never two booleans that can
                // drift into a frame storing where nothing declared it.
                if occlusion_split {
                    if let Some(levels) = hzb_levels {
                        g.image_access(
                            hzb_pyramid,
                            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                            VK_ACCESS_SHADER_READ_BIT,
                            VK_IMAGE_LAYOUT_GENERAL,
                            hzb_mips(0, levels),
                        );
                    }
                    g.buffer_access(
                        vb_late_visible,
                        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                        VK_ACCESS_SHADER_WRITE_BIT,
                    );
                    g.buffer_access(
                        vb_late_count,
                        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                        VK_ACCESS_SHADER_WRITE_BIT,
                    );
                }
                p
            });

            // Rung R2c-tail: the readback copy. Its own pass, so the COMPUTE -> TRANSFER
            // dependency against the atomics above is DERIVED rather than hand-written — the same
            // discipline every other seam in this declarator follows. Gated on the probe, so an
            // unarmed boot declares nothing and the nine pins stay byte-identical.
            let vb_cull_readback = (batch_cull_armed && scene.vb_cull_readback.is_some()).then(|| {
                let p = g.add_pass("vb_cull_readback");
                g.buffer_access(vb_cull_count, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_READ_BIT);
                g.buffer_access(vb_cull_visible, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_READ_BIT);
                // Rung R2d-5: the two regions the probe gained. `vb_indirect` carries the POST-cull
                // `instanceCount` the rasterizer fetches — the only place that number is
                // observable, since the descriptors hold the pre-cull one — and
                // `vb_visible_instance` carries the survivor regions the VS indexes through.
                //
                // BOTH are also read LATER in the frame by `vb_raster` (DRAW_INDIRECT and VERTEX
                // respectively), so inserting this TRANSFER read between the cull's COMPUTE write
                // and those readers RE-SOURCES their barriers: instead of one `COMPUTE -> reader`
                // RAW they become `COMPUTE -> TRANSFER` (which makes the write AVAILABLE) followed
                // by a `TRANSFER -> reader` visibility extension. That is sound — an available
                // write stays available and any later visibility operation picks it up, and the
                // two execution dependencies chain — and it is the SAME shape `vb_instance_ring`
                // already takes since rung R2d-3 (see the cull pass's own note above). It happens
                // ONLY under the probe: an unarmed boot declares no pass here at all, which is what
                // keeps the recorded stream, and therefore the golden pins, untouched.
                g.buffer_access(vb_indirect, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_READ_BIT);
                g.buffer_access(
                    vb_visible_instance,
                    VK_PIPELINE_STAGE_TRANSFER_BIT,
                    VK_ACCESS_TRANSFER_READ_BIT,
                );
                // VG R3 piece 3 step P3-3 (plan D8): the PRE-LATE snapshot's two regions, gated on
                // the split as well as on the probe. This pass sits BEFORE `vb_cull_late`, so what
                // it observes is the CANDIDATE list as the early phase wrote it and `n_defer` per
                // batch — and that is the ONLY place the candidate set is observable at all, because
                // the late phase compacts `vb_late_visible` IN PLACE (after compaction the region
                // holds the survivors followed by the original tail, a multiset that is not the
                // candidate set). The POST-late snapshot is `vb_cull_readback_late`, declared after
                // the late raster.
                //
                // ⚠️ Round 1 of the plan assigned these copies to the recorder alone and declared
                // neither — shipping undeclared transfer reads, the P2-7 class. They were declared
                // here one step BEFORE the copies landed (P3-5), which is the safe direction of the
                // declare/record asymmetry: a barrier that was not needed, never a read that was not
                // barriered. The recorder's two copies now carry `if occlusion_split { .. } else { 0 }`
                // on their sizes — this predicate, verbatim — so a copy can never outrun its
                // declaration.
                if occlusion_split {
                    g.buffer_access(
                        vb_late_visible,
                        VK_PIPELINE_STAGE_TRANSFER_BIT,
                        VK_ACCESS_TRANSFER_READ_BIT,
                    );
                    g.buffer_access(
                        vb_late_count,
                        VK_PIPELINE_STAGE_TRANSFER_BIT,
                        VK_ACCESS_TRANSFER_READ_BIT,
                    );
                }
                p
            });

            let vb_raster = g.add_pass("vb_raster");
            // The consumer side of that dependency: the indirect FETCH stage, not a shader read.
            // Since rung R2c0 the producer it resolves against is the CULL's compute write rather
            // than the upload's transfer write -- the same declaration, a different derived
            // source stage, because the graph tracks the last writer rather than a hand-picked one.
            if indirect_armed {
                g.buffer_access(
                    vb_indirect,
                    VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
                    VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
                );
            }
            g.image_access(
                vb_id,
                VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                SubRange::COLOR,
            );
            g.image_access(
                vb_depth,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::DEPTH,
            );
            g.buffer_access(vb_instance_ring, VK_PIPELINE_STAGE_VERTEX_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
            // Rung R2d-4: the VS's read of the per-INSTANCE survivor list (Set-0 @11), which
            // derives the COMPUTE -> VERTEX_SHADER RAW against the cull's region write. Without it
            // the raster could sample the list before the dispatch's stores landed, and it would
            // usually look right: the list is the IDENTITY this rung, so a stale read returns the
            // same numbers a correct one would. That is precisely the hazard a golden cannot see.
            //
            // Gated on `batch_cull_armed`, the SAME predicate the recorder's per-draw indirection
            // bit carries — the `indirect_armed` shape immediately above, for the same reason: on a
            // boot with no cull, nothing WRITES this buffer, so an unconditional declaration would
            // put a spurious TOP_OF_PIPE -> VERTEX edge on a resource no pass ever produces.
            //
            // Note the gate does NOT rest on the VS skipping the load. It may not skip it: DXC is
            // free to lower the `? :` to an eager load plus an `OpSelect`, in which case the
            // fetch happens and its result is DISCARDED when the bit is clear. That is why the
            // shader header carries an in-range argument for the not-taken address rather than
            // an it-never-executes one. What makes the gate sound is the absence of a WRITER to
            // synchronise against, not the absence of a reader.
            //
            // The recorder's predicate additionally carries `i < batch_count`, a per-BATCH term no
            // declarator can see (it is derived from three allocation sizes at record time). That
            // asymmetry is safe in exactly one direction, and this is that direction: it can only
            // make this declaration cover MORE draws than actually read, i.e. a barrier that was
            // not needed — never a read that was not barriered.
            if batch_cull_armed {
                g.buffer_access(
                    vb_visible_instance,
                    VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                );
            }

            // ==== VG R3 piece 2 step P2-5 (plan D6): the `[hzb_poison, hzb_build_*]` block's
            // ARMED-SPLIT slot — between the two raster scopes. ====
            //
            // WHY THE MOVE IS REQUIRED AT ALL: in the target design the late raster must write
            // `vb_id`/`vb_depth` BEFORE `vb_resolve`/`vb_shade` read `vb_id`, or the late geometry
            // is never shaded — and the pyramid the late scope's occlusion test consults must be
            // built from the EARLY depth. That fixes the armed order as
            // `vb_raster → hzb_poison → hzb_build_* → vb_raster_late → classify → lit`.
            //
            // WHY NOW: the only moment the reorder is PROVABLY neutral is while the late scope
            // draws nothing. Deferring it to piece 3 would ship a graph reorder in the same step
            // that arms a decision.
            //
            // WHY ONE PREDICATE PICKS A SLOT rather than an unconditional move: the `vb_viewt`
            // PRE-TAIL/LATE pair below is the same idiom — the accesses are IDENTICAL in both
            // slots, only the position differs — and it is what keeps an UNSPLIT frame deriving a
            // barrier stream bit-identical to the one that shipped.
            if occlusion_split {
                let (poison, build) =
                    declare_hzb_poison_build(g, hzb_levels, hzb_dump_armed, hzb_pyramid, vb_depth);
                hzb_poison = poison;
                hzb_build = build;
            }

            // ==== VG R3 piece 3 step P3-7 (plan D10, gate G-P3-E): the EARLY-DEPTH dump copy. ====
            //
            // ONE access — `vb_depth` at `(TRANSFER, TRANSFER_READ, TRANSFER_SRC_OPTIMAL, DEPTH)`,
            // the SAME shape the end-of-frame `hzb_dump` pass declares on the same image. The
            // pyramid is NOT read here: this pass exists to capture the depth the builds above just
            // reduced, and the pyramid's own copy stays where it was, at frame end, where it
            // observes the FINISHED image.
            //
            // THE POSITION IS THE CLAIM. Declared after the last `hzb_build_*`, so what it copies is
            // exactly what they read; declared before `vb_raster_late`, so nothing has drawn into
            // the depth again. Both neighbours are asserted below rather than left to this
            // function's layout — the discipline `hzb_dump`'s own two order asserts follow.
            //
            // The derived edges are a round trip on `vb_depth`:
            // `hzb_build_0`'s `SHADER_READ_ONLY_OPTIMAL` → `TRANSFER_SRC_OPTIMAL` (here) →
            // `vb_raster_late`'s `DEPTH_ATTACHMENT_OPTIMAL`. Both transitions are CONTENT-PRESERVING
            // and neither may become a first touch: `vb_raster` wrote this image earlier in the same
            // frame, and a first touch on the return leg would license discarding it.
            //
            // ⚠️ The gate is `occlusion_split && hzb_dump_armed` and nothing else.
            // `path_vb_occlusion_split()` already carries `mesh_leg` and `hzb.is_some()`, so a frame
            // that declares this pass always declares `hzb_dump` too (asserted below) — which is
            // what makes "the early region is live" a bit the frame-end header can honestly stamp.
            // On an unsplit dump frame the two depths are the same bytes by construction (one
            // raster scope), so a second copy would cost a barrier and prove nothing.
            hzb_dump_depth_early = (occlusion_split && hzb_dump_armed).then(|| {
                let p = g.add_pass("hzb_dump_depth_early");
                g.image_access(
                    vb_depth,
                    VK_PIPELINE_STAGE_TRANSFER_BIT,
                    VK_ACCESS_TRANSFER_READ_BIT,
                    VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    SubRange::DEPTH,
                );
                p
            });

            // ==== VG R3 piece 3 step P3-3 (plan D4/D5/D8): the LATE cull pass. ====
            //
            // Declared immediately after the last `hzb_build_*` and before `vb_raster_late`, which
            // is the whole of its position argument: it reads the pyramid THIS frame's build wrote,
            // and it writes the `instanceCount` the late raster fetches. Both neighbours are
            // asserted below rather than left to this function's layout.
            //
            // ONE LANE PER BATCH, `batch_count` of them — the SAME shape and the SAME host-computed
            // size as the early dispatch (plan D4). The GPU-only quantity is the per-batch candidate
            // count, which a lane reads from `vb_late_count[i]`; the DOMAIN of the dispatch is
            // batches, so no `vkCmdDispatchIndirect` and no `vkCmdDrawIndexedIndirectCount` is
            // needed — and neither is in this device's fn table.
            //
            // # WHY THE ACCESS LIST IS ASYMMETRIC WITH `vb_batch_cull`'s, and why that is sound
            //
            // `graph_bridge.rs` already records the rule the raster's own survivor read rests on:
            // DXC may lower a not-taken `? :` to an eager LOAD plus an `OpSelect`, so a not-taken
            // READ may still issue. No such licence exists for a STORE — a compiler may not
            // introduce a write the source does not perform. `pc.phase` is a PUSH constant, uniform
            // across the dispatch, so:
            //
            //   * every LOAD either phase can issue is declared on BOTH passes (the ring, the batch
            //     descriptors, the uniform, the pyramid);
            //   * `vb_indirect_late`'s store is declared HERE ONLY, because it sits under
            //     `if (pc.phase == VB_CULL_PHASE_LATE)`;
            //   * `vb_indirect` / `vb_cull_visible` / `vb_cull_count` / `vb_visible_instance` are
            //     declared on `vb_batch_cull` ONLY, because their stores sit under phase 0.
            //
            // ⚠️ `vb_late_visible` IS DECLARED AS TWO CALLS — a READ, then a WRITE — never as one
            // combined `SHADER_READ|SHADER_WRITE`. The P2-8 provenance guard tests `is_write ||
            // res_written`, so a combined access is `is_write` and the READ HALF IS NEVER TESTED.
            // Split like this, the read is a genuine first-touch test: delete `vb_batch_cull`'s
            // gated write above and this read fires `compile`'s `debug_assert!` in every dev-profile
            // run. The cost is stated rather than hidden — the split derives a second, execution-only
            // self-WAR edge on this pass, which is a NEW row in the barrier-stream pin rather than a
            // hidden one. `vb_indirect_late` needs no split: the host upload already latched it, so
            // the extra call would be inert.
            //
            // ⚠️ THE C16 RESIDUAL, CLOSED at step P3-6 and kept named so the closure is not
            // silently undone. This gate is `path_vb_occlusion_split()` and NOT `batch_cull_armed`.
            // On a device WITHOUT `storage_buffer_array_non_uniform_indexing` the split could once
            // arm while the cull was not recorded at all, and then this pass's `vb_late_visible`
            // READ is a first-touch read of a bare `add_buffer` — a dev-profile `debug_assert!`
            // here, and a `.expect()` on an absent `vb_cull_set` in the recorder. D9's
            // `vb_mesh_bounds.is_some()` conjunct makes the state unreachable, and
            // `record_vb`'s `occlusion_split ⇒ batch_cull_armed` assert checks it rather than
            // restating it.
            vb_cull_late = occlusion_split.then(|| {
                let p = g.add_pass("vb_cull_late");
                g.buffer_access(
                    vb_batch_desc,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                );
                g.buffer_access(
                    vb_instance_ring,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                );
                g.buffer_access(
                    vb_cull_uniform,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                );
                // The pyramid THIS frame's build wrote — an intra-frame RAW at `GENERAL` with no
                // layout change. Carries the same `hzb_levels.is_some()` conjunct the early pass's
                // read does, for the same C16 reason.
                if let Some(levels) = hzb_levels {
                    g.image_access(
                        hzb_pyramid,
                        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                        VK_ACCESS_SHADER_READ_BIT,
                        VK_IMAGE_LAYOUT_GENERAL,
                        hzb_mips(0, levels),
                    );
                }
                g.buffer_access(
                    vb_late_count,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                );
                g.buffer_access(
                    vb_late_visible,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                );
                g.buffer_access(
                    vb_late_visible,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_WRITE_BIT,
                );
                // Obligation 1 of piece 2, discharged: `vb_indirect_late`'s declared writer moves
                // from `(TRANSFER, TRANSFER_WRITE)` to `(COMPUTE_SHADER, SHADER_WRITE)`, and the
                // writer that changes is THIS pass — never `vb_batch_cull`, which does not touch the
                // record array at all (plan D3 moved the early phase's `n_defer` into
                // `vb_late_count` so this word keeps exactly one producer).
                g.buffer_access(
                    vb_indirect_late,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_WRITE_BIT,
                );
                p
            });

            // ==== VG R3 piece 2 step P2-5 (plan D4): the LATE raster scope. ====
            //
            // Three accesses, and each one is load-bearing:
            //
            //  * `vb_indirect_late` at DRAW_INDIRECT/INDIRECT_COMMAND_READ — the consumer half of
            //    `vb_indirect_late_upload`'s transfer write, mirroring `vb_raster`'s own indirect
            //    fetch declaration above. Together they derive the TRANSFER → DRAW_INDIRECT edge;
            //    either half alone derives a barrier that is wrong rather than absent.
            //  * `vb_id` as a COLOR write at `COLOR_ATTACHMENT_OPTIMAL` — a WAW against the early
            //    scope's store, at the layout the early scope left it in. ⚠️ NOT `UNDEFINED`: a
            //    first touch here would license DISCARDING what the early scope wrote, which is
            //    exactly the equivalence this piece rests on.
            //  * `vb_depth` as a DEPTH write at `DEPTH_ATTACHMENT_OPTIMAL` — the same WAW, and on
            //    an HZB-armed frame the return half of the round trip
            //    `DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL` (into `hzb_build_0`, just
            //    above) `→ DEPTH_ATTACHMENT_OPTIMAL`. Both are content-preserving, and neither may
            //    become a first touch.
            //
            // ⚠️ VG R3 piece 3 step P3-6 ADDED THE TWO VERTEX READS this block spent piece 2 and
            // steps P3-1..P3-5 explaining the absence of. Until this step every late record carried
            // `instanceCount = 0`, so the scope issued ZERO vertex invocations and declaring a
            // read would have declared an access the recorder does not perform — on a resource
            // whose region no pass wrote that frame, which would have derived a spurious
            // `TOP_OF_PIPE → VERTEX` edge. The count is now the LATE CULL's, so both reads are
            // real:
            //
            //  * `vb_instance_ring` at VERTEX — the same row lookup the early scope declares.
            //  * `vb_late_visible` at VERTEX — and it is the LATE list, NOT `vb_visible_instance`.
            //    The scope binds `vb_set0_late`, which is `vb_set0` with @11 changed, so the VS's
            //    expression `visible_instances[pc.base_instance + instance_id]` reads the late list
            //    at the identical base with a BYTE-UNCHANGED `vb_raster.vs.hlsl`.
            //    `vb_visible_instance` is not bound to this scope at all and is therefore not
            //    declared on it — the "a bound descriptor is declared regardless" rule applied
            //    correctly, not waived.
            //
            // The `vb_late_visible` read is what derives the `vb_cull_late (COMPUTE, SHADER_WRITE)
            // → vb_raster_late (VERTEX, SHADER_READ)` RAW. Without it the raster could sample the
            // survivor prefix before the compaction's stores landed — and on a converged frame
            // where the correct count is zero it would look right every time, which is precisely
            // the hazard no golden can see.
            //
            // ⚠️ Both are declared UNCONDITIONALLY within the split, with no `batch_cull_armed`
            // conjunct like the early scope's @11 read carries. That asymmetry is correct rather
            // than an oversight: `occlusion_split ⇒ batch_cull_armed` since step P3-6's
            // `vb_mesh_bounds` conjunct, so the WRITER this edge synchronises against provably
            // exists here, which is the property the early scope's gate is protecting.
            vb_raster_late = occlusion_split.then(|| {
                let p = g.add_pass("vb_raster_late");
                g.buffer_access(
                    vb_indirect_late,
                    VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
                    VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
                );
                g.buffer_access(
                    vb_instance_ring,
                    VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                );
                g.buffer_access(
                    vb_late_visible,
                    VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                );
                g.image_access(
                    vb_id,
                    VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                    VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                    VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    SubRange::COLOR,
                );
                g.image_access(
                    vb_depth,
                    FRAG,
                    VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                    VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                    SubRange::DEPTH,
                );
                p
            });

            // ==== VG R3 piece 3 step P3-3 (plan D8): the POST-LATE readback snapshot. ====
            //
            // Declared AFTER `vb_raster_late`, and that position is the decision rather than a
            // convenience. Sited BETWEEN `vb_cull_late`'s COMPUTE write and the late raster's
            // DRAW_INDIRECT fetch it would RE-SOURCE that fetch — one `COMPUTE → DRAW_INDIRECT` RAW
            // becoming `COMPUTE → TRANSFER` plus `TRANSFER → DRAW_INDIRECT`, exactly what the
            // `vb_cull_readback` block above documents for `vb_indirect`. Sited here it does not:
            // `vb_raster_late` only READS these three buffers, so the bytes are identical either way
            // and the SHIPPING chain
            // `vb_indirect_late_upload → vb_cull_late → vb_raster_late` is field-identical with and
            // without the probe. The probe still appends one edge, which is why the barrier-stream
            // pin is per configuration (PROBE-OFF is the normative row set) — but what it certifies
            // is the shipping chain, not a perturbed one.
            //
            // Three regions: `vb_late_visible` (the COMPACTED PREFIX — the same bytes the pre-late
            // snapshot read, now overwritten in place by the compaction), `vb_late_count` (again, so
            // the host can assert the late phase did NOT clobber the early phase's `n_defer`), and
            // `vb_indirect_late` (the `n_keep` the late cull wrote, derived INDEPENDENTLY on the
            // host from the candidate list and the dumped pyramid).
            //
            // ⚠️ Like the pre-late pair above, this pass was DECLARED at P3-3 with the recorder
            // emitting only its barriers; the three COPIES landed at P3-5. Declare/record pass
            // parity held in between, which is what made the intermediate tree shippable.
            vb_cull_readback_late =
                (occlusion_split && scene.vb_cull_readback.is_some()).then(|| {
                    let p = g.add_pass("vb_cull_readback_late");
                    g.buffer_access(
                        vb_late_visible,
                        VK_PIPELINE_STAGE_TRANSFER_BIT,
                        VK_ACCESS_TRANSFER_READ_BIT,
                    );
                    g.buffer_access(
                        vb_late_count,
                        VK_PIPELINE_STAGE_TRANSFER_BIT,
                        VK_ACCESS_TRANSFER_READ_BIT,
                    );
                    g.buffer_access(
                        vb_indirect_late,
                        VK_PIPELINE_STAGE_TRANSFER_BIT,
                        VK_ACCESS_TRANSFER_READ_BIT,
                    );
                    p
                });

            // VB-P2 classification plan (docs/VB-P2-CLASSIFICATION-PLAN.md), rung P2c: the
            // classify chain `fill -> count -> scan -> scatter`, declared BEFORE the `lit`
            // producer (below) — gated on `use_classified` (P1-4: a `!use_classified` frame pays
            // ZERO classify tax, the chain is not even declared, not merely unread as rung P2b
            // left it). Every pass's `gclassify` access is a plain `buffer_access` on the SAME
            // single ResId; the framegraph's `transition` (sync.rs) fires a RAW/WAW barrier
            // whenever the prior pass left a pending write, so `fill`(TRANSFER_WRITE) ->
            // `count`(RW) -> `scan`(RW) -> `scatter`(RW) auto-chains a conservative whole-buffer
            // barrier between every consecutive pair (P1-3, verified by construction — no manual
            // barrier / split ResId needed). Populates `gclassify` for `vb_shade` (below) to
            // consume. Rung R9b: `!split` — the split arm consults neither the classify chain
            // nor `use_classified` (§0's displacement rule).
            let (vb_classify_fill, vb_classify_count, vb_classify_scan, vb_classify_scatter) =
                if use_classified && scene.path_vb_fused() {
                    let vb_classify_fill = g.add_pass("vb_classify_fill");
                    g.buffer_access(
                        gclassify,
                        VK_PIPELINE_STAGE_TRANSFER_BIT,
                        VK_ACCESS_TRANSFER_WRITE_BIT,
                    );

                    // Pass `vb_classify_count`: reads `vb_id` (COMPUTE, SHADER_READ_ONLY_OPTIMAL
                    // — the SAME image the `lit` producer reads below; this pass runs FIRST so it
                    // derives the COLOR_ATTACHMENT_OPTIMAL->SHADER_READ_ONLY_OPTIMAL barrier, and
                    // every later same-layout read needs none) and RW `gclassify`
                    // (`InterlockedAdd(counts[mat], 1)` per non-sentinel pixel).
                    // `instance_materials`/`Camera` are NOT tracked as separate ResIds in this
                    // graph (mirrors the `lit` producer's own arm below, which never declares an
                    // access for them either — a host-fenced ring, not a framegraph-tracked
                    // resource).
                    let vb_classify_count = g.add_pass("vb_classify_count");
                    g.buffer_access(
                        gclassify,
                        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                        VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT,
                    );
                    g.image_access(
                        vb_id,
                        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                        VK_ACCESS_SHADER_READ_BIT,
                        VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                        SubRange::COLOR,
                    );

                    // Pass `vb_classify_scan`: a single workgroup, RW `gclassify` only (no
                    // image/other buffer access — the two chained exclusive-prefix-sum phases
                    // touch only the M-arrays + `group_to_mat`, all within `gclassify`).
                    let vb_classify_scan = g.add_pass("vb_classify_scan");
                    g.buffer_access(
                        gclassify,
                        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                        VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT,
                    );

                    // Pass `vb_classify_scatter`: the SAME `vb_id` + `gclassify` access shape as
                    // `vb_classify_count` above (`InterlockedAdd(cursors[mat], 1)` then
                    // `pixel_list[slot] = py*w+px`).
                    let vb_classify_scatter = g.add_pass("vb_classify_scatter");
                    g.buffer_access(
                        gclassify,
                        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                        VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT,
                    );
                    g.image_access(
                        vb_id,
                        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                        VK_ACCESS_SHADER_READ_BIT,
                        VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                        SubRange::COLOR,
                    );

                    (
                        Some(vb_classify_fill),
                        Some(vb_classify_count),
                        Some(vb_classify_scan),
                        Some(vb_classify_scatter),
                    )
                } else {
                    (None, None, None, None)
                };

            // The `lit`-producer choice (plan P1-4): `vb_shade` (material-classified, reads
            // `gclassify`) when `use_classified`, else the fused `vb_resolve` — mutually
            // exclusive by construction, exactly one runs per frame. Rung R9b: under `split`
            // NEITHER runs — `vb_shade_split` (declared after this block) is the lit producer.
            let (vb_resolve, vb_shade) = if split {
                (None, None)
            } else if use_classified {
                // Pass `vb_shade`: reads `vb_id` (COMPUTE, SHADER_READ_ONLY_OPTIMAL — already
                // transitioned by `vb_classify_count` above, so this read derives no further
                // barrier) and `gclassify` (COMPUTE, SHADER_READ ONLY — `vb_shade` never writes
                // it, unlike the classify chain's own RW access) and writes `lit` (COMPUTE,
                // COLOR_ATTACHMENT_OPTIMAL→GENERAL, extending `vb_sky`'s COLOR write, C5). Reads
                // `vb_instance_ring` (COMPUTE, the geometry-fetch instance-row lookup) +
                // `light_table`/`cascade`/`atlas` when armed this frame — the SAME conditional
                // reads `vb_resolve`'s own arm declares (`vb_shade`'s shading tail is
                // character-identical, plan D3).
                let vb_shade = g.add_pass("vb_shade");
                g.image_access(
                    vb_id,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                    SubRange::COLOR,
                );
                g.buffer_access(
                    gclassify,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                );
                g.image_access(
                    lit,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_WRITE_BIT,
                    VK_IMAGE_LAYOUT_GENERAL,
                    SubRange::COLOR,
                );
                g.buffer_access(vb_instance_ring, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
                if light_upload.is_some() {
                    g.buffer_access(light_table, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
                }
                // UNCONDITIONAL + FULL-ARRAY (09600): the shader statically references both
                // always-bound Set-1 shadow maps, so on the OFF path this derives the
                // discard-legal UNDEFINED→SHADER_READ_ONLY transition that keeps the bound
                // descriptors' layout valid (the SAME shape every other declarator uses).
                g.image_access(
                    cascade,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                    SubRange::depth_layers(MAX_CASCADES as u32),
                );
                g.image_access(
                    atlas,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                    SubRange::depth_layers(MAX_TEXTURE_LAYERS as u32),
                );
                // VB-P1a ("dark infra"): the froxel `ClusterGrid`/`LightIndexList` reads the
                // `vb_shade_froxel.comp.hlsl` variant performs every frame, gated on
                // `light_cull.is_some()` — the SAME "only need the barrier when a write happened
                // THIS frame" discipline `light_upload`'s read gate above uses. COMPUTE→COMPUTE
                // (NOT `declare_forward_graph`'s own COMPUTE→FRAGMENT — `vb_shade` is a compute
                // pass, not a raster fragment shader).
                if light_cull.is_some() {
                    g.buffer_access(
                        cluster_grid,
                        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                        VK_ACCESS_SHADER_READ_BIT,
                    );
                    g.buffer_access(
                        light_index,
                        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                        VK_ACCESS_SHADER_READ_BIT,
                    );
                }
                (None, Some(vb_shade))
            } else {
                // Pass `vb_resolve` (FUSED): reads `vb_id` (COMPUTE,
                // COLOR_ATTACHMENT_OPTIMAL→SHADER_READ_ONLY_OPTIMAL) and writes `lit` (COMPUTE,
                // COLOR_ATTACHMENT_OPTIMAL→GENERAL, extending `vb_sky`'s COLOR write, C5). Reads
                // `vb_instance_ring` (COMPUTE, for the per-instance material ring lookup) +
                // `light_table`/`cascade`/`atlas` when armed this frame.
                let vb_resolve = g.add_pass("vb_resolve");
                g.image_access(
                    vb_id,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                    SubRange::COLOR,
                );
                g.image_access(
                    lit,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_WRITE_BIT,
                    VK_IMAGE_LAYOUT_GENERAL,
                    SubRange::COLOR,
                );
                g.buffer_access(vb_instance_ring, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
                if light_upload.is_some() {
                    g.buffer_access(light_table, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
                }
                // UNCONDITIONAL + FULL-ARRAY (09600): the shader statically references both
                // always-bound Set-1 shadow maps, so on the OFF path this derives the
                // discard-legal UNDEFINED→SHADER_READ_ONLY transition that keeps the bound
                // descriptors' layout valid (the SAME shape every other declarator uses).
                g.image_access(
                    cascade,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                    SubRange::depth_layers(MAX_CASCADES as u32),
                );
                g.image_access(
                    atlas,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                    SubRange::depth_layers(MAX_TEXTURE_LAYERS as u32),
                );
                // VB-P1a ("dark infra"): the froxel `ClusterGrid`/`LightIndexList` reads the
                // `vb_resolve_froxel.comp.hlsl` variant performs every frame — see the `vb_shade`
                // arm's own comment (identical gate + rationale).
                if light_cull.is_some() {
                    g.buffer_access(
                        cluster_grid,
                        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                        VK_ACCESS_SHADER_READ_BIT,
                    );
                    g.buffer_access(
                        light_index,
                        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                        VK_ACCESS_SHADER_READ_BIT,
                    );
                }
                (Some(vb_resolve), None)
            };

            (
                vb_classify_fill,
                vb_classify_count,
                vb_classify_scan,
                vb_classify_scatter,
                vb_indirect_upload,
                vb_batch_cull,
                vb_cull_readback,
                Some(vb_raster),
                vb_resolve,
                vb_shade,
            )
        } else {
            (None, None, None, None, None, None, None, None, None, None)
        };

        // ==== VG R3 piece 1 steps P1-8/P1-5 + VG R3 piece 2 step P2-5 (plan D6): the
        // `[hzb_poison, hzb_build_*]` block's UNSPLIT slot — after the `lit` producer, before the
        // `vb_viewt` PRE-TAIL slot below. ====
        //
        // This is the position the block has held since piece 1, and it is where it stays on every
        // frame the occlusion split is not armed — which is every scene in this tree today. The
        // declaration is byte-for-byte the one that shipped (it moved into
        // `declare_hzb_poison_build` so that BOTH slots are one text and the poison cannot be left
        // behind by the builds), so an unsplit frame derives a barrier stream identical to the
        // baseline `tests/vb_barrier_stream_baseline.rs` pinned before this step existed.
        //
        // `hzb_build_0`'s `vb_depth` read at THIS slot derives the raster's
        // `DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL` transition, and every later
        // same-layout reader (`vb_viewt`, `sdf_forward_march`, `hzb_dump`) then needs none. That is
        // the property the armed-split slot changes — see the early slot's own comment.
        if !occlusion_split {
            let (poison, build) =
                declare_hzb_poison_build(g, hzb_levels, hzb_dump_armed, hzb_pyramid, vb_depth);
            hzb_poison = poison;
            hzb_build = build;
        }

        // ---- Rung R9b: the SPLIT arm (docs/R9-VB-SPLIT-PLAN.md §3) — declared between
        // `vb_raster` (whose `vb_id`/`vb_depth` both split passes consume) and
        // `sdf_forward_march` (which extends `vb_shade_split`'s `lit` GENERAL write under
        // `Both`). ---------------------------------------------------------------------------
        //
        // `vb_viewt` PRE-TAIL slot: when the split's SSAO is armed, the gViewT producer must
        // run BEFORE the gather (gViewT is the gather's ray-metric depth source), so the pass
        // is declared HERE instead of its taa-only LATE slot below — ONE `scene.ssao.is_some()`
        // predicate picks the slot at both declare and record (the accesses are IDENTICAL in
        // both slots; only the position differs).
        let vb_viewt_pre = (scene.viewt_from_vb_depth.is_some() && scene.ssao.is_some()).then(|| {
            let p = g.add_pass("vb_viewt");
            g.image_access(
                vb_depth,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                SubRange::DEPTH,
            );
            g.image_access(
                viewt,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
            p
        });
        let mut ssao_atrous_vb: [Option<crate::framegraph::PassId>;
            crate::present::MAX_SSAO_ATROUS_LEVELS as usize] =
            [None; crate::present::MAX_SSAO_ATROUS_LEVELS as usize];
        let (vb_geo, vb_ssao_pass) = if split {
            // Pass `vb_geo` — the split's thin-aux producer: the FIRST `vb_id` reader under
            // split (derives the COLOR_ATTACHMENT→SHADER_READ_ONLY transition out of the
            // raster; every later same-layout read needs none), reads the instance ring for
            // the Decision-0 geometry fetch, and first-touch writes `thin_normal`
            // (UNDEFINED→GENERAL) — UNCONDITIONAL under split (`split ⇒ NORMAL`, the R9a
            // resolver invariant; a split config with no thin-normal consumer does not exist).
            let vb_geo = g.add_pass("vb_geo");
            g.image_access(
                vb_id,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                SubRange::COLOR,
            );
            g.buffer_access(vb_instance_ring, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
            g.image_access(
                thin_normal,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
            // Rung R9d: when the hwrt shadow chain's MV variant is selected this frame
            // (`vb_geo_mv_active()` — the O1 single predicate `record_vb`'s pipeline pick reads
            // too), `vb_geo` ALSO writes each mesh pixel's camera-only motion vector `Δuv` to
            // `motion_vec` (STORAGE, GENERAL, first touch). OFF (temporal off / non-storage /
            // non-hwrt) ⇒ no access ⇒ the graph routes ZERO barriers on `motion_vec` for this
            // pass ⇒ byte-identical.
            #[cfg(feature = "hwrt")]
            if scene.vb_geo_mv_active() {
                g.image_access(
                    motion_vec,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_WRITE_BIT,
                    VK_IMAGE_LAYOUT_GENERAL,
                    SubRange::COLOR,
                );
            }

            // The SSAO gather + à-trous chain (`path_vb_ssao` — the O1 predicate `record_vb`
            // reads too). The gather reads `thin_normal` + `viewt` (GENERAL) and writes `ssao`
            // (its seed-inert first touch); the à-trous loop mirrors the DEFERRED declarator's
            // role loop verbatim (Read8/Interior/Write8 over the two interior rings, each level
            // reading `viewt` for its edge-stops).
            let vb_ssao_pass = if scene.path_vb_ssao() {
                let p = g.add_pass("ssao");
                for &res in &[thin_normal, viewt] {
                    g.image_access(
                        res,
                        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                        VK_ACCESS_SHADER_READ_BIT,
                        VK_IMAGE_LAYOUT_GENERAL,
                        SubRange::COLOR,
                    );
                }
                g.image_access(
                    ssao_img,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_WRITE_BIT,
                    VK_IMAGE_LAYOUT_GENERAL,
                    SubRange::COLOR,
                );

                let ssao_atrous_levels = scene.ssao.as_ref().map_or(0, |s| s.atrous_levels);
                debug_assert!(
                    ssao_atrous_levels == 0
                        || (2..=crate::present::MAX_SSAO_ATROUS_LEVELS).contains(&ssao_atrous_levels),
                    "invariant: ssao_atrous_levels is 0 or 2..=MAX (clamped_atrous_levels); got {ssao_atrous_levels}"
                );
                for (level, slot) in ssao_atrous_vb
                    .iter_mut()
                    .enumerate()
                    .take(ssao_atrous_levels as usize)
                {
                    let level = level as u32;
                    let rings = [ssao_ring_a, ssao_ring_b];
                    let (in_res, out_res) =
                        match crate::present::ssao_atrous_step(level, ssao_atrous_levels) {
                            crate::present::AtrousStepRole::Read8 => (ssao_img, rings[0]),
                            crate::present::AtrousStepRole::Interior { in_ring } => {
                                (rings[in_ring as usize], rings[1 - in_ring as usize])
                            }
                            crate::present::AtrousStepRole::Write8 { in_ring } => {
                                (rings[in_ring as usize], ssao_img)
                            }
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
                Some(p)
            } else {
                None
            };

            (Some(vb_geo), vb_ssao_pass)
        } else {
            (None, None)
        };

        // Rung R9d: the VB hardware shadow chain (docs/R9-VB-SPLIT-PLAN.md §6) — TLAS pack/build
        // + the RT soft-shadow VIS pre-pass + the `levels` à-trous passes + the temporal
        // reproject, declared AFTER the SSAO/à-trous section and BEFORE the R9c `ddgi_update`
        // pass (below), mirroring the DEFERRED declarator's own pack/build + VIS/à-trous/temporal
        // shapes (this module's doc) but reading the split's OWN thin-aux lanes
        // (`thin_normal`/`viewt`) instead of the fat `gNormal` G-buffer. `tlas_pack`/`tlas_build`
        // are gated on `scene.tlas.is_some()` alone (independent of the vis chain, mirroring the
        // deferred declarator) — under VB the TLAS exists only to feed this chain, so in practice
        // it is armed only when `split` also holds. `vb_final_vis_res` is used below by
        // `vb_shade_split`'s conditional denoised-vis read.
        #[cfg(feature = "hwrt")]
        let (vb_tlas_pack, vb_tlas_build) = if split && scene.tlas.is_some() {
            // Pass `tlas_pack`: writes the `tlas_instances` array (COMPUTE/SHADER_WRITE). VB v1
            // has no interp pass (`VbPassPlan`'s doc) — the instance ring is host-CPU-scattered
            // into host-coherent memory, so (mirroring the deferred declarator's own interp-off
            // shape) the pack declares ONLY its `tlas_instances` write.
            let pack = g.add_pass("tlas_pack");
            g.buffer_access(
                tlas_instances,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            );
            // Pass `tlas_build`: reads the `tlas_instances` array at the AS-build stage — the
            // graph derives the pack(SHADER_WRITE) → build(AS_BUILD/SHADER_READ) barrier. The
            // build writes the AS into the UNTRACKED backing/scratch (invisible to the graph).
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
        #[cfg(feature = "hwrt")]
        let (vb_shadow_vis_pass, vb_shadow_atrous_passes, vb_final_vis_res, vb_shadow_temporal_pass) =
            if split
                && let Some(sh) = scene.shadow.as_ref()
        {
            // Pass `shadow_vis`: reads `thin_normal`/`viewt` (GENERAL, the split's own thin-aux
            // lanes) + the tlas buffer (COMPUTE read), writes `shadow_vis` (GENERAL, first
            // touch).
            let vis = g.add_pass("shadow_vis");
            for &c in &[thin_normal, viewt] {
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

            // The `atrous_levels` à-trous passes (ping-pong) — the deferred declarator's role
            // loop verbatim, reading `thin_normal`/`viewt` instead of `gNormal`/`gViewT`.
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
                for &c in &[thin_normal, viewt] {
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
            let final_res = if atrous_levels % 2 == 1 { shadow_vis2 } else { shadow_vis };

            // The temporal reproject+accumulate pass, declared AFTER the à-trous chain when the
            // author's mode is temporal (`sh.temporal`) — the deferred declarator's shape
            // verbatim, reading the split's `viewt` lane instead of the fat `gViewT`.
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
        #[cfg(feature = "hwrt")]
        debug_assert!(
            vb_tlas_build.is_none_or(|b| vb_shadow_vis_pass.is_none_or(|v| b.index() < v.index())),
            "invariant: tlas_build must be declared before shadow_vis when both are armed"
        );

        let (vb_ddgi_update_pass, vb_shade_split) = if split {
            // Rung R9c: pass `ddgi_update` — declared between the SSAO chain and the split
            // shade (the §3 order), gated on `path_vb_ddgi()` (reachable only VB×Both — the
            // activation carries the `sdf_leg` AND). The access list is the DEFERRED
            // declarator's verbatim: light_table/ray-table reads, classification RW, and the
            // two atlas layered STORAGE writes (whose content-preserving SRO→GENERAL
            // transitions the seeds license).
            let vb_ddgi_update = if scene.path_vb_ddgi() {
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
                g.buffer_access(
                    ddgi_classification,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT,
                );
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

            // Pass `vb_shade_split` — the split's `lit` producer: RE-fetches through `vb_id`
            // (already SHADER_READ_ONLY via `vb_geo`), extends `vb_sky`'s COLOR write on `lit`
            // (COLOR_ATTACHMENT→GENERAL, the C5 shape `vb_resolve` declares), reads the
            // instance ring + the light/shadow vocab exactly as the fused arm does, and reads
            // `ssao` UNCONDITIONALLY (the 09600 stable-descriptor discipline — backed by the
            // seed on split-without-SSAO frames and by the always-allocated image).
            let s = g.add_pass("vb_shade_split");
            g.image_access(
                vb_id,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                SubRange::COLOR,
            );
            g.image_access(
                lit,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
            g.buffer_access(vb_instance_ring, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
            if light_upload.is_some() {
                g.buffer_access(light_table, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
            }
            // UNCONDITIONAL + FULL-ARRAY (09600): the SAME shape the fused arm declares.
            g.image_access(
                cascade,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                SubRange::depth_layers(MAX_CASCADES as u32),
            );
            g.image_access(
                atlas,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                SubRange::depth_layers(MAX_TEXTURE_LAYERS as u32),
            );
            g.image_access(
                ssao_img,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
            // Rung R9c: the CONDITIONAL DDGI atlas reads — declared iff the update armed (the
            // deferred resolve's own `ddgi_update.is_some()`-gated shape), deriving the
            // update-write → shade-read GENERAL→SHADER_READ_ONLY layered barriers; a DDGI-off
            // frame declares nothing here (the seeded ResIds are then named by NOTHING ⇒ zero
            // barriers ⇒ the GI-off byte-id discipline).
            if vb_ddgi_update.is_some() {
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
            // Rung R9d: when the hwrt shadow chain is armed this frame
            // (`path_vb_hwrt_shadow()`), `vb_shade_split` ALSO reads the FINAL denoised/
            // undenoised visibility (GENERAL, COMPUTE) — `temporal_out` when the temporal stage
            // ran, else the à-trous-parity final ring — deriving the last-write → shade-read
            // barrier (mirrors the deferred DENOISED resolve's own conditional read).
            #[cfg(feature = "hwrt")]
            if scene.path_vb_hwrt_shadow() {
                let vis_read = if scene.temporal_active() { temporal_out } else { vb_final_vis_res };
                g.image_access(
                    vis_read,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_GENERAL,
                    SubRange::COLOR,
                );
            }
            (vb_ddgi_update, Some(s))
        } else {
            (None, None)
        };

        // Pass `sdf_forward_march` — rung R10: the fused SDF march-then-shade COMPUTE pass, the
        // SAME pass `declare_forward_graph` declares (that fn's own doc has the full C5 rationale).
        // Gated on `scene.path_has_sdf_forward()` (`== resolved_render_path.sdf_forward_marched`).
        // Writes `lit` (COMPUTE/STORAGE/GENERAL) — under `Both` it extends `vb_resolve`'s GENERAL
        // write (COMPUTE→COMPUTE WAW, no layout change); under `Sdf` it extends `vb_sky`'s COLOR
        // write (COLOR_ATTACHMENT_OPTIMAL→GENERAL, the SAME transition the deferred resolve's own
        // `lit` write establishes). Reads `vb_depth` (COMPUTE/SHADER_READ_ONLY_OPTIMAL) ONLY under
        // `mesh_leg` (the `HAS_MESH` variant samples the mesh surface to bound the march; the
        // mesh-less variant never references the binding).
        //
        // Under `!mesh_leg` this pass is the SOLE reader of the shadow/light vocab it shades with
        // (there is no `vb_resolve` to transition `cascade`/`atlas`/`light_table` this frame), so
        // it declares those reads HERE to derive the csm/atlas/light_upload producer barriers.
        // Under `mesh_leg`, `vb_resolve` already declared them and made the writes visible to
        // COMPUTE reads; this later same-queue read inherits that (no extra barrier needed), so
        // re-declaring would be redundant — matching how the Forward path's own `sdf_forward_march`
        // relies on `forward_opaque`'s prior reads.
        //
        // TAA-under-VB: when `scene.path_sdf_forward_writes_viewt()` (the O1 single predicate the
        // record site's VIEWT-variant pipeline selection reads too), the marcher composite ALSO
        // first-touch writes `viewt` (UNDEFINED→GENERAL) — on an SDF-carrying leg set it is the
        // SOLE gViewT producer (`vb_viewt` below is mesh-only; the two armings are disjoint by
        // construction), feeding the `taa_resolve` pass's GENERAL read.
        let sdf_forward_march = if scene.path_has_sdf_forward() {
            let p = g.add_pass("sdf_forward_march");
            g.image_access(
                lit,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
            if scene.path_sdf_forward_writes_viewt() {
                g.image_access(
                    viewt,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_WRITE_BIT,
                    VK_IMAGE_LAYOUT_GENERAL,
                    SubRange::COLOR,
                );
            }
            if mesh_leg {
                g.image_access(
                    vb_depth,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                    SubRange::DEPTH,
                );
            } else {
                if light_upload.is_some() {
                    g.buffer_access(light_table, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
                }
                // UNCONDITIONAL + FULL-ARRAY (09600): the shader statically references both
                // always-bound Set-1 shadow maps, so on the OFF path this derives the
                // discard-legal UNDEFINED→SHADER_READ_ONLY transition that keeps the bound
                // descriptors' layout valid (the SAME shape every other declarator uses).
                g.image_access(
                    cascade,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                    SubRange::depth_layers(MAX_CASCADES as u32),
                );
                g.image_access(
                    atlas,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                    SubRange::depth_layers(MAX_TEXTURE_LAYERS as u32),
                );
            }
            Some(p)
        } else {
            None
        };

        // Pass `vb_viewt`, the taa-only LATE slot — gated `scene.viewt_from_vb_depth.is_some()
        // && scene.ssao.is_none()` (rung R9b: with the split's SSAO armed the pass moved to the
        // PRE-TAIL slot above; ONE `ssao.is_some()` predicate picks the slot at both declare
        // and record). Reads `vb_depth` at COMPUTE/SHADER_READ_ONLY (the SAME conditional-read
        // shape `sdf_forward_march`'s HAS_MESH arm declares) and first-touch WRITES `viewt`
        // (UNDEFINED→GENERAL) — the `gViewT` lane the `taa_resolve` pass below consumes.
        //
        // Producer disjointness (rung R9b revision of the Track-A assert): dual arming
        // (`vb_viewt` + the VIEWT-variant marcher) is legal ONLY in the split+SSAO
        // configuration on an SDF-carrying leg set — `vb_viewt` (pre-tail) feeds the gather
        // with mesh `t` while the marcher overwrites at composite as the LAST declared gViewT
        // writer (the declared order derives the WAW barrier); every other configuration keeps
        // the strict Track-A disjointness.
        debug_assert!(
            !(scene.viewt_from_vb_depth.is_some() && scene.path_sdf_forward_writes_viewt())
                || (scene.ssao.is_some() && scene.resolved_render_path.mesh_geo_shade_split),
            "invariant: dual gViewT producers (vb_viewt + the VIEWT marcher) are legal only \
             under the split+SSAO configuration"
        );
        let vb_viewt = (scene.viewt_from_vb_depth.is_some() && scene.ssao.is_none()).then(|| {
            let p = g.add_pass("vb_viewt");
            g.image_access(
                vb_depth,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                SubRange::DEPTH,
            );
            g.image_access(
                viewt,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
            p
        });

        // Pass `taa_resolve` (TAA-under-VB) — gated `scene.taa.is_some()`, declared AFTER every
        // `lit` producer and BEFORE `present_sample` (the resolve must see this frame's final
        // shaded color; `record_vb` records it in this exact order). The access list copies the
        // DEFERRED declarator's own `taa_resolve` pass verbatim: `lit` @0 is a
        // COMBINED_IMAGE_SAMPLER (descriptor records SHADER_READ_ONLY_OPTIMAL — the C2 fix), so
        // this read derives the GENERAL→SHADER_READ_ONLY transition out of `vb_resolve`/
        // `vb_shade`/`sdf_forward_march`'s GENERAL write (or COLOR_ATTACHMENT→… out of a
        // sky-only frame) and `present_sample`'s later read finds `lit` already in layout;
        // `viewt`/`taa_hist_read` are STORAGE reads in GENERAL; `taa_hist` is the STORAGE write.
        // `aa_out`/`taa_resolved` stay hand-recorded inside `record_taa`/`record_rcas` (not
        // framegraph-tracked — the deferred precedent).
        // Rung R9b revision of the Track-A TAA XOR: the strict XOR is KEPT VERBATIM for every
        // `!ssao` configuration (preserving the shipped VB TAA pins' exact producer schedule);
        // with the split's SSAO armed it degrades to "at least one producer, and on an
        // SDF-carrying leg the marcher (the LAST declared writer) is among them".
        debug_assert!(
            scene.taa.is_none()
                || if scene.ssao.is_none() {
                    scene.viewt_from_vb_depth.is_some() ^ scene.path_sdf_forward_writes_viewt()
                } else {
                    (scene.viewt_from_vb_depth.is_some() || scene.path_sdf_forward_writes_viewt())
                        && (!scene.resolved_render_path.sdf_leg
                            || scene.path_sdf_forward_writes_viewt())
                },
            "invariant: a TAA-armed VB frame has a coherent gViewT producer set \
             (strict XOR without SSAO; at-least-one + marcher-last on SDF legs with SSAO)"
        );
        let taa_resolve_pass = scene.taa.is_some().then(|| {
            let p = g.add_pass("taa_resolve");
            g.image_access(
                lit,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                SubRange::COLOR,
            );
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
            p
        });

        // Pass `present_sample`: `lit` → SHADER_READ_ONLY_OPTIMAL for the present-blit's FRAGMENT
        // sample (C5, derived from the LAST `lit` producer; with TAA armed the `taa_resolve` read
        // above already left `lit` in SHADER_READ_ONLY, so this derives no further barrier — the
        // deferred precedent). The swapchain WSI barriers stay hand-recorded.
        let present_sample = g.add_pass("present_sample");
        g.image_access(
            lit,
            VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::COLOR,
        );

        // ==== VG R3 piece 1 step P1-6 (plan §5, gate G8): the pyramid DUMP copy. ====
        //
        // A DECLARED PASS, not a hand-written barrier — the one place this seam improves on the
        // census's own `vb_id` copy, which it is otherwise modelled on. It is expressible here
        // precisely because `vkCmdCopyImageToBuffer` accepts `GENERAL` and the pyramid is
        // `GENERAL` for life: the derived edge on `hzb_pyramid` is a pure `COMPUTE(SHADER_WRITE)
        // → TRANSFER(TRANSFER_READ)` flush that changes NO layout, so a dump frame leaves the
        // resource in exactly the LAYOUT an undumped frame does, and the seed the pyramid's
        // declaration argues for is not falsified on it. ⚠️ Since VG R3 P3-0 that seed is
        // `seeded_writer_at_layout(GENERAL, …)` rather than `ResSync::undefined()`, and the
        // sentence holds in the STRONGER sense — the layout the seed names is the one this copy
        // both requires and preserves. What a dump frame DOES change is the pyramid's last ACCESS
        // (a TRANSFER read rather than a COMPUTE write); the seed's declaration site states that
        // one-frame residual in full and why it is strictly improved by the new seed.
        //
        // DECLARED LAST in the whole graph. The requirement is only that it follow every pass
        // touching these two resources (so it observes the FINISHED pyramid rather than a
        // half-built one), and "last" is the spelling of that which a later step cannot
        // invalidate by inserting a pass above it. `record_vb` records it at the matching
        // position — after the present blit, beside the census copy — so declare/record order
        // parity holds.
        //
        // ⚠️ THE GATE IS `hzb_build`'s, PLUS the probe. `scene.hzb_dump.is_some()` alone is not
        // enough: without the mesh leg `vb_raster` is not declared, nothing writes `vb_depth` this
        // frame, and a TRANSFER read of it would take `compile`'s first-touch arm on an unwritten
        // transient — the failure that guard exists to catch. Reading the arm off the SAME
        // `scene.hzb.filter(|_| mesh_leg)` the build chain reads means the two cannot disagree
        // about whether there is a pyramid to dump.
        //
        // ⚠️ `plan.levels`, NEVER `MAX_HZB_LEVELS` — see `hzb_mips`'s own doc. The capacity is 17
        // and the live count is 10 at 512×512; a capacity-wide span is out of range at every real
        // extent, and the barrier path bounds nothing.
        let hzb_dump = match (scene.hzb.filter(|_| mesh_leg), scene.hzb_dump) {
            (Some(hzb_plan), Some(_)) => {
                let p = g.add_pass("hzb_dump");
                // The SOURCE depth. Its tracked layout here is whatever the last TOUCHER left, and
                // the graph DERIVES the transition rather than this site assuming it. No restore is
                // declared and none is needed: the ring slot re-enters next frame through
                // `vb_raster`'s own `UNDEFINED → DEPTH_ATTACHMENT_OPTIMAL` first touch, which
                // discards contents by definition.
                //
                // ⚠️ VG R3 piece 2 step P2-5 FALSIFIED THIS SITE'S FORMER CLAIM, which read "on
                // every armed frame that is `SHADER_READ_ONLY_OPTIMAL`, since `hzb_build_0` itself
                // reads it there". That is now true only on an UNSPLIT frame. With the split armed
                // the `[hzb_poison, hzb_build_*]` block moves BETWEEN the two raster scopes (D6),
                // so the last toucher is `vb_raster_late` at `DEPTH_ATTACHMENT_OPTIMAL` with a
                // pending write, and this read is re-sourced from the execution-only arm (COMPUTE,
                // `src_access` 0) to a real RAW flush — `FRAG`/`DEPTH_STENCIL_ATTACHMENT_WRITE →
                // TRANSFER`/`TRANSFER_READ` with a `DEPTH_ATTACHMENT_OPTIMAL → TRANSFER_SRC_OPTIMAL`
                // transition. That is STRICTLY STRONGER than what it replaces, so the change is in
                // the model and not in the soundness; it is stated because a reader diffing the
                // derived stream would otherwise read a correct value as a regression. The same
                // re-sourcing applies to `vb_viewt` (both slots) and `sdf_forward_march`'s mesh arm.
                //
                // ⚠️ AND THE ORDERING THIS SITE CANNOT SEE, recorded for piece 3: on an armed-split
                // frame the pyramid is built from the depth as of the EARLY scope, while the dump
                // copies `vb_depth` at frame END. In piece 2 those are the same bytes because the
                // late scope draws nothing — so gate G8 still holds AND is blind to the ordering.
                // The moment piece 3 arms the late draws they diverge, and the dump must move
                // between the scopes or copy both depths.
                g.image_access(
                    vb_depth,
                    VK_PIPELINE_STAGE_TRANSFER_BIT,
                    VK_ACCESS_TRANSFER_READ_BIT,
                    VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    SubRange::DEPTH,
                );
                // Every mip in ONE access. The per-mip states differ across the span (mips
                // `[0, 6)` were written by pass 0, `[6, levels)` by pass 1, and mip 5 was also
                // READ by pass 1), which is exactly the heterogeneity step P1-5a re-keyed the sync
                // state to admit: `compile` splits the span into maximal equal-state runs and
                // derives one barrier per run.
                g.image_access(
                    hzb_pyramid,
                    VK_PIPELINE_STAGE_TRANSFER_BIT,
                    VK_ACCESS_TRANSFER_READ_BIT,
                    VK_IMAGE_LAYOUT_GENERAL,
                    hzb_mips(0, hzb_plan.levels),
                );
                Some(p)
            }
            _ => None,
        };

        // Rung R9d structural order guard (mirrors the `tlas_build < shadow_vis` assert above):
        // `vb_geo` (the `thin_normal` producer) must be declared before `shadow_temporal` (a
        // `thin_normal`-adjacent consumer via the à-trous chain it follows) whenever both are
        // armed — a ladder inversion here would derive barriers in the wrong order.
        #[cfg(feature = "hwrt")]
        debug_assert!(
            vb_geo.is_none_or(|geo| vb_shadow_temporal_pass.is_none_or(|t| geo.index() < t.index())),
            "invariant: vb_geo must be declared before shadow_temporal when both are armed"
        );

        // VG R3 piece 1 step P1-6: the dump observes the finished pyramid only if it is declared
        // after the last build pass. Asserted rather than left to the block order above, because
        // "declared last" is a property of THIS function's layout that a future insertion can
        // break silently — the derived barrier would still be legal, it would simply flush a
        // pyramid that was not finished.
        debug_assert!(
            hzb_dump.is_none_or(|dump| hzb_build
                .iter()
                .flatten()
                .all(|build| build.index() < dump.index())),
            "invariant: the HZB dump pass is declared after every hzb_build pass"
        );

        // VG R3 piece 1 step P1-8: the MIRROR of the assertion above, and it is load-bearing for
        // the same reason in the opposite direction. A poison declared after any build pass would
        // erase the levels that pass had just written — the dump would then read `-1.0` everywhere
        // and G8 would red claiming "the build never ran", which is a gate reporting the wrong
        // defect. The block order above says it today; this says it after the next insertion too.
        debug_assert!(
            hzb_poison.is_none_or(|poison| hzb_build
                .iter()
                .flatten()
                .all(|build| poison.index() < build.index())),
            "invariant: the HZB poison pass is declared before every hzb_build pass"
        );
        // The two probes are ONE arming decision (`hzb_dump`'s predicate, verbatim), so a frame
        // that poisons must be a frame that dumps: a poisoned pyramid nobody reads back would be a
        // transfer write for nothing, and a dump over an unpoisoned pyramid is exactly the vacuous
        // comparison step P1-6 measured.
        debug_assert_eq!(
            hzb_poison.is_some(),
            hzb_dump.is_some(),
            "invariant: the HZB poison and dump passes are armed by ONE predicate"
        );

        // ==== VG R3 piece 3 step P3-7 (plan D10): the early-depth copy's own four asserts. ====
        //
        // Its whole claim is POSITIONAL — "these are the bytes the pyramid was built from" — so the
        // relations that make it true are checked here rather than left to the block order above,
        // exactly as the poison's and the dump's are.
        debug_assert_eq!(
            hzb_dump_depth_early.is_some(),
            occlusion_split && hzb_dump_armed,
            "invariant: the early-depth copy is declared on EXACTLY (occlusion_split && dump armed) \
             — the same conjunction `record_vb` records it under, so a declared-but-unrecorded pass \
             (or the reverse) cannot happen"
        );
        debug_assert!(
            hzb_dump_depth_early
                .is_none_or(|d| hzb_build.iter().flatten().all(|b| b.index() < d.index())),
            "invariant: the early-depth copy is declared after every hzb_build pass — declared \
             before them it would copy a depth the builds had not yet read, and the two-sided claim \
             would compare the pyramid against bytes nothing reduced"
        );
        debug_assert!(
            hzb_dump_depth_early
                .is_none_or(|d| vb_raster_late.is_some_and(|l| d.index() < l.index())),
            "invariant: the early-depth copy is declared before the late raster scope — after it, \
             it would copy the FINAL depth into the EARLY region and both of G-P3-E's clauses would \
             be statements about the same bytes"
        );
        // A frame with an early region always has a final one to put beside it: `hzb_dump`'s gate is
        // `(hzb && mesh_leg) && dump_armed` and `path_vb_occlusion_split()` implies both of the
        // first two conjuncts since step P3-6. Stated as an assert because the header's `flags` bit
        // is written by the FRAME-END block, so an early copy with no frame-end block would leave
        // the region live and the bit unset — a live payload the reader is told to ignore.
        debug_assert!(
            hzb_dump_depth_early.is_none() || hzb_dump.is_some(),
            "invariant: an early-depth copy implies the frame-end dump that stamps its flag"
        );

        // ==== VG R3 piece 2 step P2-5 (plan, gate G4): the declare-side half of the split's
        // declare/record parity, and the ORDER the derived barriers depend on. ====
        //
        // These run in production on every dev-profile build, which is what the golden runs use
        // (`scripts/golden.ps1` carries no `--release`) — and that matters because the gate that
        // pins this stream is a hand-written REPLICA of this function (`declare_vb_graph` is
        // `pub(crate)` on a `Renderer` no test constructs). A replica proves the framegraph derives
        // the right stream from a declaration shaped like this one; only these asserts constrain
        // THIS function to write that shape.
        debug_assert_eq!(
            vb_raster_late.is_some(),
            occlusion_split,
            "invariant: the late raster scope is declared on EXACTLY path_vb_occlusion_split() — \
             a declared-but-unrecorded pass (or the reverse) is a barrier derived for work that \
             never happens"
        );
        debug_assert_eq!(
            vb_indirect_late_upload.is_some(),
            occlusion_split,
            "invariant: the late indirect upload is declared on EXACTLY path_vb_occlusion_split()"
        );
        debug_assert!(
            vb_indirect_late_upload
                .is_none_or(|u| vb_raster_late.is_some_and(|l| u.index() < l.index())),
            "invariant: the late indirect upload is declared before the late raster reads it — \
             reversed, the TRANSFER write is not the fetch's source and the derived edge orders \
             nothing"
        );
        debug_assert!(
            vb_raster_late
                .is_none_or(|l| hzb_build.iter().flatten().all(|b| b.index() < l.index())),
            "invariant: on an armed split the pyramid build precedes the late raster — the pyramid \
             must reduce the EARLY scope's depth, and the late scope writes that depth again"
        );
        debug_assert!(
            vb_raster_late.is_none_or(|l| vb_raster.is_some_and(|e| e.index() < l.index())),
            "invariant: the early raster precedes the late raster — the late scope LOAD_OP_LOADs \
             what the early scope stored, and with no early scope there is nothing to load"
        );

        // ==== VG R3 piece 3 step P3-3 (plan D8): the late cull's own four declare-order asserts. ====
        //
        // ⚠️ NONE of them equates `hzb_build`'s presence with `vb_cull_late`'s, and that is a
        // deliberate omission rather than a gap. `[vb_mesh_hzb]` sets `BOYKO_VG_HZB=1` with NO
        // `BOYKO_VG_OCC`, so "a pyramid implies a late cull" is FALSE on a correct configuration —
        // and goldens run the dev profile, so such an assert would panic on it. ⚠️ The CONVERSE
        // became true at step P3-6, when `path_vb_occlusion_split()` gained its `hzb.is_some()`
        // conjunct; it is checked where it is consumed (`passes/vb.rs`'s `ARMED ⇒ scene.hzb`
        // assert) rather than restated here, and it is why `[vb_occ_split]` now carries
        // `BOYKO_VG_HZB=1` too — two committed pins build a pyramid, only one of them splits.
        debug_assert_eq!(
            vb_cull_late.is_some(),
            occlusion_split,
            "invariant: the late cull is declared on EXACTLY path_vb_occlusion_split() — the same \
             single predicate record_vb reads, so a declared-but-unrecorded pass (or the reverse) \
             cannot happen"
        );
        debug_assert!(
            vb_cull_late.is_none_or(|c| hzb_build.iter().flatten().all(|b| b.index() < c.index())),
            "invariant: the late cull reads the pyramid this frame's build wrote — declared before \
             the builds, it would test against the PREVIOUS frame's pyramid, which is the EARLY \
             phase's predicate and not this one's"
        );
        debug_assert!(
            vb_cull_late.is_none_or(|c| vb_raster_late.is_some_and(|l| c.index() < l.index())),
            "invariant: the late cull writes the count the late raster fetches — reversed, the \
             COMPUTE write is not the fetch's source and the derived edge orders nothing"
        );
        debug_assert!(
            vb_cull_readback_late
                .is_none_or(|r| vb_raster_late.is_some_and(|l| l.index() < r.index())),
            "invariant: the post-late snapshot is declared AFTER the late raster, so it does not \
             re-source the indirect fetch and the shipping barrier chain the pin certifies is the \
             one that ships"
        );

        g.compile();

        self.vb_pass_plan = Some(VbPassPlan {
            light_upload,
            light_cull,
            csm,
            atlas: atlas_pass,
            vb_sky,
            vb_indirect_upload,
            vb_indirect_late_upload,
            vb_batch_cull,
            vb_cull_readback,
            vb_cull_late,
            vb_cull_readback_late,
            hzb_build,
            hzb_poison,
            hzb_dump,
            hzb_dump_depth_early,
            vb_raster,
            vb_raster_late,
            vb_classify_fill,
            vb_classify_count,
            vb_classify_scan,
            vb_classify_scatter,
            vb_resolve,
            vb_shade,
            vb_geo,
            vb_ssao: vb_ssao_pass,
            ssao_atrous: ssao_atrous_vb,
            ddgi_update: vb_ddgi_update_pass,
            vb_shade_split,
            #[cfg(feature = "hwrt")]
            tlas_pack: vb_tlas_pack,
            #[cfg(feature = "hwrt")]
            tlas_build: vb_tlas_build,
            #[cfg(feature = "hwrt")]
            shadow_vis: vb_shadow_vis_pass,
            #[cfg(feature = "hwrt")]
            shadow_atrous: vb_shadow_atrous_passes,
            #[cfg(feature = "hwrt")]
            shadow_temporal: vb_shadow_temporal_pass,
            sdf_forward_march,
            // ONE field for BOTH slots — the pass exists in exactly one of them per frame
            // (`ssao.is_some()` picks pre-tail vs late; the graph itself remembers the declared
            // position, the recorder just replays it at the matching site).
            viewt_from_depth: vb_viewt_pre.or(vb_viewt),
            taa_resolve: taa_resolve_pass,
            present_sample,
        });
    }

    /// Multi-paradigm render-path plan, rung R8: the [`VbBarrierSink`] sibling of
    /// [`Self::record_forward_pass`] — drives one [`VbPassPlan`] pass's derived barriers
    /// (declared by [`Self::declare_vb_graph`]) into `cmd`, resolving VB's OWN small ResId space
    /// (`[lit, vb_id, vb_depth, cascade, atlas]` images; `[light_table, vb_instance_ring, gclassify]`
    /// buffers — [`VbBarrierSink`]'s doc) to the current frame's physical handles. `lit` is read
    /// from `targets` (the SAME [`GBufferTargets::lit`] ring every path reuses, C5); `vb_id` from
    /// `vb` (the current frame's [`VbTargets`](super::targets::VbTargets)); `vb_depth` from
    /// `forward` (VB REUSES [`ForwardTargets::depth`](super::targets::ForwardTargets::depth)
    /// verbatim — `VbTargets`'s doc).
    ///
    /// `#[allow(clippy::too_many_arguments)]`: one extra `&VbTargets` param over
    /// [`Self::record_forward_pass`]'s own 7 (VB reuses `forward`'s depth/shadow set but ALSO
    /// needs its own `vb_id` image) — grouping the resource params into a struct would only move
    /// the argument list, the SAME rationale `build_shadow_denoise_sets`'s own `#[allow]`
    /// documents.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_vb_pass(
        &self,
        pass: crate::framegraph::PassId,
        cmd: VkCommandBuffer,
        targets: &GBufferTargets,
        forward: &super::targets::ForwardTargets,
        vb: &super::targets::VbTargets,
        scene: &GBufferScene<'_>,
        fi: usize,
    ) {
        let mut sink = VbBarrierSink {
            fns: self.fns,
            cmd,
            images: [
                targets.lit[fi].image,
                vb.vb_id[fi].image,
                forward.depth[fi].image,
                scene.csm_cascade_texture.image,
                scene.shadow_atlas_texture.image,
                // TAA-under-VB: `viewt` is always allocated (VbMesh runs the DeferredFull-shaped
                // body); the `taa_hist` parity pair is `Option`-guarded — `NULL` when TAA is off
                // (inert: no pass names those ResIds then, the deferred sink precedent).
                targets.viewt[fi].image,
                targets.taa_hist.as_ref().map_or(VkImage::NULL, |r| r[fi].image),
                targets.taa_hist.as_ref().map_or(VkImage::NULL, |r| r[fi ^ 1].image),
                // Rung R9b (ResIds 8..=11): `thin_normal` is `Option`-guarded (allocated iff the
                // boot-frozen split armed); `ssao` is ALWAYS allocated (the deferred ring reused);
                // the à-trous interior rings are `Option`-guarded on the atrous-storage probe.
                // NULL slots stay inert — with the split off no pass names these ResIds.
                targets.thin_normal.as_ref().map_or(VkImage::NULL, |r| r[fi].image),
                targets.ssao[fi].image,
                targets.ssao_ring_a.as_ref().map_or(VkImage::NULL, |r| r[fi].image),
                targets.ssao_ring_b.as_ref().map_or(VkImage::NULL, |r| r[fi].image),
                // Rung R9c (ResIds 12/13): the DDGI probe atlases — single-instance, always
                // allocated (the GI-off frames name their ResIds with NOTHING ⇒ inert).
                scene.ddgi_irr_texture.image,
                scene.ddgi_depth_texture.image,
                // Rung R9d (ResIds 14..19): the VB hardware shadow chain's own image tail —
                // the SAME `GBufferTargets` rings the deferred path shares (`Option`-guarded on
                // the device's `RG16`/`RG16F`/`RGBA16` storage probe). NULL slots stay inert —
                // with the chain off no pass names these ResIds.
                #[cfg(feature = "hwrt")]
                targets.shadow_vis.as_ref().map_or(VkImage::NULL, |r| r[fi].image),
                #[cfg(feature = "hwrt")]
                targets.shadow_vis2.as_ref().map_or(VkImage::NULL, |r| r[fi].image),
                #[cfg(feature = "hwrt")]
                targets.motion_vec.as_ref().map_or(VkImage::NULL, |r| r[fi].image),
                #[cfg(feature = "hwrt")]
                targets.shadow_temporal_hist.as_ref().map_or(VkImage::NULL, |r| r[fi].image),
                #[cfg(feature = "hwrt")]
                targets.temporal_out.as_ref().map_or(VkImage::NULL, |r| r[fi].image),
                // ResId 19 `shadow_temporal_hist_read` — the CROSS-FRAME READ image = the
                // SIBLING parity slot `hist[fi ^ 1]` (the deferred sink's own C1-fix rule — see
                // [`Renderer::record_graph_pass`]'s doc for why `[fi]` here would be a bug).
                #[cfg(feature = "hwrt")]
                targets.shadow_temporal_hist.as_ref().map_or(VkImage::NULL, |r| r[fi ^ 1].image),
                // VG R3 piece 1 step P1-5 (LAST in both `cfg` arms — ResId 14, or 20 under
                // `hwrt`): the HZB depth pyramid. NON-RINGED, so there is no `[fi]` here — one
                // image serves both frames in flight. `Option`-guarded on the `HzbMode::Off`
                // 0%-gate (the `taa_hist`/`thin_normal`/`ssao_ring_a` shape, NOT `viewt`'s bare
                // always-allocated one): with the pyramid disarmed no pass names its ResId, so
                // the NULL is inert.
                targets.hzb.as_ref().map_or(VkImage::NULL, |h| h.pyramid.image),
            ],
            #[cfg(not(feature = "hwrt"))]
            buffers: [
                scene.light_table.buffer,
                scene
                    .vb_instance_ring
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_instance_ring")
                    [fi]
                    .buffer,
                targets
                    .vb_classify
                    .as_ref()
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries targets.vb_classify")
                    .gclassify[fi]
                    .buffer,
                // Rung R9c (buffer ResIds 3/4): the DDGI classification + Fibonacci ray table
                // (the deferred sink's own sources — placeholder-backed on GI-off boots).
                scene.ddgi_classification.buffer,
                scene.ddgi_ray_table.buffer,
                // VB-P1a ("dark infra"): the L1 froxel trio — the light-table placeholder when
                // unarmed (hardcoded today).
                scene.cluster_grid.map_or(scene.light_table.buffer, |b| b.buffer),
                scene.light_index.map_or(scene.light_table.buffer, |b| b.buffer),
                scene.light_index_alloc.map_or(scene.light_table.buffer, |b| b.buffer),
                // Rung R2a': the indirect record buffer, declared LAST so no slot above moved.
                // Placeholder-backed when unarmed, the same bound-but-unread idiom the froxel trio
                // uses -- an unarmed frame declares no pass naming this ResId, so it is inert.
                scene
                    .vb_indirect
                    .map_or(scene.light_table.buffer, |r| r[fi].buffer),
                // Rung R2c0: the batch-cull trio, appended after `vb_indirect` for the same reason.
                scene
                    .vb_batch_desc
                    .map_or(scene.light_table.buffer, |r| r[fi].buffer),
                scene
                    .vb_cull_visible
                    .map_or(scene.light_table.buffer, |r| r[fi].buffer),
                scene
                    .vb_cull_count
                    .map_or(scene.light_table.buffer, |r| r[fi].buffer),
                // Rung R2d-3: the per-INSTANCE survivor list, appended LAST. `.expect()`ed rather
                // than placeholder-backed because it is MANDATORY on every VB boot (its own field
                // doc), the same wiring `vb_instance_ring` at index 1 has and the same `.expect()`
                // every VB Set-0 builder already applies to it.
                scene
                    .vb_visible_instance
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_visible_instance")
                    [fi]
                    .buffer,
                // VG R3 piece 2 step P2-3: the LATE indirect record array, appended LAST so no
                // slot above moved. `.expect()`ed rather than placeholder-backed for the SAME
                // reason as `vb_visible_instance` directly above — `GpuSceneBundles::boot` mints
                // it unconditionally — and the placeholder would be worse here than a `None`: it
                // resolves to `scene.light_table.buffer`, so a mis-keyed barrier would silently
                // synchronise a live wrong buffer with no VUID.
                scene
                    .vb_indirect_late
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_indirect_late")
                    [fi]
                    .buffer,
                // VG R3 piece 3 step P3-3: the occlusion split's trio, appended LAST in declaration
                // order — `.expect()`ed for verbatim the reason the two entries above are.
                scene
                    .vb_late_visible
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_late_visible")
                    [fi]
                    .buffer,
                scene
                    .vb_late_count
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_late_count")
                    [fi]
                    .buffer,
                scene
                    .vb_cull_uniform
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_cull_uniform")
                    [fi]
                    .buffer,
            ],
            // Rung R9d (buffer ResId 5): `tlas_instances` — mirrors [`GbufferBarrierSink`]'s own
            // `scene.tlas.map_or(VkBuffer::NULL, |t| t.instance_array.buffer)` source.
            #[cfg(feature = "hwrt")]
            buffers: [
                scene.light_table.buffer,
                scene
                    .vb_instance_ring
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_instance_ring")
                    [fi]
                    .buffer,
                targets
                    .vb_classify
                    .as_ref()
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries targets.vb_classify")
                    .gclassify[fi]
                    .buffer,
                scene.ddgi_classification.buffer,
                scene.ddgi_ray_table.buffer,
                scene.tlas.map_or(VkBuffer::NULL, |t| t.instance_array.buffer),
                // VB-P1a ("dark infra"): the L1 froxel trio — see the `not(hwrt)` variant's own
                // comment.
                scene.cluster_grid.map_or(scene.light_table.buffer, |b| b.buffer),
                scene.light_index.map_or(scene.light_table.buffer, |b| b.buffer),
                scene.light_index_alloc.map_or(scene.light_table.buffer, |b| b.buffer),
                // Rung R2a': the indirect record buffer, declared LAST so no slot above moved.
                // Placeholder-backed when unarmed, the same bound-but-unread idiom the froxel trio
                // uses -- an unarmed frame declares no pass naming this ResId, so it is inert.
                scene
                    .vb_indirect
                    .map_or(scene.light_table.buffer, |r| r[fi].buffer),
                // Rung R2c0: the batch-cull trio, appended after `vb_indirect` for the same reason.
                scene
                    .vb_batch_desc
                    .map_or(scene.light_table.buffer, |r| r[fi].buffer),
                scene
                    .vb_cull_visible
                    .map_or(scene.light_table.buffer, |r| r[fi].buffer),
                scene
                    .vb_cull_count
                    .map_or(scene.light_table.buffer, |r| r[fi].buffer),
                // Rung R2d-3: see the `not(hwrt)` variant's own comment for why this one is
                // `.expect()`ed rather than placeholder-backed.
                scene
                    .vb_visible_instance
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_visible_instance")
                    [fi]
                    .buffer,
                // VG R3 piece 2 step P2-3: the LATE indirect record array, appended LAST — see the
                // `not(hwrt)` variant's own comment.
                scene
                    .vb_indirect_late
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_indirect_late")
                    [fi]
                    .buffer,
                // VG R3 piece 3 step P3-3: the occlusion split's trio — see the `not(hwrt)`
                // variant's own comment.
                scene
                    .vb_late_visible
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_late_visible")
                    [fi]
                    .buffer,
                scene
                    .vb_late_count
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_late_count")
                    [fi]
                    .buffer,
                scene
                    .vb_cull_uniform
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_cull_uniform")
                    [fi]
                    .buffer,
            ],
        };
        self.frame_graph.record_pass(pass, &mut sink);
    }
}
