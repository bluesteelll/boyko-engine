//! Concrete [`AssetLoader`] implementations (asset-system rung A3b, extended by
//! textured-PBR rung T2): in-house, allocation-light byte decoders — no `ron` /
//! `serde` / third-party Wavefront-/PNG-parsing dependency.
//!
//! [`AssetLoader`]: boyko_ecs::ecs::core::asset::AssetLoader

pub mod obj;
pub mod png_texture;
pub mod ron_material;

pub use obj::ObjMeshLoader;
pub use png_texture::PngTextureLoader;
pub use ron_material::RonMaterialLoader;
