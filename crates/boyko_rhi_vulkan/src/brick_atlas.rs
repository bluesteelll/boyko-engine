//! The SDF brick-atlas campaign M2 GPU atlas — a render-Resource-owned 3D `R8_SNORM`
//! (or `R16_SFLOAT` fallback) image, its trilinear/clamp sampler, and the CPU-baked,
//! staged upload.
//!
//! The atlas is a dense `M2_ATLAS_DIM³` tile-grid: one apron'd `BRICK_ALLOC³` (10³) tile
//! per M2 grid cell, baked CPU-side from the ONE edit authority ([`SdfEditField`]) via
//! [`bake_brick_atlas`](crate::compute::bake_brick_atlas) and uploaded once per edit
//! `gen` through a host-visible staging buffer + a single fenced
//! `TRANSFER_DST`→`SHADER_READ` transition (the SAME staged-upload shape as the UI MSDF
//! atlas, generalized to a 3D image).
//!
//! This step (M2 plumbing) builds the atlas + the baker + the device probe + the UBO/push;
//! the marcher .spv that SAMPLES the atlas is the next step. The atlas is therefore created +
//! filled but NOT yet bound into the marcher descriptor set (decision (b): the t10/s1 layout
//! binding lands with the shader that references it, avoiding an unused-binding mismatch).
//!
//! # Principle 0
//!
//! The atlas is a TRANSIENT GPU mirror of the analytic field — baked from the authority each
//! regen, owning no durable per-entity state (like the M1 pointer grid / the GPU edit list).
//! The CPU-contiguous staging buffer + the VRAM 3D image are the legitimate FFI/GPU-contiguity
//! exception, not a parallel data system.

use boyko_rhi::{
    BufferDesc, BufferImageCopy, BufferUsage, Filter, Format, ImageAspect, ImageBarrierDesc,
    ImageLayout, ImageSubresourceRange, ImageUsage, MemoryLocation, MipMode, RhiCommandEncoder,
    RhiDevice, RhiQueue, SamplerDesc, TextureDesc, TextureDimension,
};
use boyko_rhi::enums::{BarrierAccess, BarrierStage};

use crate::compute::{AtlasEncoding, M2_ATLAS_DIM, bake_brick_atlas};
use crate::device::VulkanContext;
use crate::error::VulkanError;
use crate::ffi::VkResult;
use crate::rhi_impl::VulkanSampler;
use crate::texture::VulkanTexture;

use boyko_sdf_math::SdfEditField;

/// The M2 brick atlas: a `VK_IMAGE_TYPE_3D` `TRANSFER_DST | SAMPLED` image (NOT storage — the
/// M2 fill is CPU-side; a GPU compute fill is M3), its trilinear / clamp-to-edge / no-mip
/// sampler, and the chosen voxel [`AtlasEncoding`] (`R8_SNORM`, or `R16_SFLOAT` when the device
/// cannot linear-filter `R8_SNORM`).
///
/// Owned by value; torn down through [`BrickAtlas::destroy`] (the caller has drained the device
/// so no submission still samples it). Not `Copy`/`Clone`: the move encodes "destroyed once".
pub struct BrickAtlas {
    /// The `M2_ATLAS_DIM³` 3D atlas image (`TRANSFER_DST | SAMPLED`). The marcher will fetch it
    /// with the trilinear sampler in the M2 step; this step only fills it.
    texture: VulkanTexture,
    /// The trilinear, clamp-to-edge, NO-MIP sampler the marcher's hardware fetch uses (the
    /// `R8_SNORM`/`R16_SFLOAT` decode + apron read need a LINEAR filter; clamp keeps an
    /// out-of-tile fetch reading the apron, not a neighbour).
    sampler: VulkanSampler,
    /// The chosen voxel encoding (mirrors [`crate::device::DeviceCaps::atlas_format`]).
    encoding: AtlasEncoding,
}

impl BrickAtlas {
    /// Creates the M2 atlas image + sampler for the device's chosen [`AtlasEncoding`]
    /// (`AtlasEncoding::from_linear_filter_ok(ctx.device_caps().atlas_linear_filter_ok)`) and
    /// bakes + uploads the atlas from `field` once. On any partial failure every object created
    /// so far is torn down before the error returns.
    ///
    /// The image is `M2_ATLAS_DIM³`, `Format::R8Snorm` (or `Format::R16Sfloat` for the
    /// fallback), `D3` dimension, `TRANSFER_DST | SAMPLED` usage. The sampler is `Linear` /
    /// `ClampToEdge` / no-mip.
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
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
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

        if let Err(e) = Self::bake_and_upload(ctx, &texture, encoding, field) {
            // SAFETY: `sampler` + `texture` were just created on `ctx`, owned exclusively here,
            // the upload submit (if any) is fence-waited or never happened; destroy each once.
            unsafe {
                RhiDevice::destroy_sampler(ctx, sampler);
                RhiDevice::destroy_texture(ctx, texture);
            }
            return Err(e);
        }

        Ok(Self { texture, sampler, encoding })
    }

    /// Re-bakes the atlas from `field` (e.g. on an edit-`gen` change) and re-uploads it into the
    /// existing image, fence-waited. The image + sampler are reused (no recreate); the caller
    /// MUST have drained any prior submission that samples the atlas (this records a fresh
    /// `UNDEFINED`→`TRANSFER_DST`→`SHADER_READ` cycle, discarding the old contents).
    pub fn rebake(&self, ctx: &VulkanContext, field: &SdfEditField) -> Result<(), VulkanError> {
        Self::bake_and_upload(ctx, &self.texture, self.encoding, field)
    }

    /// The chosen voxel encoding (for the host UBO / smoke checks).
    #[inline]
    pub fn encoding(&self) -> AtlasEncoding {
        self.encoding
    }

    /// The atlas image (borrowed) — the marcher will bind this at `t10` in the M2 shader step.
    #[inline]
    pub fn texture(&self) -> &VulkanTexture {
        &self.texture
    }

    /// The atlas sampler (borrowed) — the marcher will bind this at `s1` in the M2 shader step.
    #[inline]
    pub fn sampler(&self) -> &VulkanSampler {
        &self.sampler
    }

    /// Bakes `field` into a host staging buffer ([`bake_brick_atlas`]) and records + submits the
    /// one-time staged copy into `texture`, fence-waited, transitioning
    /// `UNDEFINED`→`TRANSFER_DST_OPTIMAL`→`SHADER_READ_ONLY_OPTIMAL` so the atlas is
    /// sample-ready thereafter (no per-frame barrier). The staging buffer + encoder + fence are
    /// setup-class transients, torn down here.
    ///
    /// Returns the number of SURFACE cells baked (a cheap non-empty signal for a smoke check).
    fn bake_and_upload(
        ctx: &VulkanContext,
        texture: &VulkanTexture,
        encoding: AtlasEncoding,
        field: &SdfEditField,
    ) -> Result<(), VulkanError> {
        let size = encoding.atlas_byte_size() as u64;
        let staging = RhiDevice::create_buffer(
            ctx,
            &BufferDesc {
                size,
                usage: BufferUsage::TRANSFER_SRC,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )?;

        // Bake the atlas straight into the mapped staging bytes.
        let Some(dst) = RhiDevice::buffer_mapped_ptr(ctx, &staging) else {
            // SAFETY: `staging` was just created on `ctx`, owned exclusively here, never
            // submitted; destroy it once on this edge.
            unsafe { RhiDevice::destroy_buffer(ctx, staging) };
            return Err(VulkanError::Vk(
                "brick_atlas staging buffer not host-mapped",
                VkResult::ERROR_INITIALIZATION_FAILED,
            ));
        };
        // SAFETY: `dst` is the persistently-mapped first byte of the host-coherent staging
        // buffer (exactly `size` bytes, just created); the slice covers exactly those bytes; this
        // is the unique writer before any submission binds the buffer. Host-coherent ⇒ no flush.
        // The bake happens before the atlas is first sampled.
        let staging_bytes =
            unsafe { core::slice::from_raw_parts_mut(dst.as_ptr(), size as usize) };
        let _surface_cells = bake_brick_atlas(field, encoding, staging_bytes);

        let mut encoder = match RhiDevice::create_command_encoder(ctx) {
            Ok(e) => e,
            Err(e) => {
                // SAFETY: `staging` was just created on `ctx`, never submitted; destroy once.
                unsafe { RhiDevice::destroy_buffer(ctx, staging) };
                return Err(e);
            }
        };
        let fence = match RhiDevice::create_fence(ctx, false) {
            Ok(f) => f,
            Err(e) => {
                // SAFETY: encoder + staging just created, never submitted; destroy each once.
                unsafe {
                    RhiDevice::destroy_command_encoder(ctx, encoder);
                    RhiDevice::destroy_buffer(ctx, staging);
                }
                return Err(e);
            }
        };

        // One tightly-packed full-atlas 3D copy region (`image_extent_d = M2_ATLAS_DIM`).
        let region = [BufferImageCopy {
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
        }];

        let record = (|| -> Result<(), VulkanError> {
            encoder.begin()?;
            // UNDEFINED → TRANSFER_DST_OPTIMAL (the copy destination).
            encoder.image_barrier(&ImageBarrierDesc {
                texture,
                src_stage: BarrierStage::TOP_OF_PIPE,
                dst_stage: BarrierStage::TRANSFER,
                src_access: BarrierAccess::NONE,
                dst_access: BarrierAccess::TRANSFER_WRITE,
                old_layout: ImageLayout::Undefined,
                new_layout: ImageLayout::TransferDstOptimal,
                range: ImageSubresourceRange::COLOR,
            });
            encoder.copy_buffer_to_image(
                &staging,
                texture,
                ImageLayout::TransferDstOptimal,
                &region,
            );
            // TRANSFER_DST_OPTIMAL → SHADER_READ_ONLY_OPTIMAL (sample-ready). The M2 marcher
            // fetches from the COMPUTE stage, so make the writes available to COMPUTE_SHADER.
            encoder.image_barrier(&ImageBarrierDesc {
                texture,
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

        // Tear down the setup-class transients. The submit (if it ran) is fence-waited.
        // SAFETY: encoder/fence/staging were created on `ctx`; the encoder's only submission (if
        // any) completed (fence-waited above on the Ok path, or never submitted on an error
        // path), and each is moved by value ⇒ destroyed exactly once.
        unsafe {
            RhiDevice::destroy_command_encoder(ctx, encoder);
            RhiDevice::destroy_fence(ctx, fence);
            RhiDevice::destroy_buffer(ctx, staging);
        }
        record
    }

    /// Tears down the atlas (image + sampler), consuming `self`. The caller has drained the
    /// device (`wait_idle`) so no submission still samples it.
    ///
    /// # Safety
    ///
    /// `ctx` must be the live context the atlas was created on; the GPU is idle / drained (no
    /// work references the image), and the by-value `self` destroys each object exactly once.
    pub unsafe fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: per the contract `ctx` is live + drained; the image + sampler were created by
        // `create`; each is moved by value ⇒ destroyed once (image then sampler).
        unsafe {
            RhiDevice::destroy_texture(ctx, self.texture);
            RhiDevice::destroy_sampler(ctx, self.sampler);
        }
    }
}
