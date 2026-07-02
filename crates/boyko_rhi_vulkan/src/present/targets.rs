//! The per-extent on-screen G-buffer targets ([`GBufferTargets`]) + the
//! per-frame-in-flight ring ([`GBufferFrame`]) + `sync_gbuffer` (extent-change
//! recreate). Split out of the former monolithic `swapchain.rs` (audit W4).

use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, Format, ImageUsage, MAX_BIND_GROUP_BINDINGS, RhiDevice,
    TextureDesc, TextureDimension,
};

use crate::device::VulkanContext;
use crate::ffi::*;
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
    /// bound; the resolve reads it only under `ssao_mode != 0` (0 every pre-P7 scene). NO
    /// per-frame update.
    ///
    /// A RING (one per in-flight frame): slot `i` binds `scene.camera_ring[i]` @5 +
    /// `scene.csm_cascade_ring[i]` @13 — the lock-free per-frame ring fix; every other binding is
    /// identical across slots. The recorder selects `resolve_set[self.frame_index]`.
    pub(crate) resolve_set: [VulkanBindGroup; FRAMES_IN_FLIGHT],
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

impl GBufferTargets {
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
            let entries = [
                BindGroupEntry::StorageImage { texture: &albedo[slot] },
                BindGroupEntry::StorageImage { texture: &normal[slot] },
                BindGroupEntry::StorageImage { texture: &material[slot] },
                BindGroupEntry::StorageImage { texture: &lit[slot] },
                BindGroupEntry::StorageBuffer { buffer: scene.material_table },
                BindGroupEntry::UniformBuffer { buffer: &scene.camera_ring[slot] },
                BindGroupEntry::StorageBuffer { buffer: scene.light_table },
                // Lighting L0b: the gViewT lane @7 (the resolve READS it under `mask == 1`).
                BindGroupEntry::StorageImage { texture: &viewt[slot] },
                // Lighting L1: the ClusterGrid @8 + LightIndexList @9 (resolve READS the
                // pixel's froxel slice when `clusters_enabled`).
                BindGroupEntry::StorageBuffer { buffer: cluster_grid_buf },
                BindGroupEntry::StorageBuffer { buffer: light_index_buf },
                // P6 R1: the SDF edit-list `Buf` @10 — the SAME buffer the marcher binds +
                // uploads + barriers. The resolve dispatch is ordered after the marcher in the
                // same submit, so the prior upload+barrier covers this second COMPUTE read (no
                // new barrier). The resolve's `sdf_soft_shadow_ranged` march reads it read-only
                // (a strict field-CONSUMER); on a `shadow_mode==0` scene the march is never
                // executed, so the binding is a harmless valid descriptor (the 0%-gate).
                BindGroupEntry::StorageBuffer { buffer: scene.edit_list },
                // Render P7: the SSAO term `gSsao` @11 — ALWAYS bound (the resolve descriptor
                // interface is stable regardless of `ssao_mode`). The resolve reads it only under
                // `ssao_mode != 0` (0 every pre-P7 scene), so the binding is a harmless valid
                // descriptor (the 0%-gate); no SSAO pass writes it yet (C2 adds that).
                BindGroupEntry::StorageImage { texture: &ssao[slot] },
                // CSM Increment 1b (Rung A): the cascade shadow-map ARRAY + its PCF comparison
                // sampler as ONE combined descriptor @12 (DXC collapsed `gCsm`(t12)+`gCsmCmp`(s12)
                // — the BrickAtlas precedent). The cascade UBO @13 (mirrors `ResolvedCsm`). BOTH
                // ALWAYS bound (the resolve `.spv` statically references `gCsm`/`CsmCascades`), so
                // the layout MUST declare them and a valid descriptor MUST be present even on the
                // OFF path; the resolve PCF-samples ONLY under `csm_mode != 0` (0 every pre-CSM
                // scene), so the bound-but-unread cascade map/sampler/UBO are never sampled (the
                // 0%-gate). The combined-image entry needs the cascade ARRAY SAMPLE view + the
                // comparison sampler; both come from the scene's always-supplied resources (a real
                // cascade map when CSM is on, a 1×1×1 D32 array dummy + a zeroed UBO when off).
                BindGroupEntry::CombinedImage {
                    texture: scene.csm_cascade_texture,
                    sampler: scene.csm_compare_sampler,
                },
                BindGroupEntry::UniformBuffer {
                    buffer: &scene.csm_cascade_ring[slot],
                },
                // Shadow Phase 5 Inc-1-GPU: the sparse spot/point shadow-ATLAS array + its PCF
                // comparison sampler as ONE combined descriptor @14 (DXC collapsed `gShadowAtlas`(t14)
                // + `gShadowAtlasCmp`(s14) — the `gCsm` precedent). The atlas UBO @15 (mirrors
                // `ResolvedShadowAtlas`, 1296 B). BOTH ALWAYS bound (the resolve `.spv` statically
                // references `gShadowAtlas`/`ShadowAtlas`), so the layout MUST declare them and a valid
                // descriptor MUST be present even on the OFF path; the resolve PCF-samples ONLY under
                // `punctual_shadow_mode != 0` (0 every pre-Inc-1 scene), so the bound-but-unread atlas
                // map/sampler/UBO are never sampled (the 0%-gate). These are the 15th + 16th entries —
                // the resolve set now hits 16/16, the descriptor cap (`MAX_BIND_GROUP_BINDINGS`).
                BindGroupEntry::CombinedImage {
                    texture: scene.shadow_atlas_texture,
                    sampler: scene.shadow_atlas_sampler,
                },
                BindGroupEntry::UniformBuffer {
                    buffer: scene.shadow_atlas_ubo,
                },
            ];
            // The resolve set now declares 16 bindings (0..=15) — EXACTLY the 16-binding cap
            // (`MAX_BIND_GROUP_BINDINGS`), 0 free. CSM Rung A added the combined cascade map+sampler
            // @12 + the cascade UBO @13; Shadow Inc-1-GPU adds the combined atlas map+sampler @14 +
            // the atlas UBO @15 (both via the combined-image collapse — the in-house RHI has no
            // SAMPLER-only `BindGroupEntry`). Assert the EXACT cap hit (16/16): a future binding has
            // no room without raising the cap.
            debug_assert!(
                entries.len() == MAX_BIND_GROUP_BINDINGS,
                "invariant: the resolve set must fill EXACTLY the 16-binding descriptor cap (16/16)"
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

        Ok(Self {
            depth,
            albedo,
            normal,
            material,
            lit,
            viewt,
            ssao,
            vocab_set,
            resolve_set,
            cull_set,
            ssao_set,
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
        // each have `FRAMES_IN_FLIGHT` slots; the cull & SSAO RINGS are `Option`-guarded
        // (present only when L1 / SSAO were wired); the eight render-target image RINGS
        // each have `FRAMES_IN_FLIGHT` slots — every slot of every ring is drained.
        unsafe {
            for g in self.present_set {
                RhiDevice::destroy_bind_group(ctx, g);
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

