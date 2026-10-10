//! Concrete [`AssetLoader`] implementations (asset-system rung A3b, extended by
//! textured-PBR rung T2): in-house, allocation-light byte decoders — no `ron` /
//! `serde` / third-party Wavefront-/PNG-parsing dependency.
//!
//! [`AssetLoader`]: boyko_ecs::ecs::core::asset::AssetLoader

/// VG-R0 rung R0b: the in-house glTF 2.0 binary (`.glb`) mesh decoder.
pub mod glb;
pub mod obj;
pub mod png_texture;
pub mod ron_material;

pub use glb::GlbMeshLoader;
pub use obj::ObjMeshLoader;
pub use png_texture::PngTextureLoader;
pub use ron_material::RonMaterialLoader;
