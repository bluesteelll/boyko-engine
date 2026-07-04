//! SDFDDGI I1 — the DDGI probe atlas + per-probe classification GPU resources.
//!
//! Owns the three durable GPU data objects of the irradiance-probe grid (Decision D2 /
//! the plan's Principle-0 storage class 2 & 3), a declared FFI/GPU-contiguity exception —
//! NOT a host `std::Vec`/`HashMap` side store:
//!
//! 1. the **irradiance** atlas — a `B10G11R11_UFLOAT` (`R11G11B10F`-no-gamma, Decision D6)
//!    `Texture2DArray` of octahedral probe tiles;
//! 2. the **depth/visibility** atlas — an `R16G16_SFLOAT` (`RG16F` two-moment) `Texture2DArray`;
//! 3. the **classification** buffer — 1 u32/probe (bit0 = active, bit1 = converged-once), a
//!    GPU storage buffer, the resolve/feedback's "unconverged ⇒ sky-ambient" gate. One full
//!    word per probe (SDFDDGI I2) so the update pass's parallel per-probe stores are race-free.
//!
//! Plus one dedicated LINEAR (non-comparison) sampler shared by both atlases. This CLOSES the
//! I0a VUID trap: I0a bound the CSM COMPARISON sampler (`compareEnable == VK_TRUE`) as the
//! dummy, and a non-`Dref` `SampleLevel` with a comparison sampler is Vulkan UB. The atlas
//! read is a plain octahedral `SampleLevel`, so it MUST use a linear non-comparison sampler.
//!
//! # The atlas layout (probe → array layer + tile origin)
//!
//! The grid is `dims = [dx, dy, dz] = [16, 8, 16] = 2048` probes (owner-locked). A
//! `Texture2DArray` layer count is capped at [`MAX_TEXTURE_LAYERS`](crate::texture::MAX_TEXTURE_LAYERS)
//! (16), so the 2048 probes CANNOT be 2048 layers. The layout is **Y-plane-major** (the
//! standard DDGI/RTXGI tiled-plane arrangement):
//!
//! * **array layer = probe Y** (`dy = 8` layers ≤ 16 ✓);
//! * within a layer, probe `(x, z)` tiles a `dx × dz = 16 × 16` tile grid — **tile column =
//!   x, tile row = z**;
//! * so probe `(x, y, z)` ⇒ array layer `y`, tile texel origin `(x · TILE, z · TILE)`.
//!
//! Per-layer texel dimensions: irradiance `dx·TILE_IRR × dz·TILE_IRR = 128 × 128`; depth
//! `dx·TILE_DEPTH × dz·TILE_DEPTH = 256 × 256`. Both use `dy = 8` array layers. The
//! irradiance tile is `8×8` (`6×6` valid octahedral interior + a 1-texel border — the I0b
//! oracle's `goldens::DDGI_IRR_TILE_EDGE` / `DDGI_IRR_VALID_EXTENT` / `DDGI_TILE_BORDER`,
//! feature-gated); the depth tile is `16×16` (`14×14` valid + border).
//!
//! The I2 update pass writes probe `(x, y, z)`'s tile at this address; the I3 resolve reads
//! the same address; the I0b host oracle's injected `tap`/`probe_pos` addresses the same
//! probe index. All three consume THIS mapping ([`ddgi_probe_tile_origin`]).
//!
//! # Boot-clear (the uninitialized-read hazard fix)
//!
//! All three are boot-cleared to defined values as part of the boot transition: irradiance =
//! 0, depth = 0, classification = 0 (unconverged). A `0` classification byte is the anchor:
//! the resolve/feedback treat an unconverged probe as sky-ambient until its first write, so a
//! never-updated probe (GI gate off, or the first frames of a round-robin) reads a defined
//! `0`-irradiance/`0`-depth tile gated OUT by the converged bit — never uninitialized VRAM.
//! Depth `0` is a safe init: the Chebyshev `var = E[d²] − E[d]² = 0` and `dist > mean ⇒ 0`
//! visibility is moot because the converged bit is `0`, so the tile is never actually read as
//! coverage this rung.
//!
//! # Boot-transition
//!
//! Mirrors the `gCsm`/`gShadowAtlas` one-shot boot-transition lifecycle (the host
//! `CsmResources::seed_boot_layouts`): the atlases end in `SHADER_READ_ONLY_OPTIMAL` (the
//! resolve read layout), fence-waited, before the first frame. The clear runs while the image
//! is in `TRANSFER_DST_OPTIMAL`, then one barrier moves it to `SHADER_READ_ONLY_OPTIMAL`. At
//! I2 the update pass moves them to `GENERAL` for the compute store — but that transition is
//! DERIVED BY THE RDG (from an `add_image_seeded(SHADER_READ_ONLY_OPTIMAL)` seed), not
//! hand-written here; this boot path is unchanged (the STORAGE usage bit does not perturb the
//! `TRANSFER_DST` clear, preserving the byte-identical 0%-gate golden).
//!
//! # Lifetime
//!
//! Owned by value; torn down through [`DdgiAtlas::destroy`] (the caller has drained the
//! device so no submission still samples them). Not `Copy`/`Clone` (the move encodes
//! "destroyed once"); `!Send`/`!Sync` like every other Vulkan resource.

use boyko_rhi::{
    AddressMode, BufferDesc, BufferUsage, Filter, Format, ImageAspect, ImageBarrierDesc,
    ImageLayout, ImageSubresourceRange, ImageUsage, MemoryLocation, MipMode, RhiCommandEncoder,
    RhiDevice, RhiQueue, SamplerDesc, TextureDesc, TextureDimension,
};
use boyko_rhi::enums::{BarrierAccess, BarrierStage};

use crate::device::VulkanContext;
use crate::error::VulkanError;
use crate::memory::BoundBuffer;
use crate::rhi_impl::VulkanSampler;
use crate::texture::VulkanTexture;

// ---- atlas layout constants (the probe → texel mapping — see the module header) ----------

/// The default probe-grid X dimension (probes along world X). Mirrors
/// `boyko_render::ddgi_config`'s owner-locked `16` — the tile-COLUMN axis within a layer.
pub const DDGI_GRID_DIM_X: u32 = 16;
/// The default probe-grid Y dimension (probes along world Y). Mirrors the owner-locked `8` —
/// the ARRAY-LAYER axis (`8 ≤ MAX_TEXTURE_LAYERS`).
pub const DDGI_GRID_DIM_Y: u32 = 8;
/// The default probe-grid Z dimension (probes along world Z). Mirrors the owner-locked `16` —
/// the tile-ROW axis within a layer.
pub const DDGI_GRID_DIM_Z: u32 = 16;

/// The total probe count (`dx · dy · dz = 16 · 8 · 16 = 2048`) — the classification buffer's
/// byte count and the atlas tile budget.
pub const DDGI_PROBE_COUNT: u32 = DDGI_GRID_DIM_X * DDGI_GRID_DIM_Y * DDGI_GRID_DIM_Z;

/// The IRRADIANCE octahedral tile edge in texels (`8` = `6×6` valid + a 1-texel border) —
/// the SAME value as the I0b host oracle's `goldens::DDGI_IRR_TILE_EDGE` (pinned equal by a
/// feature-gated compile assert below so the atlas write/read and the oracle agree).
pub const DDGI_IRR_TILE_EDGE: u32 = 8;
/// The DEPTH/visibility tile edge in texels (`16` = `14×14` valid + a 1-texel border,
/// Decision D2). The depth tile is finer (two-moment Chebyshev) than the irradiance tile.
pub const DDGI_DEPTH_TILE_EDGE: u32 = 16;

/// The number of `Texture2DArray` layers in BOTH atlases (`= dy`, the Y-plane-major layout).
pub const DDGI_ATLAS_LAYERS: u32 = DDGI_GRID_DIM_Y;

/// The IRRADIANCE atlas per-layer width in texels (`dx · TILE = 16 · 8 = 128`).
pub const DDGI_IRR_ATLAS_WIDTH: u32 = DDGI_GRID_DIM_X * DDGI_IRR_TILE_EDGE;
/// The IRRADIANCE atlas per-layer height in texels (`dz · TILE = 16 · 8 = 128`).
pub const DDGI_IRR_ATLAS_HEIGHT: u32 = DDGI_GRID_DIM_Z * DDGI_IRR_TILE_EDGE;
/// The DEPTH atlas per-layer width in texels (`dx · TILE = 16 · 16 = 256`).
pub const DDGI_DEPTH_ATLAS_WIDTH: u32 = DDGI_GRID_DIM_X * DDGI_DEPTH_TILE_EDGE;
/// The DEPTH atlas per-layer height in texels (`dz · TILE = 16 · 16 = 256`).
pub const DDGI_DEPTH_ATLAS_HEIGHT: u32 = DDGI_GRID_DIM_Z * DDGI_DEPTH_TILE_EDGE;

// Pin the atlas tile edge to the I0b host oracle's constant: the I2 update WRITE, the I3
// resolve READ, and the oracle's texel-direction math MUST share one tile geometry. A drift
// here silently mis-addresses every probe tile (the M2 dead-branch lesson generalized). The
// oracle module (`goldens`) is feature-gated, so the cross-crate pin is checked only when it
// is compiled in (every test / `goldens`-feature build) — the value itself never varies.
#[cfg(any(test, feature = "goldens"))]
const _: () = assert!(
    DDGI_IRR_TILE_EDGE == crate::goldens::DDGI_IRR_TILE_EDGE,
    "SDFDDGI atlas irradiance tile edge must equal the I0b host-oracle tile edge"
);
// The layer budget MUST fit the shared texture array-view limit.
const _: () = assert!(
    DDGI_ATLAS_LAYERS as usize <= crate::texture::MAX_TEXTURE_LAYERS,
    "SDFDDGI atlas layers (= grid dim Y) must be <= MAX_TEXTURE_LAYERS"
);

/// Maps a probe grid index `(x, y, z)` to its atlas address: the array LAYER and the tile
/// TEXEL ORIGIN `(ox, oy)` within that layer, for the given `tile_edge` (irradiance `8` or
/// depth `16`). Y-plane-major (see the module header): `layer = y`, `ox = x · tile_edge`,
/// `oy = z · tile_edge`. The I2 update WRITE and the I3 resolve READ both call this (or its
/// GPU mirror) so the write and read address the identical texels.
///
/// # Panics (debug)
///
/// Debug-asserts each axis is in range (`x < dx`, `y < dy`, `z < dz`) — an out-of-grid probe
/// index is a caller bug.
#[inline]
pub fn ddgi_probe_tile_origin(x: u32, y: u32, z: u32, tile_edge: u32) -> (u32, u32, u32) {
    debug_assert!(
        x < DDGI_GRID_DIM_X && y < DDGI_GRID_DIM_Y && z < DDGI_GRID_DIM_Z,
        "invariant: DDGI probe ({x},{y},{z}) must be within grid [{DDGI_GRID_DIM_X},{DDGI_GRID_DIM_Y},{DDGI_GRID_DIM_Z}]"
    );
    (y, x * tile_edge, z * tile_edge)
}

// ---- the resource owner ------------------------------------------------------------------

/// The SDFDDGI I1 probe atlas + classification GPU resources (Decision D2, persistent SINGLE
/// atlas per moment — NOT ping-pong). Owns the irradiance + depth `Texture2DArray`s, the
/// per-probe classification storage buffer, and the shared LINEAR sampler. Boot-cleared +
/// boot-transitioned to `SHADER_READ_ONLY_OPTIMAL` at [`Self::create`]. Carries NO drop glue:
/// destruction is manual via [`Self::destroy`].
pub struct DdgiAtlas {
    /// The probe IRRADIANCE atlas — `B10G11R11_UFLOAT` (`R11G11B10F`-no-gamma) `Texture2DArray`,
    /// `DDGI_IRR_ATLAS_WIDTH × DDGI_IRR_ATLAS_HEIGHT × DDGI_ATLAS_LAYERS`. Sampled at resolve
    /// binding 16 via [`Self::sampler`].
    irradiance: VulkanTexture,
    /// The probe DEPTH/visibility atlas — `R16G16_SFLOAT` (`RG16F` two-moment) `Texture2DArray`,
    /// `DDGI_DEPTH_ATLAS_WIDTH × DDGI_DEPTH_ATLAS_HEIGHT × DDGI_ATLAS_LAYERS`. Sampled at resolve
    /// binding 17 via [`Self::sampler`].
    depth: VulkanTexture,
    /// The per-probe CLASSIFICATION buffer — 1 u32/probe (`DDGI_PROBE_COUNT * 4` bytes = 8 KB),
    /// a device-local STORAGE buffer. bit0 = active, bit1 = converged-once. One full u32 per probe
    /// (NOT the I1 byte-packed layout) so I2's parallel per-probe stores are race-free without
    /// atomics (single-writer-per-word-per-frame). Boot-cleared to 0 (all unconverged). NOT bound
    /// at resolve I1 (I3 gates on the converged bit); owned here for I2's compute read/write.
    classification: BoundBuffer,
    /// The dedicated LINEAR, non-comparison (`compareEnable == VK_FALSE`), clamp-to-edge sampler
    /// bundled with BOTH atlases as their combined image+sampler (bindings 16/17). NOT the CSM
    /// comparison sampler — the atlas read is a plain `SampleLevel`, which is UB with a comparison
    /// sampler (the I0a VUID trap this closes).
    sampler: VulkanSampler,
}

impl DdgiAtlas {
    /// Creates the irradiance + depth atlases, the classification buffer, and the shared LINEAR
    /// sampler; boot-clears all three to defined values and boot-transitions the atlases to
    /// `SHADER_READ_ONLY_OPTIMAL` (fence-waited, before the first frame). On any partial failure
    /// every object created so far is torn down before the error returns.
    pub fn create(ctx: &VulkanContext) -> Result<Self, VulkanError> {
        // SDFDDGI I2 — the STORAGE re-add + its GRACEFUL-DEGRADATION gate (plan §3). The probe-
        // update pass writes both atlases via storage images, but B10G11R11 storage is a device-
        // OPTIONAL format feature. DDGI is OPT-IN (unlike the always-used `gViewT`), so a device
        // lacking it MUST NOT boot-fail: read the two device caps, add STORAGE only when BOTH are
        // supported, else fall back to the I1 `SAMPLED | TRANSFER_DST` usage so `vkCreateImage`
        // cannot panic. `resolve_ddgi_grid` clamps DDGI permanently disabled on the same
        // predicate (`ddgi_storage_ok`), so an atlas-without-storage is never dispatched into.
        let storage_ok = ctx.device_caps().ddgi_storage_ok();
        let atlas_usage = if storage_ok {
            ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST | ImageUsage::STORAGE
        } else {
            ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST
        };

        let irradiance = RhiDevice::create_texture(
            ctx,
            &TextureDesc {
                width: DDGI_IRR_ATLAS_WIDTH,
                height: DDGI_IRR_ATLAS_HEIGHT,
                depth: 1,
                // R11G11B10F-no-gamma (Decision D6): the bit-exact resolve store. The Vulkan
                // format packs the components as B10-G11-R11; the sampler returns RGB order.
                format: Format::B10G11R11UfloatPack32,
                dimension: TextureDimension::D2,
                // I2: SAMPLED (resolve read) + TRANSFER_DST (boot-clear) + STORAGE (the update
                // pass's compute write) — the STORAGE bit is present ONLY when
                // `ddgi_storage_ok` (else the I1 usage, so an unsupported device still boots; the
                // resolve clamp makes disabled DDGI cost nothing). The TRANSFER_DST boot-clear /
                // transition path below is UNCHANGED by adding STORAGE (the clear is not perturbed
                // — it preserves the byte-identical 0%-gate golden).
                usage: atlas_usage,
                array_layers: DDGI_ATLAS_LAYERS,
            },
        )?;

        let depth = match RhiDevice::create_texture(
            ctx,
            &TextureDesc {
                width: DDGI_DEPTH_ATLAS_WIDTH,
                height: DDGI_DEPTH_ATLAS_HEIGHT,
                depth: 1,
                // RG16F two-moment depth (Decision D2): `.r = E[d]`, `.g = E[d²]`.
                format: Format::R16G16Sfloat,
                dimension: TextureDimension::D2,
                // I2: SAMPLED + TRANSFER_DST + STORAGE (the update pass's compute write), STORAGE
                // present only when `ddgi_storage_ok` (see the irradiance atlas note). The
                // boot-clear/transition path is UNCHANGED.
                usage: atlas_usage,
                array_layers: DDGI_ATLAS_LAYERS,
            },
        ) {
            Ok(t) => t,
            Err(e) => {
                // SAFETY: `irradiance` was just created on `ctx`, owned exclusively here, never
                // submitted; destroy it once on this edge.
                unsafe { RhiDevice::destroy_texture(ctx, irradiance) };
                return Err(e);
            }
        };

        // The classification buffer: 1 u32/probe (SDFDDGI I2 / P1-2). At I1 it was 1 byte/probe
        // rounded to a u32 multiple, so 4 probes shared a u32 word — under I2's PARALLEL per-probe
        // byte-stores that word-shares a race (a non-atomic byte store read-modify-writes the
        // whole word, clobbering the neighbours). One u32/probe (`DDGI_PROBE_COUNT * 4` = 8 KB,
        // trivial) makes each probe own a full word → single-writer-per-element-per-frame is
        // race-free with plain stores, no atomics. bit0 = active, bit1 = converged-once.
        let class_bytes = (DDGI_PROBE_COUNT as u64) * 4;
        let classification = match RhiDevice::create_buffer(
            ctx,
            &BufferDesc {
                size: class_bytes,
                // STORAGE for the I2 compute read/write; TRANSFER_DST for the boot fill-clear.
                usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST,
                location: MemoryLocation::DeviceLocal,
            },
        ) {
            Ok(b) => b,
            Err(e) => {
                // SAFETY: `depth` + `irradiance` were just created on `ctx`, owned exclusively
                // here, never submitted; destroy each once on this edge.
                unsafe {
                    RhiDevice::destroy_texture(ctx, depth);
                    RhiDevice::destroy_texture(ctx, irradiance);
                }
                return Err(e);
            }
        };

        let sampler = match RhiDevice::create_sampler(
            ctx,
            &SamplerDesc {
                // LINEAR octahedral filtering; `compare: None` ⇒ a NON-comparison sampler
                // (`compareEnable == VK_FALSE`) — the resolve's plain `SampleLevel` is UB with a
                // comparison sampler (the I0a VUID trap). Clamp-to-edge (border wrap-copy is I7).
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: AddressMode::ClampToEdge,
                mip: MipMode::None,
                compare: None,
            },
        ) {
            Ok(s) => s,
            Err(e) => {
                // SAFETY: the buffer + both textures were just created on `ctx`, owned exclusively
                // here, never submitted; destroy each once on this edge.
                unsafe {
                    RhiDevice::destroy_buffer(ctx, classification);
                    RhiDevice::destroy_texture(ctx, depth);
                    RhiDevice::destroy_texture(ctx, irradiance);
                }
                return Err(e);
            }
        };

        let atlas = Self { irradiance, depth, classification, sampler };

        // Boot-clear all three + boot-transition the atlases to SHADER_READ_ONLY_OPTIMAL. On
        // failure tear everything down.
        if let Err(e) = atlas.boot_clear_and_transition(ctx) {
            // SAFETY: the resource's objects were just created on `ctx`, owned exclusively here;
            // the boot submit (if any) is fence-waited or never happened; `destroy` moves each by
            // value ⇒ destroyed once.
            unsafe { atlas.destroy(ctx) };
            return Err(e);
        }

        Ok(atlas)
    }

    /// The IRRADIANCE atlas (borrowed) — bound at resolve binding 16 with [`Self::sampler`].
    #[inline]
    pub fn irradiance(&self) -> &VulkanTexture {
        &self.irradiance
    }

    /// The DEPTH/visibility atlas (borrowed) — bound at resolve binding 17 with [`Self::sampler`].
    #[inline]
    pub fn depth(&self) -> &VulkanTexture {
        &self.depth
    }

    /// The dedicated LINEAR, non-comparison sampler (borrowed) — bundled with BOTH atlases as
    /// their combined image+sampler at bindings 16/17.
    #[inline]
    pub fn sampler(&self) -> &VulkanSampler {
        &self.sampler
    }

    /// The per-probe classification buffer (borrowed) — the I2 compute write target / I3 resolve
    /// converged-bit gate. Not bound at I1.
    #[inline]
    pub fn classification(&self) -> &BoundBuffer {
        &self.classification
    }

    /// Boot-clears the two atlases to `0` and the classification buffer to `0` (unconverged),
    /// then boot-transitions both atlases `UNDEFINED`→`TRANSFER_DST_OPTIMAL`→(clear)→
    /// `SHADER_READ_ONLY_OPTIMAL` — fence-waited, before the first frame. The encoder + fence
    /// are setup-class transients torn down here (the `MeshSdfTexture::upload_region` boot-submit
    /// shape).
    fn boot_clear_and_transition(&self, ctx: &VulkanContext) -> Result<(), VulkanError> {
        let mut encoder = RhiDevice::create_command_encoder(ctx)?;
        let fence = match RhiDevice::create_fence(ctx, false) {
            Ok(f) => f,
            Err(e) => {
                // SAFETY: `encoder` was just created on `ctx`, never submitted; destroy once.
                unsafe { RhiDevice::destroy_command_encoder(ctx, encoder) };
                return Err(e);
            }
        };

        // The full multi-layer COLOR range covering all `DDGI_ATLAS_LAYERS` array layers (both
        // atlases share the layer count).
        let full_range = ImageSubresourceRange {
            aspect: ImageAspect::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: DDGI_ATLAS_LAYERS,
        };

        let record = (|| -> Result<(), VulkanError> {
            encoder.begin()?;

            // Both atlases: UNDEFINED → TRANSFER_DST_OPTIMAL (the clear destination; a fresh
            // image has no prior contents, so UNDEFINED discards).
            for tex in [&self.irradiance, &self.depth] {
                encoder.image_barrier(&ImageBarrierDesc {
                    texture: tex,
                    src_stage: BarrierStage::TOP_OF_PIPE,
                    dst_stage: BarrierStage::TRANSFER,
                    src_access: BarrierAccess::NONE,
                    dst_access: BarrierAccess::TRANSFER_WRITE,
                    old_layout: ImageLayout::Undefined,
                    new_layout: ImageLayout::TransferDstOptimal,
                    range: full_range,
                });
            }

            // Clear both atlases to 0 (irradiance = 0 linear black; depth = 0 mean/mean²). An
            // unconverged probe (classification bit1 == 0) is gated OUT of the resolve, so 0 is a
            // safe init that never reads as coverage.
            encoder.clear_color_image(
                &self.irradiance,
                ImageLayout::TransferDstOptimal,
                [0.0; 4],
                full_range,
            );
            encoder.clear_color_image(
                &self.depth,
                ImageLayout::TransferDstOptimal,
                [0.0; 4],
                full_range,
            );

            // Clear the classification buffer to 0 (every probe unconverged/inactive-until-proven
            // — the resolve/feedback sky-ambient fallback anchor). `fill_buffer` covers the whole
            // `DDGI_PROBE_COUNT * 4`-byte (1 u32/probe) buffer.
            encoder.fill_buffer(&self.classification, 0);

            // Both atlases: TRANSFER_DST_OPTIMAL → SHADER_READ_ONLY_OPTIMAL (the resolve read
            // layout). The resolve samples from the COMPUTE stage (the deferred resolve is a
            // compute dispatch), so make the clear writes available to COMPUTE_SHADER — mirrors
            // the `gCsm`/`gShadowAtlas` boot transition's TRANSFER→COMPUTE make-available.
            for tex in [&self.irradiance, &self.depth] {
                encoder.image_barrier(&ImageBarrierDesc {
                    texture: tex,
                    src_stage: BarrierStage::TRANSFER,
                    dst_stage: BarrierStage::COMPUTE_SHADER,
                    src_access: BarrierAccess::TRANSFER_WRITE,
                    dst_access: BarrierAccess::SHADER_READ,
                    old_layout: ImageLayout::TransferDstOptimal,
                    new_layout: ImageLayout::ShaderReadOnlyOptimal,
                    range: full_range,
                });
            }

            encoder.end()?;
            let queue = ctx.rhi_queue();
            queue.submit(&encoder, &fence)?;
            RhiDevice::wait_fence(ctx, &fence, u64::MAX)?;
            Ok(())
        })();

        // Tear down the setup-class transients. The submit (if it ran) is fence-waited.
        // SAFETY: encoder/fence were created on `ctx`; the encoder's only submission (if any)
        // completed (fence-waited above on the Ok path, or never submitted on an error path),
        // and each is moved by value ⇒ destroyed exactly once.
        unsafe {
            RhiDevice::destroy_command_encoder(ctx, encoder);
            RhiDevice::destroy_fence(ctx, fence);
        }
        record
    }

    /// Tears down the atlases + classification buffer + sampler, consuming `self`. The caller
    /// has drained the device (`wait_idle`) so no submission still samples them.
    ///
    /// # Safety
    ///
    /// `ctx` must be the live context these were created on; the GPU is idle / drained (no work
    /// references them), and the by-value `self` destroys each object exactly once. Reverse
    /// creation order (sampler → classification → depth → irradiance).
    pub unsafe fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: per the contract `ctx` is live + drained; each object was created by `create`;
        // each is moved by value ⇒ destroyed once.
        unsafe {
            RhiDevice::destroy_sampler(ctx, self.sampler);
            RhiDevice::destroy_buffer(ctx, self.classification);
            RhiDevice::destroy_texture(ctx, self.depth);
            RhiDevice::destroy_texture(ctx, self.irradiance);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the load-bearing probe → (layer, tile-origin) mapping (the contract the I2 update
    /// WRITE and the I3 resolve READ both consume) against future drift — the point-cube-drift
    /// class the module header warns about. GPU-free (pure arithmetic).
    ///
    /// Over ALL `DDGI_PROBE_COUNT` probes, for both tile edges (irradiance `8` / depth `16`),
    /// asserts: (a) the layer is in-range; (b) the tile footprint fits the per-layer atlas
    /// extent; (c) the mapping is BIJECTIVE with FULL COVERAGE — no two distinct probes share a
    /// tile footprint, and every `dx·dz` tile slot on every `dy` layer is used exactly once.
    #[test]
    fn probe_tile_origin_is_bijective_and_fully_covers_the_atlas() {
        for (tile_edge, atlas_w, atlas_h) in [
            (DDGI_IRR_TILE_EDGE, DDGI_IRR_ATLAS_WIDTH, DDGI_IRR_ATLAS_HEIGHT),
            (DDGI_DEPTH_TILE_EDGE, DDGI_DEPTH_ATLAS_WIDTH, DDGI_DEPTH_ATLAS_HEIGHT),
        ] {
            // One "seen" flag per (layer, tile-column, tile-row) slot: dy layers × dx × dz tiles.
            let tiles_per_layer = (DDGI_GRID_DIM_X * DDGI_GRID_DIM_Z) as usize;
            let slot_count = DDGI_ATLAS_LAYERS as usize * tiles_per_layer;
            let mut seen = vec![false; slot_count];

            for z in 0..DDGI_GRID_DIM_Z {
                for y in 0..DDGI_GRID_DIM_Y {
                    for x in 0..DDGI_GRID_DIM_X {
                        let (layer, ox, oy) = ddgi_probe_tile_origin(x, y, z, tile_edge);

                        // (a) layer in range.
                        assert!(
                            layer < DDGI_ATLAS_LAYERS,
                            "probe ({x},{y},{z}) layer {layer} >= {DDGI_ATLAS_LAYERS}"
                        );
                        // (b) the tile footprint fits the per-layer atlas extent.
                        assert!(
                            ox + tile_edge <= atlas_w,
                            "probe ({x},{y},{z}) tile x [{ox}..{}] exceeds atlas width {atlas_w}",
                            ox + tile_edge
                        );
                        assert!(
                            oy + tile_edge <= atlas_h,
                            "probe ({x},{y},{z}) tile y [{oy}..{}] exceeds atlas height {atlas_h}",
                            oy + tile_edge
                        );
                        // The origin sits on a tile boundary (so the slot index is exact).
                        assert_eq!(ox % tile_edge, 0, "tile origin x not tile-aligned");
                        assert_eq!(oy % tile_edge, 0, "tile origin y not tile-aligned");

                        // (c) bijectivity: the (layer, tile-col, tile-row) slot is claimed once.
                        let col = (ox / tile_edge) as usize;
                        let row = (oy / tile_edge) as usize;
                        let slot = layer as usize * tiles_per_layer
                            + row * DDGI_GRID_DIM_X as usize
                            + col;
                        assert!(
                            !seen[slot],
                            "probe ({x},{y},{z}) collides on tile slot {slot} (tile_edge {tile_edge})"
                        );
                        seen[slot] = true;
                    }
                }
            }

            // Full coverage: every tile slot on every layer was claimed exactly once.
            assert!(
                seen.iter().all(|&s| s),
                "not every atlas tile slot is covered (tile_edge {tile_edge})"
            );
            assert_eq!(
                seen.iter().filter(|&&s| s).count(),
                DDGI_PROBE_COUNT as usize,
                "the claimed tile count must equal the probe count"
            );
        }
    }
}
