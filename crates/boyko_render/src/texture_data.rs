//! [`TextureData`] — the CPU intermediate a texture [`AssetLoader`] decodes into
//! (textured-PBR campaign rung T2), mirroring [`MeshData`](crate::mesh_data::MeshData).
//!
//! [`AssetLoader`]: boyko_ecs::ecs::core::asset::AssetLoader

use crate::texture::ColorSpace;

/// The decoded CPU form of a texture asset — [`TextureGpu`](crate::texture::TextureGpu)'s
/// [`Asset::Cpu`](boyko_ecs::ecs::core::asset::Asset::Cpu).
///
/// A plain owned `Vec<u8>` (`Send + 'static`, satisfying
/// [`Asset::Cpu`](boyko_ecs::ecs::core::asset::Asset::Cpu)'s bound):
/// [`PngTextureLoader`](crate::loaders::PngTextureLoader) decodes raw PNG bytes into
/// this on any thread (mirrors [`MeshData`](crate::mesh_data::MeshData)'s decode
/// story), and the GPU-upload pass ([`GpuUpload`](crate::gpu_upload::GpuUpload) for
/// [`TextureGpu`](crate::texture::TextureGpu)) turns it into a resident, mip-chained
/// bindless asset via
/// [`build_texture_gpu`](crate::texture::build_texture_gpu) — the SAME device work
/// [`TextureAssetsExt::register_texture`](crate::texture::TextureAssetsExt::register_texture)
/// runs for a host-authored texture.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureData {
    /// Width in texels (`> 0`).
    pub width: u32,
    /// Height in texels (`> 0`).
    pub height: u32,
    /// Tightly-packed, row-major RGBA8 pixel data (`width * height * 4` bytes).
    pub rgba8: Vec<u8>,
    /// The material-slot color space this texture's samples are encoded in — decides
    /// the GPU image/view format pair (see [`ColorSpace::formats`]). Set by the
    /// loader's caller (the material slot the loader is invoked for decides gamma /
    /// linearity); defaults to [`ColorSpace::Srgb`] (the common albedo/emissive case)
    /// when a loader has no per-slot context yet.
    pub color_space: ColorSpace,
}
