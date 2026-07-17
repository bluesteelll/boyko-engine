//! [`MeshData`] — the CPU intermediate a mesh [`AssetLoader`] decodes into
//! (asset-system rung A3b).
//!
//! [`AssetLoader`]: boyko_ecs::ecs::core::asset::AssetLoader

use crate::mesh::Vertex;

/// The decoded CPU form of a mesh asset — [`MeshGpu`](crate::mesh::MeshGpu)'s
/// [`Asset::Cpu`](boyko_ecs::ecs::core::asset::Asset::Cpu), replacing the
/// pre-A3b `()` placeholder now that a real mesh loader
/// ([`ObjMeshLoader`](crate::loaders::ObjMeshLoader)) exists.
///
/// Plain owned `Vec`s (`Send + 'static`, satisfying
/// [`Asset::Cpu`](boyko_ecs::ecs::core::asset::Asset::Cpu)'s bound):
/// a loader decodes raw file bytes into this on any thread (a future
/// threadpool-dispatched decode, rung A5), and the GPU-upload pass
/// ([`GpuUpload`](crate::gpu_upload::GpuUpload) for [`MeshGpu`](crate::mesh::MeshGpu))
/// turns it into a resident asset via
/// [`build_mesh_gpu`](crate::mesh_assets::build_mesh_gpu) — the SAME device
/// work [`MeshAssetsExt::register_mesh`](crate::mesh_assets::MeshAssetsExt::register_mesh)
/// runs for a host-authored mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshData {
    /// Model-space vertices (position + normal + color + uv + tangent — see
    /// [`Vertex`]). [`ObjMeshLoader`](crate::loaders::ObjMeshLoader) generates the
    /// tangent basis as a post-dedup pass; `uv` is `[0.0, 0.0]` on a `.obj` with no
    /// `vt` lines.
    pub vertices: Vec<Vertex>,
    /// Triangle indices into `vertices` (a flat list of 3-tuples).
    pub indices: Vec<u32>,
}
