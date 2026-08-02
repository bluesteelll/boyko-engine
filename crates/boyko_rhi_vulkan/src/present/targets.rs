//! The per-extent on-screen G-buffer targets ([`GBufferTargets`]) + the
//! per-frame-in-flight ring ([`GBufferFrame`]) + `sync_gbuffer` (extent-change
//! recreate). Split out of the former monolithic `swapchain.rs` (audit W4).

use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BufferDesc, BufferUsage, Format, ImageAspect, ImageBarrierDesc,
    ImageLayout, ImageSubresourceRange, ImageUsage, MemoryLocation, RhiCommandEncoder, RhiDevice,
    RhiQueue, TextureDesc, TextureDimension,
};
use boyko_rhi::enums::{BarrierAccess, BarrierStage};

#[cfg(feature = "hwrt")]
use crate::accel::BoundAccelStruct;
use crate::compute::LOCAL_SIZE_X;
use crate::device::VulkanContext;
use crate::ffi::*;
use crate::memory::BoundBuffer;
use crate::rhi_impl::{Vulkan, VulkanBindGroup};
use crate::texture::VulkanTexture;

use super::scene_types::GBufferScene;
use super::{FRAMES_IN_FLIGHT, SwapchainError};

// Doc-link scope: types referenced only from doc-comments (the targets document how
// the `Renderer` frame methods drive them, mirroring `Scene`'s depth-image lifecycle).
#[allow(unused_imports)]
use super::frame_driver::Renderer;
#[allow(unused_imports)]
use super::scene_types::{DepthImage, Scene};

/// The per-extent on-screen G-buffer targets for [`Renderer::render_gbuffer_frame`]:
/// the D32 depth image (rasterize into + sample), the MRT storage G-buffer (albedo /
/// normal / material), and the two descriptor sets bound against them (the marcher
/// vocabulary set + the present-sample set). (Re)allocated ONLY on an extent change
/// by [`GBufferTargets::sync_gbuffer`] — NEVER per frame.
///
/// This is the renderer-owned counterpart of the [`Scene`]'s [`DepthImage`] (the
/// per-extent depth), generalized to the full image-based G-buffer + its descriptor
/// sets. Owned by value; torn down through [`GBufferTargets::destroy`].
///
/// # The descriptor sets are written ONCE per extent (NO per-frame update)
///
/// `vocab_set` binds {SSBO, sampled depth, albedo/normal/material storage, camera
/// UBO, P4b tiles SSBO, M1 pointer-grid SSBO} and `present_set` binds {ALBEDO combined-image-sampler}; both are written at
/// `create_bind_group` time inside `sync_gbuffer` and reused unchanged across every
/// frame at that extent. The recorder records NO `vkUpdateDescriptorSets` — only the
/// per-frame barriers + bind + dispatch + draw. On an extent change `sync_gbuffer`
/// waits the device idle, destroys the old targets, and rebuilds them (the same
/// belt-and-braces [`Scene::sync_depth`] uses).
pub struct GBufferTargets {
    /// The D32_SFLOAT depth image RING (one per in-flight frame): DEPTH_STENCIL_ATTACHMENT
    /// (rasterize into) | SAMPLED (the marcher's `.Load`). Re-`UNDEFINED`'d every frame by
    /// the recorder. RINGED so frame N+1 rasterizes into `depth[1]` while frame N's resolve
    /// still reads `depth[0]` — the lock-free cross-frame Write-After-Read fix (the per-slot
    /// `in_flight` fence already frees a slot's previous image before reuse; slot `i`'s
    /// vocab/resolve/ssao set @i binds `[i]`). A static scene fills every slot identically.
    pub(crate) depth: [VulkanTexture; FRAMES_IN_FLIGHT],
    /// The ALBEDO storage image RING (R8G8B8A8): the marcher's FINAL composite sink; also
    /// sampled by the present-blit (pass C). Render P5-r0: it additionally carries
    /// `COLOR_ATTACHMENT` usage — the mesh raster pass A writes it as MRT@0. RINGED (see
    /// [`Self::depth`]).
    pub(crate) albedo: [VulkanTexture; FRAMES_IN_FLIGHT],
    /// The NORMAL storage image RING (R8G8B8A8): the PBR MVP-2 marcher's `(oct.x, oct.y,
    /// matid_lo, matid_hi)` attribute — the octahedral world normal in RG + the 16-bit
    /// material id in BA. NOW READ by the deferred resolve (STORAGE, GENERAL). RINGED (see
    /// [`Self::depth`]).
    pub(crate) normal: [VulkanTexture; FRAMES_IN_FLIGHT],
    /// The MATERIAL storage image RING (R8G8B8A8): the PBR MVP-2 marcher's `(shadow, ao,
    /// mask)` attribute, consumed by the deferred resolve (STORAGE, GENERAL — never sampled).
    /// RINGED (see [`Self::depth`]).
    pub(crate) material: [VulkanTexture; FRAMES_IN_FLIGHT],
    /// The LIT storage image RING (R8G8B8A8): the deferred resolve's OUTPUT (STORAGE store);
    /// also SAMPLED by the present-blit (pass C). The deferred split added it — the
    /// present now samples THIS (not albedo). RINGED (see [`Self::depth`]); slot `i`'s
    /// `present_set` @i binds `lit[i]`, so the present samples the slot the resolve just wrote.
    pub(crate) lit: [VulkanTexture; FRAMES_IN_FLIGHT],
    /// The Lighting-L0b `gViewT` lane RING (R32_SFLOAT STORAGE): the marcher stores the
    /// surface ray param `t`, the deferred resolve reads it (under `mask == 1`) to reconstruct
    /// `P = ro + rd * t`. Bound as an OUTPUT on the vocab set (binding 8) and an INPUT on
    /// the resolve set (binding 7). Transitioned UNDEFINED→GENERAL with the other G-buffer
    /// images and joins the marcher store → resolve load barrier. RINGED (see [`Self::depth`]).
    pub(crate) viewt: [VulkanTexture; FRAMES_IN_FLIGHT],
    /// The Render P7 SSAO term `gSsao` RING (R8_UNORM STORAGE): the per-pixel HBAO-lite
    /// ambient occlusion the (C2) SSAO pass writes and the deferred resolve reads under the
    /// `ssao_mode != 0` gate. Bound as an INPUT on the resolve set (binding 11). ALWAYS
    /// allocated (the resolve descriptor interface is stable regardless of `ssao_mode`).
    /// Layout: the frame graph's resolve pass declares an UNCONDITIONAL read (the T6a `pbr`
    /// first-touch pattern — `declare_deferred_graph`'s seeded `ssao`), so an SSAO-off frame
    /// still derives a discard-legal UNDEFINED→GENERAL transition that keeps the
    /// statically-referenced descriptor's layout valid (VUID-vkCmdDispatch-None-09600); the
    /// resolve never dynamically reads the discarded contents under `ssao_mode == 0`, so the
    /// PIXELS stay byte-identical. RINGED (see [`Self::depth`]).
    pub(crate) ssao: [VulkanTexture; FRAMES_IN_FLIGHT],
    /// Textured-PBR T6a: the `gPbr` deferred-resolve MRT lane RING (`R16G16B16A16_SFLOAT`
    /// STORAGE|COLOR_ATTACHMENT): `r`=metallic, `g`=roughness, `b`=AO-texture modulation,
    /// `a`=emissive-strength modulation. UNCONDITIONAL (both feature legs) but bound at the
    /// SOFTWARE resolve set ONLY, binding 19 (the C1 fix — `RESOLVE_SOFTWARE_BINDINGS` itself
    /// stays 19; `gPbr` is appended past it, never entering any HWRT-consumed array). T6a:
    /// UNWRITTEN (no raster pass names it yet — T6c's textured raster adds the 4th MRT write);
    /// the resolve's `.Load` is INSIDE the flag-gated branch, so a flag=0 material never reads it.
    /// RINGED (see [`Self::depth`]).
    pub(crate) pbr: [VulkanTexture; FRAMES_IN_FLIGHT],
    /// Rung 3a: the RT soft-shadow VISIBILITY target `shadow_vis` RING (`R16G16_UNORM` STORAGE,
    /// full-res): `R` = the per-pixel mesh visibility the VIS pass writes, `G` = the validity mask.
    /// SAME format as [`Self::shadow_vis2`] (the uniform-RG16 ping-pong — one `"rg16"` shader pin
    /// fits every binding on every parity). RINGED per-FIF (the cross-frame WAR fix, like
    /// [`Self::ssao`]) so the à-trous denoise reads/writes the slot the VIS pass just wrote.
    /// `Option`-guarded: `Some` when the device advertises `RG16` UNORM storage
    /// ([`crate::device::DeviceCaps::shadow_denoise_storage_ok`]), `None`
    /// otherwise — the DDGI-degrade discipline (the denoise is opt-in, `feature = "hwrt"` + config
    /// `Spatial`; a device missing the format degrades it to disabled, never a boot fault). No pass
    /// reads it yet (steps 4-6 add the VIS / à-trous passes) — allocated-but-unused this step, so
    /// the render is byte-identical. `#[cfg(feature = "hwrt")]`, so a `not(hwrt)` build lacks it.
    #[cfg(feature = "hwrt")]
    pub(crate) shadow_vis: Option<[VulkanTexture; FRAMES_IN_FLIGHT]>,
    /// Rung 3a: the à-trous ping-pong target `shadow_vis2` RING (`R16G16_UNORM` STORAGE, full-res) —
    /// 16-bit precision avoids the cumulative 8-bit rounding of a multi-level filter. RINGED +
    /// `Option`-guarded exactly like [`Self::shadow_vis`] (allocated together on the same
    /// `shadow_denoise_storage_ok()` predicate; both `None` on an unsupported device). No pass reads
    /// it yet (steps 4-6 add the à-trous ping-pong). `#[cfg(feature = "hwrt")]`.
    #[cfg(feature = "hwrt")]
    pub(crate) shadow_vis2: Option<[VulkanTexture; FRAMES_IN_FLIGHT]>,
    /// HW-RT Rung 3b: the temporal motion-vector target `motion_vec` RING (`R16G16_SFLOAT`, full-
    /// res): screen-space Δuv (prev − cur), written by the raster gbuffer MV MRT (mesh) + the
    /// marcher (SDF) in step 5, read by the temporal reproject pass in step 6. RINGED per-FIF +
    /// `Option`-guarded exactly like [`Self::shadow_vis`] (built together on the same
    /// `shadow_denoise_storage_ok()` probe, degrade-to-`None` on any create failure). No pass reads
    /// it yet — allocated-but-unused this step, byte-identical. `#[cfg(feature = "hwrt")]`.
    #[cfg(feature = "hwrt")]
    pub(crate) motion_vec: Option<[VulkanTexture; FRAMES_IN_FLIGHT]>,
    /// HW-RT Rung 3b: the temporal shadow-vis HISTORY ring `shadow_temporal_hist`
    /// (`R16G16B16A16_UNORM`, full-res): frame `fi` writes `[fi]` (vis, conf, prev-depth, _) and
    /// reads `[1-fi]` — the cross-frame accumulate, seeded GENERAL in the graph (the DDGI
    /// precedent). RINGED + `Option`-guarded like [`Self::motion_vec`]. No pass reads it yet (step 6
    /// adds the temporal pass). `#[cfg(feature = "hwrt")]`.
    #[cfg(feature = "hwrt")]
    pub(crate) shadow_temporal_hist: Option<[VulkanTexture; FRAMES_IN_FLIGHT]>,
    /// HW-RT Rung 3b: the temporal-accumulate OUTPUT `temporal_out` RING (`R16G16_UNORM`, full-res)
    /// — the accumulated visibility the DENOISED resolve reads at `gShadowVis` @21 when temporal is
    /// on. A DEDICATED target (avoids the in-place neighborhood-read race). RINGED + `Option`-
    /// guarded like [`Self::motion_vec`]. No pass reads it yet (step 6). `#[cfg(feature = "hwrt")]`.
    #[cfg(feature = "hwrt")]
    pub(crate) temporal_out: Option<[VulkanTexture; FRAMES_IN_FLIGHT]>,
    /// The marcher vocabulary descriptor set RING (one per in-flight frame), each written
    /// ONCE against [`GBufferScene::vocab_layout`] (pointing at `depth`/`albedo`/`normal`/
    /// `material` + the scene's SSBO/UBO/sampler + the M1 `pointer_grid` SSBO @9). Slot `i`
    /// binds `scene.camera_ring[i]` at the camera UBO @5 — the lock-free per-frame ring fix;
    /// every other binding is identical across slots. The recorder selects
    /// `vocab_set[self.frame_index]`. NO per-frame `vkUpdateDescriptorSets`.
    pub(crate) vocab_set: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// The PBR MVP-2 RESOLVE descriptor set, written ONCE against
    /// [`GBufferScene::resolve_layout`] (10 bindings: `albedo` @0, `normal` @1, `material`
    /// @2, `lit` @3 STORAGE images, the material SSBO @4, the camera UBO @5, the L0a light
    /// table SSBO @6, the L0b `gViewT` STORAGE image @7, the L1 `ClusterGrid` SSBO @8, the L1
    /// `LightIndexList` SSBO @9, the P6 R1 SDF edit-list `Buf` SSBO @10, the Render P7 SSAO
    /// term `gSsao` STORAGE image @11). When L1 is off the scene's `cluster_grid`/`light_index`
    /// are `None`, so @8/@9 bind the light table as a harmless valid placeholder.
    ///
    /// WHICH BOOTS CAN READ IT AT ALL — the question that comes before "which term gates the
    /// read". This set is consumed by exactly one shader, `deferred_pbr.comp`, and it is bound at
    /// exactly two sites (`passes/gbuffer.rs`'s two `targets.resolve_set[self.frame_index]`
    /// arguments — the software triple and its `not(hwrt)` twin), both inside
    /// `Renderer::record_gbuffer`. `render_gbuffer_frame` (`present/frame_driver.rs`) reaches
    /// `record_gbuffer` only in the `else` arm of its `path_is_vb()` / `path_is_forward()`
    /// three-way — "the three are mutually exclusive per boot". So this set is bound on DEFERRED
    /// boots and on no other: on a `Forward`/`ForwardPlus`/`VisibilityBuffer` boot it is built and
    /// written but never bound, and nothing reads @8/@9 there. In particular, a VB boot that armed
    /// `clusters_enabled` but built no cull binds NO `ClusterGrid` reader anywhere — its
    /// `vb_set0_froxel` is `None` (that builder demands the REAL `cluster_grid`/`light_index`,
    /// with no placeholder fallback) and `record_vb` then selects the base, non-`FROXEL`
    /// `vb_resolve`/`vb_shade`, which declare no `ClusterGrid` at all. See
    /// [`GBufferScene::cluster_cull`]'s doc for that boot in full.
    ///
    /// On the Deferred boots that DO read it, the gate is `deferred_pbr.hlsl`'s `use_clusters` —
    /// THREE terms since VB-P1k: `clusters_enabled != 0 && cluster_count != 0 && cluster_count <=
    /// grid_capacity`, the capacity coming from `ClusterGrid.GetDimensions(...)`, i.e. the BOUND
    /// descriptor's own element count (SPIR-V `OpArrayLength`) rather than a host-side mirror of
    /// it. Which term actually decides:
    ///
    /// * On the DEFAULT boot — every golden, and every scene that leaves `EnginePlugins`'s
    ///   `LightingConfig::default()` seed alone — `LightingConfig::clusters_enabled` is `false`
    ///   and `LightHeaderGpu::new` packs it verbatim, so the FIRST term short-circuits: the
    ///   ENABLED BIT is what takes the flat branch here.
    /// * Only a Deferred boot that explicitly sets `clusters_enabled = true` gets past that term,
    ///   and there the DIMS term decides. `ResolvedRenderPath::froxel_light_cull` is
    ///   `clusters_enabled && path == VisibilityBuffer` (no geometry-leg term), hence `false` on
    ///   EVERY Deferred boot, and `sync_cluster_light_gate` therefore publishes a dims lane of
    ///   `0` — the same all-zero lane `LightHeaderGpu::new` hardcoded pre-VB-P1b-0.
    /// * The CAPACITY term consequently never decides on a host-booted Deferred frame, because
    ///   the bullet above pins the dims to `0` there. It is defence in depth against a
    ///   nonzero-dims header reaching this shader; today only a direct-RHI harness builds one
    ///   (`GoldenLightHeader::new_clustered`, `tests/sdf_gbuffer_hybrid.rs`).
    ///
    /// The two terms past the enabled bit are an out-of-bounds guard, not a style choice:
    /// `robustBufferAccess` is OFF in this engine and no GPU-assisted validation runs, so an
    /// out-of-range `ClusterGrid` read is real UB that no layer would report.
    /// `gSsao` @11 is always bound; the resolve reads it only under
    /// `ssao_mode != 0` (0 every pre-P7 scene). @12/@13 =
    /// the CSM cascade combined-image + UBO; @14/@15 = the punctual shadow-atlas combined-image +
    /// UBO; @16/@17/@18 = the SDFDDGI probe irradiance + depth combined images + the `ResolvedDdgi`
    /// grid UBO (all bound-but-unread when their header gate is 0). Textured-PBR T6a: `gPbr`
    /// STORAGE image @19 (SOFTWARE-ONLY — the C1 fix; never entering any HWRT-consumed array),
    /// bound-but-unread when the flag-gated branch is dead (every current material). The software
    /// set is EXACT-FILL at `RESOLVE_SOFTWARE_TOTAL_BINDINGS` (20), under the cap of
    /// `MAX_BIND_GROUP_BINDINGS` (24). NO per-frame update.
    ///
    /// A RING (one per in-flight frame): slot `i` binds `scene.camera_ring[i]` @5 +
    /// `scene.csm_cascade_ring[i]` @13 — the lock-free per-frame ring fix; every other binding is
    /// identical across slots. The recorder selects `resolve_set[self.frame_index]`.
    pub(crate) resolve_set: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// R2a-4b: the HWRT-variant RESOLVE descriptor set RING (one per in-flight frame), written
    /// ONCE against [`GBufferScene::resolve_layout_hwrt`] — the 19 software bindings PLUS binding
    /// 19 (`AccelerationStructure`) fed slot `i`'s persistent TLAS
    /// ([`GBufferScene::resolve_tlas_hwrt`]`[i].accel`). `None` on EVERY software path (non-hwrt /
    /// non-RT / config-Software) ⇒ the recorder binds the 19-binding [`Self::resolve_set`] against
    /// the software pipeline ⇒ byte-identical to the golden. `Some(_)` (built iff the scene wires
    /// [`GBufferScene::resolve_pipeline_hwrt`]) is selected as part of the `(pipeline, layout, set)`
    /// TRIPLE at the record-site when routing is Hardware. RINGED like [`Self::resolve_set`]; the
    /// per-FIF TLAS handle is frame-stable, so the once-per-FIF write model holds. NO per-frame
    /// update. The whole field is `#[cfg(feature = "hwrt")]`, so a `not(hwrt)` build has it absent.
    #[cfg(feature = "hwrt")]
    pub(crate) resolve_set_hwrt: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// The Lighting-L1 CULL descriptor set, written ONCE against
    /// [`GBufferScene::cull_layout`] (camera UBO @0, light table SSBO @1, `ClusterGrid` SSBO
    /// @2, `LightIndexList` SSBO @3, `LightIndexAlloc` SSBO @4) — `None` when L1 is off
    /// ([`GBufferScene::cluster_cull`] is `None`). NO per-frame update.
    ///
    /// A RING when `Some` (one per in-flight frame): slot `i` binds `scene.camera_ring[i]` @0 — the
    /// lock-free per-frame ring fix. The recorder selects `cull_set[self.frame_index]`.
    pub(crate) cull_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// VG rung R2c0: the per-BATCH draw-record cull descriptor set, written ONCE against
    /// [`GBufferScene::vb_cull_layout`] (`VbIndirect` @0, `VbBatchDesc` @1, `VbCullVisible` @2,
    /// `VbCullCount` @3, plus rung R2d-2's `gVbInstances` @4 / `gMeshBounds` @5 /
    /// `gVbVisibleInstance` @6 — all COMPUTE STORAGE_BUFFER). NO per-frame update.
    ///
    /// `None` unless the R2c0 arm is wired AND [`GBufferScene::vb_mesh_bounds`] is armed — i.e.
    /// `None` on every Deferred / Forward / Forward+ / `VisibilityBuffer × Sdf` boot, which is
    /// the same set of boots on which `record_vb`/`declare_vb_graph` leave `batch_cull_armed`
    /// false. The two conditions are ONE predicate by construction; see the build site.
    ///
    /// A RING when `Some`: every buffer but `gMeshBounds` is per-FIF, so slot `i` binds each of
    /// those buffers' own `[i]` (`gMeshBounds` is one boot-lived table, bound identically in every
    /// slot). The recorder selects `vb_cull_set[self.frame_index]`.
    pub(crate) vb_cull_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// The Render P7 SSAO descriptor set, written ONCE against [`SsaoActivation::layout`]
    /// (5 bindings: gNormal @0, gMaterial @1, gViewT @2 STORAGE images READ, the `ssao` out
    /// STORAGE image @3 WRITE, the camera UBO @4) — `None` when SSAO is off
    /// ([`GBufferScene::ssao`] is `None`). The recorder then skips the SSAO pass entirely (the
    /// 0%-gate, byte-identical command stream). NO per-frame update.
    ///
    /// A RING when `Some` (one per in-flight frame): slot `i` binds `scene.camera_ring[i]` @4 — the
    /// lock-free per-frame ring fix. The recorder selects `ssao_set[self.frame_index]`.
    pub(crate) ssao_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Multi-paradigm render-path plan, rung R3b: the `viewt_from_depth` descriptor set, written
    /// ONCE against [`ViewtFromDepthActivation::layout`] (2 bindings: SAMPLED depth @0, STORAGE
    /// `gViewT` @1) — `None` unless [`GBufferScene::viewt_from_depth`] is armed (`Deferred ×
    /// Mesh`). The recorder then skips the pass entirely (the 0%-gate, byte-identical
    /// command stream under every other leg). NO per-frame update.
    ///
    /// A RING when `Some` (one per in-flight frame): slot `i` binds `core.depth[i]`/`core.viewt[i]`
    /// — the SAME per-FIF images the marcher's vocab set / `ssao_set` bind. The recorder selects
    /// `viewt_from_depth_set[self.frame_index]`.
    pub(crate) viewt_from_depth_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// TAA-under-VB: the `viewt_from_depth_rz` descriptor set, written ONCE against
    /// [`ViewtFromVbDepthActivation::layout`] (3 bindings: SAMPLED depth @0, STORAGE `gViewT` @1,
    /// UNIFORM camera @2) — `None` unless [`GBufferScene::viewt_from_vb_depth`] is armed
    /// (`VisibilityBuffer × Mesh` with TAA on). The recorder then skips the pass entirely (the
    /// 0%-gate, byte-identical command stream under every other leg / with TAA off). NO
    /// per-frame update.
    ///
    /// A RING when `Some` (one per in-flight frame): slot `i` binds
    /// `forward.depth[i]`/`core.viewt[i]`/`scene.camera_ring[i]` — UNLIKE [`Self::viewt_from_depth_set`],
    /// which binds `core.depth[i]` (the Deferred custom-linear ring), this binds
    /// [`ForwardTargets::depth`]'s reverse-Z ring (VB rasterizes into the SAME depth image the
    /// forward/hwrt legs share). The recorder selects `viewt_from_vb_depth_set[self.frame_index]`.
    pub(crate) viewt_from_vb_depth_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// The SSAO à-trous denoise chain's interior ping-pong ring RING `ssao_ring_a`
    /// (`R16_UNORM` STORAGE, full-res) — mirrors `shadow_vis`'s per-FIF ringing (the cross-frame
    /// WAR fix, like [`Self::ssao`]). `Option`-guarded: `Some` when the device advertises
    /// `R16_UNORM` STORAGE ([`crate::device::DeviceCaps::ssao_atrous_storage_ok`]), `None`
    /// otherwise — the DDGI/shadow-denoise degrade discipline (opt-in, a device missing the format
    /// degrades to the raw un-denoised gather, never a boot fault). UNCONDITIONAL (both feature
    /// legs — SOFTWARE, NOT `hwrt`-gated).
    pub(crate) ssao_ring_a: Option<[VulkanTexture; FRAMES_IN_FLIGHT]>,
    /// The SSAO à-trous denoise chain's SECOND interior ping-pong ring `ssao_ring_b` — SAME
    /// format/degrade policy as [`Self::ssao_ring_a`] (built together, `None` together).
    pub(crate) ssao_ring_b: Option<[VulkanTexture; FRAMES_IN_FLIGHT]>,
    /// Rung R9b (docs/R9-VB-SPLIT-PLAN.md §4): the VB split's `thin_normal` thin-aux RING
    /// (`R8G8B8A8_UNORM` STORAGE: oct normal RG + material-scalar roughness B — the plan's
    /// no-matcache contract; NEVER a mesh albedo/material cache). Written by `vb_geo`
    /// (first-touch UNDEFINED→GENERAL every frame), read by the `-D VB_THIN` SSAO gather.
    /// `Some` iff the BOOT-frozen `mesh_geo_shade_split` armed (a fused/non-VB boot allocates
    /// nothing — the 0%-gate). Per-FIF RINGED (the cross-frame-WAR policy, like
    /// [`Self::viewt`]).
    pub(crate) thin_normal: Option<[VulkanTexture; FRAMES_IN_FLIGHT]>,
    /// Rung R9b: `vb_geo`'s Set-1 aux descriptor RING (`thin_normal[i]` @0 W; @1 = the R9d
    /// motion slot, placeholder-bound to `thin_normal[i]` — same-type inert, the R2 idiom; @2 =
    /// the R9d MotionCam slot, placeholder-bound to `camera_ring[i]`). `Some` iff the split
    /// armed AND the `thin_normal` ring allocated.
    pub(crate) vb_geo_aux_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Rung R9b: the VB `-D VB_THIN` SSAO gather's dense 4-binding descriptor RING
    /// (`thin_normal[i]` @0, `viewt[i]` @1, `ssao[i]` @2 W, `camera_ring[i]` @3). `Some` iff
    /// the split armed (covers every split config — the gather itself is `path_vb_ssao`-gated
    /// at record).
    pub(crate) vb_ssao_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Rung R9b: `vb_shade_split`'s Set-1 descriptor RING against
    /// [`GBufferScene::vb_split_layout1`] — @0-3 the shadow vocab (the `ForwardTargets::set1`
    /// sources verbatim), @4 `ssao[i]`, @5/@6 the DDGI combined atlases, @7 the `ResolvedDdgi`
    /// UBO (all DDGI entries always bound, sampled only under `ddgi_mode != 0` — the GI-off
    /// 0%-gate the deferred resolve set establishes). `Some` iff the split armed.
    pub(crate) vb_split_set1: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Rung R9d: the VB split's dedicated shadow-vis gather descriptor set RING, written against
    /// [`GBufferScene::vb_shadow_vis_layout`] (7 bindings: `thin_normal[i]` @0, `viewt[i]` @1,
    /// `light_table` @2, the camera UBO @3, the TLAS `AccelerationStructure` @4
    /// (`scene.resolve_tlas_hwrt[i]`), the `ResolvedRayShadow` UBO @5 (`scene.ray_shadow_ubo[i]`),
    /// `shadow_vis[i]` @6 (WRITE)). `Some` iff the split armed AND the boot hwrt gate built
    /// [`GBufferScene::vb_shadow_vis_pipeline`] AND every bound resource exists.
    #[cfg(feature = "hwrt")]
    pub(crate) vb_shadow_vis_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Rung R9d: the VB split's per-level à-trous denoise descriptor sets — `sets[level][fi]`,
    /// mirroring [`Self::shadow_atrous_sets`] but binding `thin_normal[fi]` at the `gNormal` slot
    /// and the SPLIT's own `viewt[fi]` (reusing the SAME [`GBufferScene::atrous_layout_denoise_hwrt`]
    /// layout object the deferred chain shares — a stable, per-path-agnostic bind-group shape).
    /// `Some` iff the split armed AND the deferred boot à-trous pipeline/layout + the
    /// `shadow_vis`/`shadow_vis2` rings exist.
    #[cfg(feature = "hwrt")]
    pub(crate) vb_shadow_atrous_sets:
        Option<[[VulkanBindGroup; FRAMES_IN_FLIGHT]; crate::present::MAX_ATROUS_LEVELS as usize]>,
    /// Rung R9d: the VB split's temporal reproject descriptor set RING, mirroring
    /// [`Self::shadow_temporal_set`] but binding `viewt[fi]` at the `gViewT` slot (reusing the
    /// SAME [`GBufferScene::temporal_layout`] layout object). `Some` iff the split armed AND the
    /// deferred boot temporal pipeline/layout + every ringed input exist.
    #[cfg(feature = "hwrt")]
    pub(crate) vb_shadow_temporal_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// The SSAO à-trous denoise chain's `level == 0` descriptor set RING (`gAoIn` @0 = the frozen
    /// R8 `gSsao[fi]` endpoint, `gAoOut` @1 = `ssao_ring_a[fi]`, `gViewT` @2 = `viewt[fi]`, the
    /// camera UBO @3 = `scene.camera_ring[fi]`), written against
    /// [`SsaoActivation::atrous_layout`]. Selected by [`crate::present::AtrousStepRole::Read8`]
    /// ([`crate::present::ssao_atrous_step`]). `None` in lock-step with [`Self::ssao_ring_a`].
    pub(crate) ssao_atrous_read8_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// The SSAO à-trous denoise chain's INTERIOR descriptor set RING reading `ssao_ring_a`
    /// (`gAoIn` @0 = `ssao_ring_a[fi]`, `gAoOut` @1 = `ssao_ring_b[fi]`). Selected by
    /// [`crate::present::AtrousStepRole::Interior`]`{ in_ring: 0 }`. `None` in lock-step with
    /// [`Self::ssao_ring_a`].
    pub(crate) ssao_atrous_interior_from0_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// The SSAO à-trous denoise chain's INTERIOR descriptor set RING reading `ssao_ring_b`
    /// (`gAoIn` @0 = `ssao_ring_b[fi]`, `gAoOut` @1 = `ssao_ring_a[fi]`). Selected by
    /// [`crate::present::AtrousStepRole::Interior`]`{ in_ring: 1 }`. `None` in lock-step with
    /// [`Self::ssao_ring_a`].
    pub(crate) ssao_atrous_interior_from1_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// The SSAO à-trous denoise chain's LAST-level descriptor set RING reading `ssao_ring_a`
    /// (`gAoIn` @0 = `ssao_ring_a[fi]`, `gAoOut` @1 = the frozen R8 `gSsao[fi]` endpoint — the
    /// write-BACK the resolve reads). Selected by
    /// [`crate::present::AtrousStepRole::Write8`]`{ in_ring: 0 }`. `None` in lock-step with
    /// [`Self::ssao_ring_a`].
    pub(crate) ssao_atrous_write8_from0_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// The SSAO à-trous denoise chain's LAST-level descriptor set RING reading `ssao_ring_b`
    /// (`gAoIn` @0 = `ssao_ring_b[fi]`, `gAoOut` @1 = the frozen R8 `gSsao[fi]` endpoint).
    /// Selected by [`crate::present::AtrousStepRole::Write8`]`{ in_ring: 1 }`. `None` in lock-step
    /// with [`Self::ssao_ring_a`].
    pub(crate) ssao_atrous_write8_from1_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Anti-aliasing Stage 1/3: the FXAA/SSAA post-process OUTPUT image RING (one per
    /// in-flight frame), `COLOR_ATTACHMENT | SAMPLED`, `R8G8B8A8_UNORM` (== [`GBUFFER_FORMAT`]).
    /// `None` when AA is off ([`GBufferScene::aa`]/`smaa`/`ssaa` are all `None`), the
    /// 0%-gate: `present_set` then samples `lit` and no AA pass is recorded.
    ///
    /// **Sizing**: `present_extent` for `Fxaa`/`Smaa` (== the FXAA/SMAA output resolution).
    /// Under `Ssaa` it is instead sized to the NATIVE `aa_extent` — `present_extent` is 2×
    /// under SSAA (the whole G-buffer/lit renders at 2×), and the downsample pass resolves
    /// that 2× `lit` into this native `aa_out`, so the present-blit's unchanged 1:1 crop
    /// samples native pixels directly (no top-left-quarter crop). Off/Fxaa/Smaa keep
    /// `aa_extent == present_extent` (byte-identical sizing to before SSAA existed).
    ///
    /// `aa_out`/`fxaa_set`/`downsample_set` carry NO material table and NO acceleration
    /// structure — deliberately OUT of
    /// [`Self::material_set_rings`]/[`Self::tlas_accel_sets`] (F7/task#11 enumerations).
    pub(crate) aa_out: Option<[VulkanTexture; FRAMES_IN_FLIGHT]>,
    /// Anti-aliasing Stage 1: the FXAA INPUT descriptor set RING (one per in-flight frame),
    /// each a single `CombinedImageSampler` binding `lit[i]` (never `aa_out`) + the LINEAR/
    /// ClampToEdge [`AaActivation::sampler`](crate::present::scene_types::AaActivation::sampler)
    /// against [`GBufferScene::present_layout`]. `None` when AA is off.
    pub(crate) fxaa_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Anti-aliasing Stage 2: the SMAA `edges` output image RING (one per in-flight frame),
    /// `COLOR_ATTACHMENT | SAMPLED`, `R8G8_UNORM`, sized to `present_extent` — `None` when
    /// SMAA is off ([`GBufferScene::smaa`] is `None`).
    ///
    /// `smaa_edges`/`smaa_weights`/the three `smaa_*_set` rings carry NO material table and
    /// NO acceleration structure — deliberately OUT of
    /// [`Self::material_set_rings`]/[`Self::tlas_accel_sets`] (F7/task#11 enumerations), the
    /// same exclusion `aa_out`/`fxaa_set` carry.
    pub(crate) smaa_edges: Option<[VulkanTexture; FRAMES_IN_FLIGHT]>,
    /// Anti-aliasing Stage 2: the SMAA `weights` output image RING (one per in-flight frame),
    /// `COLOR_ATTACHMENT | SAMPLED`, `R8G8B8A8_UNORM`, sized to `present_extent` — `None` when
    /// SMAA is off.
    pub(crate) smaa_weights: Option<[VulkanTexture; FRAMES_IN_FLIGHT]>,
    /// Anti-aliasing Stage 2: pass 1's INPUT descriptor set RING (one per in-flight frame),
    /// one `CombinedImageSampler` binding `lit[i]` against [`GBufferScene::present_layout`]
    /// (the same 1-CIS layout `fxaa_set` uses). `None` when SMAA is off.
    pub(crate) smaa_edge_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Anti-aliasing Stage 2: pass 2's INPUT descriptor set RING, 3 `CombinedImageSampler`s
    /// binding `{ smaa_edges[i] @0, area_tex @1, search_tex @2 }` against
    /// [`SmaaActivation::weight_layout`](crate::present::scene_types::SmaaActivation::weight_layout).
    /// `None` when SMAA is off.
    pub(crate) smaa_weight_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Anti-aliasing Stage 2: pass 3's INPUT descriptor set RING, 2 `CombinedImageSampler`s
    /// binding `{ lit[i] @0, smaa_weights[i] @1 }` against
    /// [`SmaaActivation::blend_layout`](crate::present::scene_types::SmaaActivation::blend_layout).
    /// `None` when SMAA is off.
    pub(crate) smaa_blend_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Anti-aliasing Stage 3: the SSAA downsample INPUT descriptor set RING (one per
    /// in-flight frame), each a single `CombinedImageSampler` binding `lit[i]` (the 2× ring
    /// slot; never `aa_out`) + the NEAREST/ClampToEdge
    /// [`SsaaActivation::sampler`](crate::present::scene_types::SsaaActivation::sampler)
    /// (ignored by the shader's `.Load`) against [`GBufferScene::present_layout`] — the same
    /// 1-CIS shape [`Self::fxaa_set`] uses. `None` when SSAA is off.
    pub(crate) downsample_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Anti-aliasing Stage 4 (TAA W4/W5): the color-history PING-PONG ring `taa_hist`
    /// (`R16G16B16A16_SFLOAT`, full-res, STORAGE), parity-indexed like
    /// [`Self::depth`]/[`Self::lit`]/etc (`FRAMES_IN_FLIGHT == 2`, hard-asserted at the build
    /// site — the SAME ping-pong discipline [`Self::shadow_temporal_hist`] uses). BOOT-CLEARED
    /// `UNDEFINED → GENERAL` at build time (the M2 fix — mirrors
    /// [`Self::build_and_clear_shadow_temporal_hist`]'s discipline: the framegraph's `taa_hist`
    /// seed assumes a REAL `GENERAL` layout, not a fresh `UNDEFINED` image, on the first
    /// cross-frame read). `record_taa` reads `taa_hist[1-fi]` (the cross-frame history) and
    /// writes `taa_hist[fi]` each frame it runs. `None` when TAA is off (`GBufferScene::taa` is
    /// `None`) — the 0%-gate: no allocation, byte-identical to every other `AaArm`. RGBA16F (not
    /// RGBA8) avoids per-blend re-quantization of the already-8-bit-post-tonemap `lit` across
    /// many accumulation frames.
    pub(crate) taa_hist: Option<[VulkanTexture; FRAMES_IN_FLIGHT]>,
    /// Anti-aliasing Stage 4 (TAA W5) + rung T2: the resolve's OWN tunables UBO ring (48 B
    /// `HostVisibleCoherent` per FIF slot, zero-seeded, mirrors [`Self::temporal_shadow_ubo`]) —
    /// `ResolvedTaa`'s `default_blend`/`min_blend`/`variance_gamma` plus the T2 mode words.
    /// `None` when TAA is off.
    pub(crate) taa_ubo: Option<[BoundBuffer; FRAMES_IN_FLIGHT]>,
    /// Anti-aliasing Stage 4 (TAA W5): the resolve's OWN DEDICATED `MotionCam` UBO ring (128 B
    /// `HostVisibleCoherent` per FIF slot, zero-seeded) — SEPARATE from the hwrt mesh-shadow
    /// `motion_cam_ubo` (see [`TaaActivation`](crate::present::scene_types::TaaActivation)'s "why
    /// a dedicated ring" doc). `None` when TAA is off.
    pub(crate) taa_motion_cam_ubo: Option<[BoundBuffer; FRAMES_IN_FLIGHT]>,
    /// Anti-aliasing Stage 4 (TAA W5): the temporal-resolve descriptor set RING (one 8-binding set
    /// per in-flight frame), written ONCE against
    /// [`TaaActivation::resolve_layout`](crate::present::scene_types::TaaActivation::resolve_layout).
    /// Slot `fi` binds `gLit` @0 = `lit[fi]` (+ the LINEAR sampler), `gViewT` @1 = `viewt[fi]`,
    /// `gHistIn` @2 = `taa_hist[1-fi]` (the cross-frame READ), `gHistOut` @3 = `taa_hist[fi]` (the
    /// WRITE), `gAaOut` @4 = `aa_out[fi]`, the `ResolvedTaa` UBO @5 = `taa_ubo[fi]`, the camera
    /// UBO @6 = `scene.camera_ring[fi]` (UNJITTERED), the `MotionCam` UBO @7 =
    /// `taa_motion_cam_ubo[fi]`. `None` when TAA is off. The recorder selects
    /// `taa_resolve_set[self.frame_index]`.
    pub(crate) taa_resolve_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// TAA rung T3: the RCAS-intermediate STORAGE image ring `taa_resolved` (`R8G8B8A8_UNORM`
    /// == [`GBUFFER_FORMAT`], `ImageUsage::STORAGE` only — never `SAMPLED`/`COLOR_ATTACHMENT`:
    /// this image is NEVER read by a fragment shader nor a render-pass attachment, only ever a
    /// compute STORAGE read/write). Sized to `aa_extent` (== `aa_out`'s size). `Some` iff
    /// [`GBufferScene::rcas`] is armed: the TAA resolve's `gAaOut` @4 binding is re-pointed here
    /// instead of [`Self::aa_out`] (see [`Self::build_taa_resolve_set`]'s call site in
    /// [`Self::create`]), and [`crate::present::passes::rcas`]'s `gRcasIn` @0 reads it, writing
    /// the FINAL sharpened result into [`Self::aa_out`] (the "ping" of the ping-pong —
    /// `rcas.comp.hlsl`'s module doc). No boot-clear needed (unlike [`Self::taa_hist`]): the
    /// resolve writes every dispatched pixel of `gAaOut` unconditionally each frame, so a fresh
    /// image's undefined initial contents are never read (mirrors [`Self::aa_out`]'s own
    /// always-fully-discarded UNDEFINED→GENERAL transition). `None` when RCAS is off (the
    /// 0%-gate — `SharpenMode::None`, the default) — the resolve writes [`Self::aa_out`]
    /// directly, byte-identical to the pre-RCAS resolve.
    pub(crate) taa_resolved: Option<[VulkanTexture; FRAMES_IN_FLIGHT]>,
    /// TAA rung T3: the RCAS descriptor set RING (one 2-binding set per in-flight frame),
    /// written ONCE against
    /// [`RcasActivation::rcas_layout`](crate::present::scene_types::RcasActivation::rcas_layout).
    /// Slot `fi` binds `gRcasIn` @0 = [`Self::taa_resolved`]`[fi]` (the READ), `gAaOut` @1 =
    /// [`Self::aa_out`]`[fi]` (the WRITE — the present-blit's input, unchanged). `None` when
    /// RCAS is off, or when [`Self::taa_resolved`]/[`Self::aa_out`] failed to allocate. The
    /// recorder selects `rcas_set[self.frame_index]`.
    pub(crate) rcas_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Which AA mode ([`AaArm`]) was armed when these targets were built.
    /// [`GBufferTargets::sync_gbuffer`] compares this against `AaArm::from_scene(scene)` and
    /// forces the same fence-safe rebuild an extent change triggers on a mismatch — a live,
    /// fence-safe runtime AA toggle across Off/Fxaa/Smaa/Ssaa/Taa.
    pub(crate) aa_arm: AaArm,
    /// HW-RT rung 3a: the VIS-variant resolve descriptor set RING (one per in-flight frame), written
    /// ONCE against [`ShadowVisActivation::resolve_layout`](crate::present::scene_types::ShadowVisActivation::resolve_layout)
    /// — the 21 RESOLVE_INLINE-hwrt bindings PLUS `gShadowVis` STORAGE image @21 fed slot `i`'s
    /// `shadow_vis[i]` (the VIS pass WRITES it). `None` unless BOTH the scene wires the denoise
    /// activation (`scene.shadow.is_some()`) AND the HWRT resolve resources exist. RINGED like
    /// [`Self::resolve_set_hwrt`]; the recorder selects `shadow_vis_resolve_set[self.frame_index]`.
    /// The whole field is `#[cfg(feature = "hwrt")]`.
    #[cfg(feature = "hwrt")]
    pub(crate) shadow_vis_resolve_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// HW-RT rung 3a: the DENOISED-variant resolve descriptor set RING (one per in-flight frame),
    /// written ONCE against the SAME 22-binding VIS/DENOISED layout — identical to
    /// [`Self::shadow_vis_resolve_set`] except `gShadowVis` @21 is fed the FINAL à-trous output
    /// (`shadow_vis[i]` when `final_is_vis2 == false`, `shadow_vis2[i]` when `true`), which the
    /// DENOISED resolve READS. `None` on the OFF path; the recorder selects
    /// `shadow_denoised_resolve_set[self.frame_index]` when routing is denoised.
    #[cfg(feature = "hwrt")]
    pub(crate) shadow_denoised_resolve_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// HW-RT rung 3a: the à-trous denoise descriptor sets — one per level (`0..MAX_ATROUS_LEVELS`),
    /// each RINGED per in-flight frame — `sets[level][fi]`. Level `i` binds `gVisIn` @0 =
    /// `i`-even ? `shadow_vis[fi]` : `shadow_vis2[fi]`, `gVisOut` @1 = the OTHER, `gNormal` @2 /
    /// `gViewT` @3 (slot `fi`), the `ResolvedShadowDenoise` UBO @4 (`shadow_denoise_ubo[fi]`), the
    /// camera UBO @5 (`scene.camera_ring[fi]`). `None` on the OFF path. Only `levels` inner rings
    /// are consumed at record time; the unused tail entries are still built (fixed per-extent cost).
    #[cfg(feature = "hwrt")]
    pub(crate) shadow_atrous_sets:
        Option<[[VulkanBindGroup; FRAMES_IN_FLIGHT]; crate::present::MAX_ATROUS_LEVELS as usize]>,
    /// HW-RT rung 3a: the à-trous edge-stop UBO RING (one 16-byte `HostVisibleCoherent` slot per
    /// in-flight frame), carrying `ResolvedShadowDenoise` (`sigma_z`/`sigma_n`, live-tunable). A
    /// RING (mirrors the rung-1b `ray_shadow_ubo` ring): each FIF frame's à-trous sets bind their
    /// own slot @4, the host writes that slot before the present, so the sibling in-flight frame
    /// reads a DIFFERENT slot (lock-free write-after-read). `None` on the OFF path.
    #[cfg(feature = "hwrt")]
    pub(crate) shadow_denoise_ubo: Option<[BoundBuffer; FRAMES_IN_FLIGHT]>,
    /// HW-RT Rung 3b step 5b: the SDF motion-vector VIS-variant resolve descriptor set RING (one per
    /// in-flight frame), written ONCE per extent against the 24-binding VIS-MV layout
    /// ([`GBufferScene::vis_mv_layout`](crate::present::scene_types::GBufferScene::vis_mv_layout)) —
    /// the SAME 22 VIS bindings as [`Self::shadow_vis_resolve_set`] (incl. `gShadowVis` @21 =
    /// `shadow_vis[i]`, the WRITE target) PLUS the `MotionCam` UBO @22 (`motion_cam_ubo[i]`) + the
    /// `motion_vec` STORAGE image @23 (`motion_vec[i]`, the SDF-Δuv WRITE target). `None` unless
    /// temporal is on AND the spatial denoise is on (so the base VIS set + `scene.shadow` exist too —
    /// `mode == Both` this rung) AND the RT + storage MV resources exist. The recorder selects
    /// `shadow_vis_mv_resolve_set[self.frame_index]` when [`GBufferScene::sdf_mv_active`](crate::present::scene_types::GBufferScene::sdf_mv_active).
    /// The whole field is `#[cfg(feature = "hwrt")]`.
    #[cfg(feature = "hwrt")]
    pub(crate) shadow_vis_mv_resolve_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// HW-RT Rung 3b step 6: the temporal reproject UBO RING (one 16-byte `HostVisibleCoherent` slot
    /// per in-flight frame), carrying `ResolvedTemporalShadow`
    /// (`feedback_max`/`feedback_min`/`variance_gamma`/`depth_tol`, live-tunable). A SEPARATE ring
    /// from [`Self::shadow_denoise_ubo`] (the à-trous edge-stop UBO) — the temporal set binds its own
    /// slot @6, the host writes that slot before the present (lock-free WAR). `None` unless the
    /// temporal denoise is armed AND its rings exist. `#[cfg(feature = "hwrt")]`.
    #[cfg(feature = "hwrt")]
    pub(crate) temporal_shadow_ubo: Option<[BoundBuffer; FRAMES_IN_FLIGHT]>,
    /// HW-RT Rung 3b step 6: the temporal reproject descriptor set RING (one 8-binding set per
    /// in-flight frame), written ONCE per extent against the boot temporal layout
    /// ([`GBufferScene::temporal_layout`]). Slot `fi` binds `gVisIn` @0 = the à-trous FINAL ring
    /// (`shadow_vis` when `atrous_levels == 0`, else the parity ring), `gMotionVec` @1 =
    /// `motion_vec[fi]`, `gViewT` @2 = `viewt[fi]`, `gHistIn` @3 = `shadow_temporal_hist[1-fi]`,
    /// `gHistOut` @4 = `shadow_temporal_hist[fi]`, `gTemporalOut` @5 = `temporal_out[fi]`, the
    /// `ResolvedTemporalShadow` UBO @6 = `temporal_shadow_ubo[fi]`, the camera UBO @7 =
    /// `scene.camera_ring[fi]`. `None` unless the temporal denoise is armed + its rings exist. The
    /// recorder selects `shadow_temporal_set[self.frame_index]`. `#[cfg(feature = "hwrt")]`.
    #[cfg(feature = "hwrt")]
    pub(crate) shadow_temporal_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// HW-RT Rung 3b step 6: the DENOISED resolve set RING for the TEMPORAL path — a sibling of
    /// [`Self::shadow_denoised_resolve_set`] identical except `gShadowVis` @21 is fed
    /// `temporal_out[i]` (the temporal-accumulate OUTPUT) instead of the à-trous FINAL ring, which the
    /// DENOISED resolve READS when temporal is active. `None` on the OFF path; the recorder selects
    /// `shadow_temporal_denoised_resolve_set[self.frame_index]` when
    /// [`GBufferScene::temporal_active`](crate::present::scene_types::GBufferScene::temporal_active).
    /// `#[cfg(feature = "hwrt")]`.
    #[cfg(feature = "hwrt")]
    pub(crate) shadow_temporal_denoised_resolve_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// SDFDDGI I2: the probe-update descriptor set, written ONCE against
    /// [`DdgiUpdateActivation::layout`](crate::present::scene_types::DdgiUpdateActivation::layout)
    /// (7 bindings: `Buf` @0 R, `gIrrOut` @1 W, `gDepthOut` @2 W storage images, `Classification` @3
    /// RW, `RayTable` @4 R, `LightBuf` @5 R, `DdgiUpdate` UBO @6) — `None` when the update pass is off
    /// ([`GBufferScene::ddgi_update`](crate::present::scene_types::GBufferScene::ddgi_update) is
    /// `None`). The recorder then skips the update pass entirely (the GI-OFF 0%-gate, byte-identical
    /// command stream). NO per-frame update.
    ///
    /// SINGLE (NOT a `[FRAMES_IN_FLIGHT]` ring): every input is non-ringed per plan §2.2 (the update
    /// pass binds neither the ringed camera UBO nor any ringed input — the atlas/classification/
    /// ray-table/UBO/edit-list/light-table are all single device-only instances), so one bind group
    /// captures no stale slot.
    pub(crate) ddgi_update_set: Option<VulkanBindGroup>,
    /// The present-blit descriptor set RING (one per in-flight frame), each written ONCE
    /// against [`GBufferScene::present_layout`] (one COMBINED_IMAGE_SAMPLER pointing at
    /// `lit[i]` + the scene's present sampler). NO per-frame update. RINGED so slot `i`'s
    /// present set samples `lit[i]` — the SAME slot the resolve wrote this frame (the `lit`
    /// ring made the single set stale: it would sample a sibling slot's image). The recorder
    /// selects `present_set[self.frame_index]`.
    pub(crate) present_set: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// Multi-paradigm render-path plan, rung R-SDFFWD: the `sdf_forward_march` compute pass's
    /// Set-0 vocabulary descriptor set RING (one per in-flight frame), written ONCE against
    /// [`GBufferScene::sdf_forward_march_layout`] — see [`DeferredSets::sdf_forward_set`]'s doc
    /// for the entry order + why this lives HERE (needs `lit[i]`, built after `ForwardTargets`).
    /// `Some` iff [`GBufferScene::path_has_sdf_forward`] holds; `None` under every Deferred
    /// config and every Forward-family config with the SDF leg absent (the 0%-gate). The recorder
    /// selects `sdf_forward_set[self.frame_index]`; Set 1 is [`ForwardTargets::set1`] (the shadow
    /// set, reused verbatim — no separate ring here).
    pub(crate) sdf_forward_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Multi-paradigm render-path plan, rung R8: the `vb_resolve` FUSED compute pass's Set-0
    /// vocabulary descriptor set RING, written ONCE against [`GBufferScene::vb_layout0`] — see
    /// [`DeferredSets`]'s `vb_set0` field doc for the entry order + why this lives HERE (needs
    /// `lit[i]` + `vb.vb_id[i]`). `Some` iff [`GBufferScene::path_is_vb`] holds; `None` under
    /// every other path (the 0%-gate). The recorder selects `vb_set0[self.frame_index]`; Set 1 is
    /// [`ForwardTargets::set1`] (the shadow set, reused verbatim); Set 2 is
    /// [`GBufferScene::vb_geometry_set`] (the Decision-0 geometry table, bound directly — no ring).
    pub(crate) vb_set0: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Textured-PBR rung TV0 (`RENDER-PARITY-PLAN.md` §2.3): the `vb_shade` TEXTURED-variant
    /// Set-0 vocabulary descriptor set RING, written ONCE against
    /// [`GBufferScene::vb_layout0`] — a DISTINCT descriptor SET instance from [`Self::vb_set0`]
    /// against the SAME layout object (binding 1 points at
    /// [`GBufferScene::vb_tex_instance_material_ring`]'s wider `PerInstanceMaterialTex` ring
    /// instead of [`GBufferScene::forward_instance_material_ring`]'s `PerInstanceMaterial` one;
    /// every other entry is IDENTICAL to `vb_set0`'s own). `Some` iff [`GBufferScene::path_is_vb`]
    /// holds AND [`GBufferScene::vb_tex_instance_material_ring`] AND
    /// [`GBufferScene::vb_shade_tex_pipeline`] are both `Some` (the TEXTURED resources + the
    /// TEXTURED `vb_shade` pipeline both exist — mirrors `vb_set0`'s own `path_is_vb` gate,
    /// narrowed further). Built right after `vb_set0` (both need `lit[i]` + `vb.vb_id[i]`); the
    /// recorder selects `vb_set0_tex[self.frame_index]` in place of `vb_set0` when
    /// [`GBufferScene::vb_tex_active`] holds this frame.
    pub(crate) vb_set0_tex: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// VB-P1a ("dark infra"): the froxel-variant Set-0 vocabulary descriptor set RING, written
    /// ONCE against [`GBufferScene::vb_layout0_froxel`] (11 bindings: `vb_set0`'s own
    /// `{0..7, 11}` PLUS `ClusterGrid` @8 + `LightIndexList` @9, bound to
    /// [`GBufferScene::cluster_grid`]/
    /// [`GBufferScene::light_index`]). `Some` iff [`GBufferScene::vb_layout0_froxel`] AND
    /// [`GBufferScene::cluster_grid`] AND [`GBufferScene::light_index`] are all `Some` (the froxel
    /// arm is built — ⚠️ default-OFF via the owner's `LightingConfig::clusters_enabled`, NOT
    /// hardcoded off) — `None` on every DEFAULT boot, which is what the 0%-gate rests on;
    /// `Some` on `vb_mesh_froxel`'s. Built right after `vb_set0_tex` (needs the SAME
    /// `lit[i]`/`vb.vb_id[i]` + the cluster buffers). The recorder selects
    /// `vb_set0_froxel[self.frame_index]` in place of `vb_set0`/`vb_set0_tex` when the froxel arm
    /// is armed.
    pub(crate) vb_set0_froxel: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// VB-P1c: the TEXTURED+FROXEL-variant Set-0 vocabulary descriptor set RING — a DISTINCT
    /// descriptor SET instance from [`Self::vb_set0_froxel`] against the SAME
    /// [`GBufferScene::vb_layout0_froxel`] layout object (binding 1 points at
    /// [`GBufferScene::vb_tex_instance_material_ring`]'s wider `PerInstanceMaterialTex` ring
    /// instead of [`GBufferScene::forward_instance_material_ring`]; every other entry is
    /// IDENTICAL to [`Self::vb_set0_froxel`]'s own — mirrors the `vb_set0`/`vb_set0_tex` pairing,
    /// R5's "one shared layout, a distinct set" rule). `Some` iff
    /// [`GBufferScene::vb_layout0_froxel`] AND [`GBufferScene::cluster_grid`] AND
    /// [`GBufferScene::light_index`] (the froxel arm — default-OFF, an owner opt-in) AND
    /// [`GBufferScene::vb_tex_instance_material_ring`] AND
    /// [`GBufferScene::vb_shade_tex_froxel_pipeline`] (the TEXTURED resources + the
    /// TEXTURED+FROXEL `vb_shade` pipeline) are all `Some` — `None` on every current boot (the
    /// 0%-gate). Built right after [`Self::vb_set0_froxel`] (needs the SAME inputs plus the tex
    /// ring). The recorder selects `vb_set0_tex_froxel[self.frame_index]` in place of
    /// `vb_set0_froxel`/`vb_set0_tex`/`vb_set0` when BOTH [`GBufferScene::vb_tex_active`] AND the
    /// froxel arm hold this frame.
    pub(crate) vb_set0_tex_froxel: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Multi-paradigm render-path plan, rung R4b-b — the Forward v1 mesh path's OWN depth image
    /// ring + descriptor sets ([`ForwardTargets`]). `Some` iff `profile ==
    /// `[`TargetsProfile::ForwardMesh`]` (built at [`Self::create`]'s TOP, before the unconditional
    /// deferred-body allocation below — see that fn's doc for the "full allocation + additive
    /// `ForwardTargets`" v1 choice). `None` under every `Deferred*` profile (the 0%-gate: no
    /// extra image, no extra descriptor set, byte-identical Deferred allocation).
    pub(crate) forward: Option<ForwardTargets>,
    /// Multi-paradigm render-path plan, rung R8 — the VisibilityBuffer v1 path's OWN per-extent
    /// targets ([`VbTargets`]). `Some` iff `profile == `[`TargetsProfile::VbMesh`]` (built at
    /// [`Self::create`]'s TOP, right after [`Self::forward`] — VB REUSES `ForwardTargets` for its
    /// depth ring + Set-1 shadow set, see [`VbTargets`]'s doc). `None` under every other profile.
    pub(crate) vb: Option<VbTargets>,
    /// VB-P2 classification plan (docs/VB-P2-CLASSIFICATION-PLAN.md), rung P2a (dark infra,
    /// unwired): the packed `gClassify` buffer RING ([`VbClassifyTargets`]). `Some` iff
    /// `profile == `[`TargetsProfile::VbMesh`]` (the SAME gate [`Self::vb`] uses — built at
    /// [`Self::create`]'s TOP, right after [`Self::vb`]). `None` under every other profile.
    /// Nothing declares/records against this buffer yet (`record_vb`/`declare_vb_graph` are
    /// untouched this rung) — [`Self::vb_set0`] binds it at `b7`, bound-but-unread.
    pub(crate) vb_classify: Option<VbClassifyTargets>,
    /// The extent the images were created at (so [`GBufferTargets::sync_gbuffer`] can
    /// detect a resize and reallocate).
    pub(crate) extent: VkExtent2D,
}

/// Multi-paradigm render-path plan, rung R4b-b: the Forward v1 mesh path's own per-extent
/// targets — a D32 HARDWARE REVERSE-Z depth image ring (a SEPARATE allocation from
/// [`GBufferTargets::depth`]'s custom-linear depth, Decision 4) plus the two Forward-only
/// descriptor set rings (Set 0 core, Set 1 shadow — §G, renumbered from Set 2 — see
/// [`Self::set1`]'s doc for the boot-panic fix). Built ONLY when
/// [`TargetsProfile::ForwardMesh`] is threaded into [`GBufferTargets::create`]; `lit`
/// ([`GBufferTargets::lit`]) is REUSED verbatim as Forward's color-attachment target (the C5
/// per-path `lit`-producer-access discipline: Forward declares `ColorAttachmentWrite`, Deferred
/// declares `StorageWrite`, on the SAME physical image — the two paths are boot-mutually-
/// exclusive, so there is no cross-path contention).
pub(crate) struct ForwardTargets {
    /// The D32_SFLOAT reverse-Z depth image RING (one per in-flight frame):
    /// `DEPTH_STENCIL_ATTACHMENT` (rasterize into, `VK_COMPARE_OP_GREATER`) | `SAMPLED`
    /// (a future consumer's inv-proj reconstruct). Re-cleared to `0.0` (the reverse-Z
    /// "nothing drawn yet" sentinel — farther than any real `depth ∈ (0, 1]`) every frame by
    /// `record_forward`. RINGED (the SAME cross-frame Write-After-Read fix
    /// [`GBufferTargets::depth`]'s doc explains).
    pub(crate) depth: [VulkanTexture; FRAMES_IN_FLIGHT],
    /// Forward-family Set-0 (core) bind-group RING, written ONCE per extent against
    /// [`GBufferScene::forward_layout0`](super::scene_types::GBufferScene::forward_layout0) — the
    /// UNIFIED 7-binding layout (rung R5 code-review fix: ONE layout object shared by every
    /// Forward-family pipeline, never two structurally-identical-but-distinct handles), entries
    /// sourced from EXISTING per-frame buffers (no new upload path): `instances` @0 =
    /// `scene.forward_instance_ring[i]`, `instance_materials` @1 =
    /// `scene.forward_instance_material_ring[i]`, `Camera` @2 = `scene.camera_ring[i]`,
    /// `LightBuf` @3 = `scene.light_table`, `Materials` @4 = `scene.material_table`,
    /// `ClusterGrid` @5 / `LightIndexList` @6 = `scene.cluster_grid`/`scene.light_index` (or the
    /// `scene.light_table` placeholder when unarmed — [`Self::build`]'s doc). RINGED (slot `i`
    /// binds `camera_ring[i]`/the instance rings' slot `i`, the lock-free per-frame-ring fix).
    pub(crate) set0: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// Forward's Set-1 (shadow) bind-group RING, written ONCE per extent against
    /// [`GBufferScene::forward_layout1`](super::scene_types::GBufferScene::forward_layout1) — 4
    /// bindings, entries sourced from the SAME cascade/atlas resources the deferred resolve binds
    /// at 12-15: `gCsm`+`gCsmCmp` @0 (combined, `scene.csm_cascade_texture` +
    /// `scene.csm_compare_sampler`), `CsmCascades` @1 = `scene.csm_cascade_ring[i]`,
    /// `gShadowAtlas`+`gShadowAtlasCmp` @2 (combined, `scene.shadow_atlas_texture` +
    /// `scene.shadow_atlas_sampler`), `ShadowAtlas` @3 = `scene.shadow_atlas_ubo` (single,
    /// NOT ringed — mirrors the resolve's own binding 15). RINGED (slot `i` binds
    /// `csm_cascade_ring[i]`, the SAME lock-free per-frame-ring fix `resolve_set`'s CSM binding
    /// uses).
    ///
    /// Boot-panic fix: this was originally Set 2, with a zero-binding Set-1 PLACEHOLDER layout
    /// declared between it and Set 0 (Vulkan's set-index contiguity rule). A zero-binding
    /// [`BindGroupLayoutDesc`](boyko_rhi::BindGroupLayoutDesc) is REJECTED by
    /// `create_bind_group_layout`'s own `1..=MAX_BIND_GROUP_BINDINGS` invariant
    /// (`rhi_impl/device.rs:205`) — a real `GpuSceneBundles::boot` panic. `forward_opaque.fs.hlsl`'s
    /// shadow bindings were renumbered to Set 1 instead, so the pipeline layout is a plain 2-set
    /// `[Set0, Set1]` and no placeholder exists.
    pub(crate) set1: [VulkanBindGroup; FRAMES_IN_FLIGHT],
}

impl ForwardTargets {
    /// Allocates the reverse-Z depth ring + the two descriptor set rings at `extent`, against
    /// `scene`'s boot-built [`GBufferScene::forward_layout0`]/[`GBufferScene::forward_layout1`].
    /// On any partial failure the slots already built are drained (mirrors [`CoreImages::build`]'s
    /// reverse-acquisition discipline); the caller has nothing else to tear down for THIS bundle.
    fn build(
        ctx: &VulkanContext,
        scene: &GBufferScene<'_>,
        extent: VkExtent2D,
    ) -> Result<Self, SwapchainError> {
        // The caller only reaches this fn under `TargetsProfile::ForwardMesh` (`GBufferTargets
        // ::create`'s doc), which is derived from a `Forward`-resolved `ResolvedRenderPath` —
        // production ALWAYS threads `Some(...)` for these 5 fields at that point
        // (`GBufferScene::forward_pipeline`'s doc: built unconditionally at boot). `None` here
        // would mean a test fixture forced `TargetsProfile::ForwardMesh` without also wiring the
        // real Forward resources — an authoring bug this `expect` surfaces immediately rather
        // than silently building against a bind-group layout that does not exist.
        let forward_layout0 = scene
            .forward_layout0
            .expect("invariant: TargetsProfile::ForwardMesh requires scene.forward_layout0");
        let forward_layout1 = scene
            .forward_layout1
            .expect("invariant: TargetsProfile::ForwardMesh requires scene.forward_layout1");
        let forward_instance_ring = scene
            .forward_instance_ring
            .expect("invariant: TargetsProfile::ForwardMesh requires scene.forward_instance_ring");
        let forward_instance_material_ring = scene.forward_instance_material_ring.expect(
            "invariant: TargetsProfile::ForwardMesh requires scene.forward_instance_material_ring",
        );

        let depth_desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format: Format::D32Sfloat,
            dimension: TextureDimension::D2,
            usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        };
        let mut depth_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in depth_slots.iter_mut() {
            match RhiDevice::create_texture(ctx, &depth_desc).map_err(SwapchainError::DepthImage) {
                Ok(t) => *slot = Some(t),
                Err(e) => {
                    // SAFETY: every drained slot was created on `ctx` above, referenced by no
                    // submission (the build phase); `Option::take` leaves the slot `None` so each
                    // is destroyed exactly once.
                    unsafe {
                        for built in depth_slots.iter_mut() {
                            if let Some(t) = built.take() {
                                RhiDevice::destroy_texture(ctx, t);
                            }
                        }
                    }
                    return Err(e);
                }
            }
        }
        let depth: [VulkanTexture; FRAMES_IN_FLIGHT] =
            depth_slots.map(|s| s.expect("invariant: every forward depth ring slot built before here"));

        // Code-review P2-1: fallible, reverse-acquisition-draining creation (mirrors
        // `vocab_set`'s own build loop above) — the prior `.expect()` form leaked the already-
        // built `depth` ring (and, for `set1`'s failure, the already-built `set0` ring) on any
        // `create_bind_group` failure.
        // Multi-paradigm render-path plan, rung R5 (ForwardPlus, code-review fix): `forward_layout0`
        // is now the ONE UNIFIED 7-binding layout shared by EVERY Forward-family pipeline
        // (`GpuSceneBundles::boot`'s doc) — so this descriptor set is ALWAYS built with 7
        // entries, regardless of path. An earlier revision branched entry COUNT on
        // `scene.path_is_forward_plus()` against TWO DISTINCT layout objects (a 5-binding one
        // for `Forward`, a 7-binding one for `ForwardPlus`); Vulkan treats structurally-
        // identical-but-distinct `VkDescriptorSetLayout` handles as INCOMPATIBLE with a pipeline
        // built against the other handle (silent no-op with validation disabled) — the bug this
        // unification fixes. `ClusterGrid`/`LightIndexList` fall back to `scene.light_table`
        // when the real L1 cull buffers are not wired (`scene.cluster_grid`/`scene.light_index`
        // `None`, e.g. under plain `Forward` or an unarmed `ForwardPlus` boot) — the SAME
        // bound-but-unread placeholder idiom the deferred resolve's OWN `cluster_grid_buf`/
        // `light_index_buf` locals already establish (`GBufferTargets::resolve_software_entries`'s
        // call site): `forward_opaque_froxel.fs.hlsl` gates every access behind the THREE-term
        // `use_clusters` (VB-P1k) — `clusters_enabled != 0 && cluster_count != 0 && cluster_count
        // <= grid_capacity`, the capacity read off the BOUND `ClusterGrid` descriptor with
        // `GetDimensions` — and the BASE `forward_opaque.fs.hlsl` never declares bindings 5/6 at
        // all, so an unarmed/unread binding is inert either way. UNLIKE the deferred resolve set,
        // this one IS bound on the boots it describes: `record_forward` runs on `Forward` and
        // `ForwardPlus` alike, and the ForwardPlus arm binds the froxel FS against it — so the
        // gate here is genuinely evaluated, and it is worth stating which term decides.
        // On the DEFAULT ForwardPlus boot (every golden; every scene that leaves `EnginePlugins`'s
        // `LightingConfig::default()` seed alone) `clusters_enabled` is `false` and
        // `LightHeaderGpu::new` packs it verbatim, so the FIRST term short-circuits — the ENABLED
        // BIT is what takes the flat branch. Only a ForwardPlus boot that explicitly sets
        // `clusters_enabled = true` gets past it, and there the DIMS term decides:
        // `ResolvedRenderPath::froxel_light_cull` is `clusters_enabled && path ==
        // VisibilityBuffer`, so it is `false` on every ForwardPlus boot and
        // `sync_cluster_light_gate` holds the dims lane at `0` (see
        // `GBufferTargets::resolve_set`'s doc for the same two-case split on the Deferred side).
        // The two terms past the enabled bit are an out-of-bounds guard, not a style choice:
        // `robustBufferAccess` is OFF here and no GPU-assisted validation runs, so an
        // out-of-range read is UB nothing would report.
        let cluster_grid_buf = scene.cluster_grid.unwrap_or(scene.light_table);
        let light_index_buf = scene.light_index.unwrap_or(scene.light_table);
        let mut set0_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for (i, slot) in set0_slots.iter_mut().enumerate() {
            let entries = [
                BindGroupEntry::StorageBuffer { buffer: &forward_instance_ring[i] },
                BindGroupEntry::StorageBuffer {
                    buffer: &forward_instance_material_ring[i],
                },
                BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[i] },
                BindGroupEntry::StorageBuffer { buffer: scene.light_table },
                BindGroupEntry::StorageBuffer { buffer: scene.material_table },
                BindGroupEntry::StorageBuffer { buffer: cluster_grid_buf },
                BindGroupEntry::StorageBuffer { buffer: light_index_buf },
            ];
            let result =
                RhiDevice::create_bind_group(ctx, &BindGroupDesc { layout: forward_layout0, entries: &entries });
            match result {
                Ok(g) => *slot = Some(g),
                Err(e) => {
                    // SAFETY: every drained `set0` slot + the fully-built `depth` ring were
                    // created on `ctx` above, referenced by no submission (the build phase);
                    // each destroyed exactly once.
                    unsafe {
                        for s in set0_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for t in depth {
                            RhiDevice::destroy_texture(ctx, t);
                        }
                    }
                    return Err(SwapchainError::DepthImage(e));
                }
            }
        }
        let set0: [VulkanBindGroup; FRAMES_IN_FLIGHT] =
            set0_slots.map(|s| s.expect("invariant: every forward Set-0 ring slot built before here"));

        let mut set1_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for (i, slot) in set1_slots.iter_mut().enumerate() {
            let desc = BindGroupDesc {
                layout: forward_layout1,
                entries: &[
                    BindGroupEntry::CombinedImage {
                        texture: scene.csm_cascade_texture,
                        sampler: scene.csm_compare_sampler,
                    },
                    BindGroupEntry::UniformBuffer { buffer: &scene.csm_cascade_ring[i] },
                    BindGroupEntry::CombinedImage {
                        texture: scene.shadow_atlas_texture,
                        sampler: scene.shadow_atlas_sampler,
                    },
                    BindGroupEntry::UniformBuffer { buffer: scene.shadow_atlas_ubo },
                ],
            };
            match RhiDevice::create_bind_group(ctx, &desc) {
                Ok(g) => *slot = Some(g),
                Err(e) => {
                    // SAFETY: every drained `set1` slot + the fully-built `set0` ring + the
                    // fully-built `depth` ring were created on `ctx` above, referenced by no
                    // submission (the build phase); each destroyed exactly once (reverse
                    // acquisition: set1 -> set0 -> depth).
                    unsafe {
                        for s in set1_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in set0 {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for t in depth {
                            RhiDevice::destroy_texture(ctx, t);
                        }
                    }
                    return Err(SwapchainError::DepthImage(e));
                }
            }
        }
        let set1: [VulkanBindGroup; FRAMES_IN_FLIGHT] =
            set1_slots.map(|s| s.expect("invariant: every forward Set-1 ring slot built before here"));

        Ok(Self { depth, set0, set1 })
    }

    /// Tears the depth ring down. The descriptor sets are pool-owned (freed with their pool at
    /// device teardown, the SAME discipline every other `VulkanBindGroup` in this module follows
    /// — none of `GBufferTargets`'s OTHER `destroy` bodies free a `VulkanBindGroup` individually
    /// either).
    ///
    /// # Safety
    /// Every image was created on `ctx`, the device is idle (the caller's teardown waited), and
    /// each is destroyed exactly once (by-value).
    unsafe fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: per the contract `ctx` is live + idle and nothing references these images.
        unsafe {
            for t in self.depth {
                RhiDevice::destroy_texture(ctx, t);
            }
        }
    }
}

/// Multi-paradigm render-path plan, rung R8: the VisibilityBuffer v1 path's own per-extent `vb_id`
/// image RING (`R32G32_UINT`, Decision 9) — the ONLY VB-specific IMAGE this rung allocates. Built
/// at [`GBufferTargets::create`]'s TOP, alongside [`ForwardTargets`] (needs only `extent`, no
/// dependency on `core`'s images), which VB REUSES verbatim for its depth ring + Set-1 shadow set
/// (`vb_raster`/`vb_resolve` both bind [`ForwardTargets::depth`]/[`ForwardTargets::set1`] — see
/// [`GBufferScene::vb_resolve_pipeline`]'s doc). The Set-0 descriptor set that BINDS `vb_id`
/// (`GBufferTargets::vb_set0`) is built separately, at the SAME "needs `core.lit`" point
/// `sdf_forward_set` is (Option 2 — additive, needs images `create`'s later sub-bundles own).
pub(crate) struct VbTargets {
    /// The `R32G32_UINT` id-channel image RING: `COLOR_ATTACHMENT` (raster write, `vb_raster`) |
    /// `SAMPLED` (`.Load` unfiltered fetch, `vb_resolve`). Cleared to the sentinel `(0xFFFFFFFF,
    /// 0)` every frame by `record_vb` (mirrors `ForwardTargets::depth`'s per-frame re-clear).
    pub(crate) vb_id: [VulkanTexture; FRAMES_IN_FLIGHT],
}

impl VbTargets {
    /// Allocates the `vb_id` ring at `extent`. Reverse-acquisition draining on partial failure
    /// (mirrors [`ForwardTargets::build`]'s own depth-ring loop).
    fn build(ctx: &VulkanContext, extent: VkExtent2D) -> Result<Self, SwapchainError> {
        let vb_id_desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format: Format::R32G32Uint,
            dimension: TextureDimension::D2,
            // ⚠️ `TRANSFER_SRC` is VG-R0 rung R0c's ONE permanent edit to the shipped render path,
            // and it is here rather than behind the census's arming knob on purpose: a usage bit is
            // fixed at image creation, so an armed-only ring would be a SECOND ring, not the one
            // the goldens render.
            //
            // It is safe on an axis that can be argued rather than merely hoped: `vb_id` is
            // `R32G32_UINT`, uncompressed, and `.Load`ed unfiltered, so no usage, tiling or layout
            // choice the widening admits can alter a texel value. That is also why R0c gate (a)
            // ("every VB image golden byte-identical with the census UNARMED") is recorded as an
            // assertion whose red is STRUCTURALLY UNAVAILABLE: four mutation sitings failed, each
            // for a different reason, and the axis this bit moves provably cannot perturb the
            // artefact the goldens hash. The gate is still asserted -- by measurement, on every
            // blessed VB pin -- it simply cannot be falsified by construction.
            usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED | ImageUsage::TRANSFER_SRC,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        };
        let mut vb_id_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] = [const { None }; FRAMES_IN_FLIGHT];
        for slot in vb_id_slots.iter_mut() {
            match RhiDevice::create_texture(ctx, &vb_id_desc).map_err(SwapchainError::DepthImage) {
                Ok(t) => *slot = Some(t),
                Err(e) => {
                    // SAFETY: every drained slot was created on `ctx` above, referenced by no
                    // submission (the build phase); `Option::take` leaves the slot `None` so each
                    // is destroyed exactly once.
                    unsafe {
                        for built in vb_id_slots.iter_mut() {
                            if let Some(t) = built.take() {
                                RhiDevice::destroy_texture(ctx, t);
                            }
                        }
                    }
                    return Err(e);
                }
            }
        }
        let vb_id: [VulkanTexture; FRAMES_IN_FLIGHT] =
            vb_id_slots.map(|s| s.expect("invariant: every vb_id ring slot built before here"));
        Ok(Self { vb_id })
    }

    /// # Safety
    /// Every image was created on `ctx`, the device is idle (the caller's teardown waited), and
    /// each is destroyed exactly once (by-value).
    unsafe fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: per the contract `ctx` is live + idle and nothing references these images.
        unsafe {
            for t in self.vb_id {
                RhiDevice::destroy_texture(ctx, t);
            }
        }
    }
}

/// VB-P2 classification plan (docs/VB-P2-CLASSIFICATION-PLAN.md), rung P2a (dark infra,
/// unwired). The material system's hard 16-bit addressing cap, mirrored here (host mirror of
/// `boyko_render::material_table::MAX_MATERIAL_ROWS`) because this crate cannot depend on
/// `boyko_render` (which sits ABOVE it in the dependency graph — the SAME plain-value boundary
/// crossing `GBufferScene::vb_geometry_set`'s doc explains). The `gClassify` buffer's M-arrays
/// (`counts`/`offsets`/`cursors`/`gbase`) are pre-sized to this cap (plan P1-2) so their
/// sub-region layout is FIXED and never invalidated by `MaterialTable` growth. SHADER mirror:
/// `shaders/vb_classify_common.hlsli`'s `VB_MAX_MATERIAL_ROWS` — keep both in sync.
pub(crate) const VB_CLASSIFY_MAX_MATERIAL_ROWS: u64 = 1 << 16;

/// VB-P2 classification plan, rung P2a. `group_to_mat[g] == VB_GROUP_SENTINEL` marks a
/// dispatch group past `total_groups` (the plan's `fill` pass sentinel-fill, P1-1) — unused by
/// this rung's dark infra (nothing writes/reads `gClassify` yet), kept here as the host-side
/// mirror of `shaders/vb_classify_common.hlsli`'s own constant for a future rung's use.
#[allow(dead_code)]
pub(crate) const VB_GROUP_SENTINEL: u32 = 0xFFFF_FFFF;

/// VB-P2 classification plan, rung P2a (dark infra, unwired): the packed `gClassify`
/// byte-address buffer RING (one per in-flight frame — STORAGE | TRANSFER_DST,
/// `MemoryLocation::DeviceLocal`, never mapped: a future rung's `fill`/`count`/`scan`/
/// `scatter`/`vb_shade` passes are all GPU-side, no CPU read/write path is needed). Layout
/// (word offsets): `[counts(MAX) | offsets(MAX) | cursors(MAX) | gbase(MAX) |
/// group_to_mat(G+MAX) | pixel_list(w*h)]` — see `shaders/vb_classify_common.hlsli`'s header
/// for the exact host<->shader sync-pinned offset formula this buffer's SIZE mirrors.
///
/// `group_to_mat`'s reserved capacity is `G + VB_CLASSIFY_MAX_MATERIAL_ROWS` (NOT `G +
/// present_material_count`, the tighter per-frame live length the plan's D2 over-dispatch
/// actually walks) — pre-sizing to the material system's hard cap keeps every offset from
/// `pixel_list` onward FIXED across every frame, exactly like the M-arrays (P1-2). The extra
/// reserved bytes vs a `present_material_count`-tight sizing are at most
/// `VB_CLASSIFY_MAX_MATERIAL_ROWS * 4` = 256 KiB per FIF — negligible next to `pixel_list`'s
/// own `w*h*4` bytes (~8 MiB at 1080p).
pub(crate) struct VbClassifyTargets {
    /// The packed classify buffer RING. Nothing reads or writes it this rung — `vb_set0`
    /// binds `gclassify[fi]` at `b7`, bound-but-unread (the R5 "one shared layout object"
    /// rule this crate's own VB Set-0 doc explains).
    pub(crate) gclassify: [BoundBuffer; FRAMES_IN_FLIGHT],
}

impl VbClassifyTargets {
    /// Allocates the `gClassify` buffer ring at `extent`, sized per this struct's doc.
    /// Reverse-acquisition draining on partial failure (mirrors [`VbTargets::build`]'s own
    /// loop).
    fn build(ctx: &VulkanContext, extent: VkExtent2D) -> Result<Self, SwapchainError> {
        // `G = ceil(w*h / 64)` — the SAME per-pixel dispatch-group count
        // `GpuSceneBundles::dispatch_group_count_x` computes host-side (`LOCAL_SIZE_X` = 64,
        // the classify/shade compute family's own group size).
        let group_count_x = (extent.width * extent.height).div_ceil(LOCAL_SIZE_X);
        let total_words = 5 * VB_CLASSIFY_MAX_MATERIAL_ROWS
            + group_count_x as u64
            + (extent.width as u64) * (extent.height as u64);
        let total_bytes = total_words * 4;

        let mut slots: [Option<BoundBuffer>; FRAMES_IN_FLIGHT] = [const { None }; FRAMES_IN_FLIGHT];
        for slot in slots.iter_mut() {
            match RhiDevice::create_buffer(
                ctx,
                &BufferDesc {
                    size: total_bytes,
                    usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST,
                    location: MemoryLocation::DeviceLocal,
                },
            ) {
                Ok(b) => *slot = Some(b),
                Err(e) => {
                    // SAFETY: every drained slot was created on `ctx` above, referenced by no
                    // submission (the build phase); `Option::take` leaves the slot `None` so
                    // each is destroyed exactly once.
                    unsafe {
                        for built in slots.iter_mut() {
                            if let Some(b) = built.take() {
                                RhiDevice::destroy_buffer(ctx, b);
                            }
                        }
                    }
                    return Err(SwapchainError::DepthImage(e));
                }
            }
        }
        let gclassify: [BoundBuffer; FRAMES_IN_FLIGHT] =
            slots.map(|s| s.expect("invariant: every VB classify buffer ring slot built before here"));
        Ok(Self { gclassify })
    }

    /// # Safety
    /// Every buffer was created on `ctx`, the device is idle (the caller's teardown waited),
    /// and each is destroyed exactly once (by-value).
    unsafe fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: per the contract `ctx` is live + idle and nothing references these buffers.
        unsafe {
            for b in self.gclassify {
                RhiDevice::destroy_buffer(ctx, b);
            }
        }
    }
}

/// Anti-aliasing campaign: which AA mode [`GBufferTargets`] is CURRENTLY armed for —
/// replaces the Stage-1 `aa_armed: bool`, closing the Fxaa↔Smaa fixed-extent resync gap a
/// boolean cannot see (a boolean only distinguishes Off↔non-Off; two DIFFERENT armed modes
/// both want `aa_out`, so a runtime Fxaa↔Smaa switch at fixed extent would not otherwise
/// trigger the rebuild that swaps `fxaa_set` for the SMAA sets). Local to this crate
/// (`AaMode` lives in the higher-layer `boyko_render`, so the arm state is derived from
/// `scene.aa`/`scene.smaa` presence, never imported).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AaArm {
    /// Neither [`GBufferScene::aa`] nor [`GBufferScene::smaa`] nor [`GBufferScene::ssaa`] is
    /// armed — the 0%-gate.
    Off,
    /// [`GBufferScene::aa`] (FXAA) is armed.
    Fxaa,
    /// [`GBufferScene::smaa`] (SMAA 1x) is armed.
    Smaa,
    /// [`GBufferScene::ssaa`] (2× supersampling) is armed. Unlike `Fxaa`/`Smaa`, this arm is
    /// host-authoritative — it only occurs when the host committed the 2× `composite_extent`
    /// at boot; the extent compare `sync_gbuffer` already performs on a mismatch covers the
    /// `aa_out` resize this arm entails (native, not `present_extent`, under `Ssaa`).
    Ssaa,
    /// Anti-aliasing Stage 4: [`GBufferScene::taa`] (TAA) is armed. Native resolution
    /// (`aa_extent == extent`, like `Fxaa`/`Smaa`) but ADDITIONALLY allocates the
    /// [`GBufferTargets::taa_hist`] cross-frame history ring — an arm-state flip into/out of
    /// `Taa` therefore forces the same fence-safe rebuild an extent change triggers, exactly
    /// like every other `AaArm` transition.
    Taa {
        /// TAA rung T3: `scene.rcas.is_some()` — carried as PAYLOAD (not folded into a
        /// separate `AaArm` variant) for the SAME reason this enum exists at all (its own doc):
        /// a bare `Taa` variant cannot distinguish `SharpenMode::None` from `SharpenMode::Rcas`,
        /// so a live Rcas on/off toggle (which allocates/frees [`GBufferTargets::taa_resolved`]/
        /// `rcas_set`) would NOT trigger `sync_gbuffer`'s fence-safe rebuild without this field —
        /// exactly the "two different armed sub-states want different resources" bug this enum
        /// was invented to close (see the enum's own doc).
        rcas: bool,
    },
}

impl AaArm {
    /// Derives the arm state from `scene` — `smaa` → `ssaa` → `taa` → `aa`-first purely as a
    /// defensive tie-break (the four are populated mutually-exclusively at the scene-build
    /// site; a `debug_assert!` in [`GBufferTargets::create`] makes that invariant explicit).
    fn from_scene(scene: &GBufferScene<'_>) -> Self {
        if scene.smaa.is_some() {
            AaArm::Smaa
        } else if scene.ssaa.is_some() {
            AaArm::Ssaa
        } else if scene.taa.is_some() {
            AaArm::Taa { rcas: scene.rcas.is_some() }
        } else if scene.aa.is_some() {
            AaArm::Fxaa
        } else {
            AaArm::Off
        }
    }
}

/// Multi-paradigm render-path plan, rung R2 (§B "Per-path framegraph") — the geometry-leg
/// profile [`GBufferTargets::sync_gbuffer`]/[`GBufferTargets::create`] allocate against.
/// Derived from `scene.resolved_render_path` at the call site (the [`AaArm::from_scene`]
/// derive-from-scene precedent) and threaded down as an explicit parameter — the seam the plan
/// names for path-conditional allocation.
///
/// # Rung R3/R3b status — profile IDENTITY landed for both legs, allocation DIFFERENCE deferred
///
/// `DeferredSdfOnly` (rung R3) and `DeferredMeshOnly` (rung R3b) are BOTH reachable now, but
/// [`GBufferTargets::create`] does NOT yet branch its allocation on `profile` — the vocab/
/// present descriptor sets are written ONCE per extent against a FIXED binding layout shared by
/// every `Deferred` config (this module's own doc, "written ONCE per extent"), so dropping an
/// image here would need a SECOND descriptor-set layout, a larger, separately-scoped change
/// (see the R3 rung report's honest VRAM accounting; R3b's `viewt_from_depth_set` IS its own
/// dedicated Option-gated ring, but the shared vocab/resolve/present rings are unaffected). These
/// variants exist so the profile IDENTITY is real today — `TargetsProfile::from_scene` covers
/// every `(mesh_leg, sdf_leg)` combination without a `debug_assert!` trip — ready for a future
/// rung to actually branch allocation on `profile`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
// The shared `Deferred` prefix names the `RenderPath::Deferred` family explicitly (the plan §H
// R3 row's own naming — `TargetsProfile::{DeferredFull,DeferredMeshOnly,DeferredSdfOnly}` — kept
// verbatim rather than renamed for a lint, since a future `Forward`/`VisibilityBuffer` sibling
// enum will need the SAME "which path family" prefix discipline for readability at call sites).
#[allow(clippy::enum_variant_names)]
pub(crate) enum TargetsProfile {
    /// Both geometry legs, the full Deferred image/descriptor-set contract.
    DeferredFull,
    /// Only the mesh raster leg (`GeometryLegs::Mesh`) — reachable as of rung R3b, allocation
    /// identical to `DeferredFull` this rung (see this type's doc). The `viewt_from_depth`
    /// producer's own dedicated set is Option-gated separately (`GBufferTargets::
    /// viewt_from_depth_set`), not tracked by this profile.
    DeferredMeshOnly,
    /// Only the SDF marched leg (`GeometryLegs::Sdf`) — reachable as of rung R3, allocation
    /// identical to `DeferredFull` this rung (see this type's doc).
    DeferredSdfOnly,
    /// Multi-paradigm render-path plan, rung R4b-b: `RenderPath::Forward` (v1, mesh-only —
    /// the resolver collapses `Forward × {Both, Sdf}` to `Mesh` until R-SDFFWD lands, so this is
    /// the only reachable Forward profile today). [`GBufferTargets::create`] runs its UNCHANGED
    /// `DeferredFull`-shaped allocation body REGARDLESS of this profile (Option 2 — "full +
    /// additive `ForwardTargets`", the v1 minimalism choice; see [`GBufferTargets::forward`]'s
    /// doc) and ADDITIONALLY builds a [`ForwardTargets`] bundle at the top of `create`.
    ForwardMesh,
    /// Multi-paradigm render-path plan, rung R8: `RenderPath::VisibilityBuffer` (v1, fused
    /// `vb_resolve` — the resolver collapses `VisibilityBuffer × {Both, Sdf}` to `Mesh` until R10
    /// lands, so this is the only reachable VB profile today). [`GBufferTargets::create`] runs
    /// its UNCHANGED `DeferredFull`-shaped allocation body REGARDLESS of this profile (the SAME
    /// Option-2 additive discipline `ForwardMesh` uses) and ADDITIONALLY builds a [`VbTargets`]
    /// bundle (the `vb_id` ring) AND a [`ForwardTargets`] bundle (REUSED for the depth ring +
    /// Set-1 shadow set — `VbTargets`'s doc) at the top of `create`.
    VbMesh,
}

impl TargetsProfile {
    /// Derives the profile from the boot-resolved render-path carrier (mirrors
    /// [`AaArm::from_scene`]'s derive-from-scene precedent). `#[inline]` — a couple of `bool`/
    /// `u32` reads, no allocation.
    ///
    /// `RenderPath::Forward`/`RenderPath::ForwardPlus` (discriminants `1`/`2`) are checked
    /// BEFORE the `(mesh_leg, sdf_leg)` match (rung R4b-b, widened at rung R5) — a
    /// Forward-family-resolved carrier still reports `mesh_leg`/`sdf_leg` per its (possibly
    /// leg-collapsed) `GeometryLegs`, but the Deferred-family match below has no arm for it;
    /// branching on `path` first keeps that match exhaustive over the `Deferred` family alone,
    /// unchanged from rung R3b. `ForwardPlus` reuses `ForwardMesh`'s SAME allocation body
    /// (`GBufferTargets::create` branches on `resolved_render_path.path` internally where the
    /// two paths diverge — the extra `ForwardTargets::build` Set-0 growth, `targets.rs`'s own
    /// doc — not on a distinct `TargetsProfile` variant).
    #[inline]
    pub(crate) fn from_scene(scene: &GBufferScene<'_>) -> Self {
        let rp = &scene.resolved_render_path;
        // Multi-paradigm render-path plan, rung R8: checked BEFORE the Forward-family check —
        // `RenderPath::VisibilityBuffer` (discriminant 3) and `Forward`/`ForwardPlus`
        // (discriminants 1/2) are boot-mutually-exclusive resolved paths (Decision 1), so the
        // check order between the two is a style choice, not a correctness one; VB first mirrors
        // this plan's own rung ordering (VB landed after Forward/ForwardPlus).
        if scene.path_is_vb() {
            return TargetsProfile::VbMesh;
        }
        if scene.path_is_forward() {
            // RenderPath::Forward == 1, RenderPath::ForwardPlus == 2
            // (boyko_render::render_path_config::RenderPath) — the SAME single predicate
            // `declare_frame_graph`'s dispatch uses (`GBufferScene::path_is_forward`'s doc).
            //
            // Multi-paradigm render-path plan, rung R-SDFFWD: `mesh_leg` is NO LONGER guaranteed
            // `true` here — `SDF_FORWARD_IMPLEMENTED` lifted, so `GeometryLegs::Sdf` (mesh_leg ==
            // false) is now a real, honored request under a Forward-family path (the
            // `sdf_forward_march` pass is the sole `lit` producer on that leg set; see
            // `GBufferScene::sdf_forward_march`'s doc). `ForwardMesh` is still the ONE
            // `TargetsProfile` variant for every Forward-family boot (mesh_leg true OR false) —
            // `ForwardTargets::build` always allocates the reverse-Z `depth` ring + the Set-0/Set-1
            // rings regardless of leg set (a harmless extra allocation on a mesh-less boot, the
            // SAME "shared allocation body" precedent this fn's own doc already establishes for
            // `Forward` vs `ForwardPlus`).
            return TargetsProfile::ForwardMesh;
        }
        match (rp.mesh_leg, rp.sdf_leg) {
            (true, true) => TargetsProfile::DeferredFull,
            (false, true) => TargetsProfile::DeferredSdfOnly,
            (true, false) => TargetsProfile::DeferredMeshOnly,
            (false, false) => {
                // invariant: `GeometryLegs` has no "both off" state (no `None` variant) — a
                // resolved carrier can never report neither leg present.
                debug_assert!(false, "invariant: a resolved render path always has >=1 leg");
                TargetsProfile::DeferredFull
            }
        }
    }
}

/// The G-buffer color format (albedo / normal / material): `R8G8B8A8_UNORM`, the
/// STORAGE-image store target the marcher writes (matches the P1b offscreen driver's
/// `GBUFFER_FORMAT`). The ALBEDO image is also `SAMPLED` (the present-blit) — never
/// stretched; presented 1:1 in the swapchain's top-left like [`SampledComposite`].
const GBUFFER_FORMAT: Format = Format::R8G8B8A8Unorm;

/// The SMAA `edges` target format: `R8G8_UNORM` (R = west/left edge, G = north/top edge) — a
/// Vulkan MANDATORY format with guaranteed `COLOR_ATTACHMENT_BIT` + `SAMPLED_IMAGE_FILTER_LINEAR_BIT`
/// format-feature support (no fallback needed — W2's decision).
const SMAA_EDGES_FORMAT: Format = Format::R8G8Unorm;

/// The SMAA `weights` target format: `R8G8B8A8_UNORM` (== [`GBUFFER_FORMAT`]) — the 4-channel
/// per-pixel blending weight (left/top/right/bottom).
const SMAA_WEIGHTS_FORMAT: Format = Format::R8G8B8A8Unorm;

/// The Lighting-L0b `gViewT` lane format: `R32_SFLOAT`, a STORAGE image the marcher
/// stores the full-fp32 surface ray param `t` into and the resolve reads to reconstruct
/// the world position `P = ro + rd * t`. fp32 (not a packed 8-bit lane) avoids the
/// attenuation/cone banding a low-precision `t` would cause. W2: `STORAGE_IMAGE` support
/// on this format is fail-fast-checked at device boot.
const GVIEWT_FORMAT: Format = Format::R32Sfloat;

/// The Render P7 SSAO term `gSsao` format: `R8_UNORM`, a single 8-bit ambient-occlusion lane
/// the (C2) SSAO pass stores and the deferred resolve loads under the `ssao_mode != 0` gate.
/// 8 bits is the engine AO tolerance (the A2 march lands in `gMaterial.g`, also 8-bit). P7:
/// `R8_UNORM`/`STORAGE_IMAGE` support is fail-fast-checked at device boot
/// ([`crate::device::DeviceCaps::r8_unorm_storage_ok`]), so the SSAO image create can never
/// fault on an unsupported format.
const SSAO_FORMAT: Format = Format::R8Unorm;

/// The SSAO edge-avoiding à-trous denoise chain's INTERIOR ping-pong ring format:
/// `R16_UNORM`, a full-res STORAGE image — 16-bit avoids the cumulative 8-bit rounding a
/// multi-level filter would accrue (one channel narrower than `SHADOW_VIS_FORMAT`'s RG16
/// design; SSAO is single-channel AO, not a `(vis, validity)` pair). `R16_UNORM`/`STORAGE_IMAGE`
/// support is device-probed at boot ([`crate::device::DeviceCaps::ssao_atrous_storage_ok`]) —
/// RECORDED-not-fail-fast (mirrors `SHADOW_VIS_FORMAT`'s degrade policy), UNCONDITIONAL (both
/// feature legs — the SSAO à-trous denoise is software, NOT `hwrt`-gated).
const SSAO_ATROUS_RING_FORMAT: Format = Format::R16Unorm;

/// Textured-PBR T6a: the `gPbr` deferred-resolve MRT lane format: `R16G16B16A16_SFLOAT`
/// (`r`=metallic, `g`=roughness, `b`=AO-texture modulation, `a`=emissive-strength modulation), a
/// full-res STORAGE image the (T6c) textured raster writes and the SOFTWARE deferred resolve
/// `.Load`s under the flag-gated `MATERIAL_FLAG_TEXTURED` branch. T6a: UNWRITTEN (no raster pass
/// exists yet) — allocated but never dynamically read (every current material's flag bit is 0).
/// `R16G16B16A16_SFLOAT`/`STORAGE_IMAGE` support is part of the Vulkan 1.0 CORE mandatory format
/// table (unlike `R8_UNORM`/`R16G16_UNORM`, which need a boot probe), so the create — like
/// [`GBUFFER_FORMAT`] — can never fault on an unsupported format. UNCONDITIONAL (both feature
/// legs; the C1 fix keeps `gPbr` a SOFTWARE-resolve-only *binding*, not a `hwrt`-only *image*).
const GPBR_FORMAT: Format = Format::R16G16B16A16Sfloat;

/// Rung 3a: the RT soft-shadow VISIBILITY target `shadow_vis` format: `R16G16_UNORM`, a full-res
/// STORAGE image the VIS pass writes (`R` = per-pixel mesh visibility, `G` = validity mask) and
/// the à-trous denoise reads/writes. UNIFIED with [`SHADOW_VIS2_FORMAT`] to R16G16_UNORM (was RG8):
/// both ping-pong rings share ONE format so the single `[[vk::image_format("rg16")]]` shader pin on
/// `gShadowVis`/`gVisIn`/`gVisOut` matches the bound view on EVERY à-trous parity and every `levels`
/// value — a mixed RG8/RG16 pair silently bound the RG16 ring into an rg8-pinned UAV on odd levels
/// (a format-class mismatch = UB, no validation layer here). `R16G16_UNORM`/`STORAGE_IMAGE` support
/// is device-probed at boot ([`crate::device::DeviceCaps::rg16_unorm_storage_ok`]) — RECORDED-not-
/// fail-fast, so on a device that lacks it the target is not allocated and the denoise stays disabled
/// (steps 4-7 read the [`shadow_denoise_storage_ok`](crate::device::DeviceCaps::shadow_denoise_storage_ok)
/// predicate, which is now rg16-only).
#[cfg(feature = "hwrt")]
const SHADOW_VIS_FORMAT: Format = Format::R16G16Unorm;

/// Rung 3a: the à-trous ping-pong target `shadow_vis2` format: `R16G16_UNORM`, a full-res STORAGE
/// image the multi-level denoise writes/reads. 16-bit avoids the 3× cumulative 8-bit rounding a
/// multi-iteration filter would accrue. `R16G16_UNORM`/`STORAGE_IMAGE` support is device-probed at
/// boot ([`crate::device::DeviceCaps::rg16_unorm_storage_ok`]) — RECORDED-not-fail-fast, same
/// degrade policy as [`SHADOW_VIS_FORMAT`].
#[cfg(feature = "hwrt")]
const SHADOW_VIS2_FORMAT: Format = Format::R16G16Unorm;

/// HW-RT Rung 3b: the temporal motion-vector target `motion_vec` format: `R16G16_SFLOAT` (screen-
/// space Δuv, `R`=Δu/`G`=Δv). fp16 ULP at 64 px ≈ 0.03 px — sufficient for reprojection.
/// Storage support is gated by the SAME `shadow_denoise_storage_ok()` probe as the vis rings.
#[cfg(feature = "hwrt")]
const MOTION_VEC_FORMAT: Format = Format::R16G16Sfloat;

/// HW-RT Rung 3b: the temporal shadow-vis HISTORY ring `shadow_temporal_hist` format:
/// `R16G16B16A16_UNORM` (`R`=accumulated vis, `G`=confidence/frame-count, `B`=prev `view_t`/depth —
/// the W2 disocclusion backstop for the moving-box case — `A`=reserved). UNORM: every lane is a
/// normalized `[0,1]` quantity.
#[cfg(feature = "hwrt")]
const SHADOW_TEMPORAL_HIST_FORMAT: Format = Format::R16G16B16A16Unorm;

// HW-RT Rung 3b C1/H2 invariant: `shadow_temporal_hist` is a PARITY-indexed cross-frame ping-pong
// POOL (frame `fi` writes `pool[fi]`, reads `pool[fi^1]`). The parity index collapses onto the
// physical `[VulkanTexture; FRAMES_IN_FLIGHT]` ring ONLY at FIF == 2 (parity == slot). The
// cross-frame ordering ALSO rests on single-queue submission order (one `vkQueueSubmit` per frame
// reaches the sibling's prior submit); moving the temporal pass to an async-compute queue would
// require a timeline semaphore, not this ring (critic M2). Arming is boot-static via
// `BOYKO_SHADOW_DENOISE`, so the pool config never changes after boot (critic M1 moot).
#[cfg(feature = "hwrt")]
const _: () = assert!(
    FRAMES_IN_FLIGHT == 2,
    "the temporal history is a PARITY-indexed cross-frame ping-pong pool; FIF>=3 needs the hist \
     read/write descriptors + the sink ResId-16 bind selected by PARITY, not the FIF slot"
);

/// HW-RT Rung 3b: the temporal-accumulate OUTPUT `temporal_out` format: `R16G16_UNORM` — the SAME
/// format as [`SHADOW_VIS_FORMAT`] (the DENOISED resolve reads it at `gShadowVis` @21). A DEDICATED
/// target (not an in-place write into the à-trous ping-pong) so the reproject's 3×3 neighborhood
/// read cannot race the accumulate write.
#[cfg(feature = "hwrt")]
const TEMPORAL_OUT_FORMAT: Format = Format::R16G16Unorm;

/// Anti-aliasing Stage 4 (TAA W4): the color-history ring `taa_hist` format:
/// `R16G16B16A16_SFLOAT` — SFLOAT (not UNORM, unlike `SHADOW_TEMPORAL_HIST_FORMAT`) because the
/// carried lane is an accumulated LDR color, and RGBA16F avoids per-blend re-quantization of the
/// already-8-bit-post-tonemap `lit` across many accumulation frames (an 8-bit history bands
/// visibly; a UNORM history would re-introduce that at 16-bit precision too, since the resolve's
/// blend is a repeated read-modify-write, not a single quantize).
const TAA_HIST_FORMAT: Format = Format::R16G16B16A16Sfloat;

// TAA W4 invariant: `taa_hist` is a PARITY-indexed cross-frame ping-pong POOL (frame `fi` writes
// `pool[fi]`, reads `pool[fi^1]`) — the SAME shape [`SHADOW_TEMPORAL_HIST_FORMAT`]'s ring uses.
// The parity index collapses onto the physical `[VulkanTexture; FRAMES_IN_FLIGHT]` ring ONLY at
// FIF == 2 (parity == slot). UNCONDITIONAL (both feature legs — TAA is not `hwrt`-only, unlike
// the shadow-temporal precedent this assert mirrors).
const _: () = assert!(
    FRAMES_IN_FLIGHT == 2,
    "taa_hist is a PARITY-indexed cross-frame ping-pong pool; FIF>=3 needs the read/write \
     descriptors + the sink taa_hist_read bind selected by PARITY, not the FIF slot"
);

/// Asset-streaming plan F7 §5: the vocabulary set's material-buffer binding
/// ([`GBufferTargets::vocab_set`]'s `scene.material_table` entry).
const VOCAB_MATERIAL_BINDING: u32 = 7;

/// Asset-streaming plan F7 §5: the resolve-family sets' material-buffer binding — the
/// index [`resolve_software_entries`] emits `scene.material_table` at, shared verbatim
/// by [`GBufferTargets::resolve_set`] and every HWRT resolve-family variant (they all
/// consume [`resolve_software_entries`]'s output unmodified for this binding).
const RESOLVE_MATERIAL_BINDING: u32 = 4;

/// Asset-streaming plan F7 §5 (C1): the minimum material-bearing ring count — the two
/// ALWAYS-present rings ([`GBufferTargets::vocab_set`] + [`GBufferTargets::resolve_set`]).
/// A sanity floor for [`GBufferTargets::material_set_rings`]'s count debug_assert.
/// `hwrt`-only: on a `not(hwrt)` build `material_set_rings()` always yields exactly these
/// two (no `Option`-guarded ring exists to enumerate), so the floor check is not wired
/// there (nothing to catch).
#[cfg(feature = "hwrt")]
const MATERIAL_SET_RING_COUNT_MIN: usize = 2;

/// The binding count of the SOFTWARE deferred-resolve set (indices 0..=18). The HWRT variant is
/// this plus one (binding 19 = the TLAS). Kept as ONE source so the exact-fill guards + both set
/// builders agree. HW-RT rung R2a-4a raised [`MAX_BIND_GROUP_BINDINGS`](boyko_rhi::MAX_BIND_GROUP_BINDINGS)
/// 19 → 20; the software set stays EXACT at 19 (the under-fill tripwire).
const RESOLVE_SOFTWARE_BINDINGS: usize = 19;

/// Textured-PBR T6a (the critic's C1 fix): the binding count of the SOFTWARE deferred-resolve set
/// INCLUDING the SOFTWARE-ONLY `gPbr` binding 19 (`RESOLVE_SOFTWARE_BINDINGS + 1` = 20). A
/// SEPARATE constant from [`RESOLVE_SOFTWARE_BINDINGS`] *deliberately*: `RESOLVE_SOFTWARE_BINDINGS`
/// stays 19 and remains the UNTOUCHED derivation base every HWRT-family resolve set builds from
/// (`TLAS_ACCEL_BINDING`/`RESOLVE_HWRT_*` all key off it) — bumping `RESOLVE_SOFTWARE_BINDINGS`
/// itself would shift the TLAS 19→20 in every HWRT resolve set and overflow
/// `RESOLVE_HWRT_VIS_MV_BINDINGS` past [`MAX_BIND_GROUP_BINDINGS`](boyko_rhi::MAX_BIND_GROUP_BINDINGS)
/// (24). `gPbr` is appended ONLY to the software set (never into [`resolve_software_entries`]'s
/// shared output), so no HWRT constant or `.spv` is affected.
const RESOLVE_SOFTWARE_TOTAL_BINDINGS: usize = RESOLVE_SOFTWARE_BINDINGS + 1;

/// Asset-streaming plan F7-hwrt (task#11): the binding index every HWRT resolve-family
/// set's `AccelerationStructure` entry occupies — always the FIRST index past the
/// [`RESOLVE_SOFTWARE_BINDINGS`] shared `0..=18` prefix (see e.g. `build_resolve_set`'s
/// `BindGroupEntry::AccelerationStructure` chain link). Derived from the SAME source of
/// truth so the two constants cannot drift.
#[cfg(feature = "hwrt")]
const TLAS_ACCEL_BINDING: u32 = RESOLVE_SOFTWARE_BINDINGS as u32;

/// HW-RT rung 3a: the binding count of the VIS/DENOISED deferred-resolve set (indices 0..=21) — the
/// 21 RESOLVE_INLINE-hwrt bindings (`RESOLVE_SOFTWARE_BINDINGS + 2` = the 19 shared + TLAS @19 +
/// soft-shadow UBO @20) PLUS `gShadowVis` STORAGE image @21. The EXACT-fill tripwire for both the
/// VIS and DENOISED sets — under the rung-3a cap of [`MAX_BIND_GROUP_BINDINGS`](boyko_rhi::MAX_BIND_GROUP_BINDINGS)
/// (22). The software resolve stays EXACT at 19, the RESOLVE_INLINE-hwrt resolve EXACT at 21; only
/// this layout fills 22.
#[cfg(feature = "hwrt")]
const RESOLVE_HWRT_DENOISE_BINDINGS: usize = RESOLVE_SOFTWARE_BINDINGS + 3;

/// HW-RT Rung 3b step 5b: the binding count of the VIS-MV deferred-resolve set (indices 0..=23) —
/// the 22 VIS/DENOISED bindings ([`RESOLVE_HWRT_DENOISE_BINDINGS`]) PLUS the `MotionCam` UNIFORM
/// buffer @22 + the `motion_vec` STORAGE image @23. The EXACT-fill tripwire for the VIS-MV set,
/// filling [`MAX_BIND_GROUP_BINDINGS`](boyko_rhi::MAX_BIND_GROUP_BINDINGS) (24) exactly.
#[cfg(feature = "hwrt")]
const RESOLVE_HWRT_VIS_MV_BINDINGS: usize = RESOLVE_HWRT_DENOISE_BINDINGS + 2;

/// The six per-in-flight-slot G-buffer image RINGS' slot views the resolve set binds — bundled so
/// [`resolve_software_entries`] takes ONE argument for them instead of six (clippy
/// `too_many_arguments`). Each is the `[slot]` view of the corresponding target ring
/// ([`GBufferTargets::albedo`] etc.).
struct ResolveSlotImages<'a> {
    albedo: &'a VulkanTexture,
    normal: &'a VulkanTexture,
    material: &'a VulkanTexture,
    lit: &'a VulkanTexture,
    viewt: &'a VulkanTexture,
    ssao: &'a VulkanTexture,
}

/// Builds the 19 SHARED deferred-resolve [`BindGroupEntry`]s (indices 0..=18) for slot `slot` —
/// the SINGLE source both the software resolve set ([`GBufferTargets::create`]) and the R2a-4b HWRT
/// resolve set consume, so their first 19 bindings CANNOT drift (a drift would be an invisible
/// set↔shader-layout mismatch → device-lost, which the golden never arms HWRT to catch). The HWRT
/// builder appends binding 19 (the TLAS) to this array; the software builder uses it verbatim.
///
/// `imgs` are slot `slot`'s six G-buffer image views; `cluster_grid_buf` / `light_index_buf` are the
/// L1 buffers (or the light-table placeholder when L1 is off). Every entry borrows from `scene` /
/// `imgs` for the caller's `create_bind_group` call.
fn resolve_software_entries<'a>(
    scene: &'a GBufferScene<'a>,
    imgs: &ResolveSlotImages<'a>,
    slot: usize,
    cluster_grid_buf: &'a BoundBuffer,
    light_index_buf: &'a BoundBuffer,
) -> [BindGroupEntry<'a, Vulkan>; RESOLVE_SOFTWARE_BINDINGS] {
    [
        BindGroupEntry::StorageImage { texture: imgs.albedo },
        BindGroupEntry::StorageImage { texture: imgs.normal },
        BindGroupEntry::StorageImage { texture: imgs.material },
        BindGroupEntry::StorageImage { texture: imgs.lit },
        BindGroupEntry::StorageBuffer { buffer: scene.material_table },
        BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[slot] },
        BindGroupEntry::StorageBuffer { buffer: scene.light_table },
        // Lighting L0b: the gViewT lane @7 (the resolve READS it under `mask == 1`).
        BindGroupEntry::StorageImage { texture: imgs.viewt },
        // Lighting L1: the ClusterGrid @8 + LightIndexList @9. The light-table placeholder goes
        // here when L1 is off. Every set built from these entries — the software `resolve_set`
        // and the HWRT/shadow-vis resolve-family variants alike — is bound only by
        // `Renderer::record_gbuffer`, which `render_gbuffer_frame` records only on a DEFERRED
        // boot, so these two entries are read on Deferred frames and on no others. There the
        // resolve READS the pixel's froxel slice only under the THREE-term `use_clusters`
        // (VB-P1k): `clusters_enabled != 0 && cluster_count != 0 && cluster_count <=
        // grid_capacity`, the capacity read off the BOUND `ClusterGrid` descriptor with
        // `GetDimensions`. On the default boot the ENABLED BIT short-circuits it; a Deferred boot
        // that sets `clusters_enabled = true` is stopped by the DIMS term instead (Deferred can
        // never arm `froxel_light_cull`, so `sync_cluster_light_gate` pins the dims to `0`) — see
        // `GBufferTargets::resolve_set`'s doc, including why the two terms past the enabled bit
        // are an out-of-bounds guard rather than a style choice.
        BindGroupEntry::StorageBuffer { buffer: cluster_grid_buf },
        BindGroupEntry::StorageBuffer { buffer: light_index_buf },
        // P6 R1: the SDF edit-list `Buf` @10 (a read-only field CONSUMER; the marcher already
        // uploaded + barriered it, and a `shadow_mode==0` scene never marches — 0%-gate).
        BindGroupEntry::StorageBuffer { buffer: scene.edit_list },
        // Render P7: the SSAO term `gSsao` @11 — ALWAYS bound (the resolve interface is stable
        // regardless of `ssao_mode`); read only under `ssao_mode != 0` (the 0%-gate).
        BindGroupEntry::StorageImage { texture: imgs.ssao },
        // CSM Increment 1b: the cascade shadow-map ARRAY + PCF sampler as ONE combined descriptor
        // @12 + the cascade UBO @13. BOTH always bound (the `.spv` statically references them);
        // PCF-sampled only under `csm_mode != 0` (the 0%-gate).
        BindGroupEntry::CombinedImage {
            texture: scene.csm_cascade_texture,
            sampler: scene.csm_compare_sampler,
        },
        BindGroupEntry::UniformBuffer {
            buffer: &scene.csm_cascade_ring[slot],
        },
        // Shadow Phase 5 Inc-1-GPU: the sparse spot/point shadow-ATLAS array + PCF sampler @14 + the
        // atlas UBO @15. BOTH always bound; PCF-sampled only under `punctual_shadow_mode != 0`.
        BindGroupEntry::CombinedImage {
            texture: scene.shadow_atlas_texture,
            sampler: scene.shadow_atlas_sampler,
        },
        BindGroupEntry::UniformBuffer {
            buffer: scene.shadow_atlas_ubo,
        },
        // SDFDDGI I0: the probe-IRRADIANCE combined image @16 + the DEPTH-MOMENT combined image @17
        // + the `ResolvedDdgi` grid UBO @18. ALL THREE always bound; sampled only under
        // `ddgi_mode != 0` (the 0%-gate). Indices 16/17/18 close the software set at EXACTLY 19.
        BindGroupEntry::CombinedImage {
            texture: scene.ddgi_irr_texture,
            sampler: scene.ddgi_irr_sampler,
        },
        BindGroupEntry::CombinedImage {
            texture: scene.ddgi_depth_texture,
            sampler: scene.ddgi_depth_sampler,
        },
        BindGroupEntry::UniformBuffer {
            buffer: scene.ddgi_grid_ubo,
        },
    ]
}

impl GBufferTargets {
    /// Asset-streaming plan F7 §5 (C1): THE canonical enumeration of every per-FIF
    /// descriptor-set ring that binds the material buffer, paired with its binding
    /// index — co-located with [`resolve_software_entries`] (the SOLE builder that
    /// emits `scene.material_table`) so a reviewer sees the builder and the repoint
    /// list together, and a new resolve variant added there must be added here too.
    /// [`GBufferFrame::repoint_material_table`] walks EXACTLY this list; nothing else
    /// enumerates the material-bearing sets.
    #[cfg(not(feature = "hwrt"))]
    fn material_set_rings(&self) -> impl Iterator<Item = (&[VulkanBindGroup; FRAMES_IN_FLIGHT], u32)> {
        [
            (&self.vocab_set, VOCAB_MATERIAL_BINDING),
            (&self.resolve_set, RESOLVE_MATERIAL_BINDING),
        ]
        .into_iter()
    }

    /// HW-RT variant of [`Self::material_set_rings`]: the two always-present rings PLUS
    /// every `Option`-guarded HWRT resolve-family ring that exists on this device/config
    /// (`None` on the OFF path — flattened out, not enumerated).
    #[cfg(feature = "hwrt")]
    fn material_set_rings(&self) -> impl Iterator<Item = (&[VulkanBindGroup; FRAMES_IN_FLIGHT], u32)> {
        [
            Some((&self.vocab_set, VOCAB_MATERIAL_BINDING)),
            Some((&self.resolve_set, RESOLVE_MATERIAL_BINDING)),
            self.resolve_set_hwrt
                .as_ref()
                .map(|s| (s, RESOLVE_MATERIAL_BINDING)),
            self.shadow_vis_resolve_set
                .as_ref()
                .map(|s| (s, RESOLVE_MATERIAL_BINDING)),
            self.shadow_denoised_resolve_set
                .as_ref()
                .map(|s| (s, RESOLVE_MATERIAL_BINDING)),
            self.shadow_vis_mv_resolve_set
                .as_ref()
                .map(|s| (s, RESOLVE_MATERIAL_BINDING)),
            self.shadow_temporal_denoised_resolve_set
                .as_ref()
                .map(|s| (s, RESOLVE_MATERIAL_BINDING)),
        ]
        .into_iter()
        .flatten()
    }

    /// Asset-streaming plan F7 §5 (C1, review O1): the count [`Self::material_set_rings`]
    /// MUST yield for THIS already-built `self` — read directly off the SAME `Option`
    /// fields `material_set_rings` enumerates (`.is_some()`), NOT re-derived from the
    /// arming predicates `create`'s builders gate on. A predicate-based re-derivation is
    /// unsound as a secondary check: a device where the arming predicate holds but a
    /// specific ring's `create_bind_group` degraded to `None` (an internal builder
    /// failure/degrade path, independent of the predicate) would make a predicate-based
    /// count diverge from `material_set_rings().count()` and trip this debug_assert
    /// SPURIOUSLY. Reading `self`'s own fields instead can only diverge from
    /// `material_set_rings()` when a NEW field is added to `Self` without a matching
    /// entry there — exactly the C1 regression this guard exists to catch.
    ///
    /// This debug_assert is a SECONDARY self-consistency net; the PRIMARY exhaustiveness
    /// guarantees are `material_set_rings`'s co-location with `resolve_software_entries`
    /// (a reviewer sees both together) and the headless C1 repoint-counter test (F7 §12).
    #[cfg(feature = "hwrt")]
    fn expected_material_ring_count(&self) -> usize {
        MATERIAL_SET_RING_COUNT_MIN
            + self.resolve_set_hwrt.is_some() as usize
            + self.shadow_vis_resolve_set.is_some() as usize
            + self.shadow_denoised_resolve_set.is_some() as usize
            + self.shadow_vis_mv_resolve_set.is_some() as usize
            + self.shadow_temporal_denoised_resolve_set.is_some() as usize
    }

    /// Asset-streaming plan F7-hwrt (task#11): THE canonical enumeration of every per-FIF
    /// AS-bearing descriptor-set ring, paired with [`TLAS_ACCEL_BINDING`] — the HWRT subset
    /// of [`Self::material_set_rings`] MINUS the two software-only sets (`vocab_set`/
    /// `resolve_set`, which declare no `AccelerationStructure` binding at all).
    /// [`GBufferFrame::repoint_tlas_accel`] walks EXACTLY this list when the per-slot TLAS
    /// grows — a resolve variant added without an entry here would dangle at the freed AS
    /// handle the instant the superseded TLAS is retired (the C1-class UAF
    /// [`Self::expected_tlas_accel_ring_count`]'s debug_assert guards against). The
    /// MV-only sets (`shadow_vis_mv_resolve_set`/`shadow_temporal_denoised_resolve_set`)
    /// are naturally `Option`-flattened away on a device with `mv.is_none()` — this is why
    /// an `mv`-absent RT device (C1's Optional gap) does not affect the AS repoint: fewer
    /// sets are simply enumerated, none missed.
    #[cfg(feature = "hwrt")]
    fn tlas_accel_sets(&self) -> impl Iterator<Item = (&[VulkanBindGroup; FRAMES_IN_FLIGHT], u32)> {
        [
            self.resolve_set_hwrt.as_ref(),
            self.shadow_vis_resolve_set.as_ref(),
            self.shadow_denoised_resolve_set.as_ref(),
            self.shadow_vis_mv_resolve_set.as_ref(),
            self.shadow_temporal_denoised_resolve_set.as_ref(),
        ]
        .into_iter()
        .flatten()
        .map(|s| (s, TLAS_ACCEL_BINDING))
    }

    /// The count [`Self::tlas_accel_sets`] MUST yield for THIS already-built `self` — read
    /// directly off the SAME `Option` fields it enumerates, mirroring
    /// [`Self::expected_material_ring_count`]'s reasoning (a re-derived arming predicate
    /// would spuriously diverge from a ring that degraded to `None` for an unrelated
    /// internal reason).
    #[cfg(feature = "hwrt")]
    fn expected_tlas_accel_ring_count(&self) -> usize {
        self.resolve_set_hwrt.is_some() as usize
            + self.shadow_vis_resolve_set.is_some() as usize
            + self.shadow_denoised_resolve_set.is_some() as usize
            + self.shadow_vis_mv_resolve_set.is_some() as usize
            + self.shadow_temporal_denoised_resolve_set.is_some() as usize
    }
}

/// HW-RT rung 3a: the bundle [`GBufferTargets::build_shadow_denoise_sets`] returns — the VIS +
/// DENOISED resolve set rings, the per-level à-trous set rings, and the à-trous edge-stop UBO ring.
/// Moved field-by-field into the [`GBufferTargets`] `Option`s at `create` time.
#[cfg(feature = "hwrt")]
struct ShadowDenoiseSets {
    /// The VIS resolve set RING (`gShadowVis` @21 = `shadow_vis[i]`, the VIS pass WRITES it).
    vis_resolve: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// The DENOISED resolve set RING (`gShadowVis` @21 = the FINAL à-trous output, READ).
    denoised_resolve: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// The per-level à-trous set rings (`sets[level][fi]`).
    atrous: [[VulkanBindGroup; FRAMES_IN_FLIGHT]; crate::present::MAX_ATROUS_LEVELS as usize],
    /// The à-trous edge-stop UBO ring (16 B `HostVisibleCoherent` per FIF slot, zero-seeded).
    ubo: [BoundBuffer; FRAMES_IN_FLIGHT],
}

/// The SSAO à-trous denoise chain: the bundle [`GBufferTargets::build_ssao_atrous_sets`] returns —
/// the FIVE role-keyed descriptor set rings [`crate::present::ssao_atrous_step`]'s
/// [`crate::present::AtrousStepRole`] selects between. Moved field-by-field into the
/// [`GBufferTargets`] `Option`s at `create` time. UNCONDITIONAL (both feature legs — SOFTWARE,
/// NOT `hwrt`-gated, unlike [`ShadowDenoiseSets`]).
struct SsaoAtrousSets {
    /// `level == 0`'s set RING: `gAoIn` @0 = the frozen R8 `gSsao[i]` endpoint, `gAoOut` @1 =
    /// `ssao_ring_a[i]`.
    read8: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// An interior set RING reading `ssao_ring_a`: `gAoIn` @0 = `ssao_ring_a[i]`, `gAoOut` @1 =
    /// `ssao_ring_b[i]`.
    interior_from0: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// An interior set RING reading `ssao_ring_b`: `gAoIn` @0 = `ssao_ring_b[i]`, `gAoOut` @1 =
    /// `ssao_ring_a[i]`.
    interior_from1: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// The LAST-level set RING reading `ssao_ring_a`: `gAoIn` @0 = `ssao_ring_a[i]`, `gAoOut` @1 =
    /// the frozen R8 `gSsao[i]` endpoint (the write-BACK the resolve reads).
    write8_from0: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// The LAST-level set RING reading `ssao_ring_b`: `gAoIn` @0 = `ssao_ring_b[i]`, `gAoOut` @1 =
    /// the frozen R8 `gSsao[i]` endpoint.
    write8_from1: [VulkanBindGroup; FRAMES_IN_FLIGHT],
}

/// HW-RT Rung 3b step 6: the bundle [`GBufferTargets::build_shadow_temporal_sets`] returns — the
/// temporal reproject UBO ring, the 8-binding temporal reproject set ring, and the DENOISED-temporal
/// resolve set ring. Moved field-by-field into the [`GBufferTargets`] `Option`s at `create` time.
#[cfg(feature = "hwrt")]
struct ShadowTemporalSets {
    /// The temporal reproject UBO ring (16 B `HostVisibleCoherent` per FIF slot, zero-seeded).
    ubo: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// The 8-binding temporal reproject set ring (`gVisIn`/motion/viewt/hist-in/hist-out/temporal-out
    /// + the temporal UBO + the camera UBO).
    temporal: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// The DENOISED-temporal resolve set ring (`gShadowVis` @21 = `temporal_out[i]`, the READ).
    denoised: [VulkanBindGroup; FRAMES_IN_FLIGHT],
}

/// Anti-aliasing Stage 4 (TAA W5): the bundle [`GBufferTargets::build_taa_resolve_set`] returns —
/// the tunables UBO ring, the DEDICATED `MotionCam` UBO ring, and the 8-binding resolve set ring.
/// Moved field-by-field into the [`GBufferTargets`] `Option`s at `create` time.
struct TaaResolveSets {
    /// The `ResolvedTaa` tunables UBO ring (48 B `HostVisibleCoherent` per FIF slot, zero-seeded;
    /// rung T2 grew this from 16 B).
    taa_ubo: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// The DEDICATED `MotionCam` UBO ring (128 B `HostVisibleCoherent` per FIF slot, zero-seeded)
    /// — SEPARATE from the hwrt mesh-shadow `motion_cam_ubo` (see `TaaActivation`'s doc).
    motion_cam_ubo: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// The 8-binding resolve set ring (`gLit`/`gViewT`/`gHistIn`/`gHistOut`/`gAaOut` + the tunables
    /// + camera + `MotionCam` UBOs).
    set: [VulkanBindGroup; FRAMES_IN_FLIGHT],
}

/// The always-present G-buffer image RINGS (one texture per in-flight frame), built FIRST in
/// [`GBufferTargets::create`] — the lock-free cross-frame Write-After-Read fix (frame N+1 writes
/// slot `i`'s images while frame N reads slot `j`'s). Extracted into a bundle so `create` builds
/// them in ONE call and its error ladder collapses: [`Self::build`] drains its OWN partial ring on
/// failure, so the orchestrator only tears down the (fully-built) prior bundles. Flattened back into
/// the [`GBufferTargets`] fields at `create` time, so `present/` readers keep the same `targets.<x>`
/// paths.
struct CoreImages {
    depth: [VulkanTexture; FRAMES_IN_FLIGHT],
    albedo: [VulkanTexture; FRAMES_IN_FLIGHT],
    normal: [VulkanTexture; FRAMES_IN_FLIGHT],
    material: [VulkanTexture; FRAMES_IN_FLIGHT],
    lit: [VulkanTexture; FRAMES_IN_FLIGHT],
    viewt: [VulkanTexture; FRAMES_IN_FLIGHT],
    ssao: [VulkanTexture; FRAMES_IN_FLIGHT],
    /// Textured-PBR T6a: the `gPbr` MRT lane RING, built LAST (after `ssao`). UNCONDITIONAL (both
    /// feature legs); UNWRITTEN this rung (no raster pass names it yet — T6c adds that).
    pbr: [VulkanTexture; FRAMES_IN_FLIGHT],
}

impl CoreImages {
    /// Allocates the eight always-present G-buffer image rings at `extent` in acquisition order
    /// (depth → albedo → normal → material → lit → viewt → ssao → pbr). On any ring's partial
    /// failure the slots already built in THAT ring are drained AND every fully-built prior ring is
    /// destroyed (reverse acquisition), so nothing leaks; the orchestrator has no partials to
    /// reason about beyond the bundles it built before this call.
    fn build(ctx: &VulkanContext, extent: VkExtent2D) -> Result<Self, SwapchainError> {
        // SAFETY (shared by both closures): `ctx` is the live context each texture was created on;
        // none is referenced by any submission (the build phase, before any record/submit); each ring
        // slot is destroyed exactly once — a completed ring is consumed by value (`destroy_ring`), a
        // partial ring is `take`-drained (`drain_partial`).
        let destroy_ring = |ring: [VulkanTexture; FRAMES_IN_FLIGHT]| unsafe {
            for t in ring {
                RhiDevice::destroy_texture(ctx, t);
            }
        };
        let drain_partial = |ring: &mut [Option<VulkanTexture>; FRAMES_IN_FLIGHT]| unsafe {
            for slot in ring.iter_mut() {
                if let Some(t) = slot.take() {
                    RhiDevice::destroy_texture(ctx, t);
                }
            }
        };

        // Depth: DEPTH_STENCIL_ATTACHMENT (rasterize into) | SAMPLED (marcher .Load).
        let depth_desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format: Format::D32Sfloat,
            dimension: TextureDimension::D2,
            usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        };
        let mut depth_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in depth_slots.iter_mut() {
            match RhiDevice::create_texture(ctx, &depth_desc).map_err(SwapchainError::DepthImage) {
                Ok(t) => *slot = Some(t),
                Err(e) => {
                    drain_partial(&mut depth_slots);
                    return Err(e);
                }
            }
        }
        let depth: [VulkanTexture; FRAMES_IN_FLIGHT] =
            depth_slots.map(|s| s.expect("invariant: every depth ring slot built before here"));

        // ALBEDO: STORAGE (marcher store) | SAMPLED (the present-blit, pass C) |
        // COLOR_ATTACHMENT (Render P5-r0: the mesh raster pass A writes it as MRT@0).
        let mut albedo_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in albedo_slots.iter_mut() {
            match GBufferTargets::create_gbuffer_image(
                ctx,
                extent,
                ImageUsage::STORAGE | ImageUsage::SAMPLED | ImageUsage::COLOR_ATTACHMENT,
            ) {
                Ok(t) => *slot = Some(t),
                Err(e) => {
                    drain_partial(&mut albedo_slots);
                    destroy_ring(depth);
                    return Err(e);
                }
            }
        }
        let albedo: [VulkanTexture; FRAMES_IN_FLIGHT] =
            albedo_slots.map(|s| s.expect("invariant: every albedo ring slot built before here"));

        // NORMAL / MATERIAL: STORAGE (marcher store) | COLOR_ATTACHMENT (Render P5-r0: the
        // mesh raster pass A writes them as MRT@1 / MRT@2). Read by the deferred resolve.
        let mut normal_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in normal_slots.iter_mut() {
            match GBufferTargets::create_gbuffer_image(
                ctx,
                extent,
                ImageUsage::STORAGE | ImageUsage::COLOR_ATTACHMENT,
            ) {
                Ok(t) => *slot = Some(t),
                Err(e) => {
                    drain_partial(&mut normal_slots);
                    destroy_ring(albedo);
                    destroy_ring(depth);
                    return Err(e);
                }
            }
        }
        let normal: [VulkanTexture; FRAMES_IN_FLIGHT] =
            normal_slots.map(|s| s.expect("invariant: every normal ring slot built before here"));

        let mut material_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in material_slots.iter_mut() {
            match GBufferTargets::create_gbuffer_image(
                ctx,
                extent,
                ImageUsage::STORAGE | ImageUsage::COLOR_ATTACHMENT,
            ) {
                Ok(t) => *slot = Some(t),
                Err(e) => {
                    drain_partial(&mut material_slots);
                    destroy_ring(normal);
                    destroy_ring(albedo);
                    destroy_ring(depth);
                    return Err(e);
                }
            }
        }
        let material: [VulkanTexture; FRAMES_IN_FLIGHT] = material_slots
            .map(|s| s.expect("invariant: every material ring slot built before here"));

        // LIT: the deferred resolve's STORAGE store output; also SAMPLED by the
        // present-blit (pass C) and TRANSFER_SRC so an offscreen golden could read it back.
        // Multi-paradigm render-path plan, rung R4b-b: ALSO `COLOR_ATTACHMENT` — Forward v1
        // reuses this SAME ring as `forward_opaque`'s color-attachment write target (Decision
        // 2's C5 per-path producer access, `ForwardTargets`'s doc: "full + additive
        // ForwardTargets", Option 2). Purely PERMISSIVE for Deferred: no Deferred pass ever
        // transitions `lit` to `COLOR_ATTACHMENT_OPTIMAL` (its own resolve writes it via
        // STORAGE/GENERAL), so this extra allowed-usage bit changes neither Deferred's derived
        // barriers nor its rendered pixels — an unexercised capability, byte-identical output.
        let mut lit_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in lit_slots.iter_mut() {
            match GBufferTargets::create_gbuffer_image(
                ctx,
                extent,
                ImageUsage::STORAGE
                    | ImageUsage::SAMPLED
                    | ImageUsage::TRANSFER_SRC
                    | ImageUsage::COLOR_ATTACHMENT,
            ) {
                Ok(t) => *slot = Some(t),
                Err(e) => {
                    drain_partial(&mut lit_slots);
                    destroy_ring(material);
                    destroy_ring(normal);
                    destroy_ring(albedo);
                    destroy_ring(depth);
                    return Err(e);
                }
            }
        }
        let lit: [VulkanTexture; FRAMES_IN_FLIGHT] =
            lit_slots.map(|s| s.expect("invariant: every lit ring slot built before here"));

        // Lighting L0b: the R32_SFLOAT `gViewT` lane (the marcher's surface `t`).
        let mut viewt_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in viewt_slots.iter_mut() {
            match GBufferTargets::create_viewt_image(ctx, extent) {
                Ok(t) => *slot = Some(t),
                Err(e) => {
                    drain_partial(&mut viewt_slots);
                    destroy_ring(lit);
                    destroy_ring(material);
                    destroy_ring(normal);
                    destroy_ring(albedo);
                    destroy_ring(depth);
                    return Err(e);
                }
            }
        }
        let viewt: [VulkanTexture; FRAMES_IN_FLIGHT] =
            viewt_slots.map(|s| s.expect("invariant: every viewt ring slot built before here"));

        // Render P7: the R8_UNORM `gSsao` term (ALWAYS allocated — the resolve descriptor
        // interface is stable regardless of `ssao_mode`; no SSAO pass writes it yet, C2 adds
        // that). Read by the resolve only under `ssao_mode != 0` (0 every pre-P7 scene).
        let mut ssao_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in ssao_slots.iter_mut() {
            match GBufferTargets::create_ssao_image(ctx, extent) {
                Ok(t) => *slot = Some(t),
                Err(e) => {
                    drain_partial(&mut ssao_slots);
                    destroy_ring(viewt);
                    destroy_ring(lit);
                    destroy_ring(material);
                    destroy_ring(normal);
                    destroy_ring(albedo);
                    destroy_ring(depth);
                    return Err(e);
                }
            }
        }
        let ssao: [VulkanTexture; FRAMES_IN_FLIGHT] =
            ssao_slots.map(|s| s.expect("invariant: every ssao ring slot built before here"));

        // Textured-PBR T6a: the `gPbr` MRT lane, built LAST (UNCONDITIONAL, both feature legs).
        // UNWRITTEN this rung (no producer names it — T6c's raster does); the SOFTWARE resolve's
        // gPbr@19 read never dynamically observes its contents (the flag-gated `.Load` is dead for
        // every current material), so the create needs no boot-clear.
        let mut pbr_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in pbr_slots.iter_mut() {
            match GBufferTargets::create_pbr_image(ctx, extent) {
                Ok(t) => *slot = Some(t),
                Err(e) => {
                    drain_partial(&mut pbr_slots);
                    destroy_ring(ssao);
                    destroy_ring(viewt);
                    destroy_ring(lit);
                    destroy_ring(material);
                    destroy_ring(normal);
                    destroy_ring(albedo);
                    destroy_ring(depth);
                    return Err(e);
                }
            }
        }
        let pbr: [VulkanTexture; FRAMES_IN_FLIGHT] =
            pbr_slots.map(|s| s.expect("invariant: every pbr ring slot built before here"));

        Ok(Self { depth, albedo, normal, material, lit, viewt, ssao, pbr })
    }

    /// Tears down the eight image rings in reverse acquisition order (pbr → depth), consuming
    /// `self`.
    ///
    /// # Safety
    ///
    /// `ctx` is the live context the textures were created on; no submission references them; each is
    /// destroyed exactly once (the by-value `self`).
    unsafe fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: per the contract `ctx` is live and nothing references these textures; each was
        // created on `ctx` and is destroyed exactly once, in reverse acquisition order.
        unsafe {
            for t in self.pbr {
                RhiDevice::destroy_texture(ctx, t);
            }
            for t in self.ssao {
                RhiDevice::destroy_texture(ctx, t);
            }
            for t in self.viewt {
                RhiDevice::destroy_texture(ctx, t);
            }
            for t in self.lit {
                RhiDevice::destroy_texture(ctx, t);
            }
            for t in self.material {
                RhiDevice::destroy_texture(ctx, t);
            }
            for t in self.normal {
                RhiDevice::destroy_texture(ctx, t);
            }
            for t in self.albedo {
                RhiDevice::destroy_texture(ctx, t);
            }
            for t in self.depth {
                RhiDevice::destroy_texture(ctx, t);
            }
        }
    }
}

/// Rung 3a (`hwrt`): the two RG16 soft-shadow-visibility ping-pong image RINGS (`shadow_vis` +
/// `shadow_vis2`), built together right after [`CoreImages`] iff the device advertises RG16 storage
/// ([`crate::device::DeviceCaps::shadow_denoise_storage_ok`]). A bundle so [`GBufferTargets::create`]
/// builds them in one call with a self-draining error path; flattened into the two `Option` fields at
/// `create` time.
#[cfg(feature = "hwrt")]
struct ShadowVisImages {
    shadow_vis: [VulkanTexture; FRAMES_IN_FLIGHT],
    shadow_vis2: [VulkanTexture; FRAMES_IN_FLIGHT],
}

#[cfg(feature = "hwrt")]
impl ShadowVisImages {
    /// Allocates the two RG16 ping-pong rings at `extent`, or `Ok(None)` on a device lacking RG16
    /// storage (the DDGI-degrade discipline: the denoise is opt-in, a missing format disables it,
    /// never a boot fault). On a mid-ring failure the partial ring is drained + the (fully-built)
    /// first ring destroyed (reverse acquisition); the orchestrator owns the prior [`CoreImages`],
    /// which it tears down on this method's `Err`.
    fn build(ctx: &VulkanContext, extent: VkExtent2D) -> Result<Option<Self>, SwapchainError> {
        if !ctx.device_caps().shadow_denoise_storage_ok() {
            return Ok(None);
        }
        // SAFETY (both closures): `ctx` is live; no submission references these textures (build
        // phase); each ring slot is destroyed exactly once (a completed ring is consumed by value, a
        // partial ring is `take`-drained).
        let destroy_ring = |ring: [VulkanTexture; FRAMES_IN_FLIGHT]| unsafe {
            for t in ring {
                RhiDevice::destroy_texture(ctx, t);
            }
        };
        let drain_partial = |ring: &mut [Option<VulkanTexture>; FRAMES_IN_FLIGHT]| unsafe {
            for slot in ring.iter_mut() {
                if let Some(t) = slot.take() {
                    RhiDevice::destroy_texture(ctx, t);
                }
            }
        };

        let mut vis_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in vis_slots.iter_mut() {
            match GBufferTargets::create_shadow_vis_image(ctx, extent) {
                Ok(t) => *slot = Some(t),
                Err(e) => {
                    drain_partial(&mut vis_slots);
                    return Err(e);
                }
            }
        }
        let shadow_vis: [VulkanTexture; FRAMES_IN_FLIGHT] =
            vis_slots.map(|s| s.expect("invariant: every shadow_vis ring slot built before here"));

        let mut vis2_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in vis2_slots.iter_mut() {
            match GBufferTargets::create_shadow_vis2_image(ctx, extent) {
                Ok(t) => *slot = Some(t),
                Err(e) => {
                    drain_partial(&mut vis2_slots);
                    destroy_ring(shadow_vis);
                    return Err(e);
                }
            }
        }
        let shadow_vis2: [VulkanTexture; FRAMES_IN_FLIGHT] =
            vis2_slots.map(|s| s.expect("invariant: every shadow_vis2 ring slot built before here"));

        Ok(Some(Self { shadow_vis, shadow_vis2 }))
    }

    /// Tears down the two ping-pong rings in reverse acquisition order (`shadow_vis2` → `shadow_vis`).
    ///
    /// # Safety
    ///
    /// `ctx` is live; no submission references these textures; each is destroyed exactly once.
    unsafe fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: per the contract `ctx` is live and nothing references these textures; each was
        // created on `ctx` and is destroyed exactly once, in reverse acquisition order.
        unsafe {
            for t in self.shadow_vis2 {
                RhiDevice::destroy_texture(ctx, t);
            }
            for t in self.shadow_vis {
                RhiDevice::destroy_texture(ctx, t);
            }
        }
    }
}

/// The SSAO à-trous denoise chain: the two `R16_UNORM` interior ping-pong image RINGS
/// (`ssao_ring_a` + `ssao_ring_b`), built together right after [`ShadowVisImages`] (or right
/// after [`CoreImages`] on a `not(hwrt)` build) iff the device advertises `R16_UNORM` storage
/// ([`crate::device::DeviceCaps::ssao_atrous_storage_ok`]). UNCONDITIONAL (both feature legs —
/// SOFTWARE, NOT `hwrt`-gated), mirroring [`ShadowVisImages`]'s bundle shape one channel
/// narrower (single AO lane, not a `(vis, validity)` pair) and gated on a SEPARATE device probe.
/// A bundle so [`GBufferTargets::create`] builds them in one call with a self-draining error
/// path; flattened into the two `Option` fields at `create` time.
struct SsaoAtrousImages {
    ssao_ring_a: [VulkanTexture; FRAMES_IN_FLIGHT],
    ssao_ring_b: [VulkanTexture; FRAMES_IN_FLIGHT],
}

impl SsaoAtrousImages {
    /// Allocates the two `R16_UNORM` ping-pong rings at `extent`, or `Ok(None)` on a device
    /// lacking `R16_UNORM` storage (the DDGI/shadow-denoise degrade discipline: the à-trous
    /// denoise is opt-in, a missing format disables it — the resolve then reads the raw,
    /// un-denoised gather, never a boot fault). On a mid-ring failure the partial ring is drained
    /// AND the (fully-built) first ring destroyed (reverse acquisition); the orchestrator owns
    /// the prior bundles, which it tears down on this method's `Err`.
    fn build(ctx: &VulkanContext, extent: VkExtent2D) -> Result<Option<Self>, SwapchainError> {
        if !ctx.device_caps().ssao_atrous_storage_ok() {
            return Ok(None);
        }
        let mut a_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] = [const { None }; FRAMES_IN_FLIGHT];
        for slot in a_slots.iter_mut() {
            match GBufferTargets::create_ssao_atrous_ring_image(ctx, extent) {
                Ok(t) => *slot = Some(t),
                Err(e) => {
                    // SAFETY: `ctx` is live; no submission references these textures (build
                    // phase); the partial ring [0..i) is drained exactly once.
                    unsafe {
                        for s in a_slots.iter_mut() {
                            if let Some(t) = s.take() {
                                RhiDevice::destroy_texture(ctx, t);
                            }
                        }
                    }
                    return Err(e);
                }
            }
        }
        let ssao_ring_a: [VulkanTexture; FRAMES_IN_FLIGHT] =
            a_slots.map(|s| s.expect("invariant: every ssao_ring_a slot built before here"));

        let mut b_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] = [const { None }; FRAMES_IN_FLIGHT];
        for slot in b_slots.iter_mut() {
            match GBufferTargets::create_ssao_atrous_ring_image(ctx, extent) {
                Ok(t) => *slot = Some(t),
                Err(e) => {
                    // SAFETY: `ctx` is live; no submission references these textures; the
                    // partial `ssao_ring_b` ring [0..i) plus the fully-built `ssao_ring_a` ring
                    // are each drained exactly once (reverse acquisition).
                    unsafe {
                        for s in b_slots.iter_mut() {
                            if let Some(t) = s.take() {
                                RhiDevice::destroy_texture(ctx, t);
                            }
                        }
                        for t in ssao_ring_a {
                            RhiDevice::destroy_texture(ctx, t);
                        }
                    }
                    return Err(e);
                }
            }
        }
        let ssao_ring_b: [VulkanTexture; FRAMES_IN_FLIGHT] =
            b_slots.map(|s| s.expect("invariant: every ssao_ring_b slot built before here"));

        Ok(Some(Self { ssao_ring_a, ssao_ring_b }))
    }

    /// Tears down the two ping-pong rings in reverse acquisition order (`ssao_ring_b` →
    /// `ssao_ring_a`).
    ///
    /// # Safety
    ///
    /// `ctx` is live; no submission references these textures; each is destroyed exactly once.
    unsafe fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: per the contract `ctx` is live and nothing references these textures; each was
        // created on `ctx` and is destroyed exactly once, in reverse acquisition order.
        unsafe {
            for t in self.ssao_ring_b {
                RhiDevice::destroy_texture(ctx, t);
            }
            for t in self.ssao_ring_a {
                RhiDevice::destroy_texture(ctx, t);
            }
        }
    }
}

/// Anti-aliasing Stage 1/2/3: the FXAA/SMAA/SSAA output image RING (`aa_out`), built right
/// after [`CoreImages`] iff any of `scene.aa`/`scene.smaa`/`scene.ssaa` is armed.
/// UNCONDITIONAL (both feature legs — unlike [`ShadowVisImages`], AA is not `hwrt`-only). A
/// one-field bundle (mirroring [`ShadowVisImages`]'s shape) so [`GBufferTargets::create`] can
/// `?`-propagate a build failure with a self-contained error path.
struct AaImages {
    aa_out: [VulkanTexture; FRAMES_IN_FLIGHT],
}

impl AaImages {
    /// Allocates the `aa_out` ring at `aa_extent`: `COLOR_ATTACHMENT | SAMPLED`,
    /// [`GBUFFER_FORMAT`] (`R8G8B8A8_UNORM`) — the FXAA/SMAA-blend pass's full-screen-triangle
    /// render target (or the SSAA downsample's), later sampled by the present-blit.
    /// `aa_extent` is `present_extent` for Fxaa/Smaa, but the NATIVE extent for Ssaa (where
    /// `present_extent` is 2×) — the caller ([`GBufferTargets::create`]) picks the right value;
    /// this is the single point where `aa_out`'s size is materialized. On a mid-ring failure
    /// the partial ring is drained (reverse acquisition); the orchestrator owns the prior
    /// [`CoreImages`], which it tears down on this method's `Err`.
    fn build(ctx: &VulkanContext, aa_extent: VkExtent2D) -> Result<Self, SwapchainError> {
        let mut slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] = [const { None }; FRAMES_IN_FLIGHT];
        for slot in slots.iter_mut() {
            match GBufferTargets::create_gbuffer_image(
                ctx,
                aa_extent,
                // COLOR_ATTACHMENT (FXAA/SMAA/SSAA write it via a fragment pass) | SAMPLED (the
                // present-blit samples it) | STORAGE (the TAA compute resolve `.Store`s into it as
                // a UAV — C1 fix: a UAV store on an image without STORAGE usage is device-lost UB).
                // R8G8B8A8_UNORM is a Vulkan-mandatory STORAGE_IMAGE format, so the extra bit
                // cannot fault; the added usage does not change the FXAA/SMAA/SSAA rendered pixels.
                ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED | ImageUsage::STORAGE,
            ) {
                Ok(t) => *slot = Some(t),
                Err(e) => {
                    // SAFETY: `ctx` is live; no submission references these textures (build
                    // phase); the partial ring [0..i) is drained exactly once.
                    unsafe {
                        for s in slots.iter_mut() {
                            if let Some(t) = s.take() {
                                RhiDevice::destroy_texture(ctx, t);
                            }
                        }
                    }
                    return Err(e);
                }
            }
        }
        let aa_out: [VulkanTexture; FRAMES_IN_FLIGHT] =
            slots.map(|s| s.expect("invariant: every aa_out ring slot built before here"));
        Ok(Self { aa_out })
    }

    /// Tears down the `aa_out` ring, consuming `self`.
    ///
    /// # Safety
    ///
    /// `ctx` is live; no submission references these textures; each is destroyed exactly once.
    unsafe fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: per the contract `ctx` is live and nothing references these textures; each was
        // created on `ctx` and is destroyed exactly once.
        unsafe {
            for t in self.aa_out {
                RhiDevice::destroy_texture(ctx, t);
            }
        }
    }
}

/// Anti-aliasing Stage 2: the SMAA `edges`/`weights` output image RINGS, built right after
/// [`AaImages`] iff `scene.smaa` is armed. UNCONDITIONAL (both feature legs). A two-field
/// bundle (mirroring [`AaImages`]'s shape) so [`GBufferTargets::create`] can `?`-propagate a
/// build failure with a self-contained error path.
struct SmaaImages {
    edges: [VulkanTexture; FRAMES_IN_FLIGHT],
    weights: [VulkanTexture; FRAMES_IN_FLIGHT],
}

impl SmaaImages {
    /// Allocates the `edges` ring (`R8G8_UNORM`) then the `weights` ring
    /// (`R8G8B8A8_UNORM`), both `COLOR_ATTACHMENT | SAMPLED` at `extent`, via
    /// [`GBufferTargets::create_gbuffer_image_fmt`]. On a mid-ring failure the partial rings
    /// are drained (reverse acquisition: weights' partial slots, then the fully-built
    /// `edges` ring); the orchestrator owns the prior bundles ([`AaImages`]/[`CoreImages`]),
    /// which it tears down on this method's `Err`.
    fn build(ctx: &VulkanContext, extent: VkExtent2D) -> Result<Self, SwapchainError> {
        let mut edge_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in edge_slots.iter_mut() {
            match GBufferTargets::create_gbuffer_image_fmt(
                ctx,
                extent,
                SMAA_EDGES_FORMAT,
                ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED,
            ) {
                Ok(t) => *slot = Some(t),
                Err(e) => {
                    // SAFETY: `ctx` is live; no submission references these textures (build
                    // phase); the partial ring [0..i) is drained exactly once.
                    unsafe {
                        for s in edge_slots.iter_mut() {
                            if let Some(t) = s.take() {
                                RhiDevice::destroy_texture(ctx, t);
                            }
                        }
                    }
                    return Err(e);
                }
            }
        }
        let edges: [VulkanTexture; FRAMES_IN_FLIGHT] =
            edge_slots.map(|s| s.expect("invariant: every smaa_edges ring slot built before here"));

        let mut weight_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in weight_slots.iter_mut() {
            match GBufferTargets::create_gbuffer_image_fmt(
                ctx,
                extent,
                SMAA_WEIGHTS_FORMAT,
                ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED,
            ) {
                Ok(t) => *slot = Some(t),
                Err(e) => {
                    // SAFETY: `ctx` is live; no submission references these textures; the
                    // partial `weights` ring [0..i) plus the fully-built `edges` ring are
                    // each drained exactly once (reverse acquisition).
                    unsafe {
                        for s in weight_slots.iter_mut() {
                            if let Some(t) = s.take() {
                                RhiDevice::destroy_texture(ctx, t);
                            }
                        }
                        for t in edges {
                            RhiDevice::destroy_texture(ctx, t);
                        }
                    }
                    return Err(e);
                }
            }
        }
        let weights: [VulkanTexture; FRAMES_IN_FLIGHT] = weight_slots
            .map(|s| s.expect("invariant: every smaa_weights ring slot built before here"));

        Ok(Self { edges, weights })
    }

    /// Tears down the `weights` ring then the `edges` ring (reverse acquisition), consuming
    /// `self`.
    ///
    /// # Safety
    ///
    /// `ctx` is live; no submission references these textures; each is destroyed exactly once.
    unsafe fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: per the contract `ctx` is live and nothing references these textures; each
        // was created on `ctx` and is destroyed exactly once, in reverse acquisition order.
        unsafe {
            for t in self.weights {
                RhiDevice::destroy_texture(ctx, t);
            }
            for t in self.edges {
                RhiDevice::destroy_texture(ctx, t);
            }
        }
    }
}

/// The per-extent deferred descriptor SETS bound ONCE against the [`CoreImages`] rings + `scene` (NO
/// per-frame update). Built as one bundle so [`GBufferTargets::create`]'s error ladder no longer
/// re-lists the image teardown at every set (the cross-bundle O(n²) collapse): [`Self::build`] drains
/// only the sets it built, and the orchestrator tears down the images. Acquisition order (matched by
/// [`Self::destroy`] in reverse): vocab → resolve → cull → ssao → viewt-from-depth → ddgi-update →
/// present → sdf-forward-march → resolve-hwrt → fxaa → smaa (edge → weight → blend) → ssaa downsample.
/// Flattened into the [`GBufferTargets`] set fields at `create` time, so `present/` readers keep the
/// same `targets.<x>` paths.
struct DeferredSets {
    vocab_set: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    resolve_set: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    cull_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    ssao_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Multi-paradigm render-path plan, rung R3b: the `viewt_from_depth` set — `None` unless
    /// [`GBufferScene::viewt_from_depth`] is armed. Built AFTER `ssao_set` (so its own error
    /// path tears down every prior set including `ssao_set`), BEFORE `ddgi_update_set`.
    viewt_from_depth_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    ddgi_update_set: Option<VulkanBindGroup>,
    present_set: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// Multi-paradigm render-path plan, rung R-SDFFWD: the `sdf_forward_march` pass's Set-0
    /// vocabulary set — `None` unless [`GBufferScene::path_has_sdf_forward`] holds. Built AFTER
    /// `present_set` (both need `core.lit[i]`, so this is the same "needs `core`" point
    /// `present_set` is built at — see [`GBufferTargets::forward`]'s doc for why it cannot live
    /// inside `ForwardTargets::build`, which runs BEFORE `core` exists), so its own error path
    /// tears down every prior set including `present_set`.
    sdf_forward_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Multi-paradigm render-path plan, rung R8: the VB v1 (fused `vb_resolve`) Set-0 vocabulary
    /// set — `None` unless [`GBufferScene::path_is_vb`] holds. Built AFTER `sdf_forward_set`
    /// (both need `core.lit[i]`), so its own error path tears down every prior set including
    /// `sdf_forward_set`.
    vb_set0: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Textured-PBR rung TV0: the `vb_shade` TEXTURED-variant Set-0 vocabulary set — `None`
    /// unless `vb_set0` is also built AND both [`GBufferScene::vb_tex_instance_material_ring`]/
    /// [`GBufferScene::vb_shade_tex_pipeline`] are `Some`. Built immediately after `vb_set0`
    /// (both need `core.lit[i]` + `vb.vb_id[i]`), so its own error path tears down every prior
    /// set including `vb_set0`.
    vb_set0_tex: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// VB-P1a ("dark infra"): the froxel-variant Set-0 vocabulary set — `None` unless the froxel
    /// arm is built (default-OFF, an owner opt-in). Built immediately after `vb_set0_tex` (both need
    /// `core.lit[i]` + `vb.vb_id[i]` + the cluster buffers), so its own error path tears down
    /// every prior set including `vb_set0_tex`.
    vb_set0_froxel: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// VB-P1c: the TEXTURED+FROXEL-variant Set-0 vocabulary set — `None` unless the froxel arm
    /// AND the TEXTURED resources both exist (see [`GBufferTargets::vb_set0_tex_froxel`]'s doc).
    /// Built immediately after `vb_set0_froxel` (both need the SAME inputs plus the tex ring),
    /// so its own error path tears down every prior set including `vb_set0_froxel`.
    vb_set0_tex_froxel: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// TAA-under-VB: the `viewt_from_depth_rz` set — `None` unless
    /// [`GBufferScene::viewt_from_vb_depth`] is armed. Built AFTER `vb_set0_tex` (both need
    /// `core.viewt[i]`/`forward.depth[i]`, the SAME "needs `core` + `forward`" point `vb_set0`
    /// itself is built at), so its own error path tears down every prior set including
    /// `vb_set0_tex`.
    viewt_from_vb_depth_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    #[cfg(feature = "hwrt")]
    resolve_set_hwrt: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Anti-aliasing Stage 1: the FXAA INPUT set RING, `None` when AA is off ([`Self::build`]'s
    /// `aa_out` param is `None`). Built AFTER every hwrt-family set (so its own error path tears
    /// down every prior set); no upstream path knows about it.
    fxaa_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Anti-aliasing Stage 2: pass 1's (edge) INPUT set RING — `None` when SMAA is off. THE NEW
    /// TERMINAL fallible set (W1), built LAST (after `fxaa_set`) so its own error path tears down
    /// every prior set including `fxaa_set` (Option-guarded no-op under SMAA, present for
    /// symmetry).
    smaa_edge_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Anti-aliasing Stage 2: pass 2's (weight) INPUT set RING — `None` when SMAA is off.
    smaa_weight_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Anti-aliasing Stage 2: pass 3's (blend) INPUT set RING — `None` when SMAA is off.
    smaa_blend_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// VG rung R2c0: the batch-cull's own 1-set ring (`VbIndirect` @0, `VbBatchDesc` @1,
    /// `VbCullVisible` @2, `VbCullCount` @3). THE NEW TERMINAL fallible set, built LAST — after
    /// `downsample_set` — so its own error path tears down every prior set and no EXISTING error
    /// path had to learn about it. `None` unless the whole R2c0 arm is wired.
    vb_cull_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// Anti-aliasing Stage 3: the SSAA downsample INPUT set RING — `None` when SSAA is off.
    /// THE NEW TERMINAL fallible set (W1), built LAST (after `smaa_*_set`) so its own error
    /// path tears down every prior set.
    downsample_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
}

impl DeferredSets {
    /// Writes the deferred descriptor sets ONCE against `core` + `scene`. `cluster_grid_buf` /
    /// `light_index_buf` are the L1 buffers (or the light-table placeholder when L1 is off), computed
    /// once by the caller and shared with the hwrt denoise/temporal set builders. `aa_out` is the
    /// AA target ring (`Some` when any of `scene.aa` / `scene.smaa` / `scene.ssaa` is armed) — it
    /// re-points `present_set` to sample `aa_out` instead of `lit` and feeds the FXAA input set.
    /// `smaa_imgs` is the SMAA `edges`/`weights` target bundle (`Some` only when `scene.smaa` is
    /// armed) — it feeds the three SMAA sets (edge → weight → blend, built in that order, AFTER
    /// `fxaa_set`). The SSAA `downsample_set` sampler is derived internally from `scene.ssaa`
    /// (mirrors how `aa_sampler` is derived from `scene.aa` above — no separate param, `scene` is
    /// already threaded through); it is built LAST (after `smaa_*_set`). `forward` is
    /// [`GBufferTargets::forward`]'s already-built value (`Some` iff `TargetsProfile::ForwardMesh`)
    /// — needed for `sdf_forward_set`'s `gForwardDepth` binding, which must reference the SAME
    /// `forward.depth[i]` ring `record_forward` samples.
    /// On any set's partial failure the slots already built in THAT set are drained + every
    /// fully-built prior set destroyed (reverse acquisition); the orchestrator owns the image
    /// rings, which it tears down on this method's `Err`.
    ///
    /// `#[allow(clippy::too_many_arguments)]`: `forward` (rung R-SDFFWD) joins the existing
    /// AA-bundle params — every argument is a distinct borrow the sets bind; grouping them into
    /// a struct would only move the argument list (the SAME rationale
    /// `build_shadow_denoise_sets`'s own `#[allow]` documents).
    #[allow(clippy::too_many_arguments)]
    fn build(
        ctx: &VulkanContext,
        scene: &GBufferScene<'_>,
        core: &CoreImages,
        cluster_grid_buf: &BoundBuffer,
        light_index_buf: &BoundBuffer,
        aa_out: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        smaa_imgs: Option<&SmaaImages>,
        forward: Option<&ForwardTargets>,
        // Multi-paradigm render-path plan, rung R8: [`GBufferTargets::vb`]'s already-built value
        // (`Some` iff `TargetsProfile::VbMesh`) — needed for `vb_set0`'s `gVbId` binding, which
        // must reference the SAME `vb.vb_id[i]` ring `record_vb` writes via the raster pass.
        vb: Option<&VbTargets>,
        // VB-P2 classification plan, rung P2a: [`GBufferTargets::vb_classify`]'s already-built
        // value (`Some` iff `TargetsProfile::VbMesh`, the SAME gate `vb` uses) — needed for
        // `vb_set0`'s new `b7` binding (`gclassify[i]`, bound-but-unread this rung).
        vb_classify: Option<&VbClassifyTargets>,
    ) -> Result<DeferredSets, SwapchainError> {
        // The marcher vocabulary set, written ONCE here (NO per-frame update). The
        // entry order matches the layout: SSBO @0, sampled depth @1, storage albedo @2,
        // storage normal @3, storage material @4, UNIFORM camera @5, STORAGE tiles @6,
        // STORAGE material-table @7, STORAGE gViewT @8 (Lighting L0b), STORAGE PointerGrid @9
        // (M1), COMBINED_IMAGE_SAMPLER BrickAtlas @10 (M2). Bindings 6/9/10 are the P4b coarse-cull
        // tiles, the M1 empty-skip pointer grid, and the M2 brick atlas: the marcher shader DECLARES
        // all three unconditionally (DXC keeps the @9/@10 references past the runtime
        // `brick_enabled`/`brick_trilinear` gates), so VALID descriptors are bound here even though
        // the windowed path gates ALL reads OFF (`coarse_enabled == 0` / `brick_enabled == 0` /
        // `brick_trilinear == 0` — byte-identical output, bindings bound-but-unread).
        // Build FRAMES_IN_FLIGHT identical copies of the vocab set, slot `i` binding
        // `scene.camera_ring[i]` at the camera UBO @5 (the lock-free per-frame ring fix; every
        // other binding is identical across slots). On a failure at slot `i`, the slots already
        // built [0..i) MUST be destroyed (no descriptor leak); the caller owns the images.
        let mut vocab_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in 0..FRAMES_IN_FLIGHT {
            let entries = [
                BindGroupEntry::StorageBuffer { buffer: scene.edit_list },
                BindGroupEntry::SampledImage {
                    texture: &core.depth[slot],
                    sampler: scene.depth_sampler,
                },
                BindGroupEntry::StorageImage { texture: &core.albedo[slot] },
                BindGroupEntry::StorageImage { texture: &core.normal[slot] },
                BindGroupEntry::StorageImage { texture: &core.material[slot] },
                BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[slot] },
                BindGroupEntry::StorageBuffer { buffer: scene.tiles_buffer },
                // PBR MVP-2: the material table SSBO @7 (the marcher fetches `base_color`).
                BindGroupEntry::StorageBuffer { buffer: scene.material_table },
                // Lighting L0b: the gViewT lane @8 (the marcher STORES the surface `t`).
                BindGroupEntry::StorageImage { texture: &core.viewt[slot] },
                // M1: the empty-skip PointerGrid SSBO @9. Statically referenced by the marcher
                // SPIR-V (`register(t9)`); the windowed path gates the read OFF
                // (`brick_enabled == 0`), so it is bound-but-unread (byte-identical output).
                BindGroupEntry::StorageBuffer { buffer: scene.pointer_grid },
                // M2: the brick-atlas 3D image @10 as a COMBINED_IMAGE_SAMPLER (the marcher's
                // hardware trilinear `.SampleLevel` needs the sampler). Statically referenced by the
                // marcher SPIR-V (`register(t10)` + `register(s10)`, collapsed to one combined
                // descriptor by DXC); the windowed path gates the read OFF (`brick_trilinear == 0`),
                // so it is bound-but-unread (byte-identical output, the M2 R2 contract).
                BindGroupEntry::CombinedImage {
                    texture: scene.atlas,
                    sampler: scene.atlas_sampler,
                },
                // M4 clip-map LOD: the LEVEL-1 + LEVEL-2 brick resources (bindings 11/12 + 13/14). The
                // marcher SPIR-V statically references `PointerGrid1`@t11, `BrickAtlas1`@t12,
                // `PointerGrid2`@t13, `BrickAtlas2`@t14 inside the runtime level branch-ladder (NOT
                // dead-stripped past the gate), so VALID descriptors are bound here even on the OFF/N=1
                // path (`brick_levels == 1` takes only the lvl==0 arm → bound-but-unread, byte-identical).
                // Order matches the layout: PointerGrid1 @11, BrickAtlas1 @12, PointerGrid2 @13, BrickAtlas2 @14.
                BindGroupEntry::StorageBuffer { buffer: scene.level_grids[0] },
                BindGroupEntry::CombinedImage {
                    texture: scene.level_atlases[0],
                    sampler: scene.level_atlas_samplers[0],
                },
                BindGroupEntry::StorageBuffer { buffer: scene.level_grids[1] },
                BindGroupEntry::CombinedImage {
                    texture: scene.level_atlases[1],
                    sampler: scene.level_atlas_samplers[1],
                },
                // MDF Stage-2c: the dedicated dense mesh-SDF shadow-caster image @15 as a
                // COMBINED_IMAGE_SAMPLER (the marcher's trilinear `.SampleLevel` needs the sampler).
                // Statically referenced by the recompiled marcher SPIR-V (`register(t15)` +
                // `register(s15)`, collapsed to one combined descriptor by DXC) inside the
                // runtime-gated `mesh_sdf_enabled` branch; a non-MDF scene gates the read OFF
                // (`mesh_sdf_enabled == false`), so it is bound-but-unread (byte-identical output, the
                // R2 contract). A non-MDF scene binds a benign placeholder (e.g. the brick atlas).
                BindGroupEntry::CombinedImage {
                    texture: scene.mesh_sdf,
                    sampler: scene.mesh_sdf_sampler,
                },
            ];
            let desc = BindGroupDesc::<Vulkan> {
                layout: scene.vocab_layout,
                entries: &entries,
            };
            match RhiDevice::create_bind_group(ctx, &desc) {
                Ok(g) => vocab_slots[slot] = Some(g),
                Err(e) => {
                    // SAFETY: the vocab slots already built [0..slot) were created on `ctx`,
                    // referenced by no submission; each destroyed exactly once (the partial ring is
                    // drained). The image rings are owned by the caller (torn down on this `Err`).
                    unsafe {
                        for s in vocab_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                    }
                    return Err(SwapchainError::DepthImage(e));
                }
            }
        }
        // Every slot is now `Some` (the loop returned on any failure); collect the ring.
        let vocab_set: [VulkanBindGroup; FRAMES_IN_FLIGHT] = vocab_slots
            .map(|s| s.expect("invariant: every vocab ring slot built before reaching here"));

        // The deferred RESOLVE set, written ONCE here (12 bindings, 0..=11): gAlbedo @0,
        // gNormal @1, gMaterial @2, lit @3 (STORAGE images), material SSBO @4, camera UBO
        // @5, light table SSBO @6 (Lighting L0a), gViewT @7 (Lighting L0b), ClusterGrid @8 +
        // LightIndexList @9 (Lighting L1) — matching `deferred_pbr.comp`'s set 0. When L1 is
        // off the scene's cluster buffers are `None`, so @8/@9 bind the light table as a
        // harmless VALID placeholder — the layout requires a valid descriptor regardless. These
        // sets are bound only by `Renderer::record_gbuffer`, i.e. only on a DEFERRED boot; on a
        // `Forward`/`ForwardPlus`/`VisibilityBuffer` boot they are written and never bound, so
        // nothing reads @8/@9 there at all. On the Deferred boots that do read them,
        // `deferred_pbr.hlsl`'s THREE-term `use_clusters` (VB-P1k: `clusters_enabled != 0 &&
        // cluster_count != 0 && cluster_count <= grid_capacity`, the capacity read off the BOUND
        // descriptor with `GetDimensions`) keeps them unread on the OFF path: the ENABLED BIT
        // short-circuits it on the default boot, and the DIMS term stops a boot that explicitly
        // set `clusters_enabled = true` (Deferred can never arm `froxel_light_cull`, so
        // `sync_cluster_light_gate` pins the dims to `0`). See `GBufferTargets::resolve_set`'s
        // doc, which also states why the two extra terms are an out-of-bounds guard, not style.
        //
        // Build FRAMES_IN_FLIGHT identical copies of the resolve set, slot `i` binding
        // `scene.camera_ring[i]` @5 + `scene.csm_cascade_ring[i]` @13 (the lock-free per-frame ring
        // fix; every other binding is identical across slots). On a failure at slot `i`, the slots
        // already built [0..i) plus the prior vocab ring MUST be destroyed (no leak).
        let mut resolve_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in 0..FRAMES_IN_FLIGHT {
            // The 19 SHARED resolve bindings (0..=18) — built by the ONE helper the HWRT set also
            // consumes, so the two sets' first 19 bindings cannot drift (a drift = an invisible
            // set↔shader-layout mismatch → device-lost). Textured-PBR T6a (C1 fix): the software
            // set appends its OWN 20th binding (`gPbr` @19) below — `resolve_software_entries`'s
            // output itself is NEVER mutated, so `gPbr` cannot leak into any HWRT-consumed array.
            let imgs = ResolveSlotImages {
                albedo: &core.albedo[slot],
                normal: &core.normal[slot],
                material: &core.material[slot],
                lit: &core.lit[slot],
                viewt: &core.viewt[slot],
                ssao: &core.ssao[slot],
            };
            let shared =
                resolve_software_entries(scene, &imgs, slot, cluster_grid_buf, light_index_buf);
            // Textured-PBR T6a: append binding 19 (`gPbr`, SOFTWARE-ONLY) to the shared 19 →
            // `RESOLVE_SOFTWARE_TOTAL_BINDINGS` (20) EXACT-fill. `BindGroupEntry` is not `Copy`
            // (it holds resource refs), so MOVE the shared entries into 0..=18 via a by-value
            // iterator chained with the `gPbr` entry — the same idiom the HWRT TLAS append below
            // uses (`resolve_set_hwrt`'s `chained` builder).
            let mut chained = shared.into_iter().chain(core::iter::once(
                BindGroupEntry::StorageImage { texture: &core.pbr[slot] },
            ));
            let entries: [BindGroupEntry<'_, Vulkan>; RESOLVE_SOFTWARE_TOTAL_BINDINGS] =
                core::array::from_fn(|_| {
                    chained.next().expect(
                        "invariant: the chained iterator yields exactly RESOLVE_SOFTWARE_TOTAL_BINDINGS entries",
                    )
                });
            // The software resolve set is EXACT-FILL at `RESOLVE_SOFTWARE_TOTAL_BINDINGS` (20: the
            // 19 shared bindings + `gPbr` @19), under the cap of `MAX_BIND_GROUP_BINDINGS` (24).
            // Keeping it EXACT (not `<= cap`) preserves the UNDER-FILL tripwire (a missing binding)
            // AND the over-fill tripwire. `RESOLVE_SOFTWARE_BINDINGS` (19) itself is UNTOUCHED and
            // stays the HWRT-family derivation base — every HWRT resolve variant still fills its
            // OWN separate count (21/22/24), guarded by its OWN constant.
            debug_assert_eq!(
                entries.len(),
                RESOLVE_SOFTWARE_TOTAL_BINDINGS,
                "invariant: the software resolve set must declare EXACTLY {RESOLVE_SOFTWARE_TOTAL_BINDINGS} bindings (exact-fill)"
            );
            let desc = BindGroupDesc::<Vulkan> {
                layout: scene.resolve_layout,
                entries: &entries,
            };
            match RhiDevice::create_bind_group(ctx, &desc) {
                Ok(g) => resolve_slots[slot] = Some(g),
                Err(e) => {
                    // SAFETY: the resolve slots already built [0..slot) + the whole vocab ring were
                    // created on `ctx`; referenced by no submission; each destroyed exactly once
                    // (reverse acquisition: resolve → vocab). The images are owned by the caller.
                    unsafe {
                        for s in resolve_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in vocab_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    return Err(SwapchainError::DepthImage(e));
                }
            }
        }
        let resolve_set: [VulkanBindGroup; FRAMES_IN_FLIGHT] = resolve_slots
            .map(|s| s.expect("invariant: every resolve ring slot built before reaching here"));

        // The Lighting-L1 CULL set, written ONCE here when L1 is wired (camera UBO @0, light
        // table SSBO @1, ClusterGrid @2, LightIndexList @3, LightIndexAlloc @4) — matching
        // `cluster_cull.comp`'s set 0. `None` when the scene does not supply the cull layout
        // (the L0b-only build); the recorder then skips the cull pass entirely.
        let cull_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> = match (scene.cull_layout, scene.cluster_grid, scene.light_index, scene.light_index_alloc) {
            (Some(cull_layout), Some(grid), Some(index), Some(alloc)) => {
                // Build FRAMES_IN_FLIGHT identical copies, slot `i` binding `scene.camera_ring[i]`
                // @0 (the lock-free per-frame ring fix). On a failure at slot `i`, the slots already
                // built [0..i) plus the prior resolve + vocab rings MUST be destroyed.
                let mut cull_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
                    [const { None }; FRAMES_IN_FLIGHT];
                let mut failure: Option<crate::error::VulkanError> = None;
                for (slot, dst) in cull_slots.iter_mut().enumerate() {
                    let entries = [
                        BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[slot] },
                        BindGroupEntry::StorageBuffer { buffer: scene.light_table },
                        BindGroupEntry::StorageBuffer { buffer: grid },
                        BindGroupEntry::StorageBuffer { buffer: index },
                        BindGroupEntry::StorageBuffer { buffer: alloc },
                    ];
                    let desc = BindGroupDesc::<Vulkan> { layout: cull_layout, entries: &entries };
                    match RhiDevice::create_bind_group(ctx, &desc) {
                        Ok(g) => *dst = Some(g),
                        Err(e) => {
                            failure = Some(e);
                            break;
                        }
                    }
                }
                if let Some(e) = failure {
                    // SAFETY: the cull slots already built [0..slot) + the resolve + vocab rings were
                    // created on `ctx`; referenced by no submission; each destroyed exactly once
                    // (reverse acquisition: cull → resolve → vocab). The images are owned by the caller.
                    unsafe {
                        for s in cull_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in resolve_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for g in vocab_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    return Err(SwapchainError::DepthImage(e));
                }
                Some(cull_slots.map(|s| {
                    s.expect("invariant: every cull ring slot built before reaching here")
                }))
            }
            _ => None,
        };

        // Render P7: the SSAO set, written ONCE here when the SSAO pass is wired (gNormal @0,
        // gMaterial @1, gViewT @2 STORAGE images READ, the `ssao` out STORAGE image @3 WRITE, the
        // camera UBO @4) — matching `sdf_ssao.comp`'s set 0. `None` when the scene does not supply
        // the SSAO activation (the default OFF path); the recorder then skips the SSAO pass
        // entirely (the 0%-gate, byte-identical command stream). The `ssao` image is the SAME one
        // the resolve set binds at @11 — the SSAO pass WRITES it, the resolve READS it (ordered by
        // the recorder's COMPUTE→COMPUTE barrier on the SSAO ON path).
        let ssao_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> = match scene.ssao {
            Some(activation) => {
                // Build FRAMES_IN_FLIGHT identical copies, slot `i` binding `scene.camera_ring[i]`
                // @4 (the lock-free per-frame ring fix). On a failure at slot `i`, the slots already
                // built [0..i) plus the prior cull/resolve/vocab rings MUST be destroyed.
                let mut ssao_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
                    [const { None }; FRAMES_IN_FLIGHT];
                let mut failure: Option<crate::error::VulkanError> = None;
                for (slot, dst) in ssao_slots.iter_mut().enumerate() {
                    let entries = [
                        BindGroupEntry::StorageImage { texture: &core.normal[slot] },
                        BindGroupEntry::StorageImage { texture: &core.material[slot] },
                        BindGroupEntry::StorageImage { texture: &core.viewt[slot] },
                        BindGroupEntry::StorageImage { texture: &core.ssao[slot] },
                        BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[slot] },
                    ];
                    let desc = BindGroupDesc::<Vulkan> { layout: activation.layout, entries: &entries };
                    match RhiDevice::create_bind_group(ctx, &desc) {
                        Ok(g) => *dst = Some(g),
                        Err(e) => {
                            failure = Some(e);
                            break;
                        }
                    }
                }
                if let Some(e) = failure {
                    // SAFETY: the ssao slots already built [0..slot) + the (optional) cull ring + the
                    // resolve + vocab rings were created on `ctx`; referenced by no submission; each
                    // destroyed exactly once (reverse acquisition: ssao → cull → resolve → vocab). The
                    // cull ring is `Option`-guarded (only when L1 wired); the images are owned by the
                    // caller.
                    unsafe {
                        for s in ssao_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(cs) = cull_set {
                            for g in cs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in resolve_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for g in vocab_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    return Err(SwapchainError::DepthImage(e));
                }
                Some(ssao_slots.map(|s| {
                    s.expect("invariant: every ssao ring slot built before reaching here")
                }))
            }
            None => None,
        };

        // Multi-paradigm render-path plan, rung R3b (`Deferred × Mesh` — the SDF leg fully off):
        // the `viewt_from_depth` set, written ONCE here when the pass is wired (SAMPLED depth
        // @0, STORAGE `gViewT` @1 WRITE) — matching `viewt_from_depth.comp`'s set 0. `None`
        // unless `scene.viewt_from_depth` is armed (`GeometryLegs::Mesh` exactly); the recorder
        // then skips the pass entirely (the 0%-gate — byte-identical command stream under
        // `Both`/`Sdf`). The `gViewT` image is the SAME one the marcher writes under every
        // OTHER leg, and the SAME one `ssao_set`/the resolve read.
        let viewt_from_depth_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> =
            if let Some(activation) = &scene.viewt_from_depth {
                // Build FRAMES_IN_FLIGHT identical copies, slot `i` binding `core.depth[i]` @0 /
                // `core.viewt[i]` @1 (the per-FIF ring the marcher's vocab set / `ssao_set` also
                // bind). On a failure at slot `i`, the slots already built [0..i) plus the prior
                // ssao/cull/resolve/vocab rings MUST be destroyed.
                let mut viewt_from_depth_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
                    [const { None }; FRAMES_IN_FLIGHT];
                let mut failure: Option<crate::error::VulkanError> = None;
                for (slot, dst) in viewt_from_depth_slots.iter_mut().enumerate() {
                    let entries = [
                        BindGroupEntry::SampledImage {
                            texture: &core.depth[slot],
                            sampler: scene.depth_sampler,
                        },
                        BindGroupEntry::StorageImage { texture: &core.viewt[slot] },
                    ];
                    let desc =
                        BindGroupDesc::<Vulkan> { layout: activation.layout, entries: &entries };
                    match RhiDevice::create_bind_group(ctx, &desc) {
                        Ok(g) => *dst = Some(g),
                        Err(e) => {
                            failure = Some(e);
                            break;
                        }
                    }
                }
                if let Some(e) = failure {
                    // SAFETY: the viewt_from_depth slots already built [0..slot) + the (optional)
                    // ssao/cull rings + the resolve + vocab rings were created on `ctx`;
                    // referenced by no submission; each destroyed exactly once (reverse
                    // acquisition: viewt_from_depth → ssao → cull → resolve → vocab). The images
                    // are owned by the caller.
                    unsafe {
                        for s in viewt_from_depth_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(ss) = ssao_set {
                            for g in ss {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(cs) = cull_set {
                            for g in cs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in resolve_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for g in vocab_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    return Err(SwapchainError::DepthImage(e));
                }
                Some(viewt_from_depth_slots.map(|s| {
                    s.expect("invariant: every viewt_from_depth ring slot built before reaching here")
                }))
            } else {
                None
            };

        // SDFDDGI I2: the SINGLE (non-ringed) probe-update set, written ONCE here when the update
        // pass is wired (`Buf` @0 R, `gIrrOut` @1 W, `gDepthOut` @2 W storage images, `Classification`
        // @3 RW, `RayTable` @4 R, `LightBuf` @5 R, `DdgiUpdate` UBO @6) — matching
        // `sdf_probe_update.comp`'s set 0. `None` when the scene does not supply the update activation
        // (the default GI-OFF path); the recorder then skips the update pass entirely (the 0%-gate,
        // byte-identical command stream). NOT ringed — every input is a single device-only instance
        // (plan §2.2 ring audit): the two atlas storage images are the SAME textures the resolve set
        // samples (the update WRITES them, the resolve READS them, ordered by the RDG-derived
        // update→resolve barrier). On a failure, the prior vocab/resolve/(optional cull/ssao) rings
        // MUST be destroyed (the ssao teardown chain shape).
        let ddgi_update_set: Option<VulkanBindGroup> = match scene.ddgi_update {
            Some(activation) => {
                let entries = [
                    BindGroupEntry::StorageBuffer { buffer: scene.edit_list },
                    BindGroupEntry::StorageImage { texture: scene.ddgi_irr_texture },
                    BindGroupEntry::StorageImage { texture: scene.ddgi_depth_texture },
                    BindGroupEntry::StorageBuffer { buffer: scene.ddgi_classification },
                    BindGroupEntry::StorageBuffer { buffer: scene.ddgi_ray_table },
                    BindGroupEntry::StorageBuffer { buffer: scene.light_table },
                    BindGroupEntry::UniformBuffer { buffer: scene.ddgi_update_ubo },
                ];
                let desc = BindGroupDesc::<Vulkan> { layout: activation.layout, entries: &entries };
                match RhiDevice::create_bind_group(ctx, &desc) {
                    Ok(g) => Some(g),
                    Err(e) => {
                        // SAFETY: the (optional) ssao & cull rings + the resolve & vocab rings were
                        // created on `ctx`; referenced by no submission; each destroyed exactly once
                        // (reverse acquisition: ssao → cull → resolve → vocab). The cull & ssao rings
                        // are `Option`-guarded (present only when L1 / SSAO wired); the images are
                        // owned by the caller.
                        unsafe {
                            if let Some(ss) = ssao_set {
                                for g in ss {
                                    RhiDevice::destroy_bind_group(ctx, g);
                                }
                            }
                            if let Some(cs) = cull_set {
                                for g in cs {
                                    RhiDevice::destroy_bind_group(ctx, g);
                                }
                            }
                            for g in resolve_set {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                            for g in vocab_set {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        return Err(SwapchainError::DepthImage(e));
                    }
                }
            }
            None => None,
        };

        // Anti-aliasing Stage 1: the sampler [`AaActivation`](crate::present::scene_types::AaActivation)
        // carries, or `None` on the OFF path. `aa_sampler` is FXAA-only (feeds the `fxaa_set`
        // builder below); it is `None` under SMAA/SSAA/TAA even though `aa_out` is `Some` (their
        // final target). Lockstep invariant: `aa_out` arms iff ONE of the four post-process
        // modes is armed — `scene.aa` (FXAA) XOR `scene.smaa` (SMAA) XOR `scene.ssaa` (SSAA) XOR
        // `scene.taa` (TAA) — all four routing through the same `aa_imgs` gate.
        let aa_sampler = scene.aa.as_ref().map(|a| a.sampler);
        debug_assert_eq!(
            aa_out.is_some(),
            scene.aa.is_some() || scene.smaa.is_some() || scene.ssaa.is_some() || scene.taa.is_some(),
            "invariant: aa_out arms/disarms with (scene.aa || scene.smaa || scene.ssaa || scene.taa)"
        );

        // The present-blit set RING, written ONCE here: slot `i` is one COMBINED_IMAGE_SAMPLER
        // pointing at `aa_out[i]` when AA is armed, else `lit[i]` (the resolve's output for that
        // slot) + the scene's present sampler (UNCHANGED — the `None` arm is line-exact with the
        // pre-AA stream). RINGED so the present samples the SAME slot the resolve/FXAA wrote this
        // frame (a single present set would go stale — it would sample a sibling slot's image).
        // On a failure at slot `i`, the slots already built [0..i) plus every prior set ring
        // (vocab/resolve/cull/ssao/ddgi) MUST be destroyed (no leak).
        let mut present_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for (slot, dst) in present_slots.iter_mut().enumerate() {
            let entries = [BindGroupEntry::CombinedImage {
                texture: match aa_out {
                    Some(a) => &a[slot],
                    None => &core.lit[slot],
                },
                sampler: scene.present_sampler,
            }];
            let desc = BindGroupDesc::<Vulkan> {
                layout: scene.present_layout,
                entries: &entries,
            };
            match RhiDevice::create_bind_group(ctx, &desc) {
                Ok(g) => *dst = Some(g),
                Err(e) => {
                    // SAFETY: the present slots already built [0..slot) + the (optional) ddgi-update
                    // set + the (optional) ssao & cull rings + the resolve & vocab rings were created
                    // on `ctx`; referenced by no submission; each destroyed exactly once (reverse
                    // acquisition). The ddgi/cull/ssao are `Option`-guarded; the images are owned by
                    // the caller.
                    unsafe {
                        for s in present_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(du) = ddgi_update_set {
                            RhiDevice::destroy_bind_group(ctx, du);
                        }
                        if let Some(ss) = ssao_set {
                            for g in ss {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(cs) = cull_set {
                            for g in cs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in resolve_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for g in vocab_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    return Err(SwapchainError::DepthImage(e));
                }
            }
        }
        let present_set: [VulkanBindGroup; FRAMES_IN_FLIGHT] = present_slots
            .map(|s| s.expect("invariant: every present ring slot built before reaching here"));

        // Multi-paradigm render-path plan, rung R-SDFFWD: the `sdf_forward_march` pass's Set-0
        // vocabulary RING, built HERE — the same point `present_set` is built, both needing
        // `core.lit[i]` (which does not exist before `core`; `ForwardTargets::build` runs BEFORE
        // it, so this set cannot live there — see `GBufferTargets::forward`'s doc). Gated on
        // `scene.path_has_sdf_forward()` (`== resolved_render_path.sdf_forward_marched`): `None`
        // under every Deferred config AND every Forward-family config with the SDF leg absent
        // (`GeometryLegs::Mesh`) — the 0%-gate. Entry order matches the shader's own binding
        // table (`shaders/sdf_forward_march.comp.hlsl`'s header doc): edit-list `Buf` @0,
        // `LightBuf` @1, `Materials` @2, `Camera` UBO @3, `gLit` STORAGE @4, `PointerGrid`/
        // `BrickAtlas` @5/6, `PointerGrid1`/`BrickAtlas1` @7/8, `PointerGrid2`/`BrickAtlas2`
        // @9/10, `BrickLevels` UBO @11, `gForwardDepth` SAMPLED @12 (paired with
        // `scene.depth_sampler` as a harmless bound-but-ignored placeholder — the shader's
        // unfiltered `.Load`, the SAME idiom `vocab_set`'s own `gDepth`@1 binding uses),
        // `gViewT` STORAGE @13 (`core.viewt[i]` — TAA-under-VB: written only by the `VIEWT`
        // pipeline variants; the no-`VIEWT` SPIR-V never statically references the slot, the
        // R2 bound-but-unread contract @12 already establishes; `core.viewt` is ALWAYS
        // allocated, so the entry is valid under every profile that builds this set).
        // `forward.depth[i]` is ALWAYS valid here regardless of `mesh_leg`:
        // `path_has_sdf_forward()` implies `TargetsProfile::ForwardMesh` OR (rung R10)
        // `TargetsProfile::VbMesh` — `create()` builds `ForwardTargets` under BOTH (targets.rs's
        // `matches!(profile, ForwardMesh | VbMesh)`), so `forward` is `Some` and its `depth` ring
        // is allocated for EVERY leg set (`ForwardTargets::build`'s doc) — the mesh-less compute
        // variant simply never reads it
        // (bound-but-unread, the R2 contract), which is why ONE shared layout serves both
        // pipeline variants (`GBufferScene::sdf_forward_march_layout`'s doc).
        let sdf_forward_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> = if scene.path_has_sdf_forward()
        {
            let layout = scene
                .sdf_forward_march_layout
                .expect("invariant: path_has_sdf_forward() requires scene.sdf_forward_march_layout");
            let brick_levels_ubo = scene
                .brick_levels_ubo
                .expect("invariant: path_has_sdf_forward() requires scene.brick_levels_ubo");
            let forward_depth = forward.expect(
                "invariant: path_has_sdf_forward() implies ForwardMesh or VbMesh (both build forward; forward is Some)",
            );
            let mut sdf_forward_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
                [const { None }; FRAMES_IN_FLIGHT];
            let mut failure: Option<crate::error::VulkanError> = None;
            for (slot, dst) in sdf_forward_slots.iter_mut().enumerate() {
                let entries = [
                    BindGroupEntry::StorageBuffer { buffer: scene.edit_list },
                    BindGroupEntry::StorageBuffer { buffer: scene.light_table },
                    BindGroupEntry::StorageBuffer { buffer: scene.material_table },
                    BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[slot] },
                    BindGroupEntry::StorageImage { texture: &core.lit[slot] },
                    BindGroupEntry::StorageBuffer { buffer: scene.pointer_grid },
                    BindGroupEntry::CombinedImage { texture: scene.atlas, sampler: scene.atlas_sampler },
                    BindGroupEntry::StorageBuffer { buffer: scene.level_grids[0] },
                    BindGroupEntry::CombinedImage {
                        texture: scene.level_atlases[0],
                        sampler: scene.level_atlas_samplers[0],
                    },
                    BindGroupEntry::StorageBuffer { buffer: scene.level_grids[1] },
                    BindGroupEntry::CombinedImage {
                        texture: scene.level_atlases[1],
                        sampler: scene.level_atlas_samplers[1],
                    },
                    BindGroupEntry::UniformBuffer { buffer: brick_levels_ubo },
                    BindGroupEntry::SampledImage {
                        texture: &forward_depth.depth[slot],
                        sampler: scene.depth_sampler,
                    },
                    BindGroupEntry::StorageImage { texture: &core.viewt[slot] },
                ];
                let desc = BindGroupDesc::<Vulkan> { layout, entries: &entries };
                match RhiDevice::create_bind_group(ctx, &desc) {
                    Ok(g) => *dst = Some(g),
                    Err(e) => {
                        failure = Some(e);
                        break;
                    }
                }
            }
            if let Some(e) = failure {
                // SAFETY: the sdf-forward slots already built [0..slot) + the present ring + the
                // (optional) ddgi-update/ssao/cull rings + the resolve & vocab rings were created
                // on `ctx`, referenced by no submission; each destroyed exactly once (reverse
                // acquisition). The optional sets are `Option`-guarded; the images are owned by
                // the caller.
                unsafe {
                    for s in sdf_forward_slots.iter_mut() {
                        if let Some(g) = s.take() {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    for g in present_set {
                        RhiDevice::destroy_bind_group(ctx, g);
                    }
                    if let Some(du) = ddgi_update_set {
                        RhiDevice::destroy_bind_group(ctx, du);
                    }
                    if let Some(ss) = ssao_set {
                        for g in ss {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    if let Some(cs) = cull_set {
                        for g in cs {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    for g in resolve_set {
                        RhiDevice::destroy_bind_group(ctx, g);
                    }
                    for g in vocab_set {
                        RhiDevice::destroy_bind_group(ctx, g);
                    }
                }
                return Err(SwapchainError::DepthImage(e));
            }
            Some(
                sdf_forward_slots
                    .map(|s| s.expect("invariant: every sdf-forward ring slot built before reaching here")),
            )
        } else {
            None
        };

        // Multi-paradigm render-path plan, rung R8: the VB v1 (fused `vb_resolve`) Set-0
        // vocabulary RING — built HERE, the SAME "needs `core.lit`" point `sdf_forward_set` is
        // (`gLit` @6 references `core.lit[i]`; `gVbId` @5 references `vb.vb_id[i]`, built at the
        // TOP alongside `forward` — see `VbTargets`'s doc). Gated on `scene.path_is_vb()`: `None`
        // under every other path — the 0%-gate. Entry order matches `vb_resolve.comp.hlsl`'s own
        // binding table doc: `gVbInstances` @0, `instance_materials` @1, `Camera` @2, `LightBuf`
        // @3, `Materials` @4, `gVbId` @5 (SAMPLED, paired with `scene.depth_sampler` as a
        // harmless bound-but-ignored placeholder — the shader's unfiltered `.Load`, the SAME
        // idiom `sdf_forward_set`'s own `gForwardDepth`@12 binding uses), `gLit` @6 (STORAGE).
        //
        // VB-P2 classification plan, rung P2a (dark infra): `b7` = `gclassify[i]` — bound but
        // UNREAD by `vb_sky`/`vb_raster`/`vb_resolve`'s frozen SPIR-V (P2a's byte-identity
        // requirement: every VB Set-0 pipeline shares this ONE layout object, R5, so a set with
        // 8 entries binds cleanly to a pipeline built before `b7` existed).
        let vb_set0: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> = if scene.path_is_vb() {
            let layout = scene.vb_layout0.expect("invariant: path_is_vb() requires scene.vb_layout0");
            let vb_instance_ring = scene
                .vb_instance_ring
                .expect("invariant: path_is_vb() requires scene.vb_instance_ring");
            let instance_material_ring = scene.forward_instance_material_ring.expect(
                "invariant: path_is_vb() requires scene.forward_instance_material_ring",
            );
            let vb_id_ring = &vb
                .expect("invariant: path_is_vb() implies TargetsProfile::VbMesh (vb is Some)")
                .vb_id;
            let gclassify_ring = &vb_classify
                .expect("invariant: path_is_vb() implies TargetsProfile::VbMesh (vb_classify is Some)")
                .gclassify;
            let vb_visible_instance = scene
                .vb_visible_instance
                .expect("invariant: path_is_vb() requires vb_visible_instance");
            let mut vb_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] = [const { None }; FRAMES_IN_FLIGHT];
            let mut failure: Option<crate::error::VulkanError> = None;
            for (slot, dst) in vb_slots.iter_mut().enumerate() {
                let entries = [
                    BindGroupEntry::StorageBuffer { buffer: &vb_instance_ring[slot] },
                    BindGroupEntry::StorageBuffer { buffer: &instance_material_ring[slot] },
                    BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[slot] },
                    BindGroupEntry::StorageBuffer { buffer: scene.light_table },
                    BindGroupEntry::StorageBuffer { buffer: scene.material_table },
                    BindGroupEntry::SampledImage {
                        texture: &vb_id_ring[slot],
                        sampler: scene.depth_sampler,
                    },
                    BindGroupEntry::StorageImage { texture: &core.lit[slot] },
                    BindGroupEntry::StorageBuffer { buffer: &gclassify_ring[slot] },
                    // VG rung R2d-2: `gVbVisibleInstance` @11 — the LAST layout entry, so it is
                    // the LAST slice element (`create_bind_group` matches positionally).
                    BindGroupEntry::StorageBuffer { buffer: &vb_visible_instance[slot] },
                ];
                let desc = BindGroupDesc::<Vulkan> { layout, entries: &entries };
                match RhiDevice::create_bind_group(ctx, &desc) {
                    Ok(g) => *dst = Some(g),
                    Err(e) => {
                        failure = Some(e);
                        break;
                    }
                }
            }
            if let Some(e) = failure {
                // SAFETY: the vb slots already built [0..slot) + the sdf-forward + present +
                // (optional) ddgi-update/ssao/cull + the resolve & vocab rings were created on
                // `ctx`, referenced by no submission; each destroyed exactly once (reverse
                // acquisition). The optional sets are `Option`-guarded; the images are owned by
                // the caller.
                unsafe {
                    for s in vb_slots.iter_mut() {
                        if let Some(g) = s.take() {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    if let Some(sfs) = sdf_forward_set {
                        for g in sfs {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    for g in present_set {
                        RhiDevice::destroy_bind_group(ctx, g);
                    }
                    if let Some(du) = ddgi_update_set {
                        RhiDevice::destroy_bind_group(ctx, du);
                    }
                    if let Some(ss) = ssao_set {
                        for g in ss {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    if let Some(cs) = cull_set {
                        for g in cs {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    for g in resolve_set {
                        RhiDevice::destroy_bind_group(ctx, g);
                    }
                    for g in vocab_set {
                        RhiDevice::destroy_bind_group(ctx, g);
                    }
                }
                return Err(SwapchainError::DepthImage(e));
            }
            Some(vb_slots.map(|s| s.expect("invariant: every vb Set-0 ring slot built before reaching here")))
        } else {
            None
        };

        // Textured-PBR rung TV0 (`RENDER-PARITY-PLAN.md` §2.3): the `vb_shade` TEXTURED-variant
        // Set-0 vocabulary RING — a DISTINCT descriptor SET instance from `vb_set0` against the
        // SAME `vb_layout0` layout object (R5: Vulkan's `STORAGE_BUFFER` binding shape carries no
        // element-stride constraint, so binding `vb_tex_instance_material_ring` — the wider
        // `PerInstanceMaterialTex` ring — at binding 1 needs no second layout). Every OTHER entry
        // is IDENTICAL to `vb_set0`'s own. Built immediately after `vb_set0` (both need
        // `core.lit`/`vb.vb_id`, the SAME "needs `core`" point). `None` unless `vb_set0` itself
        // was built AND BOTH `scene.vb_tex_instance_material_ring`/`scene.vb_shade_tex_pipeline`
        // are `Some` (the TEXTURED resources + the TEXTURED `vb_shade` pipeline both exist).
        let vb_set0_tex: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> = if scene.path_is_vb()
            && let (Some(tex_material_ring), Some(_)) =
                (scene.vb_tex_instance_material_ring, scene.vb_shade_tex_pipeline)
        {
            let layout = scene.vb_layout0.expect("invariant: path_is_vb() requires scene.vb_layout0");
            let vb_instance_ring = scene
                .vb_instance_ring
                .expect("invariant: path_is_vb() requires scene.vb_instance_ring");
            let vb_id_ring = &vb
                .expect("invariant: path_is_vb() implies TargetsProfile::VbMesh (vb is Some)")
                .vb_id;
            let gclassify_ring = &vb_classify
                .expect("invariant: path_is_vb() implies TargetsProfile::VbMesh (vb_classify is Some)")
                .gclassify;
            let vb_visible_instance = scene
                .vb_visible_instance
                .expect("invariant: path_is_vb() requires vb_visible_instance");
            let mut vb_tex_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
                [const { None }; FRAMES_IN_FLIGHT];
            let mut failure: Option<crate::error::VulkanError> = None;
            for (slot, dst) in vb_tex_slots.iter_mut().enumerate() {
                let entries = [
                    BindGroupEntry::StorageBuffer { buffer: &vb_instance_ring[slot] },
                    BindGroupEntry::StorageBuffer { buffer: &tex_material_ring[slot] },
                    BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[slot] },
                    BindGroupEntry::StorageBuffer { buffer: scene.light_table },
                    BindGroupEntry::StorageBuffer { buffer: scene.material_table },
                    BindGroupEntry::SampledImage {
                        texture: &vb_id_ring[slot],
                        sampler: scene.depth_sampler,
                    },
                    BindGroupEntry::StorageImage { texture: &core.lit[slot] },
                    BindGroupEntry::StorageBuffer { buffer: &gclassify_ring[slot] },
                    // VG rung R2d-2: `gVbVisibleInstance` @11 — identical to `vb_set0`'s own; this
                    // variant differs from it only at binding 1.
                    BindGroupEntry::StorageBuffer { buffer: &vb_visible_instance[slot] },
                ];
                let desc = BindGroupDesc::<Vulkan> { layout, entries: &entries };
                match RhiDevice::create_bind_group(ctx, &desc) {
                    Ok(g) => *dst = Some(g),
                    Err(e) => {
                        failure = Some(e);
                        break;
                    }
                }
            }
            if let Some(e) = failure {
                // SAFETY: the vb-tex slots already built [0..slot) + `vb_set0` (fully built) +
                // the sdf-forward + present + (optional) ddgi-update/ssao/cull + the resolve &
                // vocab rings were created on `ctx`, referenced by no submission; each destroyed
                // exactly once (reverse acquisition). The optional sets are `Option`-guarded; the
                // images are owned by the caller.
                unsafe {
                    for s in vb_tex_slots.iter_mut() {
                        if let Some(g) = s.take() {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    if let Some(vs) = vb_set0 {
                        for g in vs {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    if let Some(sfs) = sdf_forward_set {
                        for g in sfs {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    for g in present_set {
                        RhiDevice::destroy_bind_group(ctx, g);
                    }
                    if let Some(du) = ddgi_update_set {
                        RhiDevice::destroy_bind_group(ctx, du);
                    }
                    if let Some(ss) = ssao_set {
                        for g in ss {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    if let Some(cs) = cull_set {
                        for g in cs {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    for g in resolve_set {
                        RhiDevice::destroy_bind_group(ctx, g);
                    }
                    for g in vocab_set {
                        RhiDevice::destroy_bind_group(ctx, g);
                    }
                }
                return Err(SwapchainError::DepthImage(e));
            }
            Some(
                vb_tex_slots
                    .map(|s| s.expect("invariant: every vb-tex Set-0 ring slot built before reaching here")),
            )
        } else {
            None
        };

        // VB-P1a ("dark infra"): the froxel-variant Set-0 vocabulary RING — a DISTINCT descriptor
        // SET instance against [`GBufferScene::vb_layout0_froxel`] (a WIDER, DISTINCT layout
        // object from `vb_layout0` — 11 bindings, `vb_set0`'s own `{0..7, 11}` PLUS
        // `ClusterGrid` @8 + `LightIndexList` @9). Built immediately after `vb_set0_tex` (both
        // need `core.lit`/`vb.vb_id`, the SAME "needs `core`" point). `None` unless the arm is built
        // (`scene.vb_layout0_froxel`/`scene.cluster_grid`/`scene.light_index` all `Some` —
        // ⚠️ default-OFF via the owner's `LightingConfig::clusters_enabled`, NOT hardcoded off, so
        // this is `None` on an unarmed boot and `Some` on `vb_mesh_froxel`'s).
        let vb_set0_froxel: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> = if let (
            Some(layout),
            Some(grid),
            Some(index),
        ) = (scene.vb_layout0_froxel, scene.cluster_grid, scene.light_index)
        {
            let vb_instance_ring = scene
                .vb_instance_ring
                .expect("invariant: vb_layout0_froxel armed implies scene.vb_instance_ring");
            let instance_material_ring = scene.forward_instance_material_ring.expect(
                "invariant: vb_layout0_froxel armed implies scene.forward_instance_material_ring",
            );
            let vb_id_ring = &vb
                .expect("invariant: vb_layout0_froxel armed implies TargetsProfile::VbMesh (vb is Some)")
                .vb_id;
            let gclassify_ring = &vb_classify
                .expect("invariant: vb_layout0_froxel armed implies TargetsProfile::VbMesh (vb_classify is Some)")
                .gclassify;
            let vb_visible_instance = scene
                .vb_visible_instance
                .expect("invariant: vb_layout0_froxel armed implies scene.vb_visible_instance");
            let mut vb_froxel_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
                [const { None }; FRAMES_IN_FLIGHT];
            let mut failure: Option<crate::error::VulkanError> = None;
            for (slot, dst) in vb_froxel_slots.iter_mut().enumerate() {
                let entries = [
                    BindGroupEntry::StorageBuffer { buffer: &vb_instance_ring[slot] },
                    BindGroupEntry::StorageBuffer { buffer: &instance_material_ring[slot] },
                    BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[slot] },
                    BindGroupEntry::StorageBuffer { buffer: scene.light_table },
                    BindGroupEntry::StorageBuffer { buffer: scene.material_table },
                    BindGroupEntry::SampledImage {
                        texture: &vb_id_ring[slot],
                        sampler: scene.depth_sampler,
                    },
                    BindGroupEntry::StorageImage { texture: &core.lit[slot] },
                    BindGroupEntry::StorageBuffer { buffer: &gclassify_ring[slot] },
                    BindGroupEntry::StorageBuffer { buffer: grid },
                    BindGroupEntry::StorageBuffer { buffer: index },
                    // VG rung R2d-2: `gVbVisibleInstance` @11 — LAST in `vb_layout0_froxel` too
                    // (`{0..9, 11}`), so it stays the LAST slice element here as well.
                    BindGroupEntry::StorageBuffer { buffer: &vb_visible_instance[slot] },
                ];
                let desc = BindGroupDesc::<Vulkan> { layout, entries: &entries };
                match RhiDevice::create_bind_group(ctx, &desc) {
                    Ok(g) => *dst = Some(g),
                    Err(e) => {
                        failure = Some(e);
                        break;
                    }
                }
            }
            if let Some(e) = failure {
                // SAFETY: the vb-froxel slots already built [0..slot) + `vb_set0_tex`/`vb_set0`
                // (fully built) + the sdf-forward + present + (optional) ddgi-update/ssao/cull +
                // the resolve & vocab rings were created on `ctx`, referenced by no submission;
                // each destroyed exactly once (reverse acquisition). The optional sets are
                // `Option`-guarded; the images are owned by the caller.
                unsafe {
                    for s in vb_froxel_slots.iter_mut() {
                        if let Some(g) = s.take() {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    if let Some(vt) = vb_set0_tex {
                        for g in vt {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    if let Some(vs) = vb_set0 {
                        for g in vs {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    if let Some(sfs) = sdf_forward_set {
                        for g in sfs {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    for g in present_set {
                        RhiDevice::destroy_bind_group(ctx, g);
                    }
                    if let Some(du) = ddgi_update_set {
                        RhiDevice::destroy_bind_group(ctx, du);
                    }
                    if let Some(ss) = ssao_set {
                        for g in ss {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    if let Some(cs) = cull_set {
                        for g in cs {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    for g in resolve_set {
                        RhiDevice::destroy_bind_group(ctx, g);
                    }
                    for g in vocab_set {
                        RhiDevice::destroy_bind_group(ctx, g);
                    }
                }
                return Err(SwapchainError::DepthImage(e));
            }
            Some(
                vb_froxel_slots
                    .map(|s| s.expect("invariant: every vb-froxel Set-0 ring slot built before reaching here")),
            )
        } else {
            None
        };

        // VB-P1c: the TEXTURED+FROXEL-variant Set-0 vocabulary RING — a DISTINCT descriptor SET
        // instance against the SAME [`GBufferScene::vb_layout0_froxel`] layout object as
        // `vb_set0_froxel` (binding 1 points at `scene.vb_tex_instance_material_ring` instead of
        // `scene.forward_instance_material_ring`; every other entry is IDENTICAL to
        // `vb_set0_froxel`'s own — mirrors the `vb_set0`/`vb_set0_tex` pairing). Built immediately
        // after `vb_set0_froxel` (both need `core.lit`/`vb.vb_id` + the cluster buffers). `None`
        // unless the froxel arm is built AND the TEXTURED resources + the TEXTURED+FROXEL
        // `vb_shade` pipeline both exist (`scene.vb_layout0_froxel`/`scene.cluster_grid`/
        // `scene.light_index`/`scene.vb_tex_instance_material_ring`/
        // `scene.vb_shade_tex_froxel_pipeline` all `Some` — the arm is default-OFF via the
        // owner's `LightingConfig::clusters_enabled`, NOT hardcoded off, so this is `None` on an
        // unarmed boot and `Some` on `vb_mesh_tex_froxel`'s).
        let vb_set0_tex_froxel: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> = if let (
            Some(layout),
            Some(grid),
            Some(index),
            Some(tex_material_ring),
            Some(_),
        ) = (
            scene.vb_layout0_froxel,
            scene.cluster_grid,
            scene.light_index,
            scene.vb_tex_instance_material_ring,
            scene.vb_shade_tex_froxel_pipeline,
        ) {
            let vb_instance_ring = scene
                .vb_instance_ring
                .expect("invariant: vb_layout0_froxel armed implies scene.vb_instance_ring");
            let vb_id_ring = &vb
                .expect("invariant: vb_layout0_froxel armed implies TargetsProfile::VbMesh (vb is Some)")
                .vb_id;
            let gclassify_ring = &vb_classify
                .expect("invariant: vb_layout0_froxel armed implies TargetsProfile::VbMesh (vb_classify is Some)")
                .gclassify;
            let vb_visible_instance = scene
                .vb_visible_instance
                .expect("invariant: vb_layout0_froxel armed implies scene.vb_visible_instance");
            let mut vb_tex_froxel_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
                [const { None }; FRAMES_IN_FLIGHT];
            let mut failure: Option<crate::error::VulkanError> = None;
            for (slot, dst) in vb_tex_froxel_slots.iter_mut().enumerate() {
                let entries = [
                    BindGroupEntry::StorageBuffer { buffer: &vb_instance_ring[slot] },
                    BindGroupEntry::StorageBuffer { buffer: &tex_material_ring[slot] },
                    BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[slot] },
                    BindGroupEntry::StorageBuffer { buffer: scene.light_table },
                    BindGroupEntry::StorageBuffer { buffer: scene.material_table },
                    BindGroupEntry::SampledImage {
                        texture: &vb_id_ring[slot],
                        sampler: scene.depth_sampler,
                    },
                    BindGroupEntry::StorageImage { texture: &core.lit[slot] },
                    BindGroupEntry::StorageBuffer { buffer: &gclassify_ring[slot] },
                    BindGroupEntry::StorageBuffer { buffer: grid },
                    BindGroupEntry::StorageBuffer { buffer: index },
                    // VG rung R2d-2: `gVbVisibleInstance` @11 — identical to `vb_set0_froxel`'s
                    // own; this variant differs from it only at binding 1.
                    BindGroupEntry::StorageBuffer { buffer: &vb_visible_instance[slot] },
                ];
                let desc = BindGroupDesc::<Vulkan> { layout, entries: &entries };
                match RhiDevice::create_bind_group(ctx, &desc) {
                    Ok(g) => *dst = Some(g),
                    Err(e) => {
                        failure = Some(e);
                        break;
                    }
                }
            }
            if let Some(e) = failure {
                // SAFETY: the vb-tex-froxel slots already built [0..slot) + `vb_set0_froxel`/
                // `vb_set0_tex`/`vb_set0` (fully built) + the sdf-forward + present + (optional)
                // ddgi-update/ssao/cull + the resolve & vocab rings were created on `ctx`,
                // referenced by no submission; each destroyed exactly once (reverse acquisition).
                // The optional sets are `Option`-guarded; the images are owned by the caller.
                unsafe {
                    for s in vb_tex_froxel_slots.iter_mut() {
                        if let Some(g) = s.take() {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    if let Some(vf) = vb_set0_froxel {
                        for g in vf {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    if let Some(vt) = vb_set0_tex {
                        for g in vt {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    if let Some(vs) = vb_set0 {
                        for g in vs {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    if let Some(sfs) = sdf_forward_set {
                        for g in sfs {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    for g in present_set {
                        RhiDevice::destroy_bind_group(ctx, g);
                    }
                    if let Some(du) = ddgi_update_set {
                        RhiDevice::destroy_bind_group(ctx, du);
                    }
                    if let Some(ss) = ssao_set {
                        for g in ss {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    if let Some(cs) = cull_set {
                        for g in cs {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    for g in resolve_set {
                        RhiDevice::destroy_bind_group(ctx, g);
                    }
                    for g in vocab_set {
                        RhiDevice::destroy_bind_group(ctx, g);
                    }
                }
                return Err(SwapchainError::DepthImage(e));
            }
            Some(vb_tex_froxel_slots.map(|s| {
                s.expect("invariant: every vb-tex-froxel Set-0 ring slot built before reaching here")
            }))
        } else {
            None
        };

        // TAA-under-VB: the `viewt_from_depth_rz` set RING, written ONCE here when the pass is
        // wired (SAMPLED reverse-Z depth @0, STORAGE `gViewT` @1 WRITE, UNIFORM camera @2) —
        // matching `viewt_from_depth_rz.comp`'s set 0. `None` unless
        // `scene.viewt_from_vb_depth` is armed (`VisibilityBuffer × Mesh` with TAA on); the
        // recorder then skips the pass entirely (the 0%-gate — byte-identical command stream
        // everywhere else). Built HERE — after `vb_set0`/`vb_set0_tex`, the SAME
        // "needs `core` + `forward`" point — because slot `i` binds `forward.depth[i]` (the
        // reverse-Z ring VB rasterizes into — NOT `core.depth`, the Deferred custom-linear ring
        // the `viewt_from_depth_set` sibling binds) + `core.viewt[i]` + `scene.camera_ring[i]`
        // (the SAME slot the TAA resolve's own `generate_ray` reads, so producer `t` and
        // consumer `P = ro + rd·t` use bitwise-identical rays).
        let viewt_from_vb_depth_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> =
            if let Some(activation) = &scene.viewt_from_vb_depth {
                let fwd_depth = &forward
                    .as_ref()
                    .expect(
                        "invariant: viewt_from_vb_depth arms only under VisibilityBuffer × Mesh \
                         (TargetsProfile::VbMesh builds ForwardTargets)",
                    )
                    .depth;
                let mut rz_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
                    [const { None }; FRAMES_IN_FLIGHT];
                let mut failure: Option<crate::error::VulkanError> = None;
                for (slot, dst) in rz_slots.iter_mut().enumerate() {
                    let entries = [
                        BindGroupEntry::SampledImage {
                            texture: &fwd_depth[slot],
                            sampler: scene.depth_sampler,
                        },
                        BindGroupEntry::StorageImage { texture: &core.viewt[slot] },
                        BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[slot] },
                    ];
                    let desc =
                        BindGroupDesc::<Vulkan> { layout: activation.layout, entries: &entries };
                    match RhiDevice::create_bind_group(ctx, &desc) {
                        Ok(g) => *dst = Some(g),
                        Err(e) => {
                            failure = Some(e);
                            break;
                        }
                    }
                }
                if let Some(e) = failure {
                    // SAFETY: the rz slots already built [0..slot) + `vb_set0_tex_froxel`/
                    // `vb_set0_froxel`/`vb_set0_tex`/`vb_set0` (fully built) + the sdf-forward +
                    // present + (optional) ddgi-update/ssao/cull + the resolve & vocab rings were
                    // created on `ctx`, referenced by no submission; each destroyed exactly once
                    // (reverse acquisition). The optional sets are `Option`-guarded; the images
                    // are owned by the caller.
                    unsafe {
                        for s in rz_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(vtf) = vb_set0_tex_froxel {
                            for g in vtf {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(vf) = vb_set0_froxel {
                            for g in vf {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(vts) = vb_set0_tex {
                            for g in vts {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(vs) = vb_set0 {
                            for g in vs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(sfs) = sdf_forward_set {
                            for g in sfs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in present_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        if let Some(du) = ddgi_update_set {
                            RhiDevice::destroy_bind_group(ctx, du);
                        }
                        if let Some(ss) = ssao_set {
                            for g in ss {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(cs) = cull_set {
                            for g in cs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in resolve_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for g in vocab_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    return Err(SwapchainError::DepthImage(e));
                }
                Some(rz_slots.map(|s| {
                    s.expect("invariant: every viewt_from_vb_depth ring slot built before here")
                }))
            } else {
                None
            };

        // R2a-4b: the HWRT-variant resolve set RING — built ONLY when the scene wires BOTH the
        // 21-binding HWRT resolve layout AND the per-FIF TLAS handles (i.e. under `feature = "hwrt"`
        // + `ctx.ray_query_enabled()` + config HardwareTri). `None` on every software path ⇒ the
        // recorder binds the 19-binding `resolve_set` against the software pipeline ⇒ byte-identical
        // to the golden. Built LAST (after every other fallible set) so its own error path tears
        // down everything prior; no upstream path knows about it. Slot `i`'s set is the 19 software
        // entries PLUS binding 19 = slot `i`'s persistent TLAS PLUS rung-1b binding 20 = the HWRT
        // soft-shadow-params UBO.
        #[cfg(feature = "hwrt")]
        let resolve_set_hwrt: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> =
            match (scene.resolve_layout_hwrt, scene.resolve_tlas_hwrt) {
                (Some(hwrt_layout), Some(tlas)) => {
                    let mut hwrt_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
                        [const { None }; FRAMES_IN_FLIGHT];
                    let mut failure: Option<crate::error::VulkanError> = None;
                    for (slot, dst) in hwrt_slots.iter_mut().enumerate() {
                        // The HWRT resolve set = the SAME 19 shared bindings the software set uses
                        // (via `resolve_software_entries`, so they cannot drift) + the 20th
                        // `AccelerationStructure` at binding 19 (slot `slot`'s frame-stable TLAS) +
                        // the rung-1b 21st `UniformBuffer` at binding 20 (the soft-shadow-params UBO).
                        let imgs = ResolveSlotImages {
                            albedo: &core.albedo[slot],
                            normal: &core.normal[slot],
                            material: &core.material[slot],
                            lit: &core.lit[slot],
                            viewt: &core.viewt[slot],
                            ssao: &core.ssao[slot],
                        };
                        let shared = resolve_software_entries(
                            scene,
                            &imgs,
                            slot,
                            cluster_grid_buf,
                            light_index_buf,
                        );
                        // Append binding 19 (the `rayQuery` trace target) + rung-1b binding 20 (the
                        // HWRT soft-shadow-params UBO) to the shared 19 → `RESOLVE_SOFTWARE_BINDINGS
                        // + 2` (21) EXACT-fill. `BindGroupEntry` is not `Copy` (it holds a
                        // `&A::AccelerationStructure`), so MOVE the shared entries into 0..=18 via a
                        // by-value iterator chained with the TLAS + UBO entries — each element is
                        // placed exactly once. The UBO entry mirrors the csm/atlas
                        // `BindGroupEntry::UniformBuffer` shape.
                        const RESOLVE_HWRT_BINDINGS: usize = RESOLVE_SOFTWARE_BINDINGS + 2;
                        let mut chained = shared
                            .into_iter()
                            .chain(core::iter::once(BindGroupEntry::AccelerationStructure {
                                accel: tlas[slot],
                            }))
                            .chain(core::iter::once(BindGroupEntry::UniformBuffer {
                                buffer: &scene.ray_shadow_ubo[slot],
                            }));
                        let entries: [BindGroupEntry<'_, Vulkan>; RESOLVE_HWRT_BINDINGS] =
                            core::array::from_fn(|_| {
                                chained.next().expect(
                                    "invariant: the chained iterator yields exactly RESOLVE_HWRT_BINDINGS entries",
                                )
                            });
                        debug_assert_eq!(
                            entries.len(),
                            RESOLVE_HWRT_BINDINGS,
                            "invariant: the HWRT resolve set must declare EXACTLY {RESOLVE_HWRT_BINDINGS} bindings (exact-fill)"
                        );
                        let desc = BindGroupDesc::<Vulkan> { layout: hwrt_layout, entries: &entries };
                        match RhiDevice::create_bind_group(ctx, &desc) {
                            Ok(g) => *dst = Some(g),
                            Err(e) => {
                                failure = Some(e);
                                break;
                            }
                        }
                    }
                    if let Some(e) = failure {
                        // SAFETY: the HWRT slots already built [0..slot) + the present ring + the
                        // (optional) ddgi-update/ssao/cull + the resolve & vocab rings were created on
                        // `ctx`; referenced by no submission; each destroyed exactly once (reverse
                        // acquisition). The optional sets are `Option`-guarded; the images are owned by
                        // the caller.
                        unsafe {
                            for s in hwrt_slots.iter_mut() {
                                if let Some(g) = s.take() {
                                    RhiDevice::destroy_bind_group(ctx, g);
                                }
                            }
                            if let Some(sfs) = sdf_forward_set {
                                for g in sfs {
                                    RhiDevice::destroy_bind_group(ctx, g);
                                }
                            }
                            for g in present_set {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                            if let Some(du) = ddgi_update_set {
                                RhiDevice::destroy_bind_group(ctx, du);
                            }
                            if let Some(ss) = ssao_set {
                                for g in ss {
                                    RhiDevice::destroy_bind_group(ctx, g);
                                }
                            }
                            if let Some(cs) = cull_set {
                                for g in cs {
                                    RhiDevice::destroy_bind_group(ctx, g);
                                }
                            }
                            for g in resolve_set {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                            for g in vocab_set {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        return Err(SwapchainError::DepthImage(e));
                    }
                    Some(hwrt_slots.map(|s| {
                        s.expect("invariant: every HWRT resolve ring slot built before reaching here")
                    }))
                }
                _ => None,
            };

        // Anti-aliasing Stage 1: the FXAA INPUT set RING, built LAST (after every other fallible
        // set, including the HWRT resolve variant) so its own error path tears down everything
        // prior; no upstream path knows about it. Slot `i` binds `lit[i]` — the FXAA pass's
        // INPUT, never `aa_out` (the pass's OUTPUT, which appears in no set but `present_set`) —
        // plus the dedicated LINEAR/ClampToEdge `aa_sampler`, against `scene.present_layout` (the
        // same single-`CombinedImageSampler` shape `present_set` uses). `None` when AA is off.
        let fxaa_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> = match aa_sampler {
            Some(sampler) => {
                let mut fxaa_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
                    [const { None }; FRAMES_IN_FLIGHT];
                let mut failure: Option<crate::error::VulkanError> = None;
                for (slot, dst) in fxaa_slots.iter_mut().enumerate() {
                    let entries =
                        [BindGroupEntry::CombinedImage { texture: &core.lit[slot], sampler }];
                    let desc =
                        BindGroupDesc::<Vulkan> { layout: scene.present_layout, entries: &entries };
                    match RhiDevice::create_bind_group(ctx, &desc) {
                        Ok(g) => *dst = Some(g),
                        Err(e) => {
                            failure = Some(e);
                            break;
                        }
                    }
                }
                if let Some(e) = failure {
                    // SAFETY: the fxaa slots already built [0..slot) + the present ring + the
                    // (optional) HWRT resolve ring + the (optional) ddgi-update/ssao/cull + the
                    // resolve & vocab rings were created on `ctx`; referenced by no submission;
                    // each destroyed exactly once (reverse acquisition). The optional sets are
                    // `Option`-guarded; the images are owned by the caller.
                    unsafe {
                        for s in fxaa_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        #[cfg(feature = "hwrt")]
                        if let Some(hs) = resolve_set_hwrt {
                            for g in hs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(sfs) = sdf_forward_set {
                            for g in sfs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in present_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        if let Some(du) = ddgi_update_set {
                            RhiDevice::destroy_bind_group(ctx, du);
                        }
                        if let Some(ss) = ssao_set {
                            for g in ss {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(cs) = cull_set {
                            for g in cs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in resolve_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for g in vocab_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    return Err(SwapchainError::DepthImage(e));
                }
                Some(fxaa_slots.map(|s| {
                    s.expect("invariant: every fxaa ring slot built before reaching here")
                }))
            }
            None => None,
        };

        // Anti-aliasing Stage 2: the SMAA lockstep invariant (mirrors the `aa_sampler`
        // check above) — `smaa_imgs` is `Some` iff `scene.smaa` is `Some` (both derive from
        // the same `scene.smaa.is_some()` arm at the `create()` call site).
        debug_assert_eq!(
            smaa_imgs.is_some(),
            scene.smaa.is_some(),
            "invariant: smaa_imgs and scene.smaa must arm/disarm together"
        );

        // Anti-aliasing Stage 2: the three SMAA sets (edge → weight → blend), the NEW TERMINAL
        // fallible set (W1) — built LAST, after `fxaa_set` (mutually exclusive with it — never
        // both `Some`), so their own error path tears down EVERY prior set, `fxaa_set` included
        // (an `Option`-guarded no-op under SMAA, present for symmetry with the fxaa_set ladder
        // above). `None` when SMAA is off.
        let (smaa_edge_set, smaa_weight_set, smaa_blend_set) = match (scene.smaa.as_ref(), smaa_imgs) {
            (Some(smaa), Some(imgs)) => {
                // Pass 1 (edge): scene.present_layout, lit[i] + smaa.sampler (mirrors fxaa_set's
                // own shape exactly, distinct sampler).
                let mut edge_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
                    [const { None }; FRAMES_IN_FLIGHT];
                let mut failure: Option<crate::error::VulkanError> = None;
                for (slot, dst) in edge_slots.iter_mut().enumerate() {
                    let entries = [BindGroupEntry::CombinedImage {
                        texture: &core.lit[slot],
                        sampler: smaa.sampler,
                    }];
                    let desc = BindGroupDesc::<Vulkan> {
                        layout: scene.present_layout,
                        entries: &entries,
                    };
                    match RhiDevice::create_bind_group(ctx, &desc) {
                        Ok(g) => *dst = Some(g),
                        Err(e) => {
                            failure = Some(e);
                            break;
                        }
                    }
                }
                if let Some(e) = failure {
                    // SAFETY: the edge slots already built [0..slot) + everything built prior
                    // (the `fxaa_set` `Option`-guarded no-op under SMAA + the (optional) HWRT
                    // resolve ring + the present ring + the (optional) ddgi-update/ssao/cull +
                    // the resolve & vocab rings) were created on `ctx`; referenced by no
                    // submission; each destroyed exactly once (reverse acquisition).
                    unsafe {
                        for s in edge_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(fs) = fxaa_set {
                            for g in fs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        #[cfg(feature = "hwrt")]
                        if let Some(hs) = resolve_set_hwrt {
                            for g in hs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(sfs) = sdf_forward_set {
                            for g in sfs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in present_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        if let Some(du) = ddgi_update_set {
                            RhiDevice::destroy_bind_group(ctx, du);
                        }
                        if let Some(ss) = ssao_set {
                            for g in ss {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(cs) = cull_set {
                            for g in cs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in resolve_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for g in vocab_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    return Err(SwapchainError::DepthImage(e));
                }
                let edge_set: [VulkanBindGroup; FRAMES_IN_FLIGHT] = edge_slots.map(|s| {
                    s.expect("invariant: every smaa edge ring slot built before reaching here")
                });

                // Pass 2 (weight): smaa.weight_layout, edges[i] + area_tex + search_tex.
                let mut weight_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
                    [const { None }; FRAMES_IN_FLIGHT];
                let mut failure: Option<crate::error::VulkanError> = None;
                for (slot, dst) in weight_slots.iter_mut().enumerate() {
                    let entries = [
                        BindGroupEntry::CombinedImage {
                            texture: &imgs.edges[slot],
                            sampler: smaa.sampler,
                        },
                        BindGroupEntry::CombinedImage {
                            texture: smaa.area_tex,
                            sampler: smaa.sampler,
                        },
                        BindGroupEntry::CombinedImage {
                            texture: smaa.search_tex,
                            sampler: smaa.sampler,
                        },
                    ];
                    let desc = BindGroupDesc::<Vulkan> {
                        layout: smaa.weight_layout,
                        entries: &entries,
                    };
                    match RhiDevice::create_bind_group(ctx, &desc) {
                        Ok(g) => *dst = Some(g),
                        Err(e) => {
                            failure = Some(e);
                            break;
                        }
                    }
                }
                if let Some(e) = failure {
                    // SAFETY: the weight slots already built [0..slot) + the fully-built
                    // `edge_set` + everything built prior were created on `ctx`; referenced by
                    // no submission; each destroyed exactly once (reverse acquisition).
                    unsafe {
                        for s in weight_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in edge_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        if let Some(fs) = fxaa_set {
                            for g in fs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        #[cfg(feature = "hwrt")]
                        if let Some(hs) = resolve_set_hwrt {
                            for g in hs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(sfs) = sdf_forward_set {
                            for g in sfs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in present_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        if let Some(du) = ddgi_update_set {
                            RhiDevice::destroy_bind_group(ctx, du);
                        }
                        if let Some(ss) = ssao_set {
                            for g in ss {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(cs) = cull_set {
                            for g in cs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in resolve_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for g in vocab_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    return Err(SwapchainError::DepthImage(e));
                }
                let weight_set: [VulkanBindGroup; FRAMES_IN_FLIGHT] = weight_slots.map(|s| {
                    s.expect("invariant: every smaa weight ring slot built before reaching here")
                });

                // Pass 3 (blend): smaa.blend_layout, lit[i] + weights[i].
                let mut blend_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
                    [const { None }; FRAMES_IN_FLIGHT];
                let mut failure: Option<crate::error::VulkanError> = None;
                for (slot, dst) in blend_slots.iter_mut().enumerate() {
                    let entries = [
                        BindGroupEntry::CombinedImage {
                            texture: &core.lit[slot],
                            sampler: smaa.sampler,
                        },
                        BindGroupEntry::CombinedImage {
                            texture: &imgs.weights[slot],
                            sampler: smaa.sampler,
                        },
                    ];
                    let desc =
                        BindGroupDesc::<Vulkan> { layout: smaa.blend_layout, entries: &entries };
                    match RhiDevice::create_bind_group(ctx, &desc) {
                        Ok(g) => *dst = Some(g),
                        Err(e) => {
                            failure = Some(e);
                            break;
                        }
                    }
                }
                if let Some(e) = failure {
                    // SAFETY: the blend slots already built [0..slot) + the fully-built
                    // `weight_set` + `edge_set` + everything built prior were created on `ctx`;
                    // referenced by no submission; each destroyed exactly once (reverse
                    // acquisition).
                    unsafe {
                        for s in blend_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in weight_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for g in edge_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        if let Some(fs) = fxaa_set {
                            for g in fs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        #[cfg(feature = "hwrt")]
                        if let Some(hs) = resolve_set_hwrt {
                            for g in hs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(sfs) = sdf_forward_set {
                            for g in sfs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in present_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        if let Some(du) = ddgi_update_set {
                            RhiDevice::destroy_bind_group(ctx, du);
                        }
                        if let Some(ss) = ssao_set {
                            for g in ss {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(cs) = cull_set {
                            for g in cs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in resolve_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for g in vocab_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    return Err(SwapchainError::DepthImage(e));
                }
                let blend_set: [VulkanBindGroup; FRAMES_IN_FLIGHT] = blend_slots.map(|s| {
                    s.expect("invariant: every smaa blend ring slot built before reaching here")
                });

                (Some(edge_set), Some(weight_set), Some(blend_set))
            }
            _ => (None, None, None),
        };

        // Anti-aliasing Stage 3: the SSAA downsample INPUT set RING — THE NEW TERMINAL
        // fallible set (W1), built LAST (after `smaa_*_set`) so its own error path tears down
        // every prior set, including the (mutually exclusive) `fxaa_set`/`smaa_*_set`
        // (`Option`-guarded no-ops under SSAA, present for symmetry with the ladders above).
        // `None` when SSAA is off. Mirrors `fxaa_set`'s exact shape: slot `i` binds `lit[i]`
        // (the 2× ring slot — the downsample's INPUT, never `aa_out`) + the dedicated NEAREST
        // `ssaa_sampler`, against `scene.present_layout` (the shader's `.Load` ignores the
        // sampler; it exists only to satisfy the 1-CIS layout).
        let ssaa_sampler = scene.ssaa.as_ref().map(|s| s.sampler);
        let downsample_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> = match ssaa_sampler {
            Some(sampler) => {
                let mut ds_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
                    [const { None }; FRAMES_IN_FLIGHT];
                let mut failure: Option<crate::error::VulkanError> = None;
                for (slot, dst) in ds_slots.iter_mut().enumerate() {
                    let entries =
                        [BindGroupEntry::CombinedImage { texture: &core.lit[slot], sampler }];
                    let desc =
                        BindGroupDesc::<Vulkan> { layout: scene.present_layout, entries: &entries };
                    match RhiDevice::create_bind_group(ctx, &desc) {
                        Ok(g) => *dst = Some(g),
                        Err(e) => {
                            failure = Some(e);
                            break;
                        }
                    }
                }
                if let Some(e) = failure {
                    // SAFETY: the downsample slots already built [0..slot) + everything built
                    // prior (the `smaa_*_set`/`fxaa_set` `Option`-guarded no-ops under SSAA +
                    // the (optional) HWRT resolve ring + the present ring + the (optional)
                    // ddgi-update/ssao/cull + the resolve & vocab rings) were created on `ctx`;
                    // referenced by no submission; each destroyed exactly once (reverse
                    // acquisition).
                    unsafe {
                        for s in ds_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(bs) = smaa_blend_set {
                            for g in bs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(ws) = smaa_weight_set {
                            for g in ws {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(es) = smaa_edge_set {
                            for g in es {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(fs) = fxaa_set {
                            for g in fs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        #[cfg(feature = "hwrt")]
                        if let Some(hs) = resolve_set_hwrt {
                            for g in hs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(sfs) = sdf_forward_set {
                            for g in sfs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in present_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        if let Some(du) = ddgi_update_set {
                            RhiDevice::destroy_bind_group(ctx, du);
                        }
                        if let Some(ss) = ssao_set {
                            for g in ss {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        if let Some(cs) = cull_set {
                            for g in cs {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in resolve_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for g in vocab_set {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    return Err(SwapchainError::DepthImage(e));
                }
                Some(ds_slots.map(|s| {
                    s.expect("invariant: every downsample ring slot built before reaching here")
                }))
            }
            None => None,
        };

        // VG rung R2c0: the batch-cull's own 1-set ring — THE NEW TERMINAL fallible set (the W1
        // discipline `smaa_edge_set`'s doc states). Built LAST, so its error path tears down every
        // prior set and no EXISTING error path needed a new arm; that teardown is delegated to
        // `DeferredSets::destroy`, which already walks reverse acquisition order, rather than
        // hand-copied for the twentieth time.
        //
        // Gated on the layout plus the four R2c0 buffers plus (since R2d-2) the mesh-bounds table.
        // `GpuSceneBundles` mints `vb_cull_layout`/`vb_batch_cull_pipeline` together or not at all,
        // which is what lets `record_vb` `.expect()` this ring under a gate phrased on the PIPELINE.
        //
        // ⚠️ VG rung R2d-2 added `scene.vb_mesh_bounds` to this tuple, and it is the ONLY
        // conjunct here that is not `Some` on every boot. It has to be here: @5 of the widened
        // `vb_cull_layout` is the geometry table's `gMeshBounds[]`, which does not exist on a
        // Deferred / Forward / Forward+ / `VisibilityBuffer × Sdf` boot, and a bound set with an
        // unwritten descriptor is undefined behaviour the pipeline may read (`robustBufferAccess`
        // is OFF on this device). The consequence is that this set is now `None` on exactly those
        // boots — which is why `record_vb`/`declare_vb_graph`'s `batch_cull_armed` gained
        // `scene.vb_mesh_bounds.is_some()` in the same rung: this tuple and that predicate must
        // stay ONE predicate, or `record_vb`'s `.expect()` on this field becomes reachable.
        //
        // `vb_instance_ring` (@4) and `vb_visible_instance` (@6) are `.expect()`ed rather than
        // matched: both are unconditional `Some(...)` literals in the SAME `GpuSceneBundles::scene`
        // struct expression that wires `vb_cull_layout`, so neither can be `None` in an arm this
        // match already required `vb_cull_layout` to enter. Same treatment as `vb_set0`'s own.
        let vb_cull_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> = match (
            scene.vb_cull_layout,
            scene.vb_indirect,
            scene.vb_batch_desc,
            scene.vb_cull_visible,
            scene.vb_cull_count,
            scene.vb_mesh_bounds,
        ) {
            (
                Some(layout),
                Some(indirect),
                Some(batch_desc),
                Some(visible),
                Some(count),
                Some(mesh_bounds),
            ) => {
                let instances = scene
                    .vb_instance_ring
                    .expect("invariant: vb_cull_layout armed implies scene.vb_instance_ring");
                let visible_instance = scene
                    .vb_visible_instance
                    .expect("invariant: vb_cull_layout armed implies scene.vb_visible_instance");
                let mut slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
                    [const { None }; FRAMES_IN_FLIGHT];
                let mut failure: Option<crate::error::VulkanError> = None;
                for (slot, dst) in slots.iter_mut().enumerate() {
                    let entries = [
                        BindGroupEntry::StorageBuffer { buffer: &indirect[slot] },
                        BindGroupEntry::StorageBuffer { buffer: &batch_desc[slot] },
                        BindGroupEntry::StorageBuffer { buffer: &visible[slot] },
                        BindGroupEntry::StorageBuffer { buffer: &count[slot] },
                        // VG rung R2d-2: @4/@5/@6, positionally after the R2c0 four. The bounds
                        // table is NOT per-FIF (one host-coherent table for the whole boot), so it
                        // binds the same buffer in every slot; the other two are per-FIF.
                        BindGroupEntry::StorageBuffer { buffer: &instances[slot] },
                        BindGroupEntry::StorageBuffer { buffer: mesh_bounds },
                        BindGroupEntry::StorageBuffer { buffer: &visible_instance[slot] },
                    ];
                    let desc = BindGroupDesc::<Vulkan> { layout, entries: &entries };
                    match RhiDevice::create_bind_group(ctx, &desc) {
                        Ok(g) => *dst = Some(g),
                        Err(e) => {
                            failure = Some(e);
                            break;
                        }
                    }
                }
                if let Some(e) = failure {
                    // SAFETY: the slots already built [0..slot) were created on `ctx` and are
                    // referenced by no submission; each is destroyed exactly once here. Every
                    // PRIOR set is then destroyed exactly once by `DeferredSets::destroy`, which
                    // consumes the value and walks reverse acquisition order — the sets are moved
                    // into it, so none can be double-freed by a later path.
                    unsafe {
                        for s in slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        DeferredSets {
                            vocab_set,
                            resolve_set,
                            cull_set,
                            ssao_set,
                            viewt_from_depth_set,
                            ddgi_update_set,
                            present_set,
                            sdf_forward_set,
                            vb_set0,
                            vb_set0_tex,
                            vb_set0_froxel,
                            vb_set0_tex_froxel,
                            viewt_from_vb_depth_set,
                            #[cfg(feature = "hwrt")]
                            resolve_set_hwrt,
                            fxaa_set,
                            smaa_edge_set,
                            smaa_weight_set,
                            smaa_blend_set,
                            downsample_set,
                            vb_cull_set: None,
                        }
                        .destroy(ctx);
                    }
                    return Err(SwapchainError::DepthImage(e));
                }
                Some(slots.map(|s| s.expect("invariant: every batch-cull ring slot built before reaching here")))
            }
            _ => None,
        };

        Ok(DeferredSets {
            vocab_set,
            resolve_set,
            cull_set,
            ssao_set,
            viewt_from_depth_set,
            ddgi_update_set,
            present_set,
            sdf_forward_set,
            vb_set0,
            vb_set0_tex,
            vb_set0_froxel,
            vb_set0_tex_froxel,
            viewt_from_vb_depth_set,
            #[cfg(feature = "hwrt")]
            resolve_set_hwrt,
            fxaa_set,
            smaa_edge_set,
            smaa_weight_set,
            smaa_blend_set,
            downsample_set,
            vb_cull_set,
        })
    }

    /// Tears down the deferred sets in reverse acquisition order (ssaa-downsample → smaa →
    /// fxaa → resolve-hwrt → sdf-forward-march → present → ddgi-update → viewt-from-depth → ssao →
    /// cull → resolve → vocab), consuming `self`.
    ///
    /// # Safety
    ///
    /// `ctx` is live; no submission references these descriptor sets; each is destroyed exactly once
    /// (the by-value `self`). The `cull`/`ssao`/`viewt_from_depth`/`ddgi-update`/`resolve-hwrt`/
    /// `sdf-forward-march`/`fxaa`/`smaa_*`/`downsample` sets are `Option`-guarded (present only when
    /// their feature was wired).
    unsafe fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: per the contract `ctx` is live and nothing references these sets; each was created
        // on `ctx` and is destroyed exactly once, in reverse acquisition order.
        unsafe {
            // VG rung R2c0: the batch-cull ring (LAST-acquired ⇒ FIRST destroyed),
            // `Option`-guarded (present only when the R2c0 arm is wired).
            if let Some(bc) = self.vb_cull_set {
                for g in bc {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            // Anti-aliasing Stage 3: the SSAA downsample set, `Option`-guarded
            // (present only when `scene.ssaa` was armed).
            if let Some(ds) = self.downsample_set {
                for g in ds {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            // Anti-aliasing Stage 2: the three SMAA sets, `Option`-guarded
            // (present only when `scene.smaa` was armed). Reverse build order: blend → weight →
            // edge.
            if let Some(bs) = self.smaa_blend_set {
                for g in bs {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            if let Some(ws) = self.smaa_weight_set {
                for g in ws {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            if let Some(es) = self.smaa_edge_set {
                for g in es {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            // Anti-aliasing Stage 1: the FXAA input set RING, `Option`-guarded (present only
            // when `scene.aa` was armed).
            if let Some(fs) = self.fxaa_set {
                for g in fs {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            // R2a-4b: the HWRT resolve set RING, `Option`-guarded (present only on an
            // RT device under `feature = "hwrt"` + config HardwareTri).
            #[cfg(feature = "hwrt")]
            if let Some(hs) = self.resolve_set_hwrt {
                for g in hs {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            // VB-P1c: the TEXTURED+FROXEL-variant Set-0 vocabulary set, `Option`-guarded (present
            // only when the froxel arm AND the TEXTURED resources both exist — the arm is
            // default-OFF, an owner opt-in). Built AFTER `vb_set0_froxel` (so destroyed BEFORE
            // it, reverse acquisition).
            if let Some(vtf) = self.vb_set0_tex_froxel {
                for g in vtf {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            // VB-P1a ("dark infra"): the froxel-variant Set-0 vocabulary set, `Option`-guarded
            // (present only when the froxel arm is built — default-OFF, an owner opt-in). Built
            // AFTER `vb_set0_tex` (so destroyed BEFORE it, reverse acquisition).
            if let Some(vf) = self.vb_set0_froxel {
                for g in vf {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            // Textured-PBR rung TV0: the `vb_shade` TEXTURED-variant Set-0 vocabulary set,
            // `Option`-guarded (present only when `scene.path_is_vb()` held AND the TEXTURED
            // resources + TEXTURED `vb_shade` pipeline both exist). Built AFTER `vb_set0` (so
            // destroyed BEFORE it, reverse acquisition).
            if let Some(vt) = self.vb_set0_tex {
                for g in vt {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            // Multi-paradigm render-path plan, rung R8: the VB v1 Set-0 vocabulary set,
            // `Option`-guarded (present only when `scene.path_is_vb()` held). Built AFTER
            // `sdf_forward_set` (so destroyed BEFORE it, reverse acquisition).
            if let Some(vs) = self.vb_set0 {
                for g in vs {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            // Multi-paradigm render-path plan, rung R-SDFFWD: the `sdf_forward_march` Set-0
            // vocabulary set, `Option`-guarded (present only when `scene.path_has_sdf_forward()`
            // held). Built AFTER `present_set` (so destroyed BEFORE it, reverse acquisition).
            if let Some(sfs) = self.sdf_forward_set {
                for g in sfs {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            for g in self.present_set {
                RhiDevice::destroy_bind_group(ctx, g);
            }
            // SDFDDGI I2: the single (non-ringed) update set, `Option`-guarded (present only when the
            // update pass was wired).
            if let Some(du) = self.ddgi_update_set {
                RhiDevice::destroy_bind_group(ctx, du);
            }
            // Multi-paradigm render-path plan, rung R3b: the `viewt_from_depth` set,
            // `Option`-guarded (present only under `Deferred × Mesh`). Built AFTER `ssao_set`
            // (so destroyed BEFORE it, reverse acquisition).
            if let Some(vs) = self.viewt_from_depth_set {
                for g in vs {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            if let Some(ss) = self.ssao_set {
                for g in ss {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            if let Some(cs) = self.cull_set {
                for g in cs {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            for g in self.resolve_set {
                RhiDevice::destroy_bind_group(ctx, g);
            }
            for g in self.vocab_set {
                RhiDevice::destroy_bind_group(ctx, g);
            }
        }
    }
}

impl GBufferTargets {
    /// HW-RT rung 3a: builds the spatial-denoise descriptor sets + the à-trous edge-stop UBO ring.
    ///
    /// The build is DECOUPLED from the per-frame `scene.shadow` activation (which is `None` on the
    /// create frame — the TLAS/CSM are not yet armed): it gates on the STABLE boot signals so the
    /// sets exist before a later render frame flips the activation on. Returns `Ok(None)` when the
    /// denoise is OFF (`!scene.shadow_denoise_enabled` — mode `None`, the DEFAULT), the boot denoise
    /// LAYOUTS are absent (`scene.resolve_layout_denoise_hwrt` / `atrous_layout_denoise_hwrt` `None`
    /// on a non-RT / non-hwrt device), the `shadow_vis`/`shadow_vis2` target rings are absent (a
    /// device lacking `shadow_denoise_storage_ok()`), or the persistent TLAS ring is absent — the
    /// byte-identical OFF path. `Ok(Some(_))` on the ON path (all present). On ANY internal `create_*`
    /// failure the
    /// helper drains ITS OWN partial allocations (the reverse-acquisition order below) and returns the
    /// `VulkanError`, leaving nothing leaked for the caller to reason about beyond the rings/sets it
    /// built before calling this.
    ///
    /// The VIS + DENOISED resolve sets fill EXACTLY [`RESOLVE_HWRT_DENOISE_BINDINGS`] (22): the shared
    /// 19 (via [`resolve_software_entries`]) + TLAS @19 + soft-shadow UBO @20 + `gShadowVis` @21. The
    /// VIS set binds `gShadowVis` to `shadow_vis[i]` (write target); the DENOISED set binds it to the
    /// FINAL à-trous output (`shadow_vis[i]` for even `levels`, `shadow_vis2[i]` for odd). Each à-trous
    /// level `i` binds `gVisIn`/`gVisOut` = (`i`-even ? `shadow_vis` : `shadow_vis2`) / the OTHER.
    ///
    /// `#[allow(clippy::too_many_arguments)]`: the six image rings + the L1 placeholder buffers are
    /// all distinct borrows the sets bind (the same list [`resolve_software_entries`] consumes);
    /// grouping them into a struct would only move the argument list.
    #[cfg(feature = "hwrt")]
    #[allow(clippy::too_many_arguments)]
    fn build_shadow_denoise_sets(
        ctx: &VulkanContext,
        scene: &GBufferScene<'_>,
        albedo: &[VulkanTexture; FRAMES_IN_FLIGHT],
        normal: &[VulkanTexture; FRAMES_IN_FLIGHT],
        material: &[VulkanTexture; FRAMES_IN_FLIGHT],
        lit: &[VulkanTexture; FRAMES_IN_FLIGHT],
        viewt: &[VulkanTexture; FRAMES_IN_FLIGHT],
        ssao: &[VulkanTexture; FRAMES_IN_FLIGHT],
        shadow_vis: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        shadow_vis2: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        cluster_grid_buf: &BoundBuffer,
        light_index_buf: &BoundBuffer,
    ) -> Result<Option<ShadowDenoiseSets>, crate::error::VulkanError> {
        // The denoise SETS are decoupled from the per-frame `scene.shadow` activation (which is
        // `None` on THIS create frame — the TLAS/CSM are not yet armed): they build on the STABLE
        // boot signals so the render frame, once it flips `scene.shadow = Some`, finds the sets
        // already written. All preconditions must hold — else the byte-identical OFF path:
        //   * `scene.shadow_denoise_enabled` — the boot `ShadowDenoiseConfig::enabled()` (mode ==
        //     Spatial). `false` on the default (mode `None`) world ⇒ NO sets built (byte-identical).
        //   * `scene.resolve_layout_denoise_hwrt` / `scene.atrous_layout_denoise_hwrt` — the STABLE
        //     22-binding VIS/DENOISED + 6-binding à-trous LAYOUTS from the boot pipelines (`Some`
        //     on an RT + hwrt device REGARDLESS of the per-frame gate). These replace the former
        //     `scene.shadow.as_ref().resolve_layout` — the bug's linchpin.
        //   * `shadow_vis` / `shadow_vis2` — the RG16 ping-pong target rings (device
        //     `shadow_denoise_storage_ok()`); `scene.resolve_tlas_hwrt` — the persistent TLAS ring
        //     (@19). The soft-shadow UBO @20 comes from `scene.ray_shadow_ubo`.
        let (denoise_layout, atrous_layout, vis_ring, vis2_ring, tlas) = match (
            scene.shadow_denoise_enabled,
            scene.resolve_layout_denoise_hwrt,
            scene.atrous_layout_denoise_hwrt,
            shadow_vis,
            shadow_vis2,
            scene.resolve_tlas_hwrt,
        ) {
            (true, Some(rl), Some(al), Some(v), Some(v2), Some(t)) => (rl, al, v, v2, t),
            _ => return Ok(None),
        };
        // The final à-trous output the DENOISED resolve reads at `gShadowVis` @21 (ping-pong
        // parity). W1: the SAME `clamped_levels() % 2 == 1` the record + graph + the per-frame
        // `ShadowVisActivation::final_is_vis2` use — threaded stably so the DENOISED set binds the
        // correct ring at create. When the per-frame activation later opens, the record site
        // asserts `scene.shadow.final_is_vis2 == scene.shadow_denoise_final_is_vis2`.
        let final_is_vis2 = scene.shadow_denoise_final_is_vis2;
        let final_ring = if final_is_vis2 { vis2_ring } else { vis_ring };

        // (1) The à-trous edge-stop UBO ring — one 16-byte host-coherent slot per FIF, zero-seeded
        // (the host memcpys `ResolvedShadowDenoise` in each frame). On a slot's failure, drain [0..i).
        let mut ubo_slots: [Option<BoundBuffer>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for (i, dst) in ubo_slots.iter_mut().enumerate() {
            let b = match RhiDevice::create_buffer(
                ctx,
                &BufferDesc {
                    size: crate::present::SHADOW_DENOISE_UBO_BYTES,
                    usage: BufferUsage::UNIFORM,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            ) {
                Ok(b) => b,
                Err(e) => {
                    // SAFETY: slots [0..i) were created on `ctx`, never submitted; destroy each once.
                    unsafe {
                        for s in ubo_slots.iter_mut().take(i) {
                            if let Some(b) = s.take() {
                                RhiDevice::destroy_buffer(ctx, b);
                            }
                        }
                    }
                    return Err(e);
                }
            };
            if let Some(p) = RhiDevice::buffer_mapped_ptr(ctx, &b) {
                // SAFETY: `p` is the host-coherent mapping of a freshly-created >= 16-byte UNIFORM
                // buffer; writing `SHADOW_DENOISE_UBO_BYTES` zeroes stays in-bounds; byte `0` is a
                // valid init for the `f32` sigma lanes (host-overwritten before first read).
                unsafe {
                    core::ptr::write_bytes(
                        p.as_ptr(),
                        0,
                        crate::present::SHADOW_DENOISE_UBO_BYTES as usize,
                    );
                }
            }
            *dst = Some(b);
        }
        let ubo: [BoundBuffer; FRAMES_IN_FLIGHT] =
            ubo_slots.map(|s| s.expect("invariant: every à-trous UBO ring slot built"));

        // Builds ONE 22-binding VIS/DENOISED resolve set for `slot`, binding `gShadowVis` @21 to
        // `vis_target[slot]`. Shared by the VIS (write target = `shadow_vis`) + DENOISED
        // (read target = `final_ring`) rings — the first 21 bindings are IDENTICAL to the
        // RESOLVE_INLINE-hwrt set (via `resolve_software_entries` + TLAS @19 + soft-shadow UBO @20),
        // so they cannot drift. Exact-fill at `RESOLVE_HWRT_DENOISE_BINDINGS` (22).
        let build_resolve_set = |slot: usize,
                                 vis_target: &[VulkanTexture; FRAMES_IN_FLIGHT]|
         -> Result<VulkanBindGroup, crate::error::VulkanError> {
            let imgs = ResolveSlotImages {
                albedo: &albedo[slot],
                normal: &normal[slot],
                material: &material[slot],
                lit: &lit[slot],
                viewt: &viewt[slot],
                ssao: &ssao[slot],
            };
            let shared =
                resolve_software_entries(scene, &imgs, slot, cluster_grid_buf, light_index_buf);
            let mut chained = shared
                .into_iter()
                .chain(core::iter::once(BindGroupEntry::AccelerationStructure {
                    accel: tlas[slot],
                }))
                .chain(core::iter::once(BindGroupEntry::UniformBuffer {
                    buffer: &scene.ray_shadow_ubo[slot],
                }))
                .chain(core::iter::once(BindGroupEntry::StorageImage {
                    texture: &vis_target[slot],
                }));
            let entries: [BindGroupEntry<'_, Vulkan>; RESOLVE_HWRT_DENOISE_BINDINGS] =
                core::array::from_fn(|_| {
                    chained.next().expect(
                        "invariant: the chained iterator yields exactly RESOLVE_HWRT_DENOISE_BINDINGS entries",
                    )
                });
            debug_assert_eq!(
                entries.len(),
                RESOLVE_HWRT_DENOISE_BINDINGS,
                "invariant: the VIS/DENOISED resolve set must declare EXACTLY {RESOLVE_HWRT_DENOISE_BINDINGS} bindings (exact-fill)"
            );
            let desc = BindGroupDesc::<Vulkan> { layout: denoise_layout, entries: &entries };
            RhiDevice::create_bind_group(ctx, &desc)
        };

        // (2) The VIS resolve set ring (`gShadowVis` @21 = `shadow_vis[i]`, the WRITE target).
        let mut vis_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for (slot, dst) in vis_slots.iter_mut().enumerate() {
            match build_resolve_set(slot, vis_ring) {
                Ok(g) => *dst = Some(g),
                Err(e) => {
                    // SAFETY: the [0..slot) vis slots + the UBO ring were created on `ctx`, never
                    // submitted; destroy each once (reverse acquisition: sets → UBO).
                    unsafe {
                        for s in vis_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for b in ubo {
                            RhiDevice::destroy_buffer(ctx, b);
                        }
                    }
                    return Err(e);
                }
            }
        }
        let vis_resolve: [VulkanBindGroup; FRAMES_IN_FLIGHT] =
            vis_slots.map(|s| s.expect("invariant: every VIS resolve ring slot built"));

        // (3) The DENOISED resolve set ring (`gShadowVis` @21 = the FINAL à-trous output, the READ
        // target). On failure, drain the VIS ring + the UBO ring too.
        let mut den_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for (slot, dst) in den_slots.iter_mut().enumerate() {
            match build_resolve_set(slot, final_ring) {
                Ok(g) => *dst = Some(g),
                Err(e) => {
                    // SAFETY: the [0..slot) den slots + the whole VIS ring + the UBO ring were
                    // created on `ctx`, never submitted; destroy each once.
                    unsafe {
                        for s in den_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in vis_resolve {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for b in ubo {
                            RhiDevice::destroy_buffer(ctx, b);
                        }
                    }
                    return Err(e);
                }
            }
        }
        let denoised_resolve: [VulkanBindGroup; FRAMES_IN_FLIGHT] =
            den_slots.map(|s| s.expect("invariant: every DENOISED resolve ring slot built"));

        // (4) The per-level à-trous set rings (`sets[level][fi]`). Level `i` binds `gVisIn` @0 =
        // (`i`-even ? `shadow_vis` : `shadow_vis2`), `gVisOut` @1 = the OTHER, `gNormal` @2 /
        // `gViewT` @3 (slot `fi`), the `ResolvedShadowDenoise` UBO @4 (`ubo[fi]`), the camera UBO @5
        // (`scene.camera_ring[fi]`). ALL `MAX_ATROUS_LEVELS` × `FRAMES_IN_FLIGHT` are built (fixed
        // per-extent cost); the recorder consumes only the first `levels`. On any slot's failure,
        // drain the à-trous sets built so far + the DENOISED + VIS rings + the UBO ring.
        let mut atrous_opt: [[Option<VulkanBindGroup>; FRAMES_IN_FLIGHT];
            crate::present::MAX_ATROUS_LEVELS as usize] =
            core::array::from_fn(|_| [const { None }; FRAMES_IN_FLIGHT]);
        for level in 0..crate::present::MAX_ATROUS_LEVELS as usize {
            // Even levels: read `shadow_vis`, write `shadow_vis2`. Odd: the reverse.
            let (in_ring, out_ring) = if level % 2 == 0 {
                (vis_ring, vis2_ring)
            } else {
                (vis2_ring, vis_ring)
            };
            for slot in 0..FRAMES_IN_FLIGHT {
                let entries = [
                    BindGroupEntry::StorageImage { texture: &in_ring[slot] },
                    BindGroupEntry::StorageImage { texture: &out_ring[slot] },
                    BindGroupEntry::StorageImage { texture: &normal[slot] },
                    BindGroupEntry::StorageImage { texture: &viewt[slot] },
                    BindGroupEntry::UniformBuffer { buffer: &ubo[slot] },
                    BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[slot] },
                ];
                let desc = BindGroupDesc::<Vulkan> {
                    layout: atrous_layout,
                    entries: &entries,
                };
                match RhiDevice::create_bind_group(ctx, &desc) {
                    Ok(g) => atrous_opt[level][slot] = Some(g),
                    Err(e) => {
                        // SAFETY: every à-trous set built so far (all prior levels + this level's
                        // [0..slot)) + the DENOISED + VIS resolve rings + the UBO ring were created
                        // on `ctx`, never submitted; destroy each once (reverse acquisition).
                        unsafe {
                            for lvl in atrous_opt.iter_mut() {
                                for s in lvl.iter_mut() {
                                    if let Some(g) = s.take() {
                                        RhiDevice::destroy_bind_group(ctx, g);
                                    }
                                }
                            }
                            for g in denoised_resolve {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                            for g in vis_resolve {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                            for b in ubo {
                                RhiDevice::destroy_buffer(ctx, b);
                            }
                        }
                        return Err(e);
                    }
                }
            }
        }
        let atrous: [[VulkanBindGroup; FRAMES_IN_FLIGHT]; crate::present::MAX_ATROUS_LEVELS as usize] =
            atrous_opt.map(|lvl| lvl.map(|s| s.expect("invariant: every à-trous set slot built")));

        Ok(Some(ShadowDenoiseSets { vis_resolve, denoised_resolve, atrous, ubo }))
    }

    /// The SSAO à-trous denoise chain: builds the FIVE role-keyed descriptor sets
    /// ([`crate::present::ssao_atrous_step`]'s [`crate::present::AtrousStepRole`] selects
    /// between). UNCONDITIONAL (both feature legs — SOFTWARE, NOT `hwrt`-gated).
    ///
    /// DECOUPLED from the per-frame `scene.ssao` activation (which may be `None` at THIS create
    /// call — SSAO starts OFF by default, `SsaoConfig::default()`): gates on the STABLE boot
    /// signals (`scene.ssao_atrous_layout` + the ring images) so a later frame that arms
    /// [`SsaoActivation::atrous_levels`] finds the sets already built — the "set=None panic when
    /// the gate opens late" trap [`Self::build_shadow_denoise_sets`]'s doc names. Returns
    /// `Ok(None)` when `scene.ssao_atrous_layout` is `None` (a host that never wired the boot
    /// pipelines) or the ring images are `None` (the device lacks `R16_UNORM` storage,
    /// [`crate::device::DeviceCaps::ssao_atrous_storage_ok`]) — the byte-identical OFF path (the
    /// resolve then reads the raw, un-denoised gather). `Ok(Some(_))` on the ON path (both
    /// present). On ANY internal `create_bind_group` failure the method drains ITS OWN partial
    /// allocations (reverse acquisition: the ring already built [0..slot) + every prior fully-built
    /// ring, LATEST first) and returns the `VulkanError`; the outer `?`-arm then drains every
    /// bundle built before this call.
    ///
    /// Each set binds `gAoIn` @0 / `gAoOut` @1 (the role-keyed STORAGE-image pair), `gViewT` @2
    /// (READ, slot `i`), the camera UBO @3 (`scene.camera_ring[i]`) — exactly the à-trous shader's
    /// 4-binding interface (`ssao_atrous.comp.hlsl`).
    fn build_ssao_atrous_sets(
        ctx: &VulkanContext,
        scene: &GBufferScene<'_>,
        viewt: &[VulkanTexture; FRAMES_IN_FLIGHT],
        ssao: &[VulkanTexture; FRAMES_IN_FLIGHT],
        ssao_ring_a: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        ssao_ring_b: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
    ) -> Result<Option<SsaoAtrousSets>, crate::error::VulkanError> {
        let (layout, ring_a, ring_b) = match (scene.ssao_atrous_layout, ssao_ring_a, ssao_ring_b) {
            (Some(l), Some(a), Some(b)) => (l, a, b),
            _ => return Ok(None),
        };

        // Builds ONE 4-binding set for `slot`: `gAoIn` @0 = `in_ring[slot]`, `gAoOut` @1 =
        // `out_ring[slot]`, `gViewT` @2 = `viewt[slot]`, the camera UBO @3 =
        // `scene.camera_ring[slot]`.
        let build_set = |in_ring: &[VulkanTexture; FRAMES_IN_FLIGHT],
                          out_ring: &[VulkanTexture; FRAMES_IN_FLIGHT],
                          slot: usize|
         -> Result<VulkanBindGroup, crate::error::VulkanError> {
            let entries = [
                BindGroupEntry::StorageImage { texture: &in_ring[slot] },
                BindGroupEntry::StorageImage { texture: &out_ring[slot] },
                BindGroupEntry::StorageImage { texture: &viewt[slot] },
                BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[slot] },
            ];
            let desc = BindGroupDesc::<Vulkan> { layout, entries: &entries };
            RhiDevice::create_bind_group(ctx, &desc)
        };

        // (1) `read8`: `gAoIn` = the frozen R8 `ssao` endpoint, `gAoOut` = `ring_a`.
        let mut read8_opt: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for (slot, dst) in read8_opt.iter_mut().enumerate() {
            match build_set(ssao, ring_a, slot) {
                Ok(g) => *dst = Some(g),
                Err(e) => {
                    // SAFETY: the [0..slot) read8 slots were created on `ctx`, never submitted;
                    // destroy each once.
                    unsafe {
                        for s in read8_opt.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                    }
                    return Err(e);
                }
            }
        }
        let read8: [VulkanBindGroup; FRAMES_IN_FLIGHT] =
            read8_opt.map(|s| s.expect("invariant: every read8 set slot built"));

        // (2) `interior_from0`: `gAoIn` = `ring_a`, `gAoOut` = `ring_b`.
        let mut i0_opt: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] = [const { None }; FRAMES_IN_FLIGHT];
        for (slot, dst) in i0_opt.iter_mut().enumerate() {
            match build_set(ring_a, ring_b, slot) {
                Ok(g) => *dst = Some(g),
                Err(e) => {
                    // SAFETY: the [0..slot) interior_from0 slots + the read8 ring were created on
                    // `ctx`, never submitted; destroy each once (reverse acquisition).
                    unsafe {
                        for s in i0_opt.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in read8 {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    return Err(e);
                }
            }
        }
        let interior_from0: [VulkanBindGroup; FRAMES_IN_FLIGHT] =
            i0_opt.map(|s| s.expect("invariant: every interior_from0 set slot built"));

        // (3) `interior_from1`: `gAoIn` = `ring_b`, `gAoOut` = `ring_a`.
        let mut i1_opt: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] = [const { None }; FRAMES_IN_FLIGHT];
        for (slot, dst) in i1_opt.iter_mut().enumerate() {
            match build_set(ring_b, ring_a, slot) {
                Ok(g) => *dst = Some(g),
                Err(e) => {
                    // SAFETY: the [0..slot) interior_from1 slots + interior_from0 + read8 were
                    // created on `ctx`, never submitted; destroy each once (reverse acquisition).
                    unsafe {
                        for s in i1_opt.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in interior_from0 {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for g in read8 {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    return Err(e);
                }
            }
        }
        let interior_from1: [VulkanBindGroup; FRAMES_IN_FLIGHT] =
            i1_opt.map(|s| s.expect("invariant: every interior_from1 set slot built"));

        // (4) `write8_from0`: `gAoIn` = `ring_a`, `gAoOut` = the frozen R8 `ssao` endpoint
        // (the write-BACK the resolve reads).
        let mut w0_opt: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] = [const { None }; FRAMES_IN_FLIGHT];
        for (slot, dst) in w0_opt.iter_mut().enumerate() {
            match build_set(ring_a, ssao, slot) {
                Ok(g) => *dst = Some(g),
                Err(e) => {
                    // SAFETY: the [0..slot) write8_from0 slots + interior_from1 + interior_from0 +
                    // read8 were created on `ctx`, never submitted; destroy each once (reverse
                    // acquisition).
                    unsafe {
                        for s in w0_opt.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in interior_from1 {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for g in interior_from0 {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for g in read8 {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    return Err(e);
                }
            }
        }
        let write8_from0: [VulkanBindGroup; FRAMES_IN_FLIGHT] =
            w0_opt.map(|s| s.expect("invariant: every write8_from0 set slot built"));

        // (5) `write8_from1`: `gAoIn` = `ring_b`, `gAoOut` = the frozen R8 `ssao` endpoint.
        let mut w1_opt: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] = [const { None }; FRAMES_IN_FLIGHT];
        for (slot, dst) in w1_opt.iter_mut().enumerate() {
            match build_set(ring_b, ssao, slot) {
                Ok(g) => *dst = Some(g),
                Err(e) => {
                    // SAFETY: the [0..slot) write8_from1 slots + every prior ring were created on
                    // `ctx`, never submitted; destroy each once (reverse acquisition).
                    unsafe {
                        for s in w1_opt.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in write8_from0 {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for g in interior_from1 {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for g in interior_from0 {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for g in read8 {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                    }
                    return Err(e);
                }
            }
        }
        let write8_from1: [VulkanBindGroup; FRAMES_IN_FLIGHT] =
            w1_opt.map(|s| s.expect("invariant: every write8_from1 set slot built"));

        Ok(Some(SsaoAtrousSets {
            read8,
            interior_from0,
            interior_from1,
            write8_from0,
            write8_from1,
        }))
    }

    /// HW-RT Rung 3b step 5b: builds the SDF motion-vector VIS-variant resolve set RING (one 24-entry
    /// set per in-flight frame) against the boot VIS-MV layout ([`GBufferScene::vis_mv_layout`]).
    ///
    /// The set = the SAME 22 VIS/DENOISED entries the base VIS set builds (the 19 shared via
    /// [`resolve_software_entries`] + TLAS @19 + soft-shadow UBO @20 + `gShadowVis` @21 =
    /// `shadow_vis[slot]`, the WRITE target) PLUS the `MotionCam` UBO @22 (`motion_cam[slot]`) + the
    /// `motion_vec` STORAGE image @23 (`motion_vec[slot]`, the SDF-Δuv WRITE target). Exact-fill at
    /// [`RESOLVE_HWRT_VIS_MV_BINDINGS`] (24).
    ///
    /// DECOUPLED from the per-frame activation (the same lesson as [`Self::build_shadow_denoise_sets`]):
    /// it gates on the STABLE signals — NOT `scene.temporal_enabled` — so the set already exists
    /// before a render frame flips [`GBufferScene::sdf_mv_active`] on (else a Spatial→Both mode change
    /// with no resize would hit a `None` set: the "set=None panic when the gate opens late" trap).
    /// Returns `None` (the byte-identical OFF path) unless ALL hold: the spatial denoise is armed
    /// (`scene.shadow_denoise_enabled` — so the base VIS set + `scene.shadow` also exist) AND the
    /// VIS-MV layout + the `MotionCam` ring + the `motion_vec` target + the `shadow_vis` ring + the
    /// persistent TLAS ring are all present (an RT + storage device). In `mode == Spatial` (temporal
    /// off) the set is BUILT-BUT-UNUSED — the recorder gates USE on `sdf_mv_active()`; a small
    /// boot-time cost that makes the recorder's `expect` on this set panic-free. On ANY
    /// `create_bind_group` failure it DEGRADES to `None` (draining its own partials) — it is called
    /// LAST (after every fallible set + the `motion_vec` ring), so nothing depends on it and no
    /// teardown weaves into the ladder.
    ///
    /// `#[allow(clippy::too_many_arguments)]`: the six G-buffer image rings + the two L1 placeholder
    /// buffers are the exact borrows [`resolve_software_entries`] consumes — grouping them would only
    /// move the argument list.
    #[cfg(feature = "hwrt")]
    #[allow(clippy::too_many_arguments)]
    fn build_shadow_vis_mv_resolve_set(
        ctx: &VulkanContext,
        scene: &GBufferScene<'_>,
        albedo: &[VulkanTexture; FRAMES_IN_FLIGHT],
        normal: &[VulkanTexture; FRAMES_IN_FLIGHT],
        material: &[VulkanTexture; FRAMES_IN_FLIGHT],
        lit: &[VulkanTexture; FRAMES_IN_FLIGHT],
        viewt: &[VulkanTexture; FRAMES_IN_FLIGHT],
        ssao: &[VulkanTexture; FRAMES_IN_FLIGHT],
        shadow_vis: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        motion_vec: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        cluster_grid_buf: &BoundBuffer,
        light_index_buf: &BoundBuffer,
    ) -> Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> {
        // The STABLE-signal gate (see the doc): the spatial denoise is armed (so the base VIS set +
        // `scene.shadow` also exist) AND every input the 24-entry set binds is present. NOTE: this is
        // DECOUPLED from `scene.temporal_enabled` (the per-frame activation) exactly like
        // `build_shadow_denoise_sets` — building on the STABLE signals so the set already exists
        // before a frame flips `sdf_mv_active()` on (the "build denoise sets on stable boot config,
        // not the per-frame gate, else set=None panic when the gate opens late" lesson). The
        // `vis_mv_layout` / `motion_cam_ubo_ring` are `Some` whenever the boot MV resources exist
        // (an RT + storage device), independent of the temporal mode. Any absent ⇒ the byte-identical
        // OFF path. When temporal is OFF (`mode == Spatial`) the set is built-but-unused (the recorder
        // gates USE on `sdf_mv_active()`); a small boot-time cost that removes the panic.
        let (vis_mv_layout, motion_cam, vis_ring, mvec, tlas) = match (
            scene.shadow_denoise_enabled,
            scene.vis_mv_layout,
            scene.motion_cam_ubo_ring,
            shadow_vis,
            motion_vec,
            scene.resolve_tlas_hwrt,
        ) {
            (true, Some(l), Some(mc), Some(v), Some(mv), Some(t)) => (l, mc, v, mv, t),
            _ => return None,
        };

        let mut slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for (slot, dst) in slots.iter_mut().enumerate() {
            let imgs = ResolveSlotImages {
                albedo: &albedo[slot],
                normal: &normal[slot],
                material: &material[slot],
                lit: &lit[slot],
                viewt: &viewt[slot],
                ssao: &ssao[slot],
            };
            let shared =
                resolve_software_entries(scene, &imgs, slot, cluster_grid_buf, light_index_buf);
            // The 22 VIS bindings (identical to `build_resolve_set`'s VIS chain) + `MotionCam` @22 +
            // `motion_vec` @23. `gShadowVis` @21 binds `shadow_vis[slot]` (the WRITE target, same as
            // the base VIS set).
            let mut chained = shared
                .into_iter()
                .chain(core::iter::once(BindGroupEntry::AccelerationStructure {
                    accel: tlas[slot],
                }))
                .chain(core::iter::once(BindGroupEntry::UniformBuffer {
                    buffer: &scene.ray_shadow_ubo[slot],
                }))
                .chain(core::iter::once(BindGroupEntry::StorageImage {
                    texture: &vis_ring[slot],
                }))
                .chain(core::iter::once(BindGroupEntry::UniformBuffer {
                    buffer: &motion_cam[slot],
                }))
                .chain(core::iter::once(BindGroupEntry::StorageImage {
                    texture: &mvec[slot],
                }));
            let entries: [BindGroupEntry<'_, Vulkan>; RESOLVE_HWRT_VIS_MV_BINDINGS] =
                core::array::from_fn(|_| {
                    chained.next().expect(
                        "invariant: the chained iterator yields exactly RESOLVE_HWRT_VIS_MV_BINDINGS entries",
                    )
                });
            debug_assert_eq!(
                entries.len(),
                RESOLVE_HWRT_VIS_MV_BINDINGS,
                "invariant: the VIS-MV resolve set must declare EXACTLY {RESOLVE_HWRT_VIS_MV_BINDINGS} bindings (exact-fill)"
            );
            let desc = BindGroupDesc::<Vulkan> { layout: vis_mv_layout, entries: &entries };
            match RhiDevice::create_bind_group(ctx, &desc) {
                Ok(g) => *dst = Some(g),
                Err(_) => {
                    // Degrade to None (opt-in path, no dependents): drain the [0..slot) sets built so
                    // far. SAFETY: each was created on `ctx`, referenced by no submission; destroy
                    // exactly once.
                    unsafe {
                        for s in slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                    }
                    return None;
                }
            }
        }
        Some(slots.map(|s| s.expect("invariant: every VIS-MV resolve ring slot built")))
    }

    /// HW-RT Rung 3b step 6: builds the temporal-denoise descriptor sets — the temporal reproject UBO
    /// ring, the 8-binding temporal reproject set, and the sibling DENOISED-temporal resolve set (which
    /// binds `gShadowVis` @21 to `temporal_out` instead of the à-trous ring).
    ///
    /// DECOUPLED from the per-frame temporal activation (the same lesson as
    /// [`Self::build_shadow_vis_mv_resolve_set`]): it gates on the STABLE signals so the sets already
    /// exist before a render frame flips [`GBufferScene::temporal_active`] on. Returns `None` (the
    /// byte-identical OFF path) unless ALL hold: the denoise is armed (`scene.shadow_denoise_enabled`)
    /// AND temporal (`scene.temporal_enabled`) AND the temporal layout + the VIS/DENOISED layout + the
    /// `shadow_vis`/`shadow_vis2`/`motion_vec`/`shadow_temporal_hist`/`temporal_out` rings + the
    /// persistent TLAS ring all exist (an RT + storage device). Built LAST in `create` (after every
    /// fallible set + the temporal target rings) and DEGRADES-TO-`None` on any `create_*` failure
    /// (draining its own partials) — nothing depends on it, so no teardown weaves into the ladder.
    ///
    /// `#[allow(clippy::too_many_arguments)]`: the image rings + the two L1 placeholder buffers are the
    /// exact borrows the sets bind (the same list [`resolve_software_entries`] consumes); grouping them
    /// would only move the argument list.
    #[cfg(feature = "hwrt")]
    #[allow(clippy::too_many_arguments)]
    fn build_shadow_temporal_sets(
        ctx: &VulkanContext,
        scene: &GBufferScene<'_>,
        albedo: &[VulkanTexture; FRAMES_IN_FLIGHT],
        normal: &[VulkanTexture; FRAMES_IN_FLIGHT],
        material: &[VulkanTexture; FRAMES_IN_FLIGHT],
        lit: &[VulkanTexture; FRAMES_IN_FLIGHT],
        viewt: &[VulkanTexture; FRAMES_IN_FLIGHT],
        ssao: &[VulkanTexture; FRAMES_IN_FLIGHT],
        shadow_vis: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        shadow_vis2: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        motion_vec: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        shadow_temporal_hist: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        temporal_out: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        cluster_grid_buf: &BoundBuffer,
        light_index_buf: &BoundBuffer,
    ) -> Option<ShadowTemporalSets> {
        // The STABLE-signal gate (see the doc): the denoise is armed + temporal AND every input the
        // temporal set + the denoised-temporal set bind is present. DECOUPLED from
        // `scene.temporal_active()` (the per-frame activation) — building on the STABLE signals so the
        // sets already exist before a frame flips it on (the "build denoise sets on stable boot
        // config, not the per-frame gate, else set=None panic when the gate opens late" lesson). Any
        // absent ⇒ the byte-identical OFF path.
        let (temporal_layout, denoise_layout, vis_ring, vis2_ring, mvec, hist, tout, tlas) = match (
            scene.shadow_denoise_enabled && scene.temporal_enabled,
            scene.temporal_layout,
            scene.resolve_layout_denoise_hwrt,
            shadow_vis,
            shadow_vis2,
            motion_vec,
            shadow_temporal_hist,
            temporal_out,
            scene.resolve_tlas_hwrt,
        ) {
            (true, Some(tl), Some(dl), Some(v), Some(v2), Some(mv), Some(h), Some(to), Some(t)) => {
                (tl, dl, v, v2, mv, h, to, t)
            }
            _ => return None,
        };
        // The à-trous FINAL ring feeds `gVisIn` @0 (the temporal input): `shadow_vis2` for odd
        // `atrous_levels`, `shadow_vis` for even (incl. Temporal-only's `0` ⇒ the raw VIS). The SAME
        // parity the DENOISED resolve set + the record + graph use (W1).
        let final_ring = if scene.shadow_denoise_final_is_vis2 { vis2_ring } else { vis_ring };

        // (1) The temporal reproject UBO ring — one 16-byte host-coherent slot per FIF, zero-seeded
        // (the host memcpys `ResolvedTemporalShadow` each frame). On a slot's failure, drain [0..i).
        let mut ubo_slots: [Option<BoundBuffer>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for (i, dst) in ubo_slots.iter_mut().enumerate() {
            let b = match RhiDevice::create_buffer(
                ctx,
                &BufferDesc {
                    size: crate::present::TEMPORAL_SHADOW_UBO_BYTES,
                    usage: BufferUsage::UNIFORM,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            ) {
                Ok(b) => b,
                Err(_) => {
                    // Degrade to None (opt-in, no dependents): drain the [0..i) UBO slots.
                    // SAFETY: each was created on `ctx`, never submitted; destroy each once.
                    unsafe {
                        for s in ubo_slots.iter_mut().take(i) {
                            if let Some(b) = s.take() {
                                RhiDevice::destroy_buffer(ctx, b);
                            }
                        }
                    }
                    return None;
                }
            };
            if let Some(p) = RhiDevice::buffer_mapped_ptr(ctx, &b) {
                // SAFETY: `p` is the host-coherent mapping of a freshly-created >= 16-byte UNIFORM
                // buffer; writing `TEMPORAL_SHADOW_UBO_BYTES` zeroes stays in-bounds; byte `0` is a
                // valid init for the `f32` temporal lanes (host-overwritten before first read).
                unsafe {
                    core::ptr::write_bytes(
                        p.as_ptr(),
                        0,
                        crate::present::TEMPORAL_SHADOW_UBO_BYTES as usize,
                    );
                }
            }
            *dst = Some(b);
        }
        let ubo: [BoundBuffer; FRAMES_IN_FLIGHT] =
            ubo_slots.map(|s| s.expect("invariant: every temporal UBO ring slot built"));

        // (2) The 8-binding temporal reproject set ring. Slot `fi` binds `gVisIn` @0 = `final_ring[fi]`,
        // `gMotionVec` @1 = `motion_vec[fi]`, `gViewT` @2 = `viewt[fi]`, `gHistIn` @3 =
        // `shadow_temporal_hist[1-fi]` (the cross-frame READ — bound DIRECTLY, not framegraph-tracked),
        // `gHistOut` @4 = `shadow_temporal_hist[fi]` (the WRITE), `gTemporalOut` @5 = `temporal_out[fi]`,
        // the `ResolvedTemporalShadow` UBO @6 = `ubo[fi]`, the camera UBO @7 = `scene.camera_ring[fi]`.
        // On a slot's failure, drain the [0..slot) temporal sets + the UBO ring.
        let mut temporal_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for (slot, dst) in temporal_slots.iter_mut().enumerate() {
            let prev = FRAMES_IN_FLIGHT - 1 - slot;
            let entries = [
                BindGroupEntry::StorageImage { texture: &final_ring[slot] },
                BindGroupEntry::StorageImage { texture: &mvec[slot] },
                BindGroupEntry::StorageImage { texture: &viewt[slot] },
                BindGroupEntry::StorageImage { texture: &hist[prev] },
                BindGroupEntry::StorageImage { texture: &hist[slot] },
                BindGroupEntry::StorageImage { texture: &tout[slot] },
                BindGroupEntry::UniformBuffer { buffer: &ubo[slot] },
                BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[slot] },
            ];
            let desc = BindGroupDesc::<Vulkan> { layout: temporal_layout, entries: &entries };
            match RhiDevice::create_bind_group(ctx, &desc) {
                Ok(g) => *dst = Some(g),
                Err(_) => {
                    // SAFETY: the [0..slot) temporal sets + the whole UBO ring were created on `ctx`,
                    // never submitted; destroy each once (reverse acquisition: sets → UBO).
                    unsafe {
                        for s in temporal_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for b in ubo {
                            RhiDevice::destroy_buffer(ctx, b);
                        }
                    }
                    return None;
                }
            }
        }
        let temporal: [VulkanBindGroup; FRAMES_IN_FLIGHT] =
            temporal_slots.map(|s| s.expect("invariant: every temporal reproject set slot built"));

        // (3) The DENOISED-temporal resolve set ring — the SAME 22 VIS/DENOISED entries as the base
        // DENOISED set (the 19 shared via `resolve_software_entries` + TLAS @19 + soft-shadow UBO @20)
        // EXCEPT `gShadowVis` @21 = `temporal_out[slot]` (the DENOISED resolve READS the accumulated
        // visibility). Exact-fill at `RESOLVE_HWRT_DENOISE_BINDINGS` (22). On a slot's failure, drain
        // the [0..slot) denoised-temporal sets + the whole temporal set ring + the UBO ring.
        let mut den_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for (slot, dst) in den_slots.iter_mut().enumerate() {
            let imgs = ResolveSlotImages {
                albedo: &albedo[slot],
                normal: &normal[slot],
                material: &material[slot],
                lit: &lit[slot],
                viewt: &viewt[slot],
                ssao: &ssao[slot],
            };
            let shared =
                resolve_software_entries(scene, &imgs, slot, cluster_grid_buf, light_index_buf);
            let mut chained = shared
                .into_iter()
                .chain(core::iter::once(BindGroupEntry::AccelerationStructure {
                    accel: tlas[slot],
                }))
                .chain(core::iter::once(BindGroupEntry::UniformBuffer {
                    buffer: &scene.ray_shadow_ubo[slot],
                }))
                .chain(core::iter::once(BindGroupEntry::StorageImage {
                    texture: &tout[slot],
                }));
            let entries: [BindGroupEntry<'_, Vulkan>; RESOLVE_HWRT_DENOISE_BINDINGS] =
                core::array::from_fn(|_| {
                    chained.next().expect(
                        "invariant: the chained iterator yields exactly RESOLVE_HWRT_DENOISE_BINDINGS entries",
                    )
                });
            let dsc = BindGroupDesc::<Vulkan> { layout: denoise_layout, entries: &entries };
            match RhiDevice::create_bind_group(ctx, &dsc) {
                Ok(g) => *dst = Some(g),
                Err(_) => {
                    // SAFETY: the [0..slot) denoised-temporal sets + the whole temporal set ring + the
                    // UBO ring were created on `ctx`, never submitted; destroy each once.
                    unsafe {
                        for s in den_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for g in temporal {
                            RhiDevice::destroy_bind_group(ctx, g);
                        }
                        for b in ubo {
                            RhiDevice::destroy_buffer(ctx, b);
                        }
                    }
                    return None;
                }
            }
        }
        let denoised: [VulkanBindGroup; FRAMES_IN_FLIGHT] =
            den_slots.map(|s| s.expect("invariant: every DENOISED-temporal resolve set slot built"));

        Some(ShadowTemporalSets { ubo, temporal, denoised })
    }

    /// Rung R9d: builds the VB split's dedicated shadow-vis gather descriptor set RING — see
    /// [`Self::vb_shadow_vis_set`]'s doc for the 7-binding shape. DECOUPLED from the per-frame
    /// split/shadow activation (the "build on stable boot signals, not the per-frame gate" lesson
    /// [`Self::build_shadow_denoise_sets`]'s doc explains): gates on the BOOT-frozen split flag +
    /// `scene.shadow_denoise_enabled` (the config-requested spatial/temporal denoise) +
    /// [`GBufferScene::vb_shadow_vis_layout`] (`Some` iff the boot hwrt gate built the pipeline),
    /// and every bound resource. Returns `None` on the OFF path (byte-identical); degrades to
    /// `None` (draining its own partials) on any `create_bind_group` failure — opt-in, no
    /// dependents: `record_vb` GRACEFULLY skips the hwrt shadow chain that frame when it finds
    /// `None` (the deferred `record_gbuffer`'s own `if let (Some(sh), Some(vis_ring), ...)`
    /// precedent), never a panic.
    #[cfg(feature = "hwrt")]
    fn build_vb_shadow_vis_set(
        ctx: &VulkanContext,
        scene: &GBufferScene<'_>,
        thin_normal: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        viewt: &[VulkanTexture; FRAMES_IN_FLIGHT],
        shadow_vis: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
    ) -> Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> {
        let (layout, tn, sv, tlas) = match (
            scene.resolved_render_path.mesh_geo_shade_split && scene.shadow_denoise_enabled,
            scene.vb_shadow_vis_layout,
            thin_normal,
            shadow_vis,
            scene.resolve_tlas_hwrt,
        ) {
            (true, Some(l), Some(tn), Some(sv), Some(t)) => (l, tn, sv, t),
            _ => return None,
        };

        let mut slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for (i, dst) in slots.iter_mut().enumerate() {
            let entries = [
                BindGroupEntry::StorageImage { texture: &tn[i] },
                BindGroupEntry::StorageImage { texture: &viewt[i] },
                BindGroupEntry::StorageBuffer { buffer: scene.light_table },
                BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[i] },
                BindGroupEntry::AccelerationStructure { accel: tlas[i] },
                BindGroupEntry::UniformBuffer { buffer: &scene.ray_shadow_ubo[i] },
                BindGroupEntry::StorageImage { texture: &sv[i] },
            ];
            let desc = BindGroupDesc::<Vulkan> { layout, entries: &entries };
            match RhiDevice::create_bind_group(ctx, &desc) {
                Ok(g) => *dst = Some(g),
                Err(_) => {
                    // SAFETY: the [0..i) slots were created on `ctx`, never submitted; destroy
                    // each once.
                    unsafe {
                        for s in slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                    }
                    eprintln!(
                        "boyko_rhi_vulkan: vb_shadow_vis_set build failed — record_vb will skip the VB hwrt shadow chain this frame"
                    );
                    return None;
                }
            }
        }
        Some(slots.map(|s| s.expect("invariant: every vb_shadow_vis slot built")))
    }

    /// Rung R9d: builds the VB split's per-level à-trous denoise descriptor sets — mirrors
    /// [`Self::build_shadow_denoise_sets`]'s à-trous loop (part 4) but binds `thin_normal[fi]`
    /// at the `gNormal` slot and the split's OWN `viewt[fi]`, REUSING the SAME stable
    /// `scene.atrous_layout_denoise_hwrt` layout object + `shadow_denoise_ubo` ring the deferred
    /// chain builds (a stable, per-path-agnostic bind-group shape — the UBO/layout are boot
    /// artifacts, not path-scoped). Gates + degrades exactly like
    /// [`Self::build_vb_shadow_vis_set`].
    #[cfg(feature = "hwrt")]
    fn build_vb_shadow_atrous_sets(
        ctx: &VulkanContext,
        scene: &GBufferScene<'_>,
        thin_normal: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        viewt: &[VulkanTexture; FRAMES_IN_FLIGHT],
        shadow_vis: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        shadow_vis2: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        ubo: Option<&[BoundBuffer; FRAMES_IN_FLIGHT]>,
    ) -> Option<[[VulkanBindGroup; FRAMES_IN_FLIGHT]; crate::present::MAX_ATROUS_LEVELS as usize]> {
        let (layout, tn, vis_ring, vis2_ring, ubo) = match (
            scene.resolved_render_path.mesh_geo_shade_split && scene.shadow_denoise_enabled,
            scene.atrous_layout_denoise_hwrt,
            thin_normal,
            shadow_vis,
            shadow_vis2,
            ubo,
        ) {
            (true, Some(l), Some(tn), Some(v), Some(v2), Some(u)) => (l, tn, v, v2, u),
            _ => return None,
        };

        let mut atrous_opt: [[Option<VulkanBindGroup>; FRAMES_IN_FLIGHT];
            crate::present::MAX_ATROUS_LEVELS as usize] =
            core::array::from_fn(|_| [const { None }; FRAMES_IN_FLIGHT]);
        for level in 0..crate::present::MAX_ATROUS_LEVELS as usize {
            let (in_ring, out_ring) =
                if level % 2 == 0 { (vis_ring, vis2_ring) } else { (vis2_ring, vis_ring) };
            for slot in 0..FRAMES_IN_FLIGHT {
                let entries = [
                    BindGroupEntry::StorageImage { texture: &in_ring[slot] },
                    BindGroupEntry::StorageImage { texture: &out_ring[slot] },
                    BindGroupEntry::StorageImage { texture: &tn[slot] },
                    BindGroupEntry::StorageImage { texture: &viewt[slot] },
                    BindGroupEntry::UniformBuffer { buffer: &ubo[slot] },
                    BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[slot] },
                ];
                let desc = BindGroupDesc::<Vulkan> { layout, entries: &entries };
                match RhiDevice::create_bind_group(ctx, &desc) {
                    Ok(g) => atrous_opt[level][slot] = Some(g),
                    Err(_) => {
                        // SAFETY: every set built so far (prior levels + this level's [0..slot))
                        // was created on `ctx`, never submitted; destroy each once.
                        unsafe {
                            for lvl in atrous_opt.iter_mut() {
                                for s in lvl.iter_mut() {
                                    if let Some(g) = s.take() {
                                        RhiDevice::destroy_bind_group(ctx, g);
                                    }
                                }
                            }
                        }
                        eprintln!(
                            "boyko_rhi_vulkan: vb_shadow_atrous_sets build failed — record_vb will skip the VB hwrt shadow chain this frame"
                        );
                        return None;
                    }
                }
            }
        }
        Some(atrous_opt.map(|lvl| lvl.map(|s| s.expect("invariant: every vb_shadow_atrous slot built"))))
    }

    /// Rung R9d: builds the VB split's temporal reproject descriptor set RING — mirrors
    /// [`Self::build_shadow_temporal_sets`]'s temporal-set half (part 2) but binds `viewt[fi]`
    /// at the `gViewT` slot, REUSING the SAME stable `scene.temporal_layout` layout object +
    /// `temporal_shadow_ubo` ring the deferred chain builds. Gates + degrades exactly like
    /// [`Self::build_vb_shadow_vis_set`], with the ADDITIONAL `scene.temporal_enabled` gate
    /// (mirrors [`Self::build_shadow_temporal_sets`]'s own gate).
    #[cfg(feature = "hwrt")]
    #[allow(clippy::too_many_arguments)]
    fn build_vb_shadow_temporal_set(
        ctx: &VulkanContext,
        scene: &GBufferScene<'_>,
        viewt: &[VulkanTexture; FRAMES_IN_FLIGHT],
        shadow_vis: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        shadow_vis2: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        motion_vec: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        shadow_temporal_hist: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        temporal_out: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        ubo: Option<&[BoundBuffer; FRAMES_IN_FLIGHT]>,
    ) -> Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> {
        let (layout, vis_ring, vis2_ring, mvec, hist, tout, ubo) = match (
            scene.resolved_render_path.mesh_geo_shade_split
                && scene.shadow_denoise_enabled
                && scene.temporal_enabled,
            scene.temporal_layout,
            shadow_vis,
            shadow_vis2,
            motion_vec,
            shadow_temporal_hist,
            temporal_out,
            ubo,
        ) {
            (true, Some(l), Some(v), Some(v2), Some(mv), Some(h), Some(to), Some(u)) => {
                (l, v, v2, mv, h, to, u)
            }
            _ => return None,
        };
        let final_ring = if scene.shadow_denoise_final_is_vis2 { vis2_ring } else { vis_ring };

        let mut slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for (slot, dst) in slots.iter_mut().enumerate() {
            let prev = FRAMES_IN_FLIGHT - 1 - slot;
            let entries = [
                BindGroupEntry::StorageImage { texture: &final_ring[slot] },
                BindGroupEntry::StorageImage { texture: &mvec[slot] },
                BindGroupEntry::StorageImage { texture: &viewt[slot] },
                BindGroupEntry::StorageImage { texture: &hist[prev] },
                BindGroupEntry::StorageImage { texture: &hist[slot] },
                BindGroupEntry::StorageImage { texture: &tout[slot] },
                BindGroupEntry::UniformBuffer { buffer: &ubo[slot] },
                BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[slot] },
            ];
            let desc = BindGroupDesc::<Vulkan> { layout, entries: &entries };
            match RhiDevice::create_bind_group(ctx, &desc) {
                Ok(g) => *dst = Some(g),
                Err(_) => {
                    // SAFETY: the [0..slot) sets were created on `ctx`, never submitted; destroy
                    // each once.
                    unsafe {
                        for s in slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                    }
                    eprintln!(
                        "boyko_rhi_vulkan: vb_shadow_temporal_set build failed — record_vb will skip the VB temporal reproject this frame"
                    );
                    return None;
                }
            }
        }
        Some(slots.map(|s| s.expect("invariant: every vb_shadow_temporal slot built")))
    }

    /// Creates a 2D `R8G8B8A8_UNORM` storage image at `extent` with `usage`. A small
    /// helper shared by the albedo/normal/material allocations in [`Self::create`].
    fn create_gbuffer_image(
        ctx: &VulkanContext,
        extent: VkExtent2D,
        usage: ImageUsage,
    ) -> Result<VulkanTexture, SwapchainError> {
        let desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format: GBUFFER_FORMAT,
            dimension: TextureDimension::D2,
            usage,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        };
        RhiDevice::create_texture(ctx, &desc).map_err(SwapchainError::DepthImage)
    }

    /// Anti-aliasing Stage 2 (W4 — Decision "left byte-for-byte untouched"): a
    /// FORMAT-PARAMETERIZED sibling of [`Self::create_gbuffer_image`], used ONLY by
    /// [`SmaaImages`] (the SMAA `edges`/`weights` targets need `R8G8_UNORM`/`R8G8B8A8_UNORM`
    /// respectively — NOT the fixed [`GBUFFER_FORMAT`] `create_gbuffer_image` hardcodes). A
    /// NEW standalone function, not a re-point of `create_gbuffer_image` — every existing
    /// caller of `create_gbuffer_image` stays byte-for-byte unchanged.
    fn create_gbuffer_image_fmt(
        ctx: &VulkanContext,
        extent: VkExtent2D,
        format: Format,
        usage: ImageUsage,
    ) -> Result<VulkanTexture, SwapchainError> {
        let desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format,
            dimension: TextureDimension::D2,
            usage,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        };
        RhiDevice::create_texture(ctx, &desc).map_err(SwapchainError::DepthImage)
    }

    /// Creates the Lighting-L0b `gViewT` lane: a 2D `R32_SFLOAT` STORAGE image at `extent`
    /// (the marcher's surface ray param `t`). A separate helper from
    /// [`Self::create_gbuffer_image`] because the lane is `R32_SFLOAT`, not the RGBA8
    /// [`GBUFFER_FORMAT`]. W2: `R32_SFLOAT`/`STORAGE_IMAGE` support is fail-fast-checked
    /// at device boot ([`crate::device::DeviceCaps::viewt_storage_format_ok`]), so this
    /// create can never fault on an unsupported format.
    fn create_viewt_image(
        ctx: &VulkanContext,
        extent: VkExtent2D,
    ) -> Result<VulkanTexture, SwapchainError> {
        let desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format: GVIEWT_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        };
        RhiDevice::create_texture(ctx, &desc).map_err(SwapchainError::DepthImage)
    }

    /// Creates the Render P7 SSAO term `gSsao`: a 2D `R8_UNORM` STORAGE image at `extent`
    /// (the per-pixel HBAO-lite ambient occlusion). A separate helper from
    /// [`Self::create_gbuffer_image`] because the lane is `R8_UNORM`, not the RGBA8
    /// [`GBUFFER_FORMAT`]. P7: `R8_UNORM`/`STORAGE_IMAGE` support is fail-fast-checked at
    /// device boot ([`crate::device::DeviceCaps::r8_unorm_storage_ok`]), so this create can
    /// never fault on an unsupported format.
    fn create_ssao_image(
        ctx: &VulkanContext,
        extent: VkExtent2D,
    ) -> Result<VulkanTexture, SwapchainError> {
        let desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format: SSAO_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        };
        RhiDevice::create_texture(ctx, &desc).map_err(SwapchainError::DepthImage)
    }

    /// The SSAO à-trous denoise chain: creates one slot of an interior ping-pong ring
    /// (`ssao_ring_a` or `ssao_ring_b`): a 2D `R16_UNORM` STORAGE image at `extent`. The caller
    /// only invokes this after `ssao_atrous_storage_ok()` is `true` (the boot probe), so the
    /// create cannot fault on an unsupported storage format (the `shadow_vis` create's
    /// probe-gated-not-fail-fast discipline).
    fn create_ssao_atrous_ring_image(
        ctx: &VulkanContext,
        extent: VkExtent2D,
    ) -> Result<VulkanTexture, SwapchainError> {
        let desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format: SSAO_ATROUS_RING_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        };
        RhiDevice::create_texture(ctx, &desc).map_err(SwapchainError::DepthImage)
    }

    /// Textured-PBR T6a: creates one slot of the `gPbr` deferred-resolve MRT lane: a 2D
    /// `R16G16B16A16_SFLOAT` image at `extent`. `STORAGE` (the SOFTWARE resolve's flag-gated
    /// `.Load`) | `COLOR_ATTACHMENT` (the T6c textured raster's 4th MRT write; UNWRITTEN this
    /// rung). `R16G16B16A16_SFLOAT`/`STORAGE_IMAGE` support is part of the Vulkan 1.0 CORE
    /// mandatory format table (unlike `R8_UNORM`/`R16G16_UNORM`, which need a boot probe), so —
    /// like [`Self::create_gbuffer_image`] — this create can never fault on an unsupported format.
    fn create_pbr_image(
        ctx: &VulkanContext,
        extent: VkExtent2D,
    ) -> Result<VulkanTexture, SwapchainError> {
        let desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format: GPBR_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE | ImageUsage::COLOR_ATTACHMENT,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        };
        RhiDevice::create_texture(ctx, &desc).map_err(SwapchainError::DepthImage)
    }

    /// Rung 3a: creates one slot of the RT soft-shadow VISIBILITY target `shadow_vis`: a 2D
    /// `R16G16_UNORM` STORAGE image at `extent` (`R` = mesh visibility, `G` = validity). Shares the
    /// format with [`Self::create_shadow_vis2_image`] (the uniform-RG16 design, so one `"rg16"`
    /// shader pin fits both ping-pong rings). The caller only invokes this after
    /// `shadow_denoise_storage_ok()` is `true` (the boot probe), so the create cannot fault on an
    /// unsupported storage format (the SSAO-helper discipline, but probe-gated rather than
    /// boot-fail-fast — the denoise is opt-in).
    #[cfg(feature = "hwrt")]
    fn create_shadow_vis_image(
        ctx: &VulkanContext,
        extent: VkExtent2D,
    ) -> Result<VulkanTexture, SwapchainError> {
        let desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format: SHADOW_VIS_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        };
        RhiDevice::create_texture(ctx, &desc).map_err(SwapchainError::DepthImage)
    }

    /// Rung 3a: creates one slot of the à-trous ping-pong target `shadow_vis2`: a 2D
    /// `R16G16_UNORM` STORAGE image at `extent` — the SAME [`SHADOW_VIS2_FORMAT`] ==
    /// [`SHADOW_VIS_FORMAT`] as [`Self::create_shadow_vis_image`] (the uniform-RG16 ping-pong, so one
    /// `"rg16"` shader pin fits both rings). Kept a separate named helper for call-site clarity (the
    /// second ping-pong ring). Probe-gated exactly like [`Self::create_shadow_vis_image`].
    #[cfg(feature = "hwrt")]
    fn create_shadow_vis2_image(
        ctx: &VulkanContext,
        extent: VkExtent2D,
    ) -> Result<VulkanTexture, SwapchainError> {
        let desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format: SHADOW_VIS2_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        };
        RhiDevice::create_texture(ctx, &desc).map_err(SwapchainError::DepthImage)
    }

    /// HW-RT Rung 3b: creates one slot of the motion-vector target `motion_vec` — a 2D
    /// [`MOTION_VEC_FORMAT`] (`R16G16_SFLOAT`) image at `extent`. `STORAGE` (the temporal reproject
    /// reads it) | `SAMPLED` (bilinear reproject) | `COLOR_ATTACHMENT` (the raster gbuffer MV MRT
    /// writes it in step 5). Probe-gated like [`Self::create_shadow_vis_image`].
    #[cfg(feature = "hwrt")]
    fn create_motion_vec_image(
        ctx: &VulkanContext,
        extent: VkExtent2D,
    ) -> Result<VulkanTexture, SwapchainError> {
        let desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format: MOTION_VEC_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE | ImageUsage::SAMPLED | ImageUsage::COLOR_ATTACHMENT,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        };
        RhiDevice::create_texture(ctx, &desc).map_err(SwapchainError::DepthImage)
    }

    /// HW-RT Rung 3b: creates one slot of the temporal shadow-vis HISTORY ring
    /// `shadow_temporal_hist` — a 2D [`SHADOW_TEMPORAL_HIST_FORMAT`] (`R16G16B16A16_UNORM`) image at
    /// `extent`. `STORAGE` (the temporal pass reads/writes) | `SAMPLED` (the bilinear reproject of
    /// the previous slot). Probe-gated like [`Self::create_shadow_vis_image`].
    #[cfg(feature = "hwrt")]
    fn create_shadow_temporal_hist_image(
        ctx: &VulkanContext,
        extent: VkExtent2D,
    ) -> Result<VulkanTexture, SwapchainError> {
        let desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format: SHADOW_TEMPORAL_HIST_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE | ImageUsage::SAMPLED,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        };
        RhiDevice::create_texture(ctx, &desc).map_err(SwapchainError::DepthImage)
    }

    /// HW-RT Rung 3b: creates one slot of the temporal-accumulate OUTPUT `temporal_out` — a 2D
    /// [`TEMPORAL_OUT_FORMAT`] (`R16G16_UNORM`, same as `shadow_vis`) image at `extent`. `STORAGE`
    /// (the temporal pass writes) | `SAMPLED` (the DENOISED resolve reads it as `gShadowVis`).
    /// Probe-gated like [`Self::create_shadow_vis_image`].
    #[cfg(feature = "hwrt")]
    fn create_temporal_out_image(
        ctx: &VulkanContext,
        extent: VkExtent2D,
    ) -> Result<VulkanTexture, SwapchainError> {
        let desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format: TEMPORAL_OUT_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE | ImageUsage::SAMPLED,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        };
        RhiDevice::create_texture(ctx, &desc).map_err(SwapchainError::DepthImage)
    }

    /// Anti-aliasing Stage 4 (TAA W4): creates one slot of the `taa_hist` color-history ring — a
    /// 2D [`TAA_HIST_FORMAT`] (`R16G16B16A16_SFLOAT`) image at `extent`. `STORAGE` (the resolve
    /// reads/writes it — v1's history reproject is a manual `Load`-based reconstruction, mirroring
    /// [`Self::create_shadow_temporal_hist_image`]) | `SAMPLED` (reserved, unused by v1 — kept for
    /// shape-parity with every other GBuffer image, matching the shadow-temporal precedent's own
    /// `STORAGE | SAMPLED` usage even though it too reads via `Load`).
    fn create_taa_hist_image(
        ctx: &VulkanContext,
        extent: VkExtent2D,
    ) -> Result<VulkanTexture, SwapchainError> {
        let desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format: TAA_HIST_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE | ImageUsage::SAMPLED,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        };
        RhiDevice::create_texture(ctx, &desc).map_err(SwapchainError::DepthImage)
    }

    /// Anti-aliasing Stage 4 (TAA W4): builds the FIF-ringed `taa_hist` target, DEGRADING to
    /// `None` (leak-safe) on any per-slot create failure — the opt-in "recorded-not-fail-fast"
    /// policy mirroring [`Self::build_denoise_ring`] (UNCONDITIONAL here, unlike that hwrt-only
    /// helper — TAA is not hwrt-gated). Built LAST (after every fallible descriptor set) and never
    /// propagates `Err`, so it needs NO teardown weaving into the earlier error ladder.
    fn build_taa_hist_ring(
        ctx: &VulkanContext,
        extent: VkExtent2D,
    ) -> Option<[VulkanTexture; FRAMES_IN_FLIGHT]> {
        let mut slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in slots.iter_mut() {
            match Self::create_taa_hist_image(ctx, extent) {
                Ok(t) => *slot = Some(t),
                Err(_) => {
                    // SAFETY: each `Some` slot was created on `ctx` just above, is referenced by no
                    // submission (build phase), and is destroyed exactly once (the `take`).
                    for s in slots.iter_mut() {
                        if let Some(t) = s.take() {
                            unsafe { RhiDevice::destroy_texture(ctx, t) };
                        }
                    }
                    return None;
                }
            }
        }
        Some(slots.map(|s| s.expect("invariant: every taa_hist ring slot built above")))
    }

    /// Anti-aliasing Stage 4 (TAA W5, the M2 fix): builds the FIF-ringed `taa_hist` target AND
    /// boot-clears BOTH physical slots before the first frame reads them — mirrors
    /// [`Self::build_and_clear_shadow_temporal_hist`]'s C1/H2 discipline (UNCONDITIONAL here,
    /// unlike that hwrt-only helper). `taa_hist` is a CROSS-FRAME PERSISTENT parity ping-pong
    /// pool; the framegraph seeds its ResIds at `GENERAL` (`graph_bridge.rs`'s `taa_hist`/
    /// `taa_hist_read` declaration), which ASSUMES the image already holds a real `GENERAL`
    /// layout — but a fresh image is `UNDEFINED`. This clears each slot to `[0, 0, 0, 0]` (RGB =
    /// 0, confidence = 0 — inert; `TaaState.reset` forces `blend_factor == 1.0` on the frame that
    /// actually reads it, so the cleared color is never blended) and transitions
    /// `UNDEFINED` → `GENERAL`, satisfying the seed's layout assumption AND making the clear
    /// visible to the first `COMPUTE` read.
    ///
    /// Called from [`Self::create`] (like `build_taa_hist_ring` was), NOT a boot-only one-shot:
    /// `sync_gbuffer`'s resize path rebuilds targets through `create`, so a resize RE-clears the
    /// fresh pool. DEGRADES to `None` (leak-safe, TAA off ⇒ byte-identical) on any build /
    /// encoder / submit / fence failure, like [`Self::build_denoise_ring`].
    fn build_and_clear_taa_hist(
        ctx: &VulkanContext,
        extent: VkExtent2D,
    ) -> Option<[VulkanTexture; FRAMES_IN_FLIGHT]> {
        let pool = Self::build_taa_hist_ring(ctx, extent)?;

        match Self::boot_clear_taa_hist(ctx, &pool) {
            Ok(()) => Some(pool),
            Err(_) => {
                // Degrade to None (opt-in, no dependents). The boot-clear submit (if it ran)
                // faulted — drain the device so no in-flight clear still references the pool
                // before destroy.
                let _ = RhiDevice::wait_idle(ctx);
                // SAFETY: each pool texture was created on `ctx` in `build_taa_hist_ring`; the
                // device is drained above ⇒ no submission references them; each is moved by value
                // out of `pool` ⇒ destroyed exactly once.
                for t in pool {
                    unsafe { RhiDevice::destroy_texture(ctx, t) };
                }
                None
            }
        }
    }

    /// Records + submits ONE encoder that boot-clears BOTH `pool` slots (`UNDEFINED` →
    /// `TRANSFER_DST_OPTIMAL` → clear → `GENERAL`) and fence-waits it — mirrors
    /// [`Self::boot_clear_shadow_temporal_hist`] (UNCONDITIONAL here, unlike that hwrt-only
    /// helper). The encoder + fence are setup-class transients torn down here on every path.
    fn boot_clear_taa_hist(
        ctx: &VulkanContext,
        pool: &[VulkanTexture; FRAMES_IN_FLIGHT],
    ) -> Result<(), SwapchainError> {
        let mut encoder =
            RhiDevice::create_command_encoder(ctx).map_err(SwapchainError::DepthImage)?;
        let fence = match RhiDevice::create_fence(ctx, false) {
            Ok(f) => f,
            Err(e) => {
                // SAFETY: `encoder` was just created on `ctx`, never submitted; destroy once.
                unsafe { RhiDevice::destroy_command_encoder(ctx, encoder) };
                return Err(SwapchainError::DepthImage(e));
            }
        };

        // The full COLOR range of a 2D single-layer image (per `create_taa_hist_image`).
        let range = ImageSubresourceRange {
            aspect: ImageAspect::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };

        let record = (|| -> Result<(), SwapchainError> {
            encoder.begin().map_err(SwapchainError::DepthImage)?;

            // Both slots: UNDEFINED → TRANSFER_DST_OPTIMAL (a fresh image has no prior contents,
            // so UNDEFINED discards — this is the clear destination).
            for tex in pool {
                encoder.image_barrier(&ImageBarrierDesc {
                    texture: tex,
                    src_stage: BarrierStage::TOP_OF_PIPE,
                    dst_stage: BarrierStage::TRANSFER,
                    src_access: BarrierAccess::NONE,
                    dst_access: BarrierAccess::TRANSFER_WRITE,
                    old_layout: ImageLayout::Undefined,
                    new_layout: ImageLayout::TransferDstOptimal,
                    range,
                });
            }

            // Clear each slot to RGB = 0, confidence = 0 — inert (the first read that actually
            // consumes it does so under a host-forced `TaaState.reset`, replacing rather than
            // blending it).
            for tex in pool {
                encoder.clear_color_image(
                    tex,
                    ImageLayout::TransferDstOptimal,
                    [0.0, 0.0, 0.0, 0.0],
                    range,
                );
            }

            // Both slots: TRANSFER_DST_OPTIMAL → GENERAL, made available to
            // COMPUTE_SHADER/SHADER_READ — the first resolve read must SEE the clear, and GENERAL
            // also satisfies the framegraph's `taa_hist`/`taa_hist_read` seed layout assumption.
            for tex in pool {
                encoder.image_barrier(&ImageBarrierDesc {
                    texture: tex,
                    src_stage: BarrierStage::TRANSFER,
                    dst_stage: BarrierStage::COMPUTE_SHADER,
                    src_access: BarrierAccess::TRANSFER_WRITE,
                    dst_access: BarrierAccess::SHADER_READ,
                    old_layout: ImageLayout::TransferDstOptimal,
                    new_layout: ImageLayout::General,
                    range,
                });
            }

            encoder.end().map_err(SwapchainError::DepthImage)?;
            let queue = ctx.rhi_queue();
            queue.submit(&encoder, &fence).map_err(SwapchainError::DepthImage)?;
            RhiDevice::wait_fence(ctx, &fence, u64::MAX).map_err(SwapchainError::DepthImage)?;
            Ok(())
        })();

        // Tear down the setup-class transients. The submit (if it ran) is fence-waited on the Ok
        // path.
        // SAFETY: encoder/fence were created on `ctx`; the encoder's only submission (if any) is
        // fence-waited above on the Ok path (or never submitted / faulted on an error path), and
        // each is moved by value ⇒ destroyed exactly once.
        unsafe {
            RhiDevice::destroy_command_encoder(ctx, encoder);
            RhiDevice::destroy_fence(ctx, fence);
        }
        record
    }

    /// TAA rung T3: builds the FIF-ringed `taa_resolved` RCAS-intermediate target, DEGRADING to
    /// `None` (leak-safe) on any per-slot create failure — mirrors [`Self::build_taa_hist_ring`]'s
    /// opt-in "recorded-not-fail-fast" policy (UNCONDITIONAL here — RCAS is not hwrt-gated). Built
    /// ONLY when `scene.rcas.is_some()` (`SharpenMode::None`, the default, never calls this — the
    /// 0%-gate). No boot-clear (unlike `taa_hist`): the resolve writes every dispatched pixel of
    /// `gAaOut` unconditionally each frame it runs, so a fresh image's undefined initial contents
    /// are never read (see [`GBufferTargets::taa_resolved`]'s field doc).
    fn build_taa_resolved_ring(
        ctx: &VulkanContext,
        extent: VkExtent2D,
    ) -> Option<[VulkanTexture; FRAMES_IN_FLIGHT]> {
        let mut slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in slots.iter_mut() {
            match Self::create_gbuffer_image(ctx, extent, ImageUsage::STORAGE) {
                Ok(t) => *slot = Some(t),
                Err(_) => {
                    // SAFETY: each `Some` slot was created on `ctx` just above, is referenced by
                    // no submission (build phase), and is destroyed exactly once (the `take`).
                    for s in slots.iter_mut() {
                        if let Some(t) = s.take() {
                            unsafe { RhiDevice::destroy_texture(ctx, t) };
                        }
                    }
                    return None;
                }
            }
        }
        Some(slots.map(|s| s.expect("invariant: every taa_resolved ring slot built above")))
    }

    /// Anti-aliasing Stage 4 (TAA W5): builds the temporal-resolve descriptor set + its two OWN
    /// UBO rings — the `ResolvedTaa` tunables ring (48 B, rung T2) and the DEDICATED `MotionCam`
    /// ring (128 B, SEPARATE from the hwrt mesh-shadow `motion_cam_ubo` — see `TaaActivation`'s
    /// "why a dedicated ring" doc for the ONE-call-per-frame `MotionCamState::advance` rationale).
    /// Mirrors [`Self::build_shadow_temporal_sets`]'s shape (own UBO ring(s) + one set), built
    /// LAST in [`Self::create`] (after `taa_hist`, which it binds) and DEGRADES-TO-`None` on any
    /// failure (leak-safe, opt-in — UNCONDITIONAL here, unlike the hwrt-only temporal builder).
    /// `None` when `scene.taa` is absent (the 0%-gate) or `taa_hist`/`aa_out` failed to allocate.
    ///
    /// TAA rung T3: `aa_out`'s param name is kept generic — the CALLER ([`Self::create`]) passes
    /// whichever ring `gAaOut` @4 should bind THIS frame: [`GBufferTargets::taa_resolved`] when
    /// `scene.rcas.is_some()` (RCAS armed — the resolve's output is an intermediate, re-pointed
    /// here), else [`GBufferTargets::aa_out`] (the unchanged direct present-blit input). This fn's
    /// OWN body is untouched by the repoint — it just binds whatever `aa_out` slice it is given.
    fn build_taa_resolve_set(
        ctx: &VulkanContext,
        scene: &GBufferScene<'_>,
        lit: &[VulkanTexture; FRAMES_IN_FLIGHT],
        viewt: &[VulkanTexture; FRAMES_IN_FLIGHT],
        taa_hist: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        aa_out: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
    ) -> Option<TaaResolveSets> {
        let (taa, hist, out) = match (scene.taa.as_ref(), taa_hist, aa_out) {
            (Some(t), Some(h), Some(o)) => (t, h, o),
            _ => return None,
        };

        // (1) The `ResolvedTaa` tunables UBO ring — 48 B (rung T2), zero-seeded (the host
        // memcpys each armed frame). On a slot's failure, drain [0..i).
        let mut taa_ubo_slots: [Option<BoundBuffer>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for (i, dst) in taa_ubo_slots.iter_mut().enumerate() {
            let b = match RhiDevice::create_buffer(
                ctx,
                &BufferDesc {
                    size: crate::present::TAA_UBO_BYTES,
                    usage: BufferUsage::UNIFORM,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            ) {
                Ok(b) => b,
                Err(_) => {
                    // Degrade to None (opt-in, no dependents): drain the [0..i) UBO slots.
                    // SAFETY: each was created on `ctx`, never submitted; destroy each once.
                    unsafe {
                        for s in taa_ubo_slots.iter_mut().take(i) {
                            if let Some(b) = s.take() {
                                RhiDevice::destroy_buffer(ctx, b);
                            }
                        }
                    }
                    return None;
                }
            };
            if let Some(p) = RhiDevice::buffer_mapped_ptr(ctx, &b) {
                // SAFETY: `p` is the host-coherent mapping of a freshly-created >= 48-byte UNIFORM
                // buffer; writing `TAA_UBO_BYTES` zeroes stays in-bounds; byte `0` is a valid init
                // for the `f32` tunable lanes AND every T2 mode word (the zero-is-shipped-default
                // invariant — see `boyko_render::aa_config::ResolvedTaa`'s doc), host-overwritten
                // before first read regardless.
                unsafe {
                    core::ptr::write_bytes(p.as_ptr(), 0, crate::present::TAA_UBO_BYTES as usize);
                }
            }
            *dst = Some(b);
        }
        let taa_ubo: [BoundBuffer; FRAMES_IN_FLIGHT] =
            taa_ubo_slots.map(|s| s.expect("invariant: every TAA tunables UBO ring slot built"));

        // (2) The DEDICATED `MotionCam` UBO ring — 128 B, zero-seeded. On a slot's failure, drain
        // [0..i) + the tunables ring.
        let mut mc_slots: [Option<BoundBuffer>; FRAMES_IN_FLIGHT] = [const { None }; FRAMES_IN_FLIGHT];
        for (i, dst) in mc_slots.iter_mut().enumerate() {
            let b = match RhiDevice::create_buffer(
                ctx,
                &BufferDesc {
                    size: crate::present::TAA_MOTION_CAM_UBO_BYTES,
                    usage: BufferUsage::UNIFORM,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            ) {
                Ok(b) => b,
                Err(_) => {
                    // SAFETY: the [0..i) MotionCam slots + the whole tunables ring were created on
                    // `ctx`, never submitted; destroy each once (reverse acquisition).
                    unsafe {
                        for s in mc_slots.iter_mut().take(i) {
                            if let Some(b) = s.take() {
                                RhiDevice::destroy_buffer(ctx, b);
                            }
                        }
                        for b in taa_ubo {
                            RhiDevice::destroy_buffer(ctx, b);
                        }
                    }
                    return None;
                }
            };
            if let Some(p) = RhiDevice::buffer_mapped_ptr(ctx, &b) {
                // SAFETY: `p` is the host-coherent mapping of a freshly-created >= 128-byte
                // UNIFORM buffer; writing `TAA_MOTION_CAM_UBO_BYTES` zeroes stays in-bounds; byte
                // `0` is a valid init for the `float4x4` lanes (host-overwritten before first
                // read — a zeroed pair yields `MV == 0`, the disocclusion-safe seed).
                unsafe {
                    core::ptr::write_bytes(
                        p.as_ptr(),
                        0,
                        crate::present::TAA_MOTION_CAM_UBO_BYTES as usize,
                    );
                }
            }
            *dst = Some(b);
        }
        let motion_cam_ubo: [BoundBuffer; FRAMES_IN_FLIGHT] =
            mc_slots.map(|s| s.expect("invariant: every TAA MotionCam UBO ring slot built"));

        // (3) The 8-binding resolve set ring. Slot `fi` binds `gLit` @0 = `lit[fi]` (+ the LINEAR
        // sampler), `gViewT` @1 = `viewt[fi]`, `gHistIn` @2 = `taa_hist[1-fi]` (the cross-frame
        // READ — bound DIRECTLY, not framegraph-tracked), `gHistOut` @3 = `taa_hist[fi]` (the
        // WRITE), `gAaOut` @4 = `aa_out[fi]`, the `ResolvedTaa` UBO @5 = `taa_ubo[fi]`, the camera
        // UBO @6 = `scene.camera_ring[fi]` (UNJITTERED, C1 cut), the `MotionCam` UBO @7 =
        // `motion_cam_ubo[fi]`. On a slot's failure, drain [0..slot) + both UBO rings.
        let mut set_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for (slot, dst) in set_slots.iter_mut().enumerate() {
            let prev = FRAMES_IN_FLIGHT - 1 - slot;
            let entries = [
                BindGroupEntry::CombinedImage { texture: &lit[slot], sampler: taa.linear_sampler },
                BindGroupEntry::StorageImage { texture: &viewt[slot] },
                BindGroupEntry::StorageImage { texture: &hist[prev] },
                BindGroupEntry::StorageImage { texture: &hist[slot] },
                BindGroupEntry::StorageImage { texture: &out[slot] },
                BindGroupEntry::UniformBuffer { buffer: &taa_ubo[slot] },
                BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[slot] },
                BindGroupEntry::UniformBuffer { buffer: &motion_cam_ubo[slot] },
            ];
            let desc = BindGroupDesc::<Vulkan> { layout: taa.resolve_layout, entries: &entries };
            match RhiDevice::create_bind_group(ctx, &desc) {
                Ok(g) => *dst = Some(g),
                Err(_) => {
                    // SAFETY: the [0..slot) resolve sets + both whole UBO rings were created on
                    // `ctx`, never submitted; destroy each once (reverse acquisition: sets → mc →
                    // taa_ubo).
                    unsafe {
                        for s in set_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        for b in motion_cam_ubo {
                            RhiDevice::destroy_buffer(ctx, b);
                        }
                        for b in taa_ubo {
                            RhiDevice::destroy_buffer(ctx, b);
                        }
                    }
                    return None;
                }
            }
        }
        let set: [VulkanBindGroup; FRAMES_IN_FLIGHT] =
            set_slots.map(|s| s.expect("invariant: every TAA resolve set slot built"));

        Some(TaaResolveSets { taa_ubo, motion_cam_ubo, set })
    }

    /// TAA rung T3: builds the RCAS descriptor set ring (2 STORAGE-image bindings, no UBO)
    /// against [`RcasActivation::rcas_layout`] — `gRcasIn` @0 = `taa_resolved[fi]` (the
    /// resolve's re-pointed intermediate write), `gAaOut` @1 = `aa_out[fi]` (the present-blit's
    /// input, unchanged). Built LAST (after [`Self::build_taa_resolve_set`], which repoints the
    /// resolve's OWN `gAaOut` at `taa_resolved` instead) and DEGRADES-TO-`None` on any failure
    /// (leak-safe, opt-in — mirrors [`Self::build_taa_resolve_set`]'s per-slot drain). `None`
    /// when `scene.rcas` is absent (the 0%-gate) or `taa_resolved`/`aa_out` failed to allocate.
    fn build_rcas_set(
        ctx: &VulkanContext,
        scene: &GBufferScene<'_>,
        taa_resolved: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
        aa_out: Option<&[VulkanTexture; FRAMES_IN_FLIGHT]>,
    ) -> Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> {
        let (rcas, resolved, out) = match (scene.rcas.as_ref(), taa_resolved, aa_out) {
            (Some(r), Some(resolved), Some(out)) => (r, resolved, out),
            _ => return None,
        };

        let mut slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for (slot, dst) in slots.iter_mut().enumerate() {
            let entries = [
                BindGroupEntry::StorageImage { texture: &resolved[slot] },
                BindGroupEntry::StorageImage { texture: &out[slot] },
            ];
            let desc = BindGroupDesc::<Vulkan> { layout: rcas.rcas_layout, entries: &entries };
            match RhiDevice::create_bind_group(ctx, &desc) {
                Ok(g) => *dst = Some(g),
                Err(_) => {
                    // SAFETY: the [0..slot) RCAS set slots were created on `ctx`, never
                    // submitted; destroy each once (reverse acquisition within this ring).
                    unsafe {
                        for s in slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                    }
                    return None;
                }
            }
        }
        Some(slots.map(|s| s.expect("invariant: every RCAS set slot built above")))
    }

    /// HW-RT Rung 3b: builds one FIF-ringed temporal denoise target, DEGRADING to `None` (leak-
    /// safe) on any per-slot create failure — the opt-in "recorded-not-fail-fast" policy: a device
    /// that faults on the RG16F/RGBA16 storage format disables temporal denoise rather than failing
    /// the whole swapchain. Because these rings are built LAST (after every fallible descriptor set)
    /// and never propagate `Err`, they need NO teardown weaving into the earlier error ladder.
    #[cfg(feature = "hwrt")]
    fn build_denoise_ring(
        ctx: &VulkanContext,
        extent: VkExtent2D,
        create: impl Fn(&VulkanContext, VkExtent2D) -> Result<VulkanTexture, SwapchainError>,
    ) -> Option<[VulkanTexture; FRAMES_IN_FLIGHT]> {
        let mut slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in slots.iter_mut() {
            match create(ctx, extent) {
                Ok(t) => *slot = Some(t),
                Err(_) => {
                    // SAFETY: each `Some` slot was created on `ctx` just above, is referenced by no
                    // submission (build phase), and is destroyed exactly once (the `take`).
                    for s in slots.iter_mut() {
                        if let Some(t) = s.take() {
                            unsafe { RhiDevice::destroy_texture(ctx, t) };
                        }
                    }
                    return None;
                }
            }
        }
        Some(slots.map(|s| s.expect("invariant: every denoise ring slot built above")))
    }

    /// HW-RT Rung 3b C1/H2: builds the temporal shadow-vis HISTORY POOL ring AND boot-clears BOTH
    /// physical slots before the first frame reads them. `shadow_temporal_hist` is a CROSS-FRAME
    /// PERSISTENT parity ping-pong POOL; the framegraph seeds ResId 14/16 at `GENERAL`, which ASSUMES
    /// the image already holds a real `GENERAL` layout — but a fresh image is `UNDEFINED`. This clears
    /// each slot to `[1, 0, 0, 0]` (R = vis = 1, G = conf = 0 ⇒ the first temporal read sees
    /// `conf == 0` = a disocclusion reset / the I5 single-frame fallback, never stale accumulation)
    /// and transitions `UNDEFINED` → `GENERAL`, satisfying the seed's layout assumption AND making the
    /// clear visible to the first `COMPUTE` read.
    ///
    /// # Placement (H2(a))
    ///
    /// Called from [`Self::create`] (the tuple build), NOT a boot-only one-shot: `sync_gbuffer`'s
    /// resize path rebuilds targets through `create`, so a resize RE-clears the fresh pool.
    ///
    /// DEGRADES to `None` (leak-safe, temporal off ⇒ byte-identical) on any build / encoder / submit /
    /// fence failure, like [`Self::build_denoise_ring`].
    #[cfg(feature = "hwrt")]
    fn build_and_clear_shadow_temporal_hist(
        ctx: &VulkanContext,
        extent: VkExtent2D,
    ) -> Option<[VulkanTexture; FRAMES_IN_FLIGHT]> {
        let pool = Self::build_denoise_ring(ctx, extent, Self::create_shadow_temporal_hist_image)?;

        match Self::boot_clear_shadow_temporal_hist(ctx, &pool) {
            Ok(()) => Some(pool),
            Err(_) => {
                // Degrade to None (opt-in, no dependents). The boot-clear submit (if it ran) faulted
                // — drain the device so no in-flight clear still references the pool before destroy.
                let _ = RhiDevice::wait_idle(ctx);
                // SAFETY: each pool texture was created on `ctx` in `build_denoise_ring`; the device is
                // drained above ⇒ no submission references them; each is moved by value out of `pool`
                // ⇒ destroyed exactly once.
                for t in pool {
                    unsafe { RhiDevice::destroy_texture(ctx, t) };
                }
                None
            }
        }
    }

    /// Records + submits ONE encoder that boot-clears BOTH `pool` slots
    /// (`UNDEFINED` → `TRANSFER_DST_OPTIMAL` → clear → `GENERAL`) and fence-waits it — mirrors the
    /// `DdgiAtlas::boot_clear_and_transition` precedent (all barriers/clears for both slots in one
    /// encoder + one submit + one fence). The encoder + fence are setup-class transients torn down
    /// here on every path.
    #[cfg(feature = "hwrt")]
    fn boot_clear_shadow_temporal_hist(
        ctx: &VulkanContext,
        pool: &[VulkanTexture; FRAMES_IN_FLIGHT],
    ) -> Result<(), SwapchainError> {
        let mut encoder =
            RhiDevice::create_command_encoder(ctx).map_err(SwapchainError::DepthImage)?;
        let fence = match RhiDevice::create_fence(ctx, false) {
            Ok(f) => f,
            Err(e) => {
                // SAFETY: `encoder` was just created on `ctx`, never submitted; destroy once.
                unsafe { RhiDevice::destroy_command_encoder(ctx, encoder) };
                return Err(SwapchainError::DepthImage(e));
            }
        };

        // The full COLOR range of a 2D single-layer image (per `create_shadow_temporal_hist_image`).
        let range = ImageSubresourceRange {
            aspect: ImageAspect::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };

        let record = (|| -> Result<(), SwapchainError> {
            encoder.begin().map_err(SwapchainError::DepthImage)?;

            // Both slots: UNDEFINED → TRANSFER_DST_OPTIMAL (a fresh image has no prior contents, so
            // UNDEFINED discards — this is the clear destination).
            for tex in pool {
                encoder.image_barrier(&ImageBarrierDesc {
                    texture: tex,
                    src_stage: BarrierStage::TOP_OF_PIPE,
                    dst_stage: BarrierStage::TRANSFER,
                    src_access: BarrierAccess::NONE,
                    dst_access: BarrierAccess::TRANSFER_WRITE,
                    old_layout: ImageLayout::Undefined,
                    new_layout: ImageLayout::TransferDstOptimal,
                    range,
                });
            }

            // Clear each slot to R = vis = 1, G = conf = 0, B = depth = 0, A = 0. G = conf = 0 makes
            // the FIRST temporal read a `conf == 0` disocclusion reset (the I5 single-frame fallback).
            for tex in pool {
                encoder.clear_color_image(
                    tex,
                    ImageLayout::TransferDstOptimal,
                    [1.0, 0.0, 0.0, 0.0],
                    range,
                );
            }

            // Both slots: TRANSFER_DST_OPTIMAL → GENERAL, made available to COMPUTE_SHADER/SHADER_READ.
            // H2(b): the fence wait below signals the CPU only — the first temporal read must SEE the
            // clear, so the make-available targets COMPUTE/SHADER_READ; the GENERAL layout also
            // satisfies the ResId-14/16 framegraph seed's GENERAL-layout assumption.
            for tex in pool {
                encoder.image_barrier(&ImageBarrierDesc {
                    texture: tex,
                    src_stage: BarrierStage::TRANSFER,
                    dst_stage: BarrierStage::COMPUTE_SHADER,
                    src_access: BarrierAccess::TRANSFER_WRITE,
                    dst_access: BarrierAccess::SHADER_READ,
                    old_layout: ImageLayout::TransferDstOptimal,
                    new_layout: ImageLayout::General,
                    range,
                });
            }

            encoder.end().map_err(SwapchainError::DepthImage)?;
            let queue = ctx.rhi_queue();
            queue.submit(&encoder, &fence).map_err(SwapchainError::DepthImage)?;
            RhiDevice::wait_fence(ctx, &fence, u64::MAX).map_err(SwapchainError::DepthImage)?;
            Ok(())
        })();

        // Tear down the setup-class transients. The submit (if it ran) is fence-waited on the Ok path.
        // SAFETY: encoder/fence were created on `ctx`; the encoder's only submission (if any) is
        // fence-waited above on the Ok path (or never submitted / faulted on an error path), and each
        // is moved by value ⇒ destroyed exactly once.
        unsafe {
            RhiDevice::destroy_command_encoder(ctx, encoder);
            RhiDevice::destroy_fence(ctx, fence);
        }
        record
    }

    /// Allocates the depth + MRT G-buffer images at `extent` and writes the marcher
    /// vocabulary set + the present-sample set against them (ONCE). The caller
    /// ([`GBufferTargets::sync_gbuffer`]) destroys any prior targets + waits idle
    /// first; this only builds the new ones.
    ///
    /// On any partial failure every object created so far in this call is torn down
    /// in reverse order before the error returns (no leak on the error path), exactly
    /// like [`Scene::sync_depth`]'s build-before-teardown discipline.
    ///
    /// `profile` is the [`TargetsProfile`] rung R2 threads down from the caller (mirrors
    /// `aa_extent`'s explicit-parameter discipline) — asserted below against a fresh
    /// [`TargetsProfile::from_scene`] derivation (an O1-style parity check). As of rung R3 it may
    /// be [`TargetsProfile::DeferredSdfOnly`] too, but this fn does NOT yet branch its allocation
    /// on `profile` — see [`TargetsProfile`]'s doc for why (the fixed, once-per-extent vocab set
    /// layout) and the R3 rung report for the honest VRAM accounting.
    fn create(
        ctx: &VulkanContext,
        scene: &GBufferScene<'_>,
        extent: VkExtent2D,
        aa_extent: VkExtent2D,
        profile: TargetsProfile,
    ) -> Result<Self, SwapchainError> {
        // Multi-paradigm render-path plan, rung R2: the threaded `profile` must match what
        // `scene.resolved_render_path` would derive directly — the SAME "declare/record can
        // never diverge" discipline `path_has_raster`/`path_has_marcher` enforce in
        // `graph_bridge.rs`/`gbuffer.rs` (W1).
        debug_assert_eq!(
            profile,
            TargetsProfile::from_scene(scene),
            "invariant: the threaded TargetsProfile must match scene.resolved_render_path"
        );

        // Anti-aliasing campaign O1: `scene.aa` (FXAA), `scene.smaa` (SMAA), `scene.ssaa`
        // (SSAA), `scene.taa` (TAA) are mutually exclusive by construction (the `scene()` call
        // site arms at most one) — an explicit, zero-release-cost invariant check.
        debug_assert!(
            [
                scene.aa.is_some(),
                scene.smaa.is_some(),
                scene.ssaa.is_some(),
                scene.taa.is_some()
            ]
            .into_iter()
            .filter(|&armed| armed)
            .count()
                <= 1,
            "invariant: scene.aa, scene.smaa, scene.ssaa, scene.taa are mutually exclusive"
        );

        // Multi-paradigm render-path plan, rung R4b-b: built ONLY under `ForwardMesh`, at the TOP
        // of `create` (before the deferred body's sub-bundle builds), so an early failure here
        // has nothing else to tear down yet. The deferred body below then runs UNCONDITIONALLY
        // (Option 2 — "full + additive `ForwardTargets`", see [`Self::forward`]'s doc) — a
        // `ForwardMesh` profile pays the full Deferred allocation too; VRAM minimization for
        // Forward is a follow-up (see this rung's report).
        // Multi-paradigm render-path plan, rung R8: built ONLY under `VbMesh`, BEFORE `forward`
        // (nothing else has been built yet, so a failure here needs no teardown — the SAME
        // "first fallible thing, `?` is safe" reasoning `forward`'s own build below relies on).
        let vb = if matches!(profile, TargetsProfile::VbMesh) {
            Some(VbTargets::build(ctx, extent)?)
        } else {
            None
        };

        // VB-P2 classification plan, rung P2a (dark infra): built ONLY under `VbMesh`, right
        // after `vb` (the SAME gate, the SAME "sibling" placement `VbClassifyTargets`'s doc
        // describes) — nothing else besides `vb` has been built yet, so a failure here only
        // needs to tear down `vb`.
        let vb_classify = if matches!(profile, TargetsProfile::VbMesh) {
            match VbClassifyTargets::build(ctx, extent) {
                Ok(v) => Some(v),
                Err(e) => {
                    // SAFETY: `vb` (if built, under `VbMesh`) was created on `ctx` above,
                    // referenced by no submission; destroyed once on this edge.
                    if let Some(v) = vb {
                        unsafe { v.destroy(ctx) };
                    }
                    return Err(e);
                }
            }
        } else {
            None
        };

        // `forward` is built under EITHER `ForwardMesh` OR `VbMesh` — VB REUSES `ForwardTargets`
        // verbatim for its depth ring + Set-1 shadow set (`VbTargets`'s doc). Explicit `match`
        // (not `?`) because a failure here, under `VbMesh`, must first tear down the ALREADY-BUILT
        // `vb_classify`/`vb` above.
        let forward = if matches!(profile, TargetsProfile::ForwardMesh | TargetsProfile::VbMesh) {
            match ForwardTargets::build(ctx, scene, extent) {
                Ok(f) => Some(f),
                Err(e) => {
                    // SAFETY: `vb_classify`/`vb` (if built, under `VbMesh`) were created on
                    // `ctx` above, referenced by no submission; each destroyed once on this
                    // edge, reverse acquisition (`vb_classify` then `vb`).
                    unsafe {
                        if let Some(vc) = vb_classify {
                            vc.destroy(ctx);
                        }
                        if let Some(v) = vb {
                            v.destroy(ctx);
                        }
                    }
                    return Err(e);
                }
            }
        } else {
            None
        };

        // === Sub-bundle builds (order-preserving — see the `CoreImages` / `DeferredSets` docs). ===
        // Each `build` drains its OWN partials on failure; the orchestrator tears down the
        // (fully-built) earlier bundles in reverse acquisition order — the cross-bundle O(n²)
        // teardown-ladder collapse. The SUCCESSFUL create ORDER is preserved EXACTLY: core images →
        // shadow-vis images → SSAO à-trous ring images → deferred sets → (hwrt) denoise sets → SSAO
        // à-trous sets → temporal images → mv set → temporal sets, so the render stays
        // byte-identical.
        let core = CoreImages::build(ctx, extent)?;

        #[cfg(feature = "hwrt")]
        let shadow_vis_imgs = match ShadowVisImages::build(ctx, extent) {
            Ok(v) => v,
            Err(e) => {
                // SAFETY: `core` was built above on `ctx`, referenced by no submission; destroyed once.
                unsafe { core.destroy(ctx) };
                return Err(e);
            }
        };

        // The SSAO à-trous denoise chain's two interior ping-pong ring images. UNCONDITIONAL (both
        // feature legs — SOFTWARE, NOT `hwrt`-gated); built right after `shadow_vis_imgs` so its
        // own Err arm destroys shadow-vis (hwrt) + core, mirroring the existing image-stage error
        // weave. `None` on a device lacking `R16_UNORM` storage (the DDGI/shadow-denoise degrade).
        let ssao_atrous_imgs = match SsaoAtrousImages::build(ctx, extent) {
            Ok(v) => v,
            Err(e) => {
                // SAFETY: the shadow-vis images (hwrt) + `core` were built above on `ctx`,
                // referenced by no submission; each destroyed exactly once, reverse acquisition
                // (shadow-vis → core).
                unsafe {
                    #[cfg(feature = "hwrt")]
                    if let Some(v) = shadow_vis_imgs {
                        v.destroy(ctx);
                    }
                    core.destroy(ctx);
                }
                return Err(e);
            }
        };

        // Anti-aliasing campaign: the aa_out image ring, built ONLY when ANY of `scene.aa`
        // (FXAA) / `scene.smaa` (SMAA) / `scene.ssaa` (SSAA) / `scene.taa` (TAA) is armed —
        // `None` is the 0%-gate (no image, no fxaa_set/smaa/downsample sets, present samples
        // `lit`). Built after the SSAO à-trous ring images (so its own Err arm destroys those +
        // shadow-vis + core, mirroring the existing image-stage error weave). Sized to
        // `aa_extent` — NATIVE under SSAA (`present_extent`, i.e. `extent`, is 2× there), `==
        // extent` for Off/Fxaa/Smaa/Taa (byte-identical sizing to before SSAA existed). TAA's
        // resolve writes `aa_out` directly (no dedicated FXAA/SMAA-style INPUT set — see
        // `taa_hist` below).
        let aa_armed = scene.aa.is_some()
            || scene.smaa.is_some()
            || scene.ssaa.is_some()
            || scene.taa.is_some();
        let aa_imgs: Option<AaImages> = if aa_armed {
            match AaImages::build(ctx, aa_extent) {
                Ok(a) => Some(a),
                Err(e) => {
                    // SAFETY: the SSAO à-trous ring images + the shadow-vis images (hwrt) + `core`
                    // were built above on `ctx`, referenced by no submission; each destroyed
                    // exactly once, reverse acquisition (ssao_atrous_imgs → shadow-vis → core).
                    unsafe {
                        if let Some(s) = ssao_atrous_imgs {
                            s.destroy(ctx);
                        }
                        #[cfg(feature = "hwrt")]
                        if let Some(v) = shadow_vis_imgs {
                            v.destroy(ctx);
                        }
                        core.destroy(ctx);
                    }
                    return Err(e);
                }
            }
        } else {
            None
        };
        debug_assert_eq!(
            aa_armed,
            aa_imgs.is_some(),
            "invariant: aa_imgs must arm/disarm in lockstep with (scene.aa || scene.smaa || scene.ssaa || scene.taa)"
        );
        // SSAA-armed only: `aa_out`'s dims must equal the native `aa_extent`, not
        // `present_extent` (`extent`) — this is the crux invariant that keeps the present-blit's
        // unchanged 1:1 crop from sampling a 2× (top-left-quarter-cropped) image.
        debug_assert!(
            scene.ssaa.is_none() || aa_imgs.is_some(),
            "invariant: scene.ssaa armed implies aa_imgs is built at aa_extent"
        );

        // Anti-aliasing Stage 2: the SMAA `edges`/`weights` image rings, built ONLY when
        // `scene.smaa` is armed — built AFTER `aa_imgs` so its own Err arm destroys aa_imgs +
        // shadow-vis + core, mirroring the existing image-stage error weave.
        let smaa_imgs: Option<SmaaImages> = if scene.smaa.is_some() {
            match SmaaImages::build(ctx, extent) {
                Ok(s) => Some(s),
                Err(e) => {
                    // SAFETY: `aa_imgs` + the SSAO à-trous ring images + the shadow-vis images
                    // (hwrt) + `core` were built above on `ctx`, referenced by no submission; each
                    // destroyed exactly once, reverse acquisition (aa_imgs → ssao_atrous_imgs →
                    // shadow-vis → core).
                    unsafe {
                        if let Some(a) = aa_imgs {
                            a.destroy(ctx);
                        }
                        if let Some(s) = ssao_atrous_imgs {
                            s.destroy(ctx);
                        }
                        #[cfg(feature = "hwrt")]
                        if let Some(v) = shadow_vis_imgs {
                            v.destroy(ctx);
                        }
                        core.destroy(ctx);
                    }
                    return Err(e);
                }
            }
        } else {
            None
        };
        debug_assert_eq!(
            scene.smaa.is_some(),
            smaa_imgs.is_some(),
            "invariant: smaa_imgs must arm/disarm in lockstep with scene.smaa"
        );

        // The L1 froxel buffers (or the light-table placeholder when L1 is off) — computed ONCE and
        // shared with the deferred-set builder AND the hwrt denoise/temporal set builders below.
        let cluster_grid_buf = scene.cluster_grid.unwrap_or(scene.light_table);
        let light_index_buf = scene.light_index.unwrap_or(scene.light_table);

        let deferred = match DeferredSets::build(
            ctx,
            scene,
            &core,
            cluster_grid_buf,
            light_index_buf,
            aa_imgs.as_ref().map(|a| &a.aa_out),
            smaa_imgs.as_ref(),
            forward.as_ref(),
            vb.as_ref(),
            vb_classify.as_ref(),
        ) {
            Ok(s) => s,
            Err(e) => {
                // SAFETY: `smaa_imgs` + `aa_imgs` + the SSAO à-trous ring images + the shadow-vis
                // images (hwrt) + `core` were built above on `ctx`, referenced by no submission;
                // each destroyed exactly once, reverse acquisition (smaa_imgs → aa_imgs →
                // ssao_atrous_imgs → shadow-vis → core).
                unsafe {
                    if let Some(s) = smaa_imgs {
                        s.destroy(ctx);
                    }
                    if let Some(a) = aa_imgs {
                        a.destroy(ctx);
                    }
                    if let Some(s) = ssao_atrous_imgs {
                        s.destroy(ctx);
                    }
                    #[cfg(feature = "hwrt")]
                    if let Some(v) = shadow_vis_imgs {
                        v.destroy(ctx);
                    }
                    core.destroy(ctx);
                }
                return Err(e);
            }
        };

        // HW-RT rung 3a: the spatial-denoise descriptor sets + the à-trous edge-stop UBO ring.
        // Built ONLY when the scene wires `scene.shadow` (the step-7 gate; the host keeps it `None`
        // this rung, so this is a `None` no-op on EVERY current frame → byte-identical). Requires the
        // `shadow_vis`/`shadow_vis2` rings (device `shadow_denoise_storage_ok()`). The builder cleans
        // up its OWN partial allocations on internal failure and returns the `VulkanError`; the outer
        // `?`-arm then drains every ring/set already built above (reverse acquisition) before
        // returning. `None` when the activation / targets are absent.
        #[cfg(feature = "hwrt")]
        let (
            shadow_vis_resolve_set,
            shadow_denoised_resolve_set,
            shadow_atrous_sets,
            shadow_denoise_ubo,
        ) = match Self::build_shadow_denoise_sets(
            ctx,
            scene,
            &core.albedo,
            &core.normal,
            &core.material,
            &core.lit,
            &core.viewt,
            &core.ssao,
            shadow_vis_imgs.as_ref().map(|v| &v.shadow_vis),
            shadow_vis_imgs.as_ref().map(|v| &v.shadow_vis2),
            cluster_grid_buf,
            light_index_buf,
        ) {
            Ok(Some(sets)) => (
                Some(sets.vis_resolve),
                Some(sets.denoised_resolve),
                Some(sets.atrous),
                Some(sets.ubo),
            ),
            Ok(None) => (None, None, None, None),
            Err(e) => {
                // SAFETY: the deferred sets + `smaa_imgs` + `aa_imgs` + the SSAO à-trous ring
                // images + the shadow-vis images + `core` were built above on `ctx`, referenced
                // by no submission; each is destroyed exactly once, in reverse acquisition order
                // (deferred sets → smaa_imgs → aa_imgs → ssao_atrous_imgs → shadow-vis → core).
                // `build_shadow_denoise_sets` already drained its OWN partial allocations before
                // returning `Err`.
                unsafe {
                    deferred.destroy(ctx);
                    if let Some(s) = smaa_imgs {
                        s.destroy(ctx);
                    }
                    if let Some(a) = aa_imgs {
                        a.destroy(ctx);
                    }
                    if let Some(s) = ssao_atrous_imgs {
                        s.destroy(ctx);
                    }
                    if let Some(v) = shadow_vis_imgs {
                        v.destroy(ctx);
                    }
                    core.destroy(ctx);
                }
                return Err(SwapchainError::DepthImage(e));
            }
        };

        // The SSAO à-trous denoise chain's FIVE role-keyed descriptor sets. UNCONDITIONAL (both
        // feature legs — SOFTWARE, NOT `hwrt`-gated), built right after the (hwrt) shadow denoise
        // sets so its own Err arm destroys deferred + smaa_imgs + aa_imgs + ssao_atrous_imgs +
        // shadow-vis (hwrt) + core, mirroring the existing set-stage error weave. DECOUPLED from
        // `scene.ssao` (see `build_ssao_atrous_sets`'s doc) — `None` when the boot pipelines /
        // ring images are absent.
        let ssao_atrous_sets = match Self::build_ssao_atrous_sets(
            ctx,
            scene,
            &core.viewt,
            &core.ssao,
            ssao_atrous_imgs.as_ref().map(|r| &r.ssao_ring_a),
            ssao_atrous_imgs.as_ref().map(|r| &r.ssao_ring_b),
        ) {
            Ok(v) => v,
            Err(e) => {
                // SAFETY: the deferred sets + `smaa_imgs` + `aa_imgs` + the SSAO à-trous ring
                // images + the shadow-vis images (hwrt) + `core` were built above on `ctx`,
                // referenced by no submission; each destroyed exactly once, reverse acquisition
                // (deferred sets → smaa_imgs → aa_imgs → ssao_atrous_imgs → shadow-vis → core).
                // `build_ssao_atrous_sets` already drained its OWN partial allocations before
                // returning `Err`.
                unsafe {
                    deferred.destroy(ctx);
                    if let Some(s) = smaa_imgs {
                        s.destroy(ctx);
                    }
                    if let Some(a) = aa_imgs {
                        a.destroy(ctx);
                    }
                    if let Some(s) = ssao_atrous_imgs {
                        s.destroy(ctx);
                    }
                    #[cfg(feature = "hwrt")]
                    if let Some(v) = shadow_vis_imgs {
                        v.destroy(ctx);
                    }
                    core.destroy(ctx);
                }
                return Err(SwapchainError::DepthImage(e));
            }
        };

        // Flatten the image + set bundles into the original local names so the remaining (infallible)
        // hwrt tail below + the `Self` construction stay byte-identical.
        let CoreImages { depth, albedo, normal, material, lit, viewt, ssao, pbr } = core;
        let aa_out: Option<[VulkanTexture; FRAMES_IN_FLIGHT]> = aa_imgs.map(|a| a.aa_out);
        let (smaa_edges, smaa_weights) = match smaa_imgs {
            Some(SmaaImages { edges, weights }) => (Some(edges), Some(weights)),
            None => (None, None),
        };
        let (ssao_ring_a, ssao_ring_b) = match ssao_atrous_imgs {
            Some(SsaoAtrousImages { ssao_ring_a, ssao_ring_b }) => (Some(ssao_ring_a), Some(ssao_ring_b)),
            None => (None, None),
        };
        let (
            ssao_atrous_read8_set,
            ssao_atrous_interior_from0_set,
            ssao_atrous_interior_from1_set,
            ssao_atrous_write8_from0_set,
            ssao_atrous_write8_from1_set,
        ) = match ssao_atrous_sets {
            Some(SsaoAtrousSets {
                read8,
                interior_from0,
                interior_from1,
                write8_from0,
                write8_from1,
            }) => (
                Some(read8),
                Some(interior_from0),
                Some(interior_from1),
                Some(write8_from0),
                Some(write8_from1),
            ),
            None => (None, None, None, None, None),
        };
        #[cfg(feature = "hwrt")]
        let (shadow_vis, shadow_vis2) = match shadow_vis_imgs {
            Some(ShadowVisImages { shadow_vis, shadow_vis2 }) => {
                (Some(shadow_vis), Some(shadow_vis2))
            }
            None => (None, None),
        };
        let DeferredSets {
            vocab_set,
            resolve_set,
            cull_set,
            vb_cull_set,
            ssao_set,
            viewt_from_depth_set,
            ddgi_update_set,
            present_set,
            sdf_forward_set,
            vb_set0,
            vb_set0_tex,
            vb_set0_froxel,
            vb_set0_tex_froxel,
            viewt_from_vb_depth_set,
            #[cfg(feature = "hwrt")]
            resolve_set_hwrt,
            fxaa_set,
            smaa_edge_set,
            smaa_weight_set,
            smaa_blend_set,
            downsample_set,
        } = deferred;

        // HW-RT Rung 3b: the three temporal denoise target rings (motion_vec RG16F,
        // shadow_temporal_hist RGBA16, temporal_out RG16), built LAST — after every fallible
        // descriptor set — and DEGRADE-TO-NONE on any create failure (leak-safe, opt-in). Because
        // nothing fallible follows, they need NO teardown weaving into the ladder above. Gated on
        // the SAME `shadow_denoise_storage_ok()` probe as `shadow_vis`/`shadow_vis2`. No pass names
        // them this step (steps 5-6 add the MV producers + the temporal pass) — allocated-but-
        // unused, byte-identical render.
        #[cfg(feature = "hwrt")]
        let (motion_vec, shadow_temporal_hist, temporal_out) =
            if ctx.device_caps().shadow_denoise_storage_ok() {
                (
                    Self::build_denoise_ring(ctx, extent, Self::create_motion_vec_image),
                    Self::build_and_clear_shadow_temporal_hist(ctx, extent),
                    Self::build_denoise_ring(ctx, extent, Self::create_temporal_out_image),
                )
            } else {
                (None, None, None)
            };

        // HW-RT Rung 3b step 5b: the SDF motion-vector VIS-variant resolve set ring. Built LAST
        // (after `motion_vec`, which it binds @23) and DEGRADE-TO-NONE (opt-in, no dependents) —
        // like the temporal target rings, it needs no teardown weaving. `None` on every OFF path
        // (temporal off / spatial off / non-storage device) ⇒ byte-identical.
        #[cfg(feature = "hwrt")]
        let shadow_vis_mv_resolve_set = Self::build_shadow_vis_mv_resolve_set(
            ctx,
            scene,
            &albedo,
            &normal,
            &material,
            &lit,
            &viewt,
            &ssao,
            shadow_vis.as_ref(),
            motion_vec.as_ref(),
            cluster_grid_buf,
            light_index_buf,
        );

        // HW-RT Rung 3b step 6: the temporal reproject UBO ring + the 8-binding temporal set + the
        // DENOISED-temporal resolve set. Built LAST (after every fallible set + the temporal target
        // rings it binds) and DEGRADE-TO-NONE (opt-in, no dependents) — like the VIS-MV set, it needs
        // no teardown weaving. `None` on every OFF path (denoise off / temporal off / non-storage /
        // non-RT device) ⇒ byte-identical.
        #[cfg(feature = "hwrt")]
        let (temporal_shadow_ubo, shadow_temporal_set, shadow_temporal_denoised_resolve_set) =
            match Self::build_shadow_temporal_sets(
                ctx,
                scene,
                &albedo,
                &normal,
                &material,
                &lit,
                &viewt,
                &ssao,
                shadow_vis.as_ref(),
                shadow_vis2.as_ref(),
                motion_vec.as_ref(),
                shadow_temporal_hist.as_ref(),
                temporal_out.as_ref(),
                cluster_grid_buf,
                light_index_buf,
            ) {
                Some(sets) => (Some(sets.ubo), Some(sets.temporal), Some(sets.denoised)),
                None => (None, None, None),
            };

        // Anti-aliasing Stage 4 (TAA W4/W5, the M2 fix): the `taa_hist` cross-frame history ring,
        // built LAST (after every fallible descriptor set) — DEGRADE-TO-NONE on any create/clear
        // failure (leak-safe, opt-in), mirroring the hwrt temporal rings' shape above
        // (UNCONDITIONAL here — TAA is not hwrt-gated). Gated on `scene.taa.is_some()`, so this is
        // a `None` no-op on every other `AaMode` ⇒ byte-identical. Sized to `aa_extent` (==
        // `extent` for Taa — native resolution, like Fxaa/Smaa). `build_and_clear_taa_hist`
        // boot-clears BOTH physical slots `UNDEFINED → GENERAL` (mirrors
        // `build_and_clear_shadow_temporal_hist`'s C1/H2 discipline — the framegraph's `taa_hist`
        // seed assumes a REAL GENERAL layout, not a fresh UNDEFINED image, on the first
        // cross-frame read).
        let taa_hist: Option<[VulkanTexture; FRAMES_IN_FLIGHT]> =
            if scene.taa.is_some() { Self::build_and_clear_taa_hist(ctx, aa_extent) } else { None };

        // TAA rung T3: `GBufferScene::rcas` is a pure post-process over the resolve's OWN
        // output — it can never be armed without the resolve itself (`GBufferScene::taa`)
        // being armed too (the scene-assembly seam, `boyko_app::gpu_scene`, ANDs the two at the
        // arm site). This debug_assert makes that lockstep explicit at the ONE place every
        // `GBufferScene` flows through before its targets are built.
        debug_assert!(
            scene.rcas.is_none() || scene.taa.is_some(),
            "invariant: GBufferScene::rcas armed implies GBufferScene::taa armed (RCAS runs \
             post-TAA-resolve, never standalone)"
        );
        // TAA rung T3: the RCAS-intermediate `taa_resolved` ring, built right after `taa_hist`
        // (both TAA-Stage-4-adjacent) so it exists BEFORE `build_taa_resolve_set` below needs to
        // pick which ring the resolve's `gAaOut` @4 binds this frame. Gated on `scene.rcas.
        // is_some()` — `None` (the 0%-gate, `SharpenMode::None`) never calls this.
        let taa_resolved: Option<[VulkanTexture; FRAMES_IN_FLIGHT]> =
            if scene.rcas.is_some() { Self::build_taa_resolved_ring(ctx, aa_extent) } else { None };

        // Anti-aliasing Stage 4 (TAA W5): the resolve's own tunables + DEDICATED `MotionCam` UBO
        // rings + the 8-binding resolve set. Built LAST (after `taa_hist`/`aa_out`, which it
        // binds) and DEGRADE-TO-NONE (opt-in, no dependents) — like the hwrt temporal sets, it
        // needs no teardown weaving. `None` on the OFF path (TAA off, or `taa_hist`/`aa_out`
        // failed to allocate) ⇒ byte-identical.
        //
        // TAA rung T3: `gAaOut` @4 is RE-POINTED at `taa_resolved` instead of `aa_out` whenever
        // RCAS is armed (`record_rcas` then reads `taa_resolved` and writes the FINAL sharpened
        // result into `aa_out` itself) — `resolve_gaaout_target` picks the right ring;
        // `build_taa_resolve_set`'s own body is untouched (its `aa_out` param just binds
        // whichever slice it is handed). `SharpenMode::None` (`scene.rcas.is_none()`) keeps
        // `resolve_gaaout_target == aa_out.as_ref()` — byte-identical to the pre-RCAS resolve.
        let resolve_gaaout_target =
            if scene.rcas.is_some() { taa_resolved.as_ref() } else { aa_out.as_ref() };
        let (taa_ubo, taa_motion_cam_ubo, taa_resolve_set) =
            match Self::build_taa_resolve_set(ctx, scene, &lit, &viewt, taa_hist.as_ref(), resolve_gaaout_target) {
                Some(sets) => (Some(sets.taa_ubo), Some(sets.motion_cam_ubo), Some(sets.set)),
                None => (None, None, None),
            };

        // TAA rung T3: the RCAS descriptor set, built LAST (after `taa_resolve_set`, which
        // repoints the resolve's own `gAaOut`) and DEGRADE-TO-NONE (opt-in, no dependents) — like
        // the resolve set above, it needs no teardown weaving. `None` on the OFF path (RCAS off,
        // or `taa_resolved`/`aa_out` failed to allocate) ⇒ byte-identical.
        let rcas_set = Self::build_rcas_set(ctx, scene, taa_resolved.as_ref(), aa_out.as_ref());

        // Rung R9b (docs/R9-VB-SPLIT-PLAN.md §4): the VB split's `thin_normal` ring — built in
        // the leak-safe DEGRADE-TO-NONE tail (the `taa_hist` discipline: each slot drains on a
        // partial failure, no teardown weaving) and gated on the BOOT-frozen
        // `mesh_geo_shade_split` (`None` on every fused/non-VB boot — the 0%-gate). UNLIKE the
        // opt-in AA rings, an armed split genuinely NEEDS this ring — allocation failure here
        // (OOM-class: RGBA8 STORAGE support is boot-fail-fast-checked like the G-buffer images)
        // surfaces at `record_vb`'s `.expect` instead of a silent degrade.
        let thin_normal: Option<[VulkanTexture; FRAMES_IN_FLIGHT]> =
            if scene.resolved_render_path.mesh_geo_shade_split {
                let mut slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
                    [const { None }; FRAMES_IN_FLIGHT];
                let mut ok = true;
                for slot in slots.iter_mut() {
                    match Self::create_gbuffer_image(ctx, extent, ImageUsage::STORAGE) {
                        Ok(t) => *slot = Some(t),
                        Err(_) => {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    Some(slots.map(|s| {
                        s.expect("invariant: every thin_normal ring slot built before here")
                    }))
                } else {
                    // SAFETY: the partial slots were created above on `ctx`, referenced by no
                    // submission; each destroyed exactly once.
                    unsafe {
                        for s in slots.iter_mut() {
                            if let Some(t) = s.take() {
                                RhiDevice::destroy_texture(ctx, t);
                            }
                        }
                    }
                    eprintln!(
                        "boyko_rhi_vulkan: thin_normal ring allocation failed under an armed \
                         VB split (OOM-class) — record_vb will refuse the frame"
                    );
                    None
                }
            } else {
                None
            };

        // Rung R9b: the three split descriptor rings — leak-safe DEGRADE-TO-NONE tail builders
        // (any per-slot create failure drains the partial ring and yields `None`; `record_vb`
        // `.expect`s them under an armed split — the thin_normal discipline above).
        let vb_geo_aux_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> = match (
            scene.resolved_render_path.mesh_geo_shade_split,
            thin_normal.as_ref(),
            scene.vb_geo_aux_layout,
        ) {
            (true, Some(tn), Some(layout)) => {
                let mut slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
                    [const { None }; FRAMES_IN_FLIGHT];
                let mut ok = true;
                for (i, dst) in slots.iter_mut().enumerate() {
                    // Rung R9d: bind the REAL `motion_vec`/`MotionCam` sources when the device
                    // stably carries them (RT + storage — device capability, independent of
                    // whether temporal is the currently-configured mode: a harmless "just in
                    // case" real bind, mirroring every other stably-built-but-maybe-unarmed set
                    // in this file); otherwise the R9b same-type inert placeholder.
                    #[cfg(feature = "hwrt")]
                    let (motion_entry, motion_cam_entry) =
                        match (motion_vec.as_ref(), scene.motion_cam_ubo_ring) {
                            (Some(mv), Some(mc)) => (
                                BindGroupEntry::StorageImage { texture: &mv[i] },
                                BindGroupEntry::UniformBuffer { buffer: &mc[i] },
                            ),
                            _ => (
                                BindGroupEntry::StorageImage { texture: &tn[i] },
                                BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[i] },
                            ),
                        };
                    #[cfg(not(feature = "hwrt"))]
                    let (motion_entry, motion_cam_entry) = (
                        BindGroupEntry::StorageImage { texture: &tn[i] },
                        BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[i] },
                    );
                    let entries = [
                        BindGroupEntry::StorageImage { texture: &tn[i] },
                        motion_entry,
                        motion_cam_entry,
                    ];
                    match RhiDevice::create_bind_group(ctx, &BindGroupDesc::<Vulkan> { layout, entries: &entries }) {
                        Ok(g) => *dst = Some(g),
                        Err(_) => {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    Some(slots.map(|s| s.expect("invariant: every vb_geo_aux slot built")))
                } else {
                    // SAFETY: partial groups created above on `ctx`, unreferenced; each
                    // destroyed once.
                    unsafe {
                        for s in slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                    }
                    eprintln!("boyko_rhi_vulkan: vb_geo_aux_set build failed — record_vb will refuse the frame");
                    None
                }
            }
            _ => None,
        };
        let vb_ssao_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> = match (
            scene.resolved_render_path.mesh_geo_shade_split,
            thin_normal.as_ref(),
            scene.vb_ssao_layout,
        ) {
            (true, Some(tn), Some(layout)) => {
                let mut slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
                    [const { None }; FRAMES_IN_FLIGHT];
                let mut ok = true;
                for (i, dst) in slots.iter_mut().enumerate() {
                    let entries = [
                        BindGroupEntry::StorageImage { texture: &tn[i] },
                        BindGroupEntry::StorageImage { texture: &viewt[i] },
                        BindGroupEntry::StorageImage { texture: &ssao[i] },
                        BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[i] },
                    ];
                    match RhiDevice::create_bind_group(ctx, &BindGroupDesc::<Vulkan> { layout, entries: &entries }) {
                        Ok(g) => *dst = Some(g),
                        Err(_) => {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    Some(slots.map(|s| s.expect("invariant: every vb_ssao slot built")))
                } else {
                    // SAFETY: as above.
                    unsafe {
                        for s in slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                    }
                    eprintln!("boyko_rhi_vulkan: vb_ssao_set build failed — record_vb will refuse the frame");
                    None
                }
            }
            _ => None,
        };
        let vb_split_set1: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> =
            match (scene.resolved_render_path.mesh_geo_shade_split, scene.vb_split_layout1) {
                (true, Some(layout)) => {
                    let mut slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
                        [const { None }; FRAMES_IN_FLIGHT];
                    let mut ok = true;
                    for (i, dst) in slots.iter_mut().enumerate() {
                        let base = [
                            BindGroupEntry::CombinedImage {
                                texture: scene.csm_cascade_texture,
                                sampler: scene.csm_compare_sampler,
                            },
                            BindGroupEntry::UniformBuffer { buffer: &scene.csm_cascade_ring[i] },
                            BindGroupEntry::CombinedImage {
                                texture: scene.shadow_atlas_texture,
                                sampler: scene.shadow_atlas_sampler,
                            },
                            BindGroupEntry::UniformBuffer { buffer: scene.shadow_atlas_ubo },
                            BindGroupEntry::StorageImage { texture: &ssao[i] },
                            BindGroupEntry::CombinedImage {
                                texture: scene.ddgi_irr_texture,
                                sampler: scene.ddgi_irr_sampler,
                            },
                            BindGroupEntry::CombinedImage {
                                texture: scene.ddgi_depth_texture,
                                sampler: scene.ddgi_depth_sampler,
                            },
                            BindGroupEntry::UniformBuffer { buffer: scene.ddgi_grid_ubo },
                        ];
                        // Rung R9d: the hwrt-only @8 `gShadowVis` entry — the layout's 9th
                        // binding exists whenever `feature = "hwrt"`, so the SET must always
                        // fill it, even on a frame where the hwrt shade variant is never bound
                        // (the software `vb_shade_split_pipeline` never statically references
                        // this slot). The STABLE-signal selection `build_shadow_temporal_sets`'s
                        // own DENOISED-temporal set uses (`scene.shadow_denoise_enabled`/
                        // `scene.temporal_enabled`/`scene.shadow_denoise_final_is_vis2` — NOT
                        // `temporal_out.is_some()` alone: `shadow_vis`/`temporal_out` are
                        // allocated TOGETHER on the SAME device probe, so an allocation-only
                        // check would always prefer `temporal_out` even under Spatial-only
                        // config). Falls all the way to `ssao[i]` as a never-selected placeholder
                        // when the denoise config is off entirely (same-set-already-bound image —
                        // harmless, mirrors the R9b `vb_geo_aux_set` placeholder-binding idiom).
                        #[cfg(feature = "hwrt")]
                        let entries: [BindGroupEntry<'_, Vulkan>; 9] = {
                            let final_ring = if scene.shadow_denoise_final_is_vis2 {
                                shadow_vis2.as_ref()
                            } else {
                                shadow_vis.as_ref()
                            };
                            let ninth = if scene.shadow_denoise_enabled
                                && scene.temporal_enabled
                                && let Some(t) = temporal_out.as_ref()
                            {
                                BindGroupEntry::StorageImage { texture: &t[i] }
                            } else if scene.shadow_denoise_enabled
                                && let Some(r) = final_ring
                            {
                                BindGroupEntry::StorageImage { texture: &r[i] }
                            } else {
                                BindGroupEntry::StorageImage { texture: &ssao[i] }
                            };
                            let mut chained = base.into_iter().chain(core::iter::once(ninth));
                            core::array::from_fn(|_| {
                                chained.next().expect("invariant: exactly 9 entries")
                            })
                        };
                        #[cfg(not(feature = "hwrt"))]
                        let entries = base;
                        match RhiDevice::create_bind_group(ctx, &BindGroupDesc::<Vulkan> { layout, entries: &entries }) {
                            Ok(g) => *dst = Some(g),
                            Err(_) => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if ok {
                        Some(slots.map(|s| s.expect("invariant: every vb_split_set1 slot built")))
                    } else {
                        // SAFETY: as above.
                        unsafe {
                            for s in slots.iter_mut() {
                                if let Some(g) = s.take() {
                                    RhiDevice::destroy_bind_group(ctx, g);
                                }
                            }
                        }
                        eprintln!("boyko_rhi_vulkan: vb_split_set1 build failed — record_vb will refuse the frame");
                        None
                    }
                }
                _ => None,
            };

        // Rung R9d: the VB hardware shadow chain's own descriptor sets — built in the leak-safe
        // tail (after every other VB split set) and DEGRADE-TO-NONE on any internal failure
        // (opt-in, no dependents: `record_vb` GRACEFULLY skips the hwrt shadow chain that frame
        // when it finds `None`, the deferred `record_gbuffer`'s own precedent — UNLIKE
        // `vb_geo_aux_set`/`vb_ssao_set`/`vb_split_set1`, which are the split's own mandatory
        // core and `.expect()`-panic if missing).
        #[cfg(feature = "hwrt")]
        let vb_shadow_vis_set = Self::build_vb_shadow_vis_set(
            ctx,
            scene,
            thin_normal.as_ref(),
            &viewt,
            shadow_vis.as_ref(),
        );
        #[cfg(feature = "hwrt")]
        let vb_shadow_atrous_sets = Self::build_vb_shadow_atrous_sets(
            ctx,
            scene,
            thin_normal.as_ref(),
            &viewt,
            shadow_vis.as_ref(),
            shadow_vis2.as_ref(),
            shadow_denoise_ubo.as_ref(),
        );
        #[cfg(feature = "hwrt")]
        let vb_shadow_temporal_set = Self::build_vb_shadow_temporal_set(
            ctx,
            scene,
            &viewt,
            shadow_vis.as_ref(),
            shadow_vis2.as_ref(),
            motion_vec.as_ref(),
            shadow_temporal_hist.as_ref(),
            temporal_out.as_ref(),
            temporal_shadow_ubo.as_ref(),
        );

        Ok(Self {
            depth,
            albedo,
            normal,
            material,
            lit,
            viewt,
            ssao,
            pbr,
            #[cfg(feature = "hwrt")]
            shadow_vis,
            #[cfg(feature = "hwrt")]
            shadow_vis2,
            #[cfg(feature = "hwrt")]
            motion_vec,
            #[cfg(feature = "hwrt")]
            shadow_temporal_hist,
            #[cfg(feature = "hwrt")]
            temporal_out,
            vocab_set,
            resolve_set,
            #[cfg(feature = "hwrt")]
            resolve_set_hwrt,
            cull_set,
            vb_cull_set,
            ssao_set,
            viewt_from_depth_set,
            viewt_from_vb_depth_set,
            ssao_ring_a,
            ssao_ring_b,
            thin_normal,
            vb_geo_aux_set,
            vb_ssao_set,
            vb_split_set1,
            #[cfg(feature = "hwrt")]
            vb_shadow_vis_set,
            #[cfg(feature = "hwrt")]
            vb_shadow_atrous_sets,
            #[cfg(feature = "hwrt")]
            vb_shadow_temporal_set,
            ssao_atrous_read8_set,
            ssao_atrous_interior_from0_set,
            ssao_atrous_interior_from1_set,
            ssao_atrous_write8_from0_set,
            ssao_atrous_write8_from1_set,
            aa_out,
            fxaa_set,
            smaa_edges,
            smaa_weights,
            smaa_edge_set,
            smaa_weight_set,
            smaa_blend_set,
            downsample_set,
            taa_hist,
            taa_ubo,
            taa_motion_cam_ubo,
            taa_resolve_set,
            taa_resolved,
            rcas_set,
            aa_arm: AaArm::from_scene(scene),
            #[cfg(feature = "hwrt")]
            shadow_vis_resolve_set,
            #[cfg(feature = "hwrt")]
            shadow_denoised_resolve_set,
            #[cfg(feature = "hwrt")]
            shadow_atrous_sets,
            #[cfg(feature = "hwrt")]
            shadow_denoise_ubo,
            #[cfg(feature = "hwrt")]
            shadow_vis_mv_resolve_set,
            #[cfg(feature = "hwrt")]
            temporal_shadow_ubo,
            #[cfg(feature = "hwrt")]
            shadow_temporal_set,
            #[cfg(feature = "hwrt")]
            shadow_temporal_denoised_resolve_set,
            ddgi_update_set,
            present_set,
            sdf_forward_set,
            vb_set0,
            vb_set0_tex,
            vb_set0_froxel,
            vb_set0_tex_froxel,
            forward,
            vb,
            vb_classify,
            extent,
        })
    }

    /// Ensures the G-buffer images + descriptor sets exist and match `extent`,
    /// (re)building them through `ctx` when absent (first frame), stale (resize), OR an
    /// anti-aliasing arm-state change (`AaArm::from_scene(scene)` flips — Off↔Fxaa↔Smaa↔Ssaa)
    /// — a genuine, fence-safe live AA toggle riding the SAME rebuild path a resize uses. The
    /// vocabulary + present descriptor sets are re-written here — and ONLY here — so the
    /// per-frame recorder records no `vkUpdateDescriptorSets`.
    ///
    /// `aa_extent` is the `aa_out` size — NATIVE under SSAA (`extent`, i.e. `present_extent`,
    /// is 2× there), `== extent` for Off/Fxaa/Smaa. The resync predicate compares only
    /// `extent`/`aa_arm` (NOT a separate `aa_extent` compare): `aa_arm` already flips on every
    /// Off↔Ssaa transition (`AaArm::from_scene`), and both extents are boot-fixed together, so
    /// `aa_extent` cannot change without `aa_arm` changing too — a redundant size-compare would
    /// add a stored field for no additional coverage.
    ///
    /// The caller ([`Renderer::render_gbuffer_frame`]) calls this only after
    /// fence-waiting the frame slot, so no in-flight frame still references the old
    /// targets; on a REPLACE this additionally waits the device idle (a sibling
    /// frame-in-flight slot may still reference the old images — the same
    /// belt-and-braces [`Scene::sync_depth`] uses) before destroying them.
    ///
    /// `profile` is rung R2's [`TargetsProfile`] seam — see its doc. Threaded straight through
    /// to [`Self::create`] on a (re)build; unread on the fast-path `extent`/`aa_arm` match
    /// above (a profile-only change with no extent/AA change cannot occur today — R3 revisits
    /// this once a live path/legs toggle exists, which Decision 1 forbids in any case).
    pub(crate) fn sync_gbuffer(
        targets: &mut Option<Self>,
        ctx: &VulkanContext,
        scene: &GBufferScene<'_>,
        extent: VkExtent2D,
        aa_extent: VkExtent2D,
        profile: TargetsProfile,
    ) -> Result<(), SwapchainError> {
        if let Some(t) = targets.as_ref()
            && t.extent.width == extent.width
            && t.extent.height == extent.height
            && t.aa_arm == AaArm::from_scene(scene)
        {
            return Ok(());
        }

        // A (re)create is rare (first frame + resize). When REPLACING, wait idle first:
        // a sibling frame-in-flight slot may still reference the old targets, and the
        // caller only fence-waited THIS slot. The first-ever create needs no idle.
        if targets.is_some() {
            // SAFETY: `ctx` is live; waiting idle guarantees every prior submission —
            // including a sibling-slot frame still referencing the old targets — has
            // completed before they are destroyed below.
            unsafe { (ctx.device_fns().device_wait_idle)(ctx.device()) };
        }

        // Build the new targets BEFORE tearing down the old ones, so an allocation
        // failure leaves the previous (still-valid) targets in place.
        let fresh = Self::create(ctx, scene, extent, aa_extent, profile)?;

        // Asset-streaming plan F7 §5 (C1, review O1): a SECONDARY self-consistency net —
        // every material-bearing ring `create` just built must be enumerated by
        // `material_set_rings`, else a repointed material-table grow would silently miss
        // one (a UAF the moment its buffer is later freed). `expected_material_ring_count`
        // reads `fresh`'s own `Option` fields directly (not a re-derived arming predicate),
        // so this cannot spuriously fire on a device where a ring degraded to `None` for a
        // reason the predicate wouldn't see. The PRIMARY exhaustiveness guarantees are
        // `material_set_rings`'s co-location with `resolve_software_entries` and the
        // headless C1 repoint-counter test (F7 §12) — this debug_assert is a cheap backstop.
        #[cfg(feature = "hwrt")]
        {
            let ring_count = fresh.material_set_rings().count();
            debug_assert!(
                ring_count >= MATERIAL_SET_RING_COUNT_MIN,
                "invariant: at least the vocab + resolve material rings must always exist"
            );
            debug_assert_eq!(
                ring_count,
                fresh.expected_material_ring_count(),
                "invariant (F7 C1): material_set_rings() must enumerate EXACTLY every \
                 material-bearing ring create() built — a new resolve variant was added \
                 without adding its ring to material_set_rings()"
            );

            // Asset-streaming plan F7-hwrt (task#11): the AS-repoint counterpart of the
            // material-ring check above — every AS-bearing ring `create` just built must
            // be enumerated by `tlas_accel_sets`, else a TLAS grow's repoint would
            // silently miss one (a UAF the moment the superseded TLAS is later freed).
            debug_assert_eq!(
                fresh.tlas_accel_sets().count(),
                fresh.expected_tlas_accel_ring_count(),
                "invariant (task#11): tlas_accel_sets() must enumerate EXACTLY every \
                 AS-bearing ring create() built — a new resolve variant was added \
                 without adding its ring to tlas_accel_sets()"
            );
        }

        if let Some(old) = targets.take() {
            // SAFETY: the new targets were built above; the device was waited idle (a
            // replace), so no submission references the old targets; `destroy` consumes
            // them exactly once on the live `ctx` they were created on.
            unsafe { old.destroy(ctx) };
        }

        *targets = Some(fresh);
        Ok(())
    }

    /// Tears down the G-buffer targets (descriptor sets first, then the images),
    /// consuming `self`. The caller MUST have made the device idle (the renderer's
    /// `Drop` waits idle, or `sync_gbuffer` waits idle on a replace) so no submission
    /// still references them.
    ///
    /// # Safety
    ///
    /// `ctx` is the live context the targets were created on; no GPU work referencing
    /// them is in flight; each is destroyed exactly once (the by-value `self`).
    unsafe fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: per the contract `ctx` is live and nothing references these
        // resources; each was created on `ctx` and is destroyed exactly once, in
        // reverse acquisition order (sets → images). The vocab, resolve & present RINGS
        // each have `FRAMES_IN_FLIGHT` slots; the cull & SSAO RINGS + the single DDGI
        // update set are `Option`-guarded (present only when L1 / SSAO / the DDGI update
        // pass were wired); the seven render-target image RINGS each have `FRAMES_IN_FLIGHT`
        // slots — every slot of every ring (and the single set) is drained. Rung 3a (`hwrt`) adds
        // the two `Option`-guarded shadow-vis image RINGS, drained before ssao (reverse acquisition).
        unsafe {
            // TAA rung T3: the RCAS descriptor set — acquired LAST (after `taa_resolved`/
            // `aa_out`/`taa_resolve_set`), so destroyed FIRST here (before everything it reads
            // from). `Option`-guarded (`None` unless `scene.rcas` was armed).
            if let Some(s) = self.rcas_set {
                for g in s {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            // Anti-aliasing Stage 4 (TAA W5): the resolve set + its two UBO rings — acquired LAST
            // (after `taa_hist`, in `build_taa_resolve_set`), so destroyed FIRST here (before the
            // history ring they bind). `Option`-guarded (`None` on every non-TAA `AaArm`).
            if let Some(s) = self.taa_resolve_set {
                for g in s {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            if let Some(r) = self.taa_motion_cam_ubo {
                for b in r {
                    RhiDevice::destroy_buffer(ctx, b);
                }
            }
            if let Some(r) = self.taa_ubo {
                for b in r {
                    RhiDevice::destroy_buffer(ctx, b);
                }
            }
            // TAA rung T3: the `taa_resolved` RCAS-intermediate ring — acquired AFTER `taa_hist`
            // but BEFORE the resolve set/UBOs above (which bind it), so destroyed AFTER those
            // (above) and BEFORE `taa_hist` (below) — reverse acquisition. `Option`-guarded
            // (`None` unless `scene.rcas` was armed, or its ring failed to allocate).
            if let Some(r) = self.taa_resolved {
                for t in r {
                    RhiDevice::destroy_texture(ctx, t);
                }
            }
            // Anti-aliasing Stage 4 (TAA W4): the `taa_hist` history ring — the LAST IMAGE
            // `create()` builds (after every fallible descriptor set), so destroyed FIRST among
            // the images (reverse acquisition; the resolve set/UBOs above bind it, so they are
            // destroyed first overall). `Option`-guarded (`None` on every non-TAA `AaArm`).
            if let Some(r) = self.taa_hist {
                for t in r {
                    RhiDevice::destroy_texture(ctx, t);
                }
            }
            // Rung 3a: the spatial-denoise sets + UBO ring (LAST-acquired, so destroyed FIRST in
            // reverse acquisition). Each `Option`-guarded (present only on the denoise ON path — the
            // host keeps `scene.shadow == None` this rung, so these are `None` on every current
            // frame). Order within: à-trous sets → DENOISED resolve → VIS resolve → UBO ring.
            // HW-RT Rung 3b step 6: the temporal reproject sets + UBO ring — acquired LAST (after the
            // temporal target rings), so destroyed FIRST here (before the textures they bind). Order
            // within: DENOISED-temporal set → temporal set → UBO ring. `Option`-guarded (present only
            // on the temporal path).
            #[cfg(feature = "hwrt")]
            if let Some(dr) = self.shadow_temporal_denoised_resolve_set {
                for g in dr {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            #[cfg(feature = "hwrt")]
            if let Some(ts) = self.shadow_temporal_set {
                for g in ts {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            #[cfg(feature = "hwrt")]
            if let Some(ubo) = self.temporal_shadow_ubo {
                for b in ubo {
                    RhiDevice::destroy_buffer(ctx, b);
                }
            }
            // HW-RT Rung 3b step 5b: the SDF motion-vector VIS resolve set RING — acquired LAST among
            // the denoise sets (after `motion_vec`), so destroyed FIRST here (before the textures it
            // references). `Option`-guarded (present only on the `mode == Both` temporal path).
            #[cfg(feature = "hwrt")]
            if let Some(mvr) = self.shadow_vis_mv_resolve_set {
                for g in mvr {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            #[cfg(feature = "hwrt")]
            if let Some(sets) = self.shadow_atrous_sets {
                for lvl in sets {
                    for g in lvl {
                        RhiDevice::destroy_bind_group(ctx, g);
                    }
                }
            }
            #[cfg(feature = "hwrt")]
            if let Some(dr) = self.shadow_denoised_resolve_set {
                for g in dr {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            #[cfg(feature = "hwrt")]
            if let Some(vr) = self.shadow_vis_resolve_set {
                for g in vr {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            #[cfg(feature = "hwrt")]
            if let Some(ubo) = self.shadow_denoise_ubo {
                for b in ubo {
                    RhiDevice::destroy_buffer(ctx, b);
                }
            }
            // The SSAO à-trous denoise chain's FIVE role-keyed descriptor sets — LAST-acquired (in
            // `build_ssao_atrous_sets`, after `deferred`), so destroyed FIRST here (before
            // `deferred`'s `ssao_set`, which binds the SAME `ssao`/`viewt` images but is an
            // independent set — order between the two does not matter functionally, only that
            // both precede the images below). UNCONDITIONAL (both feature legs — SOFTWARE, NOT
            // `hwrt`-gated). Each `Option`-guarded (`None` on a device lacking `R16_UNORM`
            // storage, or when the boot pipelines were never wired).
            if let Some(s) = self.ssao_atrous_write8_from1_set {
                for g in s {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            if let Some(s) = self.ssao_atrous_write8_from0_set {
                for g in s {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            if let Some(s) = self.ssao_atrous_interior_from1_set {
                for g in s {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            if let Some(s) = self.ssao_atrous_interior_from0_set {
                for g in s {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            if let Some(s) = self.ssao_atrous_read8_set {
                for g in s {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            // Rung R9d: the VB hardware shadow chain's own descriptor sets — LAST-acquired (after
            // `vb_split_set1`), so destroyed FIRST here.
            #[cfg(feature = "hwrt")]
            if let Some(s) = self.vb_shadow_temporal_set {
                for g in s {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            #[cfg(feature = "hwrt")]
            if let Some(sets) = self.vb_shadow_atrous_sets {
                for lvl in sets {
                    for g in lvl {
                        RhiDevice::destroy_bind_group(ctx, g);
                    }
                }
            }
            #[cfg(feature = "hwrt")]
            if let Some(s) = self.vb_shadow_vis_set {
                for g in s {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            // Rung R9b: the three split descriptor rings (built in the tail — destroyed first).
            for ring in [self.vb_split_set1, self.vb_ssao_set, self.vb_geo_aux_set]
                .into_iter()
                .flatten()
            {
                for g in ring {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            // The deferred descriptor SETS (resolve-hwrt → sdf-forward-march → present →
            // ddgi-update → viewt-from-depth → ssao → cull → resolve → vocab), via the
            // `DeferredSets` bundle's reverse-acquisition teardown — the SAME order +
            // `Option`-guards the old flat teardown used.
            DeferredSets {
                vocab_set: self.vocab_set,
                resolve_set: self.resolve_set,
                cull_set: self.cull_set,
                vb_cull_set: self.vb_cull_set,
                ssao_set: self.ssao_set,
                viewt_from_depth_set: self.viewt_from_depth_set,
                ddgi_update_set: self.ddgi_update_set,
                present_set: self.present_set,
                sdf_forward_set: self.sdf_forward_set,
                vb_set0: self.vb_set0,
                vb_set0_tex: self.vb_set0_tex,
                vb_set0_froxel: self.vb_set0_froxel,
                vb_set0_tex_froxel: self.vb_set0_tex_froxel,
                viewt_from_vb_depth_set: self.viewt_from_vb_depth_set,
                #[cfg(feature = "hwrt")]
                resolve_set_hwrt: self.resolve_set_hwrt,
                fxaa_set: self.fxaa_set,
                smaa_edge_set: self.smaa_edge_set,
                smaa_weight_set: self.smaa_weight_set,
                smaa_blend_set: self.smaa_blend_set,
                downsample_set: self.downsample_set,
            }
            .destroy(ctx);
            // HW-RT Rung 3b: the three temporal denoise target RINGS (motion_vec / hist /
            // temporal_out), built LAST so destroyed FIRST in reverse-acquisition order. `Option`-
            // guarded (degrade-to-None on an unsupported device), each a
            // `[VulkanTexture; FRAMES_IN_FLIGHT]` ring.
            #[cfg(feature = "hwrt")]
            if let Some(r) = self.temporal_out {
                for t in r {
                    RhiDevice::destroy_texture(ctx, t);
                }
            }
            #[cfg(feature = "hwrt")]
            if let Some(r) = self.shadow_temporal_hist {
                for t in r {
                    RhiDevice::destroy_texture(ctx, t);
                }
            }
            #[cfg(feature = "hwrt")]
            if let Some(r) = self.motion_vec {
                for t in r {
                    RhiDevice::destroy_texture(ctx, t);
                }
            }
            // Rung 3a: the two shadow-vis image RINGS (built AFTER ssao, so destroyed BEFORE it in
            // reverse-acquisition order). `Option`-guarded (`None` on a device lacking RG8/RG16
            // storage), each a `[VulkanTexture; FRAMES_IN_FLIGHT]` ring.
            #[cfg(feature = "hwrt")]
            if let Some(r) = self.shadow_vis2 {
                for t in r {
                    RhiDevice::destroy_texture(ctx, t);
                }
            }
            #[cfg(feature = "hwrt")]
            if let Some(r) = self.shadow_vis {
                for t in r {
                    RhiDevice::destroy_texture(ctx, t);
                }
            }
            // Rung R9b: the VB split's thin_normal ring (destroyed with the other
            // `Option`-guarded aux rings; built in the leak-safe tail — reverse acquisition).
            if let Some(r) = self.thin_normal {
                for t in r {
                    RhiDevice::destroy_texture(ctx, t);
                }
            }
            // The SSAO à-trous denoise chain's two interior ping-pong image RINGS — grouped with
            // the shadow-vis images above (both denoise ring pairs, `Option`-guarded on a device
            // storage-format probe). UNCONDITIONAL (both feature legs — SOFTWARE, NOT `hwrt`-gated).
            if let Some(r) = self.ssao_ring_b {
                for t in r {
                    RhiDevice::destroy_texture(ctx, t);
                }
            }
            if let Some(r) = self.ssao_ring_a {
                for t in r {
                    RhiDevice::destroy_texture(ctx, t);
                }
            }
            // Anti-aliasing Stage 2: the smaa_weights then smaa_edges image RINGS (built AFTER
            // aa_imgs, so destroyed BEFORE aa_out here — reverse acquisition). `Option`-guarded
            // (`None` when SMAA was off).
            if let Some(r) = self.smaa_weights {
                for t in r {
                    RhiDevice::destroy_texture(ctx, t);
                }
            }
            if let Some(r) = self.smaa_edges {
                for t in r {
                    RhiDevice::destroy_texture(ctx, t);
                }
            }
            // Anti-aliasing Stage 1: the aa_out image RING (built AFTER shadow-vis, so destroyed
            // BEFORE core here — the same reverse-acquisition placement as shadow-vis).
            // `Option`-guarded (`None` when AA was off).
            if let Some(r) = self.aa_out {
                for t in r {
                    RhiDevice::destroy_texture(ctx, t);
                }
            }
            // The eight always-present G-buffer image RINGS (pbr → depth), via the `CoreImages`
            // bundle's reverse-acquisition teardown.
            CoreImages {
                depth: self.depth,
                albedo: self.albedo,
                normal: self.normal,
                material: self.material,
                lit: self.lit,
                viewt: self.viewt,
                ssao: self.ssao,
                pbr: self.pbr,
            }
            .destroy(ctx);
            // Multi-paradigm render-path plan, rung R4b-b: `ForwardTargets` was built FIRST in
            // `create` (before `core`), so it is destroyed LAST here (reverse acquisition).
            // `Option`-guarded (`None` under every `Deferred*` profile).
            if let Some(f) = self.forward {
                f.destroy(ctx);
            }
            // VB-P2 classification plan, rung P2a: `VbClassifyTargets` was built right after
            // `vb` (before `forward`), so it is destroyed BEFORE `vb` here (reverse
            // acquisition). `Option`-guarded (`None` under every non-`VbMesh` profile).
            if let Some(vc) = self.vb_classify {
                vc.destroy(ctx);
            }
            // Multi-paradigm render-path plan, rung R8: `VbTargets` was built FIRST in `create`
            // (before `forward`/`core`), so it is destroyed LAST here (reverse acquisition).
            // `Option`-guarded (`None` under every non-`VbMesh` profile).
            if let Some(v) = self.vb {
                v.destroy(ctx);
            }
        }
    }
}

/// The renderer-side state for the on-screen Render-P1c G-buffer frame: the
/// per-extent [`GBufferTargets`], created lazily on the first
/// [`Renderer::render_gbuffer_frame`] and reallocated on resize. A caller drives one
/// across the present loop (analogous to a [`Scene`], but image-based).
///
/// Held by value; torn down through [`GBufferFrame::destroy`] AFTER the renderer is
/// dropped (the renderer's `Drop` waits the device idle).
pub struct GBufferFrame {
    /// The per-extent depth + MRT G-buffer + descriptor sets, `None` until the first
    /// frame syncs them.
    pub(crate) targets: Option<GBufferTargets>,
}

impl Default for GBufferFrame {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl GBufferFrame {
    /// Creates the on-screen G-buffer frame state with no targets yet (the first
    /// [`Renderer::render_gbuffer_frame`] allocates them sized to the swapchain
    /// extent).
    #[inline]
    pub fn new() -> Self {
        Self { targets: None }
    }

    /// Asset-streaming plan F7 §11.3 (Q3): `true` once the first
    /// [`Renderer::render_gbuffer_frame`] has synced [`Self::targets`]. A material-table
    /// grow before targets exist is safe either way (no set references the old buffer
    /// yet, and the first sync binds the new one), but the runner gates the rebind on
    /// this so `MaterialTable::rebind_pending`
    /// is only cleared once a repoint actually happened.
    #[inline]
    pub fn targets_ready(&self) -> bool {
        self.targets.is_some()
    }

    /// Asset-streaming plan F7 §5/§6: repoints the material-table binding of EVERY
    /// material-bearing descriptor set for `fenced_slot` to `buf`
    /// ([`GBufferTargets::material_set_rings`], one in-place `vkUpdateDescriptorSets`
    /// each). A no-op until [`Self::targets_ready`] (frame 0, before the first sync).
    ///
    /// # Safety
    ///
    /// `fenced_slot`'s in-flight fence must already be waited THIS frame (via
    /// [`Renderer::wait_frame_in_flight`]) — none of its descriptor sets is command-
    /// buffer-pending (VUID-vkUpdateDescriptorSets-None-03047). `ctx` must be the live
    /// context every set + `buf` were created on; `buf` must outlive every submit that
    /// could read it.
    pub unsafe fn repoint_material_table(
        &self,
        ctx: &VulkanContext,
        fenced_slot: usize,
        buf: &BoundBuffer,
    ) {
        let Some(targets) = self.targets.as_ref() else {
            return;
        };
        for (ring, binding) in targets.material_set_rings() {
            // SAFETY: `fenced_slot`'s set is non-pending (this fn's caller contract
            // above); `ctx` is the live context both the set and `buf` were created on.
            unsafe { crate::rhi_impl::rebind_storage_buffer(ctx, &ring[fenced_slot], binding, buf) };
        }
    }

    /// Asset-streaming plan F7-hwrt (task#11): repoints the AS binding of EVERY
    /// AS-bearing descriptor set for `fenced_slot` to `accel`
    /// ([`GBufferTargets::tlas_accel_sets`], one in-place `vkUpdateDescriptorSets` each) —
    /// the acceleration-structure counterpart of [`Self::repoint_material_table`], fired
    /// when the per-slot TLAS grows (a NEW `VkAccelerationStructureKHR` handle replaces
    /// the old one). A no-op until [`Self::targets_ready`] (frame 0, before the first
    /// sync) and a no-op on a device/config with no HWRT resolve rings
    /// (`tlas_accel_sets` then yields nothing).
    ///
    /// # Safety
    ///
    /// `fenced_slot`'s in-flight fence must already be waited THIS frame (via
    /// [`Renderer::wait_frame_in_flight`]) — none of its descriptor sets is command-
    /// buffer-pending (VUID-vkUpdateDescriptorSets-None-03047). `ctx` must be the live
    /// context every set + `accel` were created on; `accel` must outlive every submit
    /// that could reference it.
    #[cfg(feature = "hwrt")]
    pub unsafe fn repoint_tlas_accel(
        &self,
        ctx: &VulkanContext,
        fenced_slot: usize,
        accel: &BoundAccelStruct,
    ) {
        let Some(targets) = self.targets.as_ref() else {
            return;
        };
        for (ring, binding) in targets.tlas_accel_sets() {
            // SAFETY: `fenced_slot`'s set is non-pending (this fn's caller contract
            // above); `ctx` is the live context both the set and `accel` were created on.
            unsafe { crate::rhi_impl::rebind_accel_struct(ctx, &ring[fenced_slot], binding, accel) };
        }
    }

    /// HW-RT rung 3a: the fenced à-trous edge-stop UBO ring slot the host memcpys
    /// [`ResolvedShadowDenoise`](boyko_render's `ResolvedShadowDenoise`) into each frame
    /// (the per-level à-trous sets bind `shadow_denoise_ubo[fi]` @4). Returns `None` when
    /// the targets are not yet synced (frame 0, before the first
    /// [`Renderer::render_gbuffer_frame`]) OR the device lacks
    /// [`shadow_denoise_storage_ok`](crate::device::DeviceCaps::shadow_denoise_storage_ok)
    /// (the `shadow_denoise_ubo` ring was never minted) — in both cases the denoise pass is
    /// not recorded, so the (absent) slot is never read. The slot is per-FIF ringed, so the
    /// caller writes the FENCED slot (`token.slot()`) under the same WAR discipline as the
    /// other host-written rings.
    #[cfg(feature = "hwrt")]
    #[inline]
    pub fn shadow_denoise_ubo_slot(&self, slot: usize) -> Option<&BoundBuffer> {
        self.targets
            .as_ref()
            .and_then(|t| t.shadow_denoise_ubo.as_ref())
            .map(|ring| &ring[slot])
    }

    /// HW-RT Rung 3b step 6: the fenced temporal reproject UBO ring slot the host memcpys
    /// [`ResolvedTemporalShadow`](boyko_render's `ResolvedTemporalShadow`) into each frame (the
    /// temporal set binds `temporal_shadow_ubo[fi]` @6). Returns `None` when the targets are not yet
    /// synced (frame 0) OR the temporal denoise is not armed (the `temporal_shadow_ubo` ring was never
    /// minted) — in both cases the temporal pass is not recorded, so the (absent) slot is never read.
    /// Per-FIF ringed under the same WAR discipline as [`Self::shadow_denoise_ubo_slot`].
    #[cfg(feature = "hwrt")]
    #[inline]
    pub fn temporal_shadow_ubo_slot(&self, slot: usize) -> Option<&BoundBuffer> {
        self.targets
            .as_ref()
            .and_then(|t| t.temporal_shadow_ubo.as_ref())
            .map(|ring| &ring[slot])
    }

    /// Anti-aliasing Stage 4 (TAA W5): the fenced TAA tunables UBO ring slot the host memcpys
    /// boyko_render's `ResolvedTaa` into each frame (the resolve set binds
    /// `taa_ubo[fi]` @5). Returns `None` when the targets are not yet synced (frame 0) OR TAA is
    /// not armed (the `taa_ubo` ring was never minted) — in both cases the resolve is not
    /// recorded, so the (absent) slot is never read. Per-FIF ringed under the same WAR discipline
    /// as `Self::shadow_denoise_ubo_slot`. NOT `hwrt`-gated.
    #[inline]
    pub fn taa_ubo_slot(&self, slot: usize) -> Option<&BoundBuffer> {
        self.targets.as_ref().and_then(|t| t.taa_ubo.as_ref()).map(|ring| &ring[slot])
    }

    /// Anti-aliasing Stage 4 (TAA W5): the fenced DEDICATED `MotionCam` UBO ring slot the host
    /// memcpys boyko_render's `MotionCam` into each frame (the resolve set binds
    /// `taa_motion_cam_ubo[fi]` @7) — SEPARATE from the hwrt mesh-shadow `motion_cam_ubo` (see
    /// `TaaActivation`'s doc). Returns `None` when the targets are not yet synced OR TAA is not
    /// armed. NOT `hwrt`-gated.
    #[inline]
    pub fn taa_motion_cam_ubo_slot(&self, slot: usize) -> Option<&BoundBuffer> {
        self.targets.as_ref().and_then(|t| t.taa_motion_cam_ubo.as_ref()).map(|ring| &ring[slot])
    }

    /// Tears down the per-extent G-buffer targets through `ctx`, consuming `self`. The
    /// caller MUST have made the device idle (dropped the [`Renderer`], whose `Drop`
    /// waits idle) so no submission still references them.
    ///
    /// # Safety
    ///
    /// `ctx` is the live context the targets were created on; no GPU work referencing
    /// them is in flight (the caller `wait_idle`'d / dropped the renderer); they are
    /// destroyed exactly once (the by-value `self`).
    pub unsafe fn destroy(self, ctx: &VulkanContext) {
        if let Some(targets) = self.targets {
            // SAFETY: per this fn's contract `ctx` is live and nothing references the
            // targets; they are destroyed exactly once (moved out of `self`).
            unsafe { targets.destroy(ctx) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texture::MAX_TEXTURE_LAYERS;

    /// A device-inert `VulkanTexture` — every handle field is `VK_NULL_HANDLE`
    /// (the `dummy_mesh_gpu`/`BoundBuffer::NULL` idiom `asset_streaming_f5_
    /// validation.rs` established), so building a full `GBufferTargets` value in
    /// a CPU unit test never calls a Vulkan function: `GBufferTargets` has no
    /// `Drop` impl (teardown is the explicit `unsafe fn destroy(self, ctx)`
    /// above), so a fake instance just drops its plain handle fields harmlessly.
    fn null_texture() -> VulkanTexture {
        VulkanTexture {
            image: VkImage::NULL,
            view: VkImageView::NULL,
            memory: VkDeviceMemory::NULL,
            layer_views: [VkImageView::NULL; MAX_TEXTURE_LAYERS],
            active_layers: 1,
            array_view: VkImageView::NULL,
        }
    }

    fn null_bind_group() -> VulkanBindGroup {
        VulkanBindGroup { descriptor_pool: VkDescriptorPool::NULL, descriptor_set: VkDescriptorSet::NULL }
    }

    fn tex_ring() -> [VulkanTexture; FRAMES_IN_FLIGHT] {
        core::array::from_fn(|_| null_texture())
    }

    fn bg_ring() -> [VulkanBindGroup; FRAMES_IN_FLIGHT] {
        core::array::from_fn(|_| null_bind_group())
    }

    /// A non-hwrt `GBufferTargets`: only the 14 always-present fields exist on
    /// this build (every `shadow_*`/`motion_vec`/`temporal_*`/`resolve_set_hwrt`
    /// field is `#[cfg(feature = "hwrt")]`-gated out entirely, not merely `None`).
    #[cfg(not(feature = "hwrt"))]
    fn fake_targets() -> GBufferTargets {
        GBufferTargets {
            depth: tex_ring(),
            albedo: tex_ring(),
            normal: tex_ring(),
            material: tex_ring(),
            lit: tex_ring(),
            viewt: tex_ring(),
            ssao: tex_ring(),
            pbr: tex_ring(),
            vocab_set: bg_ring(),
            resolve_set: bg_ring(),
            cull_set: None,
            vb_cull_set: None,
            ssao_set: None,
            viewt_from_depth_set: None,
            viewt_from_vb_depth_set: None,
            ssao_ring_a: None,
            ssao_ring_b: None,
            thin_normal: None,
            vb_geo_aux_set: None,
            vb_ssao_set: None,
            vb_split_set1: None,
            #[cfg(feature = "hwrt")]
            vb_shadow_vis_set: None,
            #[cfg(feature = "hwrt")]
            vb_shadow_atrous_sets: None,
            #[cfg(feature = "hwrt")]
            vb_shadow_temporal_set: None,
            ssao_atrous_read8_set: None,
            ssao_atrous_interior_from0_set: None,
            ssao_atrous_interior_from1_set: None,
            ssao_atrous_write8_from0_set: None,
            ssao_atrous_write8_from1_set: None,
            aa_out: None,
            fxaa_set: None,
            smaa_edges: None,
            smaa_weights: None,
            smaa_edge_set: None,
            smaa_weight_set: None,
            smaa_blend_set: None,
            downsample_set: None,
            taa_hist: None,
            taa_ubo: None,
            taa_motion_cam_ubo: None,
            taa_resolve_set: None,
            taa_resolved: None,
            rcas_set: None,
            aa_arm: AaArm::Off,
            ddgi_update_set: None,
            present_set: bg_ring(),
            sdf_forward_set: None,
            vb_set0: None,
            vb_set0_tex: None,
            vb_set0_froxel: None,
            vb_set0_tex_froxel: None,
            forward: None,
            vb: None,
            vb_classify: None,
            extent: VkExtent2D::default(),
        }
    }

    /// Asset-streaming plan F7 C1: on a `not(hwrt)` build `material_set_rings()`
    /// always yields exactly the two always-present rings — no `Option`-guarded
    /// HWRT ring exists to enumerate on this build (see that fn's doc); this is
    /// the non-hwrt companion of the exhaustive hwrt combination test below
    /// (`expected_material_ring_count`/`MATERIAL_SET_RING_COUNT_MIN` are
    /// themselves `#[cfg(feature = "hwrt")]`-only, so there is nothing else to
    /// cross-check here).
    #[test]
    #[cfg(not(feature = "hwrt"))]
    fn material_set_rings_is_always_exactly_two_on_a_non_hwrt_build() {
        let targets = fake_targets();
        assert_eq!(
            targets.material_set_rings().count(),
            2,
            "a not(hwrt) build has only vocab_set + resolve_set to enumerate"
        );
    }

    /// A `GBufferTargets` with every ALWAYS-present field filled + the 5
    /// material-bearing `Option`-guarded HWRT resolve rings set per the caller's
    /// `bool`s (every OTHER hwrt-only field — `shadow_vis`/`motion_vec`/
    /// `shadow_atrous_sets`/the two UBO rings/`shadow_temporal_set` — stays
    /// `None`, since none of them is enumerated by `material_set_rings`).
    #[cfg(feature = "hwrt")]
    fn fake_targets(
        resolve_set_hwrt: bool,
        shadow_vis_resolve: bool,
        shadow_denoised_resolve: bool,
        shadow_vis_mv_resolve: bool,
        shadow_temporal_denoised_resolve: bool,
    ) -> GBufferTargets {
        GBufferTargets {
            depth: tex_ring(),
            albedo: tex_ring(),
            normal: tex_ring(),
            material: tex_ring(),
            lit: tex_ring(),
            viewt: tex_ring(),
            ssao: tex_ring(),
            pbr: tex_ring(),
            shadow_vis: None,
            shadow_vis2: None,
            motion_vec: None,
            shadow_temporal_hist: None,
            temporal_out: None,
            vocab_set: bg_ring(),
            resolve_set: bg_ring(),
            resolve_set_hwrt: resolve_set_hwrt.then(bg_ring),
            cull_set: None,
            vb_cull_set: None,
            ssao_set: None,
            viewt_from_depth_set: None,
            viewt_from_vb_depth_set: None,
            ssao_ring_a: None,
            ssao_ring_b: None,
            thin_normal: None,
            vb_geo_aux_set: None,
            vb_ssao_set: None,
            vb_split_set1: None,
            #[cfg(feature = "hwrt")]
            vb_shadow_vis_set: None,
            #[cfg(feature = "hwrt")]
            vb_shadow_atrous_sets: None,
            #[cfg(feature = "hwrt")]
            vb_shadow_temporal_set: None,
            ssao_atrous_read8_set: None,
            ssao_atrous_interior_from0_set: None,
            ssao_atrous_interior_from1_set: None,
            ssao_atrous_write8_from0_set: None,
            ssao_atrous_write8_from1_set: None,
            aa_out: None,
            fxaa_set: None,
            smaa_edges: None,
            smaa_weights: None,
            smaa_edge_set: None,
            smaa_weight_set: None,
            smaa_blend_set: None,
            downsample_set: None,
            taa_hist: None,
            taa_ubo: None,
            taa_motion_cam_ubo: None,
            taa_resolve_set: None,
            taa_resolved: None,
            rcas_set: None,
            aa_arm: AaArm::Off,
            shadow_vis_resolve_set: shadow_vis_resolve.then(bg_ring),
            shadow_denoised_resolve_set: shadow_denoised_resolve.then(bg_ring),
            shadow_atrous_sets: None,
            shadow_denoise_ubo: None,
            shadow_vis_mv_resolve_set: shadow_vis_mv_resolve.then(bg_ring),
            temporal_shadow_ubo: None,
            shadow_temporal_set: None,
            shadow_temporal_denoised_resolve_set: shadow_temporal_denoised_resolve.then(bg_ring),
            ddgi_update_set: None,
            present_set: bg_ring(),
            sdf_forward_set: None,
            vb_set0: None,
            vb_set0_tex: None,
            vb_set0_froxel: None,
            vb_set0_tex_froxel: None,
            forward: None,
            vb: None,
            vb_classify: None,
            extent: VkExtent2D::default(),
        }
    }

    /// Asset-streaming plan F7 C1 completeness (the UAF blocker's regression
    /// guard): `material_set_rings().count()` must equal
    /// `expected_material_ring_count()` — the SAME invariant `sync_gbuffer`'s
    /// debug_assert checks at every (re)create — across EVERY arming
    /// combination of the 5 `Option`-guarded HWRT resolve rings (2^5 = 32
    /// combinations), exhaustively. A combination where they diverge would mean
    /// a resolve variant's ring can silently escape `repoint_material_table`'s
    /// walk — the exact C1 UAF this rung fixed.
    #[test]
    #[cfg(feature = "hwrt")]
    fn material_set_rings_count_matches_expected_across_every_hwrt_arming_combination() {
        for mask in 0u32..32 {
            let flags =
                [mask & 1 != 0, mask & 2 != 0, mask & 4 != 0, mask & 8 != 0, mask & 16 != 0];
            let targets = fake_targets(flags[0], flags[1], flags[2], flags[3], flags[4]);

            let actual = targets.material_set_rings().count();
            let expected = targets.expected_material_ring_count();
            assert_eq!(
                actual, expected,
                "mask {mask:05b}: material_set_rings().count() ({actual}) must equal \
                 expected_material_ring_count() ({expected}) — a forgotten ring would \
                 silently escape repoint_material_table's walk (C1)"
            );

            let armed_count = flags.iter().filter(|&&f| f).count();
            assert_eq!(
                expected,
                MATERIAL_SET_RING_COUNT_MIN + armed_count,
                "mask {mask:05b}: expected_material_ring_count must be the 2 always-present \
                 rings plus exactly the armed optional rings"
            );
        }
    }

    /// The floor itself: even with every optional ring disarmed, at least the
    /// vocab + resolve rings must be enumerated.
    #[test]
    #[cfg(feature = "hwrt")]
    fn material_set_rings_never_drops_below_the_always_present_floor() {
        let targets = fake_targets(false, false, false, false, false);
        assert_eq!(targets.material_set_rings().count(), MATERIAL_SET_RING_COUNT_MIN);
    }

    /// Every optional ring armed: the count must reach the full 7-ring surface
    /// design §5 documents (2 always-present + 5 optional).
    #[test]
    #[cfg(feature = "hwrt")]
    fn material_set_rings_reaches_the_full_seven_ring_surface_when_everything_is_armed() {
        let targets = fake_targets(true, true, true, true, true);
        assert_eq!(targets.material_set_rings().count(), 7);
    }

    /// Textured-PBR T6a: `GBufferTargets::pbr` (the `gPbr` MRT-lane ring) exists on BOTH feature
    /// legs and is sized `FRAMES_IN_FLIGHT`, like every other core G-buffer ring.
    #[test]
    fn pbr_ring_is_present_and_frames_in_flight_sized() {
        #[cfg(not(feature = "hwrt"))]
        let targets = fake_targets();
        #[cfg(feature = "hwrt")]
        let targets = fake_targets(false, false, false, false, false);

        assert_eq!(targets.pbr.len(), FRAMES_IN_FLIGHT);
    }

    /// Textured-PBR T6a (the critic's C1 fix): the SOFTWARE resolve set's exact-fill grows to 20
    /// (19 shared + the SOFTWARE-ONLY `gPbr` @19) while `RESOLVE_SOFTWARE_BINDINGS` itself — the
    /// HWRT-family derivation base — stays 19, UNCHANGED.
    #[test]
    fn resolve_software_total_bindings_is_exact_fill_20() {
        assert_eq!(RESOLVE_SOFTWARE_BINDINGS, 19);
        assert_eq!(RESOLVE_SOFTWARE_TOTAL_BINDINGS, 20);
        assert_eq!(RESOLVE_SOFTWARE_TOTAL_BINDINGS, RESOLVE_SOFTWARE_BINDINGS + 1);
    }

    /// Textured-PBR T6a (the critic's C1 fix): every HWRT-family resolve binding count derived
    /// from `RESOLVE_SOFTWARE_BINDINGS` is UNCHANGED by the software-only `gPbr` append — the
    /// TLAS stays at binding 19, and the largest HWRT set (`RESOLVE_HWRT_VIS_MV_BINDINGS`) stays
    /// EXACTLY at the `MAX_BIND_GROUP_BINDINGS` cap (24), not 25 (which would panic the fixed
    /// `[VkDescriptorSetLayoutBinding; 24]`-class arrays the rhi_impl backend allocates).
    #[test]
    #[cfg(feature = "hwrt")]
    fn hwrt_resolve_binding_counts_unchanged_by_the_c1_fix() {
        assert_eq!(TLAS_ACCEL_BINDING, 19, "TLAS must stay at binding 19 (unshifted by gPbr)");
        assert_eq!(RESOLVE_HWRT_DENOISE_BINDINGS, 22);
        assert_eq!(RESOLVE_HWRT_VIS_MV_BINDINGS, 24, "must stay exactly at MAX_BIND_GROUP_BINDINGS");
    }
}

