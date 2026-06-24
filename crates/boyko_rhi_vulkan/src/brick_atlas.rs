//! The SDF brick-atlas campaign M2/M3 GPU atlas — a render-Resource-owned 3D `R8_SNORM`
//! (or `R16_SFLOAT` fallback) image, its trilinear/clamp sampler, the CPU-baked staged upload,
//! and (M3) the INCREMENTAL dirty-brick rebake.
//!
//! The atlas is a dense `M2_ATLAS_DIM³` tile-grid: one apron'd `BRICK_ALLOC³` (10³) tile
//! per M2 grid cell, baked CPU-side from the ONE edit authority ([`SdfEditField`]) via
//! [`bake_brick_atlas`](crate::compute::bake_brick_atlas) and uploaded through a host-visible
//! PERSISTENT staging buffer (retained across rebakes — M3) + a single fenced
//! `TRANSFER_DST`→`SHADER_READ` transition (the SAME staged-upload shape as the UI MSDF atlas,
//! generalized to a 3D image).
//!
//! # M2 vs M3
//!
//! - [`BrickAtlas::create`] / [`BrickAtlas::rebake`] (M2) bake + upload the WHOLE 40³ atlas — the
//!   initial bake and the full-rebuild fallback.
//! - [`BrickAtlas::rebake_dirty`] (M3) re-bakes ONLY the cells whose edits changed (the authority's
//!   `aabbs` vs `prev_aabb` dirty ledger) and uploads ONLY the dirty cell-bounding-box via a
//!   SUB-REGION `copy_buffer_to_image` — the dynamic-edit enabler. After it the atlas is
//!   BIT-IDENTICAL to a full `rebake` (the persistent staging holds the un-dirtied tiles).
//!
//! # Principle 0
//!
//! The atlas is a TRANSIENT GPU mirror of the analytic field — baked from the authority each
//! regen, owning no durable per-entity state (like the M1 pointer grid / the GPU edit list). The
//! dirty set is DERIVED from the authority (`SdfEditField::prev_aabb`), not a side ledger. The
//! CPU-contiguous staging buffer + the VRAM 3D image are the legitimate FFI/GPU-contiguity
//! exception, not a parallel data system.
//!
//! # Barrier choice (W5)
//!
//! M3 keeps the M2 gated-on-`gen` FENCED submit: the incremental win is the SMALLER upload (the
//! dirty cell box, not the full 40³), not a barrier elision. Each rebake records a fresh
//! `SHADER_READ`/`UNDEFINED`→`TRANSFER_DST`→`SHADER_READ` cycle scoped to the dirty subresource;
//! the caller has drained any prior sampling submit. An in-encoder per-frame barrier (no fence,
//! no drain) is a later refinement (it needs the render encoder to thread the atlas upload into
//! the frame graph) — noted, NOT done here.

use boyko_rhi::{
    BufferDesc, BufferImageCopy, BufferUsage, Filter, Format, ImageAspect, ImageBarrierDesc,
    ImageLayout, ImageSubresourceRange, ImageUsage, MemoryLocation, MipMode, RhiCommandEncoder,
    RhiDevice, RhiQueue, SamplerDesc, TextureDesc, TextureDimension,
};
use boyko_rhi::enums::{BarrierAccess, BarrierStage};

use crate::compute::{
    AtlasEncoding, BrickLevelParams, M2_ATLAS_DIM, M4GridParams, atlas_voxel_index,
    bake_brick_atlas_at, m2_dirty_cell_bbox_at, m2_tile_atlas_origin, rebake_dirty_brick_atlas_at,
};
use crate::device::VulkanContext;
use crate::error::VulkanError;
use crate::ffi::VkResult;
use crate::memory::BoundBuffer;
use crate::rhi_impl::VulkanSampler;
use crate::texture::VulkanTexture;

use boyko_sdf_math::SdfEditField;
use boyko_sdf_math::brick::{self, BRICK_ALLOC, PointerGrid, build_pointer_grid};

/// The M2/M3 brick atlas: a `VK_IMAGE_TYPE_3D` `TRANSFER_DST | SAMPLED` image (NOT storage — the
/// M2 fill is CPU-side; a GPU compute fill is a later step), its NEAREST / clamp-to-edge / no-mip
/// sampler (BUG-M2-GPU-1: the M2 cubic point-samples exact texel corners, not a trilinear blend),
/// the chosen voxel [`AtlasEncoding`] (`R8_SNORM`, or `R16_SFLOAT` when the device cannot sample
/// `R8_SNORM`), and a PERSISTENT host-visible staging buffer (M3 — retained across rebakes so the
/// incremental dirty rebake keeps the un-dirtied tiles' bytes).
///
/// Owned by value; torn down through [`BrickAtlas::destroy`] (the caller has drained the device
/// so no submission still samples it). Not `Copy`/`Clone`: the move encodes "destroyed once".
pub struct BrickAtlas {
    /// The `M2_ATLAS_DIM³` 3D atlas image (`TRANSFER_DST | SAMPLED`). The marcher fetches it
    /// with the sampler; the bake fills it (full or incremental).
    texture: VulkanTexture,
    /// The NEAREST (point), clamp-to-edge, NO-MIP sampler the marcher's corner fetch uses
    /// (BUG-M2-GPU-1: the M2 DDA cubic reads the EXACT decoded texel corners, bit-matching the
    /// host `decode_snorm8`; a LINEAR filter would blend neighbours and drift the cubic. Clamp
    /// keeps an apron-edge fetch reading the edge texel, not a neighbour wrap).
    sampler: VulkanSampler,
    /// The PERSISTENT host-visible staging buffer holding the LAST full atlas bytes (M3). Reused
    /// across rebakes: the incremental dirty rebake patches only the dirty cells' bytes here, so
    /// the un-dirtied tiles keep their prior values and the dirty sub-region upload stays
    /// byte-identical to a full re-bake. `encoding.atlas_byte_size()` bytes.
    staging: BoundBuffer,
    /// The chosen voxel encoding (mirrors [`crate::device::DeviceCaps::atlas_format`]).
    encoding: AtlasEncoding,
}

impl BrickAtlas {
    /// Creates the M2 atlas image + sampler + persistent staging buffer for the device's chosen
    /// [`AtlasEncoding`] and bakes + uploads the FULL atlas from `field` once. On any partial
    /// failure every object created so far is torn down before the error returns.
    ///
    /// The image is `M2_ATLAS_DIM³`, `Format::R8Snorm` (or `Format::R16Sfloat` for the fallback),
    /// `D3` dimension, `TRANSFER_DST | SAMPLED` usage. The sampler is `Nearest` / `ClampToEdge` /
    /// no-mip (BUG-M2-GPU-1).
    pub fn create(ctx: &VulkanContext, field: &SdfEditField) -> Result<Self, VulkanError> {
        Self::create_at_level(ctx, field, &BrickLevelParams::m2_near_field())
    }

    /// Creates the atlas image + sampler + persistent staging and bakes + uploads the FULL atlas at
    /// ONE clip-map level's [`BrickLevelParams`] (M4) — the per-level sibling of [`create`](Self::create)
    /// (which delegates here at the level-0 [`BrickLevelParams::m2_near_field`]). The image GEOMETRY
    /// (`M2_ATLAS_DIM³`, the encoding) is level-invariant; only `params` (origin/brick_world/voxel/band)
    /// differs per level, so the M4 clip-map creates `N` of these — one per level — over the SAME baker.
    pub fn create_at_level(
        ctx: &VulkanContext,
        field: &SdfEditField,
        params: &BrickLevelParams,
    ) -> Result<Self, VulkanError> {
        let encoding =
            AtlasEncoding::from_linear_filter_ok(ctx.device_caps().atlas_linear_filter_ok);
        let format = match encoding {
            AtlasEncoding::Snorm8 => Format::R8Snorm,
            AtlasEncoding::Sfloat16 => Format::R16Sfloat,
        };

        let texture = RhiDevice::create_texture(
            ctx,
            &TextureDesc {
                width: M2_ATLAS_DIM,
                height: M2_ATLAS_DIM,
                depth: M2_ATLAS_DIM,
                format,
                dimension: TextureDimension::D3,
                usage: ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST,
            },
        )?;

        let sampler = match RhiDevice::create_sampler(
            ctx,
            &SamplerDesc {
                // NEAREST (point) — BUG-M2-GPU-1. The M2 DDA cubic needs the EXACT texel corner
                // values (bit-matching the host's integer-index `decode_snorm8`), NOT trilinear
                // interpolation; the marcher point-samples each corner at the texel center
                // `(texel + 0.5)/atlas_dim`. A LINEAR filter would blend neighbours and drift the
                // 8 cubic corners off the host fetch (the degenerate-cubic / dead-branch failure).
                mag_filter: Filter::Nearest,
                min_filter: Filter::Nearest,
                address_mode: boyko_rhi::AddressMode::ClampToEdge,
                mip: MipMode::None,
            },
        ) {
            Ok(s) => s,
            Err(e) => {
                // SAFETY: `texture` was just created on `ctx`, owned exclusively here, never
                // submitted; destroy it once on this edge.
                unsafe { RhiDevice::destroy_texture(ctx, texture) };
                return Err(e);
            }
        };

        // The persistent staging buffer (M3): retained on the atlas, reused by every rebake.
        let staging = match RhiDevice::create_buffer(
            ctx,
            &BufferDesc {
                size: encoding.atlas_byte_size() as u64,
                usage: BufferUsage::TRANSFER_SRC,
                location: MemoryLocation::HostVisibleCoherent,
            },
        ) {
            Ok(b) => b,
            Err(e) => {
                // SAFETY: `sampler` + `texture` were just created on `ctx`, owned exclusively
                // here, never submitted; destroy each once on this edge.
                unsafe {
                    RhiDevice::destroy_sampler(ctx, sampler);
                    RhiDevice::destroy_texture(ctx, texture);
                }
                return Err(e);
            }
        };

        let atlas = Self { texture, sampler, staging, encoding };

        // Bake the FULL atlas into the persistent staging + upload the whole image once
        // (`UNDEFINED`→`TRANSFER_DST`→`SHADER_READ`). On failure tear everything down.
        if let Err(e) = atlas.bake_full_and_upload(ctx, field, params) {
            // SAFETY: the atlas's objects were just created on `ctx`, owned exclusively here; the
            // upload submit (if any) is fence-waited or never happened; `destroy` moves each by
            // value ⇒ destroyed once.
            unsafe { atlas.destroy(ctx) };
            return Err(e);
        }

        Ok(atlas)
    }

    /// Re-bakes the FULL atlas from `field` (the full-rebuild fallback / initial-bake path) and
    /// re-uploads the whole image, fence-waited. The image + sampler + staging are reused; the
    /// caller MUST have drained any prior submission that samples the atlas (this records a fresh
    /// `UNDEFINED`→`TRANSFER_DST`→`SHADER_READ` cycle, discarding the old contents).
    ///
    /// Prefer [`rebake_dirty`](Self::rebake_dirty) for a dynamic edit (it re-bakes + uploads ONLY
    /// the changed cells); use `rebake` for the first bake or a full rebuild.
    pub fn rebake(&self, ctx: &VulkanContext, field: &SdfEditField) -> Result<(), VulkanError> {
        self.bake_full_and_upload(ctx, field, &BrickLevelParams::m2_near_field())
    }

    /// Re-bakes + re-uploads the FULL atlas at ONE clip-map level's [`BrickLevelParams`] (M4) — the
    /// per-level sibling of [`rebake`](Self::rebake) (which delegates here at the level-0
    /// [`BrickLevelParams::m2_near_field`]). The image/sampler/staging are reused; the caller MUST
    /// have drained any prior sampling submit (a fresh `UNDEFINED`→`TRANSFER_DST`→`SHADER_READ`
    /// cycle). The M4 `gen`-changed fallback re-snaps + re-bakes every level via this.
    pub fn rebake_at_level(
        &self,
        ctx: &VulkanContext,
        field: &SdfEditField,
        params: &BrickLevelParams,
    ) -> Result<(), VulkanError> {
        self.bake_full_and_upload(ctx, field, params)
    }

    /// Incrementally re-bakes ONLY the dirty cells (M3 — the dynamic-edit fast path).
    ///
    /// The authority's dirty set ([`m2_dirty_cell_bbox`], the swept old+new union of every edit
    /// whose `aabbs[i] != prev_aabb[i]`) is patched into the persistent staging
    /// ([`rebake_dirty_brick_atlas`]), then ONLY the dirty cell-bounding-box's atlas voxels are
    /// uploaded via a SUB-REGION `copy_buffer_to_image` (one `BufferImageCopy` with the box's
    /// `image_offset`/`image_extent`). The un-dirtied tiles keep their prior staging bytes, so the
    /// GPU atlas stays BIT-IDENTICAL to a full [`rebake`](Self::rebake).
    ///
    /// Returns `true` when an upload ran, `false` when no edit was dirty (the atlas is already
    /// current — the caller skips, no submit). The caller should
    /// [`SdfEditField::clear_dirty`](boyko_sdf_math::SdfEditField::clear_dirty) the authority after
    /// a `true` so the next mutation diffs against the freshly-baked state.
    ///
    /// The caller MUST have drained any prior submission that samples the atlas (this transitions
    /// the dirty subresource `SHADER_READ`→`TRANSFER_DST`→`SHADER_READ`).
    pub fn rebake_dirty(
        &self,
        ctx: &VulkanContext,
        field: &SdfEditField,
    ) -> Result<bool, VulkanError> {
        self.rebake_dirty_at_level(ctx, field, &BrickLevelParams::m2_near_field())
    }

    /// Incrementally re-bakes ONLY the dirty cells at ONE clip-map level's [`BrickLevelParams`] (M4
    /// — the per-level sibling of [`rebake_dirty`](Self::rebake_dirty), which delegates here at the
    /// level-0 [`BrickLevelParams::m2_near_field`]). The level diffs the SAME authority against its
    /// OWN grid ([`m2_dirty_cell_bbox_at`]) and patches + uploads only that level's dirty cell box.
    /// Returns `true` when an upload ran, `false` when no edit was dirty for this level's grid.
    pub fn rebake_dirty_at_level(
        &self,
        ctx: &VulkanContext,
        field: &SdfEditField,
        params: &BrickLevelParams,
    ) -> Result<bool, VulkanError> {
        let Some((lo, hi)) = m2_dirty_cell_bbox_at(field, params) else {
            return Ok(false); // No edit dirty (or wholly outside the grid): atlas already current.
        };

        // Patch ONLY the dirty cell box into the persistent staging (the un-dirtied tiles keep
        // their prior bytes — full/incremental parity).
        let size = self.encoding.atlas_byte_size();
        let Some(dst) = RhiDevice::buffer_mapped_ptr(ctx, &self.staging) else {
            return Err(VulkanError::Vk(
                "brick_atlas staging buffer not host-mapped",
                VkResult::ERROR_INITIALIZATION_FAILED,
            ));
        };
        // SAFETY: `dst` is the persistently-mapped first byte of the host-coherent staging buffer
        // (exactly `size` bytes, allocated in `create`); the slice covers exactly those bytes; this
        // is the unique writer before the upload binds the buffer (the caller has drained any prior
        // sampling submit). Host-coherent ⇒ no flush.
        let staging_bytes = unsafe { core::slice::from_raw_parts_mut(dst.as_ptr(), size) };
        rebake_dirty_brick_atlas_at(field, self.encoding, params, lo, hi, staging_bytes);

        // Upload ONLY the dirty cell-bounding-box sub-region.
        let region = Self::dirty_copy_region(self.encoding, lo, hi);
        self.upload_region(ctx, &region, true)?;
        Ok(true)
    }

    /// The chosen voxel encoding (for the host UBO / smoke checks).
    #[inline]
    pub fn encoding(&self) -> AtlasEncoding {
        self.encoding
    }

    /// The atlas image (borrowed) — the marcher binds this at `t10` in the M2 shader step.
    #[inline]
    pub fn texture(&self) -> &VulkanTexture {
        &self.texture
    }

    /// The atlas sampler (borrowed) — the marcher binds this at `s1` in the M2 shader step.
    #[inline]
    pub fn sampler(&self) -> &VulkanSampler {
        &self.sampler
    }

    /// Bakes the FULL atlas from `field` into the persistent staging and uploads the whole image,
    /// fence-waited, transitioning `UNDEFINED`→`TRANSFER_DST_OPTIMAL`→`SHADER_READ_ONLY_OPTIMAL`.
    fn bake_full_and_upload(
        &self,
        ctx: &VulkanContext,
        field: &SdfEditField,
        params: &BrickLevelParams,
    ) -> Result<(), VulkanError> {
        let size = self.encoding.atlas_byte_size();
        let Some(dst) = RhiDevice::buffer_mapped_ptr(ctx, &self.staging) else {
            return Err(VulkanError::Vk(
                "brick_atlas staging buffer not host-mapped",
                VkResult::ERROR_INITIALIZATION_FAILED,
            ));
        };
        // SAFETY: `dst` is the persistently-mapped first byte of the host-coherent staging buffer
        // (exactly `size` bytes, allocated in `create`); the slice covers exactly those bytes; this
        // is the unique writer before the upload binds the buffer. Host-coherent ⇒ no flush.
        let staging_bytes = unsafe { core::slice::from_raw_parts_mut(dst.as_ptr(), size) };
        let _surface_cells = bake_brick_atlas_at(field, self.encoding, params, staging_bytes);

        // One tightly-packed FULL-atlas 3D copy region (`image_extent_d = M2_ATLAS_DIM`).
        let region = BufferImageCopy {
            buffer_offset: 0,
            buffer_row_length: 0,
            buffer_image_height: 0,
            aspect: ImageAspect::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
            image_offset_x: 0,
            image_offset_y: 0,
            image_offset_z: 0,
            image_extent_w: M2_ATLAS_DIM,
            image_extent_h: M2_ATLAS_DIM,
            image_extent_d: M2_ATLAS_DIM,
        };
        // A FULL bake has no defined prior contents to preserve: discard via `UNDEFINED`.
        self.upload_region(ctx, &region, false)
    }

    /// The single sub-region [`BufferImageCopy`] covering the inclusive M2 cell box `(lo, hi)` —
    /// the M3 dirty upload. The box's tiles occupy the CONTIGUOUS atlas voxel range
    /// `[lo*BRICK_ALLOC, (hi+1)*BRICK_ALLOC)` on each axis; `buffer_row_length`/
    /// `buffer_image_height` are set to the FULL atlas dims so the source reads the box's bytes
    /// from their existing position in the persistent staging (no re-pack), and `buffer_offset` is
    /// the byte offset of the box's min atlas voxel.
    fn dirty_copy_region(
        encoding: AtlasEncoding,
        lo: [u32; 3],
        hi: [u32; 3],
    ) -> BufferImageCopy {
        let [ox, oy, oz] = m2_tile_atlas_origin(lo);
        // The box's voxel extent: `(hi - lo + 1)` cells, each `BRICK_ALLOC` voxels wide.
        let ext = |a: usize| (hi[a] - lo[a] + 1) * BRICK_ALLOC as u32;
        // The byte offset of the box's min atlas voxel; the source rows/slices are addressed at the
        // FULL atlas stride (`buffer_row_length`/`buffer_image_height` = `M2_ATLAS_DIM`).
        let buffer_offset =
            (atlas_voxel_index(ox, oy, oz) * encoding.bytes_per_voxel()) as u64;
        BufferImageCopy {
            buffer_offset,
            buffer_row_length: M2_ATLAS_DIM,
            buffer_image_height: M2_ATLAS_DIM,
            aspect: ImageAspect::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
            image_offset_x: ox as i32,
            image_offset_y: oy as i32,
            image_offset_z: oz as i32,
            image_extent_w: ext(0),
            image_extent_h: ext(1),
            image_extent_d: ext(2),
        }
    }

    /// Records + submits the staged copy of `region` from the persistent staging into the atlas
    /// image, fence-waited. `from_shader_read` chooses the source layout/scope of the pre-copy
    /// barrier: `false` for a FULL bake (no prior contents — `UNDEFINED`, discard); `true` for an
    /// incremental dirty upload (the image was last left `SHADER_READ_ONLY_OPTIMAL` by the previous
    /// bake, and the un-touched voxels OUTSIDE `region` MUST be preserved, so the barrier
    /// transitions FROM `SHADER_READ` rather than discarding via `UNDEFINED`). Both end
    /// `SHADER_READ_ONLY_OPTIMAL` (sample-ready, no per-frame barrier). The encoder + fence are
    /// setup-class transients torn down here; the staging is the persistent atlas member (kept).
    fn upload_region(
        &self,
        ctx: &VulkanContext,
        region: &BufferImageCopy,
        from_shader_read: bool,
    ) -> Result<(), VulkanError> {
        let mut encoder = RhiDevice::create_command_encoder(ctx)?;
        let fence = match RhiDevice::create_fence(ctx, false) {
            Ok(f) => f,
            Err(e) => {
                // SAFETY: `encoder` was just created on `ctx`, never submitted; destroy once.
                unsafe { RhiDevice::destroy_command_encoder(ctx, encoder) };
                return Err(e);
            }
        };

        // A FULL bake discards the prior contents (UNDEFINED). An incremental upload preserves the
        // voxels outside `region`, so it transitions FROM the SHADER_READ state the prior bake left
        // the whole image in (NOT UNDEFINED, which the driver may treat as discard-all).
        let (src_stage, src_access, old_layout) = if from_shader_read {
            (
                BarrierStage::COMPUTE_SHADER,
                BarrierAccess::SHADER_READ,
                ImageLayout::ShaderReadOnlyOptimal,
            )
        } else {
            (BarrierStage::TOP_OF_PIPE, BarrierAccess::NONE, ImageLayout::Undefined)
        };

        let region = *region;
        let record = (|| -> Result<(), VulkanError> {
            encoder.begin()?;
            // → TRANSFER_DST_OPTIMAL (the copy destination).
            encoder.image_barrier(&ImageBarrierDesc {
                texture: &self.texture,
                src_stage,
                dst_stage: BarrierStage::TRANSFER,
                src_access,
                dst_access: BarrierAccess::TRANSFER_WRITE,
                old_layout,
                new_layout: ImageLayout::TransferDstOptimal,
                range: ImageSubresourceRange::COLOR,
            });
            encoder.copy_buffer_to_image(
                &self.staging,
                &self.texture,
                ImageLayout::TransferDstOptimal,
                core::slice::from_ref(&region),
            );
            // TRANSFER_DST_OPTIMAL → SHADER_READ_ONLY_OPTIMAL (sample-ready). The M2 marcher
            // fetches from the COMPUTE stage, so make the writes available to COMPUTE_SHADER.
            encoder.image_barrier(&ImageBarrierDesc {
                texture: &self.texture,
                src_stage: BarrierStage::TRANSFER,
                dst_stage: BarrierStage::COMPUTE_SHADER,
                src_access: BarrierAccess::TRANSFER_WRITE,
                dst_access: BarrierAccess::SHADER_READ,
                old_layout: ImageLayout::TransferDstOptimal,
                new_layout: ImageLayout::ShaderReadOnlyOptimal,
                range: ImageSubresourceRange::COLOR,
            });
            encoder.end()?;
            let queue = ctx.rhi_queue();
            queue.submit(&encoder, &fence)?;
            RhiDevice::wait_fence(ctx, &fence, u64::MAX)?;
            Ok(())
        })();

        // Tear down the setup-class transients (NOT the persistent staging). The submit (if it
        // ran) is fence-waited.
        // SAFETY: encoder/fence were created on `ctx`; the encoder's only submission (if any)
        // completed (fence-waited above on the Ok path, or never submitted on an error path), and
        // each is moved by value ⇒ destroyed exactly once.
        unsafe {
            RhiDevice::destroy_command_encoder(ctx, encoder);
            RhiDevice::destroy_fence(ctx, fence);
        }
        record
    }

    /// Tears down the atlas (image + sampler + staging), consuming `self`. The caller has drained
    /// the device (`wait_idle`) so no submission still samples it.
    ///
    /// # Safety
    ///
    /// `ctx` must be the live context the atlas was created on; the GPU is idle / drained (no
    /// work references the image or staging), and the by-value `self` destroys each object exactly
    /// once.
    pub unsafe fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: per the contract `ctx` is live + drained; the image + sampler + staging were
        // created by `create`; each is moved by value ⇒ destroyed once.
        unsafe {
            RhiDevice::destroy_texture(ctx, self.texture);
            RhiDevice::destroy_sampler(ctx, self.sampler);
            RhiDevice::destroy_buffer(ctx, self.staging);
        }
    }
}

/// The geometry of one clip-map level's EMPTY-SKIP pointer grid (binding 9/11/13) — DISTINCT from the
/// level's SURFACE-atlas grid (the `M2_GRID_DIM³ @ params.brick_world` lattice the atlas tiles map). The
/// empty-skip grid and the surface grid are kept separate exactly as M2 does (the conflation was
/// BUG-M4-SLICE-C-1: the N=3 near field stopped reducing to the N=1 near field).
///
/// - **Level 0** is the FINE [`PointerGrid::default_near_field`] (`DEFAULT_GRID_DIM³ @ DEFAULT_BRICK_WORLD`
///   = `16³ @ 0.5`, origin `[-4, -4, -4]`) — the SAME grid the single-level M2/N=1 path binds at binding 9
///   and the shader's `lvl == 0` arm reads via `pc.grid_*`. Binding the clip-map's level-0 grid at the
///   SURFACE granularity (`4³ @ 2.0`) instead made the `pc.grid_*` (16³) index a 64-cell SSBO out of
///   bounds, so the GPU level-0 empty-skip diverged from N=1 (the near-field-golden failure). Returning
///   `default_near_field` here keeps `grid_buffer(0)` consistent with `pc.grid_*` and the host oracle.
/// - **Levels ≥ 1** use the per-level COARSE grid (`M2_GRID_DIM³` cells of `params.brick_world` from the
///   snapped `params.origin`) — the geometry the shader's coarse arms read via `m2_levels[L]`, so the SSBO
///   the GPU binds at binding 11/13 aligns with the params the shader uses.
#[inline]
fn level_empty_skip_grid(params: &BrickLevelParams, level: usize) -> PointerGrid {
    if level == 0 {
        PointerGrid::default_near_field()
    } else {
        PointerGrid {
            origin: params.origin,
            dims: [brick::M2_GRID_DIM, brick::M2_GRID_DIM, brick::M2_GRID_DIM],
            brick_world: params.brick_world,
        }
    }
}

/// Creates ONE clip-map level's EMPTY-SKIP pointer-grid StorageBuffer (binding 9/11/13) and seeds it
/// from the authority via [`build_pointer_grid`] at the level's [`level_empty_skip_grid`] geometry —
/// level 0 the FINE `16³ @ 0.5` near-field grid (consistent with `pc.grid_*` + the host oracle), coarse
/// levels the `M2_GRID_DIM³ @ params.brick_world` grid the shader's coarse arms read. The host-visible
/// coherent buffer holds `cell_count` `u32` [`BrickClass`](boyko_sdf_math::BrickClass) codes — the SAME
/// `StructuredBuffer<uint>` the shader (Slice C) reads per level.
fn create_level_grid(
    ctx: &VulkanContext,
    field: &SdfEditField,
    params: &BrickLevelParams,
    level: usize,
) -> Result<BoundBuffer, VulkanError> {
    let grid = level_empty_skip_grid(params, level);
    let mut cells = vec![0u32; grid.cell_count()];
    build_pointer_grid(field, &grid, &mut cells);

    let buffer = RhiDevice::create_buffer(
        ctx,
        &BufferDesc {
            size: (cells.len() * core::mem::size_of::<u32>()) as u64,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )?;
    write_grid_cells(ctx, &buffer, &cells)?;
    Ok(buffer)
}

/// Re-seeds an EXISTING level EMPTY-SKIP grid SSBO from the authority (the `gen`-changed full re-bake).
/// Re-runs [`build_pointer_grid`] at the level's [`level_empty_skip_grid`] geometry (level 0 the fixed
/// fine `16³ @ 0.5` near-field grid; coarse levels the re-snapped `params`) and overwrites the
/// host-coherent buffer. The buffer is reused (no re-allocation); the caller has drained any prior
/// sampling submit.
fn reseed_level_grid(
    ctx: &VulkanContext,
    field: &SdfEditField,
    params: &BrickLevelParams,
    level: usize,
    buffer: &BoundBuffer,
) -> Result<(), VulkanError> {
    let grid = level_empty_skip_grid(params, level);
    let mut cells = vec![0u32; grid.cell_count()];
    build_pointer_grid(field, &grid, &mut cells);
    write_grid_cells(ctx, buffer, &cells)
}

/// Writes the `u32` pointer-grid `cells` into the host-coherent StorageBuffer `buffer` (the GPU
/// reads them little-endian as `uint`, matching the `u32` byte image). The unique writer before the
/// shader binds the buffer (the caller has drained any prior sampling submit); host-coherent ⇒ no
/// flush.
fn write_grid_cells(
    ctx: &VulkanContext,
    buffer: &BoundBuffer,
    cells: &[u32],
) -> Result<(), VulkanError> {
    let Some(dst) = RhiDevice::buffer_mapped_ptr(ctx, buffer) else {
        return Err(VulkanError::Vk(
            "brick clipmap grid buffer not host-mapped",
            VkResult::ERROR_INITIALIZATION_FAILED,
        ));
    };
    // SAFETY: `dst` is the persistently-mapped first byte of the host-coherent grid buffer, sized to
    // `cells.len() * 4` bytes in `create_level_grid`; `cells` is exactly that many `u32`s. The
    // `u32` source is `Copy`/POD with no padding, so `cells.len()*4` bytes are a valid byte image;
    // this is the unique writer before the upload binds the buffer. Host-coherent ⇒ no flush.
    unsafe {
        core::ptr::copy_nonoverlapping(
            cells.as_ptr().cast::<u8>(),
            dst.as_ptr(),
            core::mem::size_of_val(cells),
        );
    }
    Ok(())
}

/// The M4 brick CLIP-MAP: [`brick::BRICK_LEVELS`] nested, camera-centered brick-cache levels, each
/// a full [`BrickAtlas`] (the proven M2 atlas, baked at that level's [`BrickLevelParams`]) + its own
/// pointer-grid SSBO, plus the [`M4GridParams`] b5 UBO tail (the per-level snapped origins baked in).
///
/// Level `L` uses a brick `2^L`× larger ([`brick::brick_world_at_level`]) at a voxel `2^L`× coarser,
/// reaching `2^L`× farther from the camera; the levels nest strictly (the conservative-lower-bound
/// contract holds per level — see [`boyko_sdf_math::brick`]'s per-level soundness predicates).
///
/// # Principle 0
///
/// The clip-map is a TRANSIENT GPU mirror of the ONE [`SdfEditField`] — every level is baked from
/// the SAME authority each regen (like the single M2 atlas), owning no durable per-entity state. The
/// per-level atlas image + grid SSBO are the legitimate FFI/GPU-contiguity exception, not a parallel
/// data system; no bake logic is forked (each level runs the proven [`bake_brick_atlas_at`]).
///
/// Owned by value, torn down through [`BrickClipmap::destroy`]. `!Send`/`!Sync` like the rest of the
/// Vulkan resources (no new threading: per-level fenced upload is the established M2/M3 race-free
/// discipline).
pub struct BrickClipmap {
    /// One M2 [`BrickAtlas`] per level (same `M2_ATLAS_DIM³` image — the atlas geometry is
    /// level-invariant; only the baked content differs per level's [`BrickLevelParams`]).
    atlases: [BrickAtlas; brick::BRICK_LEVELS],

    // `create`'s fallible array-init (the `MaybeUninit` slot-fill below) is sound ONLY because both
    // element types have NO drop glue: `transmute_copy` reads the assembled `Self` out by COPY,
    // leaving the source `MaybeUninit` arrays live and droppable at scope exit — a no-op today.
    // The day someone adds `impl Drop` to either type, that un-consumed source drop would re-destroy
    // every Vulkan image / sampler / buffer the moved-out `Self` already owns (a double-free), with
    // no compile warning. These guards turn that into a BUILD error at the exact hazard instead.
    // (Destruction stays manual via `destroy` / `RhiDevice::destroy_buffer`.)
    /// One pointer-grid StorageBuffer per level (the M1/M2 empty-skip grid, `M2_GRID_DIM³` cells of
    /// that level's brick_world, snapped on the camera).
    grids: [BoundBuffer; brick::BRICK_LEVELS],
    /// The b5 camera-UBO tail: the per-level [`M4LevelParams`](crate::compute::M4LevelParams) array
    /// with the snapped origins baked in (the value the levels were baked at).
    params: M4GridParams,
}

// The `create` fallible-array-init + the by-value `destroy` teardown both assume these element types
// carry NO drop glue (see the field-level note above and `create`'s `transmute_copy` SAFETY). If a
// future `impl Drop` is added to either, the source-array scope-exit drop in `create` becomes a
// double-free of every GPU handle — break the build HERE, at the exact hazard, rather than silently.
const _: () = assert!(
    !core::mem::needs_drop::<BrickAtlas>(),
    "BrickClipmap's array-init/teardown assumes BrickAtlas has no Drop glue; adding Drop makes create()'s source-array teardown a double-free"
);
const _: () = assert!(
    !core::mem::needs_drop::<BoundBuffer>(),
    "BrickClipmap's grid array-init/teardown assumes BoundBuffer has no Drop glue"
);

impl BrickClipmap {
    /// Creates the N-level clip-map: builds the [`M4GridParams`] (camera-centered snapped origins),
    /// then for each level `L` creates a [`BrickAtlas`] (baked at level `L`'s [`BrickLevelParams`])
    /// and its pointer-grid SSBO (built from the SAME authority at level `L`'s grid). On any partial
    /// failure every resource created so far is torn down before the error returns.
    ///
    /// The atlas IMAGE geometry (`M2_ATLAS_DIM³`) is level-invariant; only the per-level
    /// origin/brick_world/voxel/band (threaded via [`BrickLevelParams::at_level`]) differ — so each
    /// level reuses [`BrickAtlas::create_at_level`] verbatim over the proven baker.
    pub fn create(
        ctx: &VulkanContext,
        field: &SdfEditField,
        camera: [f32; 3],
    ) -> Result<Self, VulkanError> {
        let params = M4GridParams::camera_centered(camera);

        // Build the level resources incrementally, tearing down on the first failure. The two
        // `MaybeUninit` arrays are written slot-by-slot; `atlases[..level+1]` is initialized iff the
        // atlas at `level` was created, and `grids[..level]` (or `..level+1` once the grid is in)
        // tracks the grids — so on the atlas-OK / grid-FAIL edge the two prefixes differ by one.
        let mut atlases: [core::mem::MaybeUninit<BrickAtlas>; brick::BRICK_LEVELS] =
            [const { core::mem::MaybeUninit::uninit() }; brick::BRICK_LEVELS];
        let mut grids: [core::mem::MaybeUninit<BoundBuffer>; brick::BRICK_LEVELS] =
            [const { core::mem::MaybeUninit::uninit() }; brick::BRICK_LEVELS];

        for level in 0..brick::BRICK_LEVELS {
            let geo = BrickLevelParams::at_level(camera, level as u32);

            let atlas = match BrickAtlas::create_at_level(ctx, field, &geo) {
                Ok(a) => a,
                Err(e) => {
                    // SAFETY: `atlases[..level]` and `grids[..level]` are the fully-built resources
                    // from the prior iterations on `ctx`, owned exclusively here and never sampled;
                    // `teardown_partial` reads exactly those prefixes and destroys each once. The
                    // atlas at `level` was NOT written (this is its failure edge).
                    unsafe { Self::teardown_partial(ctx, &mut atlases, level, &mut grids, level) };
                    return Err(e);
                }
            };
            atlases[level].write(atlas);

            let grid = match create_level_grid(ctx, field, &geo, level) {
                Ok(g) => g,
                Err(e) => {
                    // SAFETY: `atlases[..level + 1]` (the atlas at `level` was just written) and
                    // `grids[..level]` (prior iterations) are fully built, owned, never sampled;
                    // destroyed once each. The grid at `level` was NOT written (its failure edge).
                    unsafe { Self::teardown_partial(ctx, &mut atlases, level + 1, &mut grids, level) };
                    return Err(e);
                }
            };
            grids[level].write(grid);
        }

        // SAFETY: every slot of both arrays was written above (the loop ran all BRICK_LEVELS
        // iterations without an early return), so both are fully initialized; `transmute_copy` reads
        // each `MaybeUninit<T>` as its now-initialized `T` (identical layout) by COPY. It does NOT
        // consume the source: the `atlases`/`grids` `MaybeUninit` arrays stay live and are dropped at
        // scope exit. That scope-exit drop is a no-op ONLY because `BrickAtlas` and `BoundBuffer` have
        // no Drop glue (destruction is manual via `destroy` / `RhiDevice::destroy_buffer`), so the
        // resources are NOT double-destroyed by the moved-out `Self` plus the source. A future
        // `impl Drop` on either would make this a double-free — guarded by the `needs_drop`
        // const-asserts above the `impl` block (and the matching note in `destroy`).
        let atlases = unsafe {
            core::mem::transmute_copy::<_, [BrickAtlas; brick::BRICK_LEVELS]>(&atlases)
        };
        let grids = unsafe {
            core::mem::transmute_copy::<_, [BoundBuffer; brick::BRICK_LEVELS]>(&grids)
        };
        Ok(Self { atlases, grids, params })
    }

    /// Re-snaps + FULLY re-bakes every level (the `gen`-changed fallback): rebuilds the
    /// [`M4GridParams`] on the new `camera`, then for each level re-bakes the atlas
    /// ([`BrickAtlas::rebake_at_level`]) and re-seeds the grid SSBO at the level's re-snapped
    /// [`BrickLevelParams`]. The caller MUST have drained any prior sampling submit.
    pub fn rebake_all(
        &mut self,
        ctx: &VulkanContext,
        field: &SdfEditField,
        camera: [f32; 3],
    ) -> Result<(), VulkanError> {
        self.params = M4GridParams::camera_centered(camera);
        for level in 0..brick::BRICK_LEVELS {
            let geo = BrickLevelParams::at_level(camera, level as u32);
            self.atlases[level].rebake_at_level(ctx, field, &geo)?;
            reseed_level_grid(ctx, field, &geo, level, &self.grids[level])?;
        }
        Ok(())
    }

    /// Incrementally re-bakes ONLY the dirty cells of every level (M3 reused per level): each level
    /// diffs the SAME authority against its OWN grid ([`BrickAtlas::rebake_dirty_at_level`]) and
    /// patches + uploads only that level's dirty cell box. Returns the TOTAL number of dirty levels
    /// that ran an upload (0 when no level was dirty — the clip-map is already current). The grid
    /// SSBOs are NOT re-seeded here (a dirty edit keeps the snapped origins; the empty-skip grid is
    /// re-seeded only on the `gen`-changed `rebake_all`).
    pub fn rebake_dirty_all(
        &self,
        ctx: &VulkanContext,
        field: &SdfEditField,
    ) -> Result<u32, VulkanError> {
        let mut dirty_levels = 0u32;
        for level in 0..brick::BRICK_LEVELS {
            let geo = BrickLevelParams::at_level_from_params(&self.params, level);
            if self.atlases[level].rebake_dirty_at_level(ctx, field, &geo)? {
                dirty_levels += 1;
            }
        }
        Ok(dirty_levels)
    }

    /// The level-`level` atlas (borrowed) — bound at the marcher's atlas slot for that level.
    #[inline]
    pub fn atlas(&self, level: usize) -> &BrickAtlas {
        &self.atlases[level]
    }

    /// The level-`level` atlas sampler (borrowed).
    #[inline]
    pub fn sampler(&self, level: usize) -> &VulkanSampler {
        self.atlases[level].sampler()
    }

    /// The level-`level` pointer-grid StorageBuffer (borrowed) — the empty-skip grid SSBO.
    #[inline]
    pub fn grid_buffer(&self, level: usize) -> &BoundBuffer {
        &self.grids[level]
    }

    /// The b5 camera-UBO tail ([`M4GridParams`]) the levels were baked at (the per-level snapped
    /// origins + scales). The Slice-C write path blits [`M4GridParams::as_ubo_bytes`] into the UBO.
    #[inline]
    pub fn params(&self) -> &M4GridParams {
        &self.params
    }

    /// Tears down every level's atlas + grid SSBO, consuming `self`. The caller has drained the
    /// device (`wait_idle`) so no submission still samples the clip-map.
    ///
    /// # Safety
    ///
    /// `ctx` must be the live context the clip-map was created on; the GPU is idle / drained (no
    /// work references any level's image, staging, or grid), and the by-value `self` destroys each
    /// resource exactly once.
    pub unsafe fn destroy(self, ctx: &VulkanContext) {
        let Self { atlases, grids, params: _ } = self;
        // SAFETY: per the contract `ctx` is live + drained; every atlas + grid was created by
        // `create`/`rebake_all`; consuming the arrays by value moves each resource out once, so
        // `BrickAtlas::destroy` / `destroy_buffer` run exactly once per resource.
        for atlas in atlases {
            unsafe { atlas.destroy(ctx) };
        }
        for grid in grids {
            unsafe { RhiDevice::destroy_buffer(ctx, grid) };
        }
    }

    /// Destroys the fully-initialized PREFIXES of the partially-built `atlases`/`grids` arrays (the
    /// `create` error-unwind). `atlases[..atlases_built]` and `grids[..grids_built]` are initialized;
    /// every slot at or beyond the built count is uninit and left untouched.
    ///
    /// # Safety
    ///
    /// The caller guarantees `atlases[..atlases_built]` and `grids[..grids_built]` hold
    /// fully-initialized, owned, never-sampled resources created on `ctx`, and that this is called
    /// exactly once on the error edge (each prefix slot is read + destroyed once).
    unsafe fn teardown_partial(
        ctx: &VulkanContext,
        atlases: &mut [core::mem::MaybeUninit<BrickAtlas>; brick::BRICK_LEVELS],
        atlases_built: usize,
        grids: &mut [core::mem::MaybeUninit<BoundBuffer>; brick::BRICK_LEVELS],
        grids_built: usize,
    ) {
        for slot in atlases.iter_mut().take(atlases_built) {
            // SAFETY: `slot` is in the initialized prefix (index < atlases_built); `assume_init_read`
            // moves the owned `BrickAtlas` out once, and `destroy` consumes it (destroyed once). The
            // slot is not read again (the prefix is visited once).
            let atlas = unsafe { slot.assume_init_read() };
            unsafe { atlas.destroy(ctx) };
        }
        for slot in grids.iter_mut().take(grids_built) {
            // SAFETY: `slot` is in the initialized prefix (index < grids_built); `assume_init_read`
            // moves the owned grid buffer out once, and `destroy_buffer` consumes it (destroyed once).
            let grid = unsafe { slot.assume_init_read() };
            unsafe { RhiDevice::destroy_buffer(ctx, grid) };
        }
    }
}
