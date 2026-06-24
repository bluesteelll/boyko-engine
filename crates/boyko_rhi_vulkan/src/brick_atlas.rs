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
    AtlasEncoding, M2_ATLAS_DIM, atlas_voxel_index, bake_brick_atlas, m2_dirty_cell_bbox,
    m2_tile_atlas_origin, rebake_dirty_brick_atlas,
};
use crate::device::VulkanContext;
use crate::error::VulkanError;
use crate::ffi::VkResult;
use crate::memory::BoundBuffer;
use crate::rhi_impl::VulkanSampler;
use crate::texture::VulkanTexture;

use boyko_sdf_math::SdfEditField;
use boyko_sdf_math::brick::BRICK_ALLOC;

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
        if let Err(e) = atlas.bake_full_and_upload(ctx, field) {
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
        self.bake_full_and_upload(ctx, field)
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
        let Some((lo, hi)) = m2_dirty_cell_bbox(field) else {
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
        rebake_dirty_brick_atlas(field, self.encoding, lo, hi, staging_bytes);

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
        let _surface_cells = bake_brick_atlas(field, self.encoding, staging_bytes);

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
