//! The MDF (Mesh Distance Field) Stage-2c shadow-caster texture — a DEDICATED DENSE
//! `R8_SNORM` `VK_IMAGE_TYPE_3D` image holding one STATIC mesh's baked signed-distance
//! grid, its LINEAR / clamp-to-edge sampler, and the one-shot CPU-baked upload.
//!
//! Unlike the streaming brick-atlas ([`crate::brick_atlas::BrickAtlas`]), a single static
//! mesh at 64-128³ does not need sparsity — a dense 3D texture is a legitimate GPU buffer
//! (principle 0: an FFI/GPU-contiguity mirror of the CPU-baked field, not a parallel data
//! system). The grid is baked CPU-side by
//! [`boyko_sdf_math::mesh_sdf::bake_dense_grid`] (the SAME `EPSILON_Q` down-bias +
//! `encode_snorm8` the brick fill uses, so the decoded TRILINEAR reconstruction is a
//! CONSERVATIVE LOWER BOUND of the true mesh distance — the marcher's shadow march samples
//! it sphere-trace-soundly).
//!
//! # The marcher integration (Stage 2c)
//!
//! The marcher's SHADOW march (`sdf_soft_shadow_mesh` in `sdf_gbuffer_composite.hlsl`)
//! unions this texture's distance into the analytic shadow field (`min(field_distance(q),
//! mesh_sdf_sample(q))`) ONLY when the `mesh_sdf_enabled` push gate is set — so a non-MDF
//! scene is byte-identical (the texture is bound-but-unread). The texture is sampled with a
//! LINEAR sampler (the dense grid IS a lower bound, so hardware trilinear is sound — unlike
//! the brick atlas's NEAREST cubic-corner fetch, BUG-M2-GPU-1).
//!
//! # Lifetime
//!
//! Owned by value; torn down through [`MeshSdfTexture::destroy`] (the caller has drained the
//! device so no submission still samples it). Not `Copy`/`Clone`: the move encodes
//! "destroyed once". `!Send`/`!Sync` like every other Vulkan resource.

use boyko_rhi::{
    AddressMode, BufferDesc, BufferImageCopy, BufferUsage, Filter, Format, ImageAspect,
    ImageBarrierDesc, ImageLayout, ImageSubresourceRange, ImageUsage, MemoryLocation, MipMode,
    RhiCommandEncoder, RhiDevice, RhiQueue, SamplerDesc, TextureDesc, TextureDimension,
};
use boyko_rhi::enums::{BarrierAccess, BarrierStage};

use boyko_sdf_math::mesh_sdf::{BakeMesh, MeshSdfField, bake_dense_grid};

use crate::device::VulkanContext;
use crate::error::VulkanError;
use crate::ffi::VkResult;
use crate::memory::BoundBuffer;
use crate::rhi_impl::VulkanSampler;
use crate::texture::VulkanTexture;

/// The MDF Stage-2c shadow-caster texture: a dense `grid_dim` `R8_SNORM` `VK_IMAGE_TYPE_3D`
/// `TRANSFER_DST | SAMPLED` image, its LINEAR / clamp-to-edge / no-mip sampler, and the host
/// staging buffer used for the one-shot upload (retained on the resource so a future re-bake
/// — a moved mesh — reuses it). Carries NO drop glue: destruction is manual via
/// [`MeshSdfTexture::destroy`].
pub struct MeshSdfTexture {
    /// The dense `grid_dim` 3D distance image (`TRANSFER_DST | SAMPLED`). The marcher's shadow
    /// march trilinear-samples it via [`Self::sampler`].
    texture: VulkanTexture,
    /// The LINEAR (trilinear), clamp-to-edge, NO-MIP sampler. LINEAR is sound here (the dense
    /// grid is a conservative lower bound, so a trilinear blend never overshoots — the Hart
    /// precondition holds); clamp keeps an out-of-grid fetch reading the edge texel (a large
    /// positive band sample → no false occlusion) rather than wrapping.
    sampler: VulkanSampler,
    /// The host-visible staging buffer holding the baked grid bytes (one byte per voxel).
    /// Retained on the resource (reused by a re-bake); `grid_dim.x*y*z` bytes.
    staging: BoundBuffer,
    /// The grid descriptor (origin / voxel / dims / band) the marcher's `MeshSdfParams` UBO
    /// tail mirrors — the texture-space transform `mesh_sdf_sample` applies.
    field: MeshSdfField,
}

impl MeshSdfTexture {
    /// Creates the dense `R8_SNORM` 3D image + LINEAR sampler + host staging for `field`'s
    /// `grid_dim`, bakes `mesh` into the grid ([`bake_dense_grid`]), and uploads it once
    /// (`UNDEFINED`→`TRANSFER_DST`→`SHADER_READ`). On any partial failure every object created
    /// so far is torn down before the error returns.
    ///
    /// `field` MUST have been laid out for `mesh` ([`MeshSdfField::for_mesh`]) so the grid covers
    /// the mesh + its narrow-band margin and the P2 budget holds (the bake `debug_assert!`s it).
    pub fn create(
        ctx: &VulkanContext,
        mesh: &BakeMesh,
        field: &MeshSdfField,
    ) -> Result<Self, VulkanError> {
        let [w, h, d] = field.grid_dim;
        let voxel_count = w as usize * h as usize * d as usize;

        let texture = RhiDevice::create_texture(
            ctx,
            &TextureDesc {
                width: w,
                height: h,
                depth: d,
                // The mesh SDF is baked as snorm i8 codes (`encode_snorm8`); the hardware
                // R8_SNORM decode (-128→-1 asymmetry) matches the host `decode_snorm8`.
                format: Format::R8Snorm,
                dimension: TextureDimension::D3,
                usage: ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST,
                array_layers: 1,
            },
        )?;

        let sampler = match RhiDevice::create_sampler(
            ctx,
            &SamplerDesc {
                // LINEAR (trilinear): the dense grid is a conservative lower bound, so a
                // trilinear blend of neighbour voxels never overshoots the true surface (the
                // marcher's shadow march stays sound). Clamp-to-edge keeps an out-of-grid fetch
                // reading the edge texel (a far-positive band sample) instead of wrapping.
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: AddressMode::ClampToEdge,
                mip: MipMode::None,
                compare: None,
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

        let staging = match RhiDevice::create_buffer(
            ctx,
            &BufferDesc {
                size: voxel_count as u64,
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

        let tex = Self { texture, sampler, staging, field: *field };

        // Bake the dense grid into the staging + upload the whole image once. On failure tear
        // everything down.
        if let Err(e) = tex.bake_and_upload(ctx, mesh) {
            // SAFETY: the resource's objects were just created on `ctx`, owned exclusively here;
            // the upload submit (if any) is fence-waited or never happened; `destroy` moves each
            // by value ⇒ destroyed once.
            unsafe { tex.destroy(ctx) };
            return Err(e);
        }

        Ok(tex)
    }

    /// The grid descriptor the marcher's `MeshSdfParams` UBO tail mirrors.
    #[inline]
    pub fn field(&self) -> &MeshSdfField {
        &self.field
    }

    /// The dense distance image (borrowed) — bound at the marcher's mesh-SDF combined
    /// image+sampler slot (binding 15).
    #[inline]
    pub fn texture(&self) -> &VulkanTexture {
        &self.texture
    }

    /// The LINEAR sampler (borrowed) — bound alongside [`Self::texture`] in the combined
    /// image+sampler at binding 15.
    #[inline]
    pub fn sampler(&self) -> &VulkanSampler {
        &self.sampler
    }

    /// Bakes the dense grid from `mesh` into the host staging and uploads the whole image,
    /// fence-waited, transitioning `UNDEFINED`→`TRANSFER_DST_OPTIMAL`→`SHADER_READ_ONLY_OPTIMAL`.
    fn bake_and_upload(&self, ctx: &VulkanContext, mesh: &BakeMesh) -> Result<(), VulkanError> {
        let [w, h, d] = self.field.grid_dim;
        let voxel_count = w as usize * h as usize * d as usize;

        // CPU-bake the dense grid (the SAME bias/encode the brick fill uses → conservative
        // lower bound). `i8` and `u8` are byte-identical bit patterns, so the snorm codes copy
        // verbatim into the staging.
        let grid = bake_dense_grid(mesh, &self.field);
        debug_assert_eq!(grid.len(), voxel_count, "baked grid != grid_dim voxel count");

        let Some(dst) = RhiDevice::buffer_mapped_ptr(ctx, &self.staging) else {
            return Err(VulkanError::Vk(
                "mesh_sdf_texture staging buffer not host-mapped",
                VkResult::ERROR_INITIALIZATION_FAILED,
            ));
        };
        // SAFETY: `dst` is the persistently-mapped first byte of the host-coherent staging
        // buffer (exactly `voxel_count` bytes, allocated in `create`); `grid` is exactly
        // `voxel_count` `i8`s. `i8` is `Copy`/POD with no padding, so the byte image is valid
        // for a `u8` destination; this is the unique writer before the upload binds the buffer.
        // Host-coherent ⇒ no flush.
        unsafe {
            core::ptr::copy_nonoverlapping(grid.as_ptr().cast::<u8>(), dst.as_ptr(), voxel_count);
        }

        // One tightly-packed FULL-grid 3D copy region (row-major `x + y*W + z*W*H`).
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
            image_extent_w: w,
            image_extent_h: h,
            image_extent_d: d,
        };
        self.upload_region(ctx, &region)
    }

    /// Records + submits the staged copy of `region` from the host staging into the image,
    /// fence-waited, transitioning `UNDEFINED`→`TRANSFER_DST_OPTIMAL`→`SHADER_READ_ONLY_OPTIMAL`
    /// (the static one-shot upload discards any prior contents via `UNDEFINED`). The encoder +
    /// fence are setup-class transients torn down here; the staging is the retained member.
    fn upload_region(
        &self,
        ctx: &VulkanContext,
        region: &BufferImageCopy,
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

        let region = *region;
        let record = (|| -> Result<(), VulkanError> {
            encoder.begin()?;
            // UNDEFINED → TRANSFER_DST_OPTIMAL (the copy destination; the static bake has no
            // prior contents to preserve, so UNDEFINED discards).
            encoder.image_barrier(&ImageBarrierDesc {
                texture: &self.texture,
                src_stage: BarrierStage::TOP_OF_PIPE,
                dst_stage: BarrierStage::TRANSFER,
                src_access: BarrierAccess::NONE,
                dst_access: BarrierAccess::TRANSFER_WRITE,
                old_layout: ImageLayout::Undefined,
                new_layout: ImageLayout::TransferDstOptimal,
                range: ImageSubresourceRange::COLOR,
            });
            encoder.copy_buffer_to_image(
                &self.staging,
                &self.texture,
                ImageLayout::TransferDstOptimal,
                core::slice::from_ref(&region),
            );
            // TRANSFER_DST_OPTIMAL → SHADER_READ_ONLY_OPTIMAL (sample-ready). The marcher fetches
            // from the COMPUTE stage, so make the transfer writes available to COMPUTE_SHADER.
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

        // Tear down the setup-class transients (NOT the retained staging). The submit (if it
        // ran) is fence-waited.
        // SAFETY: encoder/fence were created on `ctx`; the encoder's only submission (if any)
        // completed (fence-waited above on the Ok path, or never submitted on an error path),
        // and each is moved by value ⇒ destroyed exactly once.
        unsafe {
            RhiDevice::destroy_command_encoder(ctx, encoder);
            RhiDevice::destroy_fence(ctx, fence);
        }
        record
    }

    /// Tears down the texture (image + sampler + staging), consuming `self`. The caller has
    /// drained the device (`wait_idle`) so no submission still samples it.
    ///
    /// # Safety
    ///
    /// `ctx` must be the live context the texture was created on; the GPU is idle / drained (no
    /// work references the image or staging), and the by-value `self` destroys each object
    /// exactly once.
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
