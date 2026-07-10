//! Concrete [`AssetLoader`] implementations (asset-system rung A3b):
//! in-house, allocation-light byte decoders — no `ron` / `serde` / third-party
//! Wavefront-parsing dependency.
//!
//! [`AssetLoader`]: boyko_ecs::ecs::core::asset::AssetLoader

pub mod obj;
pub mod ron_material;

pub use obj::ObjMeshLoader;
pub use ron_material::RonMaterialLoader;
