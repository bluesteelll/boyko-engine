//! The per-extent on-screen G-buffer targets ([`GBufferTargets`]) + the
//! per-frame-in-flight ring ([`GBufferFrame`]) + `sync_gbuffer` (extent-change
//! recreate). Split out of the former monolithic `swapchain.rs` (audit W4).

use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, Format, ImageUsage, RhiDevice,
    TextureDesc, TextureDimension,
};
#[cfg(feature = "hwrt")]
use boyko_rhi::{BufferDesc, BufferUsage, MemoryLocation};
#[cfg(feature = "hwrt")]
use boyko_rhi::{
    ImageAspect, ImageBarrierDesc, ImageLayout, ImageSubresourceRange, RhiCommandEncoder, RhiQueue,
};
#[cfg(feature = "hwrt")]
use boyko_rhi::enums::{BarrierAccess, BarrierStage};

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
    /// allocated (the resolve descriptor interface is stable regardless of `ssao_mode`);
    /// transitioned UNDEFINED→GENERAL with `lit`/`viewt` and kept in GENERAL its whole life.
    /// No SSAO pass writes it yet (C2 adds that) — with `ssao_mode == 0` the resolve never
    /// reads it, so its undefined contents are irrelevant (the 0%-gate is the byte-identical
    /// PIXELS + command stream, which the always-allocate preserves). RINGED (see [`Self::depth`]).
    pub(crate) ssao: [VulkanTexture; FRAMES_IN_FLIGHT],
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
    /// are `None`, so @8/@9 bind the light table as a harmless valid placeholder (the resolve's
    /// `clusters_enabled` header gate never reads them on the OFF path). `gSsao` @11 is always
    /// bound; the resolve reads it only under `ssao_mode != 0` (0 every pre-P7 scene). @12/@13 =
    /// the CSM cascade combined-image + UBO; @14/@15 = the punctual shadow-atlas combined-image +
    /// UBO; @16/@17/@18 = the SDFDDGI probe irradiance + depth combined images + the `ResolvedDdgi`
    /// grid UBO (all bound-but-unread when their header gate is 0). The software set is EXACT-FILL
    /// at `RESOLVE_SOFTWARE_BINDINGS` (19), under the R2a-4a cap of `MAX_BIND_GROUP_BINDINGS` (20).
    /// NO per-frame update.
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
    /// The Render P7 SSAO descriptor set, written ONCE against [`SsaoActivation::layout`]
    /// (5 bindings: gNormal @0, gMaterial @1, gViewT @2 STORAGE images READ, the `ssao` out
    /// STORAGE image @3 WRITE, the camera UBO @4) — `None` when SSAO is off
    /// ([`GBufferScene::ssao`] is `None`). The recorder then skips the SSAO pass entirely (the
    /// 0%-gate, byte-identical command stream). NO per-frame update.
    ///
    /// A RING when `Some` (one per in-flight frame): slot `i` binds `scene.camera_ring[i]` @4 — the
    /// lock-free per-frame ring fix. The recorder selects `ssao_set[self.frame_index]`.
    pub(crate) ssao_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
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
    /// The extent the images were created at (so [`GBufferTargets::sync_gbuffer`] can
    /// detect a resize and reallocate).
    pub(crate) extent: VkExtent2D,
}

/// The G-buffer color format (albedo / normal / material): `R8G8B8A8_UNORM`, the
/// STORAGE-image store target the marcher writes (matches the P1b offscreen driver's
/// `GBUFFER_FORMAT`). The ALBEDO image is also `SAMPLED` (the present-blit) — never
/// stretched; presented 1:1 in the swapchain's top-left like [`SampledComposite`].
const GBUFFER_FORMAT: Format = Format::R8G8B8A8Unorm;

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
        // Lighting L1: the ClusterGrid @8 + LightIndexList @9 (resolve READS the pixel's froxel
        // slice when `clusters_enabled`); the light-table placeholder when L1 is off.
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
}

impl CoreImages {
    /// Allocates the seven always-present G-buffer image rings at `extent` in acquisition order
    /// (depth → albedo → normal → material → lit → viewt → ssao). On any ring's partial failure the
    /// slots already built in THAT ring are drained AND every fully-built prior ring is destroyed
    /// (reverse acquisition), so nothing leaks; the orchestrator has no partials to reason about
    /// beyond the bundles it built before this call.
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
        let mut lit_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in lit_slots.iter_mut() {
            match GBufferTargets::create_gbuffer_image(
                ctx,
                extent,
                ImageUsage::STORAGE | ImageUsage::SAMPLED | ImageUsage::TRANSFER_SRC,
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

        Ok(Self { depth, albedo, normal, material, lit, viewt, ssao })
    }

    /// Tears down the seven image rings in reverse acquisition order (ssao → depth), consuming
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

/// The per-extent deferred descriptor SETS bound ONCE against the [`CoreImages`] rings + `scene` (NO
/// per-frame update). Built as one bundle so [`GBufferTargets::create`]'s error ladder no longer
/// re-lists the image teardown at every set (the cross-bundle O(n²) collapse): [`Self::build`] drains
/// only the sets it built, and the orchestrator tears down the images. Acquisition order (matched by
/// [`Self::destroy`] in reverse): vocab → resolve → cull → ssao → ddgi-update → present → resolve-hwrt.
/// Flattened into the [`GBufferTargets`] set fields at `create` time, so `present/` readers keep the
/// same `targets.<x>` paths.
struct DeferredSets {
    vocab_set: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    resolve_set: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    cull_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    ssao_set: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    ddgi_update_set: Option<VulkanBindGroup>,
    present_set: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    #[cfg(feature = "hwrt")]
    resolve_set_hwrt: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
}

impl DeferredSets {
    /// Writes the seven deferred descriptor sets ONCE against `core` + `scene`. `cluster_grid_buf` /
    /// `light_index_buf` are the L1 buffers (or the light-table placeholder when L1 is off), computed
    /// once by the caller and shared with the hwrt denoise/temporal set builders. On any set's partial
    /// failure the slots already built in THAT set are drained + every fully-built prior set destroyed
    /// (reverse acquisition); the orchestrator owns the image rings, which it tears down on this
    /// method's `Err`.
    fn build(
        ctx: &VulkanContext,
        scene: &GBufferScene<'_>,
        core: &CoreImages,
        cluster_grid_buf: &BoundBuffer,
        light_index_buf: &BoundBuffer,
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
        // harmless VALID placeholder (the resolve's `clusters_enabled` header gate never reads
        // them on the OFF path — the layout requires a valid descriptor regardless).
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
            // set↔shader-layout mismatch → device-lost). The software set uses them verbatim.
            let imgs = ResolveSlotImages {
                albedo: &core.albedo[slot],
                normal: &core.normal[slot],
                material: &core.material[slot],
                lit: &core.lit[slot],
                viewt: &core.viewt[slot],
                ssao: &core.ssao[slot],
            };
            let entries =
                resolve_software_entries(scene, &imgs, slot, cluster_grid_buf, light_index_buf);
            // The software resolve set is EXACT-FILL at `RESOLVE_SOFTWARE_BINDINGS` (19), under the
            // rung-1b cap of `MAX_BIND_GROUP_BINDINGS` (21). Keeping it EXACT (not `<= cap`) preserves
            // the UNDER-FILL tripwire (a missing binding) AND the over-fill tripwire. The HWRT variant
            // is a SEPARATE 21-binding set (TLAS @19 + shadow-params UBO @20), guarded against
            // `RESOLVE_HWRT_BINDINGS` — the software fill is untouched.
            debug_assert_eq!(
                entries.len(),
                RESOLVE_SOFTWARE_BINDINGS,
                "invariant: the software resolve set must declare EXACTLY {RESOLVE_SOFTWARE_BINDINGS} bindings (exact-fill)"
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

        // The present-blit set RING, written ONCE here: slot `i` is one COMBINED_IMAGE_SAMPLER
        // pointing at `lit[i]` (the resolve's output for that slot) + the scene's present
        // sampler. RINGED so the present samples the SAME slot the resolve wrote this frame (the
        // `lit` ring made a single present set stale — it would sample a sibling slot's image).
        // On a failure at slot `i`, the slots already built [0..i) plus every prior set ring
        // (vocab/resolve/cull/ssao/ddgi) MUST be destroyed (no leak).
        let mut present_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for (slot, dst) in present_slots.iter_mut().enumerate() {
            let entries = [BindGroupEntry::CombinedImage {
                texture: &core.lit[slot],
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

        Ok(DeferredSets {
            vocab_set,
            resolve_set,
            cull_set,
            ssao_set,
            ddgi_update_set,
            present_set,
            #[cfg(feature = "hwrt")]
            resolve_set_hwrt,
        })
    }

    /// Tears down the deferred sets in reverse acquisition order (resolve-hwrt → present →
    /// ddgi-update → ssao → cull → resolve → vocab), consuming `self`.
    ///
    /// # Safety
    ///
    /// `ctx` is live; no submission references these descriptor sets; each is destroyed exactly once
    /// (the by-value `self`). The `cull`/`ssao`/`ddgi-update`/`resolve-hwrt` sets are `Option`-guarded
    /// (present only when their feature was wired).
    unsafe fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: per the contract `ctx` is live and nothing references these sets; each was created
        // on `ctx` and is destroyed exactly once, in reverse acquisition order.
        unsafe {
            // R2a-4b: the HWRT resolve set RING (last-acquired), `Option`-guarded (present only on an
            // RT device under `feature = "hwrt"` + config HardwareTri).
            #[cfg(feature = "hwrt")]
            if let Some(hs) = self.resolve_set_hwrt {
                for g in hs {
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
        };
        RhiDevice::create_texture(ctx, &desc).map_err(SwapchainError::DepthImage)
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
    fn create(
        ctx: &VulkanContext,
        scene: &GBufferScene<'_>,
        extent: VkExtent2D,
    ) -> Result<Self, SwapchainError> {
        // === Sub-bundle builds (order-preserving — see the `CoreImages` / `DeferredSets` docs). ===
        // Each `build` drains its OWN partials on failure; the orchestrator tears down the
        // (fully-built) earlier bundles in reverse acquisition order — the cross-bundle O(n²)
        // teardown-ladder collapse. The SUCCESSFUL create ORDER is preserved EXACTLY: core images →
        // shadow-vis images → deferred sets → (hwrt) denoise sets → temporal images → mv set →
        // temporal sets, so the render stays byte-identical.
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

        // The L1 froxel buffers (or the light-table placeholder when L1 is off) — computed ONCE and
        // shared with the deferred-set builder AND the hwrt denoise/temporal set builders below.
        let cluster_grid_buf = scene.cluster_grid.unwrap_or(scene.light_table);
        let light_index_buf = scene.light_index.unwrap_or(scene.light_table);

        let deferred =
            match DeferredSets::build(ctx, scene, &core, cluster_grid_buf, light_index_buf) {
                Ok(s) => s,
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
                // SAFETY: the deferred sets + the shadow-vis images + `core` were built above on
                // `ctx`, referenced by no submission; each is destroyed exactly once, in reverse
                // acquisition order (deferred sets → shadow-vis → core). `build_shadow_denoise_sets`
                // already drained its OWN partial allocations before returning `Err`.
                unsafe {
                    deferred.destroy(ctx);
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
        let CoreImages { depth, albedo, normal, material, lit, viewt, ssao } = core;
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
            ssao_set,
            ddgi_update_set,
            present_set,
            #[cfg(feature = "hwrt")]
            resolve_set_hwrt,
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

        Ok(Self {
            depth,
            albedo,
            normal,
            material,
            lit,
            viewt,
            ssao,
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
            ssao_set,
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
            extent,
        })
    }

    /// Ensures the G-buffer images + descriptor sets exist and match `extent`,
    /// (re)building them through `ctx` when absent (first frame) or stale (resize).
    /// The vocabulary + present descriptor sets are re-written here — and ONLY here —
    /// so the per-frame recorder records no `vkUpdateDescriptorSets`.
    ///
    /// The caller ([`Renderer::render_gbuffer_frame`]) calls this only after
    /// fence-waiting the frame slot, so no in-flight frame still references the old
    /// targets; on a REPLACE this additionally waits the device idle (a sibling
    /// frame-in-flight slot may still reference the old images — the same
    /// belt-and-braces [`Scene::sync_depth`] uses) before destroying them.
    pub(crate) fn sync_gbuffer(
        targets: &mut Option<Self>,
        ctx: &VulkanContext,
        scene: &GBufferScene<'_>,
        extent: VkExtent2D,
    ) -> Result<(), SwapchainError> {
        if let Some(t) = targets.as_ref()
            && t.extent.width == extent.width
            && t.extent.height == extent.height
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
        let fresh = Self::create(ctx, scene, extent)?;

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
            // The deferred descriptor SETS (resolve-hwrt → present → ddgi-update → ssao → cull →
            // resolve → vocab), via the `DeferredSets` bundle's reverse-acquisition teardown — the
            // SAME order + `Option`-guards the old flat teardown used.
            DeferredSets {
                vocab_set: self.vocab_set,
                resolve_set: self.resolve_set,
                cull_set: self.cull_set,
                ssao_set: self.ssao_set,
                ddgi_update_set: self.ddgi_update_set,
                present_set: self.present_set,
                #[cfg(feature = "hwrt")]
                resolve_set_hwrt: self.resolve_set_hwrt,
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
            // The seven always-present G-buffer image RINGS (ssao → depth), via the `CoreImages`
            // bundle's reverse-acquisition teardown.
            CoreImages {
                depth: self.depth,
                albedo: self.albedo,
                normal: self.normal,
                material: self.material,
                lit: self.lit,
                viewt: self.viewt,
                ssao: self.ssao,
            }
            .destroy(ctx);
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
    /// this so [`MaterialTable::rebind_pending`](boyko_render::MaterialTable::rebind_pending)
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
            vocab_set: bg_ring(),
            resolve_set: bg_ring(),
            cull_set: None,
            ssao_set: None,
            ddgi_update_set: None,
            present_set: bg_ring(),
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
            shadow_vis: None,
            shadow_vis2: None,
            motion_vec: None,
            shadow_temporal_hist: None,
            temporal_out: None,
            vocab_set: bg_ring(),
            resolve_set: bg_ring(),
            resolve_set_hwrt: resolve_set_hwrt.then(bg_ring),
            cull_set: None,
            ssao_set: None,
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
}

