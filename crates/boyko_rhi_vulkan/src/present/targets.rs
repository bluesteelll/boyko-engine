//! The per-extent on-screen G-buffer targets ([`GBufferTargets`]) + the
//! per-frame-in-flight ring ([`GBufferFrame`]) + `sync_gbuffer` (extent-change
//! recreate). Split out of the former monolithic `swapchain.rs` (audit W4).

use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, Format, ImageUsage, RhiDevice,
    TextureDesc, TextureDimension,
};
#[cfg(feature = "hwrt")]
use boyko_rhi::{BufferDesc, BufferUsage, MemoryLocation};

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
        // === The G-buffer render-target IMAGE RINGS (lock-free cross-frame WAR fix). ===
        // Each render-target image is RINGED to `FRAMES_IN_FLIGHT` copies so frame N+1 writes
        // slot `i`'s images while frame N still reads slot `j`'s. A `[Option<_>; N]` builder
        // per ring lets every early-return error path drain the partial ring it failed in PLUS
        // every fully-built prior ring (no VkImage/VkImageView leak) — the exhaustive ladder.
        //
        // `destroy_ring` tears down a COMPLETED ring (consumes the `[VulkanTexture; N]`);
        // `drain_partial` tears down the slots already built in the FAILING ring (drains the
        // `[Option<_>; N]` in place). Both are `unsafe` (they call `destroy_texture`): every
        // texture was created on `ctx` just above, is referenced by no submission, and is
        // destroyed exactly once on the single error path that runs.
        //
        // SAFETY (shared by every destroy below): `ctx` is the live context each texture was
        // created on; none is referenced by any submission (this is the build phase, before
        // any record/submit); each ring slot is destroyed exactly once (a completed ring is
        // consumed by value, a partial ring is `take`-drained).
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
        // Rung 3a: tears down a COMPLETED `Option`-guarded ring (the two shadow-vis targets, which
        // are `None` on a device lacking RG8/RG16 storage) IN PLACE via `take()`, so it can be
        // called from inside the descriptor-set error loops below without moving the outer `let`
        // (a move-in-loop the borrow checker rejects). The `None` case (and a second call after a
        // take) is a no-op. Same SAFETY as `destroy_ring` (`ctx` live, no submission references it,
        // each slot destroyed once — the `take` guarantees at-most-once).
        #[cfg(feature = "hwrt")]
        let destroy_vis_opt = |ring: &mut Option<[VulkanTexture; FRAMES_IN_FLIGHT]>| unsafe {
            if let Some(r) = ring.take() {
                for t in r {
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

        // Render P5-r0: the throwaway depth-prepass color attachment is DELETED — pass A
        // now binds the three REAL G-buffer images (albedo/normal/material) as MRT color
        // attachments, so a separate throwaway color image is obsolete.

        // ALBEDO: STORAGE (marcher store) | SAMPLED (the present-blit, pass C) |
        // COLOR_ATTACHMENT (Render P5-r0: the mesh raster pass A writes it as MRT@0).
        let mut albedo_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in albedo_slots.iter_mut() {
            match Self::create_gbuffer_image(
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
            match Self::create_gbuffer_image(
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
            match Self::create_gbuffer_image(
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
            match Self::create_gbuffer_image(
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
            match Self::create_viewt_image(ctx, extent) {
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
            match Self::create_ssao_image(ctx, extent) {
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

        // Rung 3a: the two RT soft-shadow-visibility target RINGS (`shadow_vis` + `shadow_vis2`,
        // BOTH R16G16_UNORM — the uniform-RG16 ping-pong), allocated together ONLY when the device
        // advertises RG16 storage (`shadow_denoise_storage_ok()` — the DDGI-degrade discipline; on
        // an unsupported device
        // BOTH stay `None` and the denoise is disabled, never a boot fault). Ringed per-FIF like the
        // ssao ring, built with the same `[Option<_>; N]` drain-on-error ladder. No pass reads them
        // this step (steps 4-6 add the VIS / à-trous passes) — allocated-but-unused, byte-identical.
        // On a mid-ring failure, drain the partial ring + every prior image ring (ssao..depth).
        #[cfg(feature = "hwrt")]
        let (mut shadow_vis, mut shadow_vis2): (
            Option<[VulkanTexture; FRAMES_IN_FLIGHT]>,
            Option<[VulkanTexture; FRAMES_IN_FLIGHT]>,
        ) = if ctx.device_caps().shadow_denoise_storage_ok() {
            let mut vis_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
                [const { None }; FRAMES_IN_FLIGHT];
            for slot in vis_slots.iter_mut() {
                match Self::create_shadow_vis_image(ctx, extent) {
                    Ok(t) => *slot = Some(t),
                    Err(e) => {
                        drain_partial(&mut vis_slots);
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
            let vis: [VulkanTexture; FRAMES_IN_FLIGHT] = vis_slots
                .map(|s| s.expect("invariant: every shadow_vis ring slot built before here"));

            let mut vis2_slots: [Option<VulkanTexture>; FRAMES_IN_FLIGHT] =
                [const { None }; FRAMES_IN_FLIGHT];
            for slot in vis2_slots.iter_mut() {
                match Self::create_shadow_vis2_image(ctx, extent) {
                    Ok(t) => *slot = Some(t),
                    Err(e) => {
                        drain_partial(&mut vis2_slots);
                        destroy_ring(vis);
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
            let vis2: [VulkanTexture; FRAMES_IN_FLIGHT] = vis2_slots
                .map(|s| s.expect("invariant: every shadow_vis2 ring slot built before here"));
            (Some(vis), Some(vis2))
        } else {
            (None, None)
        };

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
        // built [0..i) MUST be destroyed (no descriptor leak) along with the prior images.
        let mut vocab_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in 0..FRAMES_IN_FLIGHT {
            let entries = [
                BindGroupEntry::StorageBuffer { buffer: scene.edit_list },
                BindGroupEntry::SampledImage {
                    texture: &depth[slot],
                    sampler: scene.depth_sampler,
                },
                BindGroupEntry::StorageImage { texture: &albedo[slot] },
                BindGroupEntry::StorageImage { texture: &normal[slot] },
                BindGroupEntry::StorageImage { texture: &material[slot] },
                BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[slot] },
                BindGroupEntry::StorageBuffer { buffer: scene.tiles_buffer },
                // PBR MVP-2: the material table SSBO @7 (the marcher fetches `base_color`).
                BindGroupEntry::StorageBuffer { buffer: scene.material_table },
                // Lighting L0b: the gViewT lane @8 (the marcher STORES the surface `t`).
                BindGroupEntry::StorageImage { texture: &viewt[slot] },
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
                    // SAFETY: the eight image RINGS + the vocab slots already built [0..slot)
                    // were created on `ctx`; referenced by no submission; each destroyed
                    // exactly once on this error path (the partial vocab ring is drained
                    // first, then every image ring via `destroy_ring`).
                    unsafe {
                        for s in vocab_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                    }
                    // Rung 3a: the two shadow-vis rings (last-built images), `Option`-drained first
                    // (reverse acquisition: vis2 before vis before ssao). No-op when unallocated.
                    #[cfg(feature = "hwrt")]
                    destroy_vis_opt(&mut shadow_vis2);
                    #[cfg(feature = "hwrt")]
                    destroy_vis_opt(&mut shadow_vis);
                    destroy_ring(ssao);
                    destroy_ring(viewt);
                    destroy_ring(lit);
                    destroy_ring(material);
                    destroy_ring(normal);
                    destroy_ring(albedo);
                    destroy_ring(depth);
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
        let cluster_grid_buf = scene.cluster_grid.unwrap_or(scene.light_table);
        let light_index_buf = scene.light_index.unwrap_or(scene.light_table);
        // Build FRAMES_IN_FLIGHT identical copies of the resolve set, slot `i` binding
        // `scene.camera_ring[i]` @5 + `scene.csm_cascade_ring[i]` @13 (the lock-free per-frame ring
        // fix; every other binding is identical across slots). On a failure at slot `i`, the slots
        // already built [0..i) plus the prior vocab ring + images MUST be destroyed (no leak).
        let mut resolve_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for slot in 0..FRAMES_IN_FLIGHT {
            // The 19 SHARED resolve bindings (0..=18) — built by the ONE helper the HWRT set also
            // consumes, so the two sets' first 19 bindings cannot drift (a drift = an invisible
            // set↔shader-layout mismatch → device-lost). The software set uses them verbatim.
            let imgs = ResolveSlotImages {
                albedo: &albedo[slot],
                normal: &normal[slot],
                material: &material[slot],
                lit: &lit[slot],
                viewt: &viewt[slot],
                ssao: &ssao[slot],
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
                    // SAFETY: the eight image RINGS + the whole vocab ring + the resolve slots
                    // already built [0..slot) were created on `ctx`; referenced by no submission;
                    // each destroyed exactly once on this error path (sets → images via
                    // `destroy_ring`).
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
                    // Rung 3a: the two shadow-vis rings (last-built images), `Option`-drained first
                    // (reverse acquisition: vis2 before vis before ssao). No-op when unallocated.
                    #[cfg(feature = "hwrt")]
                    destroy_vis_opt(&mut shadow_vis2);
                    #[cfg(feature = "hwrt")]
                    destroy_vis_opt(&mut shadow_vis);
                    destroy_ring(ssao);
                    destroy_ring(viewt);
                    destroy_ring(lit);
                    destroy_ring(material);
                    destroy_ring(normal);
                    destroy_ring(albedo);
                    destroy_ring(depth);
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
                // built [0..i) plus the prior resolve + vocab rings + images MUST be destroyed.
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
                    // SAFETY: the resolve + vocab rings + the eight image RINGS above + the cull
                    // slots already built [0..slot) were created on `ctx`; referenced by no
                    // submission; each destroyed exactly once on this error path (sets → images
                    // via `destroy_ring`).
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
                    // Rung 3a: the two shadow-vis rings (last-built images), `Option`-drained first
                    // (reverse acquisition: vis2 before vis before ssao). No-op when unallocated.
                    #[cfg(feature = "hwrt")]
                    destroy_vis_opt(&mut shadow_vis2);
                    #[cfg(feature = "hwrt")]
                    destroy_vis_opt(&mut shadow_vis);
                    destroy_ring(ssao);
                    destroy_ring(viewt);
                    destroy_ring(lit);
                    destroy_ring(material);
                    destroy_ring(normal);
                    destroy_ring(albedo);
                    destroy_ring(depth);
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
                // built [0..i) plus the prior cull/resolve/vocab rings + images MUST be destroyed.
                let mut ssao_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
                    [const { None }; FRAMES_IN_FLIGHT];
                let mut failure: Option<crate::error::VulkanError> = None;
                for (slot, dst) in ssao_slots.iter_mut().enumerate() {
                    let entries = [
                        BindGroupEntry::StorageImage { texture: &normal[slot] },
                        BindGroupEntry::StorageImage { texture: &material[slot] },
                        BindGroupEntry::StorageImage { texture: &viewt[slot] },
                        BindGroupEntry::StorageImage { texture: &ssao[slot] },
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
                    // SAFETY: the resolve + vocab rings + the (optional) cull ring + the eight
                    // image RINGS above + the ssao slots already built [0..slot) were created on
                    // `ctx`; referenced by no submission; each destroyed exactly once (sets →
                    // images via `destroy_ring`). The cull ring is `Option`-guarded (only when L1
                    // wired).
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
                    // Rung 3a: the two shadow-vis rings (last-built images), `Option`-drained first
                    // (reverse acquisition: vis2 before vis before ssao). No-op when unallocated.
                    #[cfg(feature = "hwrt")]
                    destroy_vis_opt(&mut shadow_vis2);
                    #[cfg(feature = "hwrt")]
                    destroy_vis_opt(&mut shadow_vis);
                    destroy_ring(ssao);
                    destroy_ring(viewt);
                    destroy_ring(lit);
                    destroy_ring(material);
                    destroy_ring(normal);
                    destroy_ring(albedo);
                    destroy_ring(depth);
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
        // update→resolve barrier). On a failure, the prior vocab/resolve/(optional cull/ssao) rings +
        // the eight image rings MUST be destroyed (the ssao teardown chain shape).
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
                        // SAFETY: the vocab & resolve rings + the (optional) cull & ssao rings + the
                        // eight image rings were created on `ctx`; referenced by no submission; each
                        // destroyed exactly once (sets → images via `destroy_ring`). The cull & ssao
                        // rings are `Option`-guarded (present only when L1 / SSAO wired).
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
                        // Rung 3a: the two shadow-vis rings (last-built images), `Option`-drained
                        // first (reverse acquisition: vis2 before vis before ssao). No-op when
                        // unallocated.
                        #[cfg(feature = "hwrt")]
                        destroy_vis_opt(&mut shadow_vis2);
                        #[cfg(feature = "hwrt")]
                        destroy_vis_opt(&mut shadow_vis);
                        destroy_ring(ssao);
                        destroy_ring(viewt);
                        destroy_ring(lit);
                        destroy_ring(material);
                        destroy_ring(normal);
                        destroy_ring(albedo);
                        destroy_ring(depth);
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
        // On a failure at slot `i`, the slots already built [0..i) plus every prior ring
        // (vocab/resolve/cull/ssao + the eight image rings) MUST be destroyed (no leak).
        let mut present_slots: [Option<VulkanBindGroup>; FRAMES_IN_FLIGHT] =
            [const { None }; FRAMES_IN_FLIGHT];
        for (slot, dst) in present_slots.iter_mut().enumerate() {
            let entries = [BindGroupEntry::CombinedImage {
                texture: &lit[slot],
                sampler: scene.present_sampler,
            }];
            let desc = BindGroupDesc::<Vulkan> {
                layout: scene.present_layout,
                entries: &entries,
            };
            match RhiDevice::create_bind_group(ctx, &desc) {
                Ok(g) => *dst = Some(g),
                Err(e) => {
                    // SAFETY: the eight image RINGS + the vocab & resolve RINGS + the (optional)
                    // cull & (optional) SSAO RINGS + the present slots already built [0..slot) above
                    // were created on `ctx`; referenced by no submission; each destroyed exactly
                    // once on this error path (sets → images via `destroy_ring`). The cull & SSAO
                    // rings are `Option`-guarded (present only when L1 / SSAO wired); every ring
                    // slot is drained.
                    unsafe {
                        for s in present_slots.iter_mut() {
                            if let Some(g) = s.take() {
                                RhiDevice::destroy_bind_group(ctx, g);
                            }
                        }
                        // SDFDDGI I2: the single (non-ringed) update set, `Option`-guarded (present
                        // only when the update pass is wired).
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
                    // Rung 3a: the two shadow-vis rings (last-built images), `Option`-drained first
                    // (reverse acquisition: vis2 before vis before ssao). No-op when unallocated.
                    #[cfg(feature = "hwrt")]
                    destroy_vis_opt(&mut shadow_vis2);
                    #[cfg(feature = "hwrt")]
                    destroy_vis_opt(&mut shadow_vis);
                    destroy_ring(ssao);
                    destroy_ring(viewt);
                    destroy_ring(lit);
                    destroy_ring(material);
                    destroy_ring(normal);
                    destroy_ring(albedo);
                    destroy_ring(depth);
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
                            albedo: &albedo[slot],
                            normal: &normal[slot],
                            material: &material[slot],
                            lit: &lit[slot],
                            viewt: &viewt[slot],
                            ssao: &ssao[slot],
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
                        // SAFETY: every prior set RING (vocab/resolve/present + the optional
                        // cull/ssao/ddgi-update) + the eight image RINGS + the HWRT slots already
                        // built [0..slot) were created on `ctx`; referenced by no submission; each
                        // destroyed exactly once on this error path. The optional sets are
                        // `Option`-guarded; every ring slot is drained.
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
                        // Rung 3a: the two shadow-vis rings (last-built images), `Option`-drained
                        // first (reverse acquisition: vis2 before vis before ssao). No-op when
                        // unallocated.
                        #[cfg(feature = "hwrt")]
                        destroy_vis_opt(&mut shadow_vis2);
                        #[cfg(feature = "hwrt")]
                        destroy_vis_opt(&mut shadow_vis);
                        destroy_ring(ssao);
                        destroy_ring(viewt);
                        destroy_ring(lit);
                        destroy_ring(material);
                        destroy_ring(normal);
                        destroy_ring(albedo);
                        destroy_ring(depth);
                        return Err(SwapchainError::DepthImage(e));
                    }
                    Some(hwrt_slots.map(|s| {
                        s.expect("invariant: every HWRT resolve ring slot built before reaching here")
                    }))
                }
                _ => None,
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
            &albedo,
            &normal,
            &material,
            &lit,
            &viewt,
            &ssao,
            shadow_vis.as_ref(),
            shadow_vis2.as_ref(),
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
                // SAFETY: every ring/set built above (vocab/resolve/present + the optional
                // cull/ssao/ddgi-update/resolve-hwrt) was created on `ctx`, referenced by no
                // submission, and is destroyed exactly once here (the denoise builder already drained
                // its own partial allocations before returning). Reverse acquisition: sets → images.
                unsafe {
                    if let Some(hs) = resolve_set_hwrt {
                        for g in hs {
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
                destroy_vis_opt(&mut shadow_vis2);
                destroy_vis_opt(&mut shadow_vis);
                destroy_ring(ssao);
                destroy_ring(viewt);
                destroy_ring(lit);
                destroy_ring(material);
                destroy_ring(normal);
                destroy_ring(albedo);
                destroy_ring(depth);
                return Err(SwapchainError::DepthImage(e));
            }
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
            // R2a-4b: the HWRT resolve set RING (last-acquired), `Option`-guarded (present only on
            // an RT device under `feature = "hwrt"` + config HardwareTri).
            #[cfg(feature = "hwrt")]
            if let Some(hs) = self.resolve_set_hwrt {
                for g in hs {
                    RhiDevice::destroy_bind_group(ctx, g);
                }
            }
            for g in self.present_set {
                RhiDevice::destroy_bind_group(ctx, g);
            }
            // SDFDDGI I2: the single (non-ringed) update set, `Option`-guarded (present only when
            // the update pass was wired).
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

