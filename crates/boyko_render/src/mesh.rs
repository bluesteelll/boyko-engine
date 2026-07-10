//! The GPU-resident mesh asset record (mesh foundation M2, asset-system rung A2 —
//! the `MeshRegistry` fold).
//!
//! [`MeshGpu`] OWNS the RHI vertex + index buffers a mesh is drawn from — a
//! legitimate Principle-0 FFI/GPU exception (the same class as `RhiContext`'s
//! owned device buffers and the swapchain's images), NOT a parallel ECS data
//! system: the durable *per-entity* render state (which mesh an entity uses,
//! its transform, its visibility) lives in ECS `ComponentPool` columns;
//! `MeshHandle` was DESIGNED to be a small dense INDEX into the world's
//! [`Assets<MeshGpu>`](boyko_ecs::ecs::core::asset::Assets) table of immutable,
//! shared GPU assets.
//!
//! # Rung A2 — the "Resource residency" fold (no separate GPU-mirror table)
//!
//! Unlike [`MaterialGpu`](crate::material::MaterialGpu) (whose CPU authority,
//! `Assets<MaterialGpu>`, is mirrored into a SEPARATE device SSBO by
//! [`MaterialTable`](crate::material_table::MaterialTable)), [`MeshGpu`] OWNS its
//! GPU buffers directly, so `Assets<MeshGpu>` itself IS the GPU-resident table —
//! there is no separate mirror. A draw binds straight from the resolved record.
//! `Assets<MeshGpu>` is `!Send` (a mesh record owns RHI buffers, device-bound and
//! single-thread-touch), so it is registered as a
//! [`NonSendResource`](boyko_ecs::ecs::core::resources::resource::NonSendResource)
//! alongside [`RhiContext`](crate::RhiContext), exactly as the standalone
//! `MeshRegistry` was. The impl itself lives in `boyko_ecs`'s own blanket
//! `impl<T: Asset> NonSendResource for Assets<T>` (a downstream crate hits the
//! orphan rule writing it for a concrete `Assets<MeshGpu>` — see that impl's doc).
//!
//! The mint/resolve/teardown domain API (`register_mesh`, `cube`, `plane`,
//! `mesh`, `try_get`, `destroy`, the HW-RT `blas_address`/`blas_generation`) is
//! NOT defined here as inherent methods — `Assets<T>` is a bare generic kernel
//! type in `boyko_ecs` with no room for mesh-specific methods — it is attached
//! via the [`MeshAssetsExt`](crate::mesh_assets::MeshAssetsExt) extension trait
//! in [`mesh_assets`](crate::mesh_assets).
//!
//! # `Asset::Cpu` — a placeholder, not `MeshGpu` itself
//!
//! Unlike `MaterialGpu` (`type Cpu = MaterialGpu`, since its GPU layout doubles
//! as its own decoded form — it owns no device handle), [`MeshGpu`] owns
//! non-`Send` RHI buffers and therefore CANNOT itself satisfy `Asset::Cpu: Send`.
//! No mesh loader exists yet at this rung (`register_mesh` mints `MeshGpu`
//! directly from host-provided vertex/index slices, never from raw file bytes),
//! so `Cpu = ()` is the placeholder a future mesh loader replaces with a real
//! `Send`-safe decoded intermediate (e.g. raw vertex/index bytes) — this mirrors
//! `boyko_ecs`'s own `Assets` unit-test `Asset` impl, which documents the
//! identical "never exercised without a loader, just needs to satisfy the bound"
//! pattern.
//!
//! # The vertex contract
//!
//! [`Vertex`] mirrors the `gbuffer_mrt.vs` vertex input EXACTLY: `position` @0,
//! `normal` @12, `color` @24, a 40-byte `#[repr(C)]` stride. The instanced arm reads
//! these as MODEL-SPACE positions and transforms them by the per-instance 3x4 affine
//! the instance SSBO carries; the table stores the model-space mesh once and every
//! instance reuses it.

use boyko_ecs::ecs::core::asset::Asset;
use boyko_rhi::enums::IndexType;
use boyko_rhi_vulkan::memory::BoundBuffer;

/// The `gbuffer_mrt.vs` vertex: a model-space position (offset 0), an outward world
/// normal (offset 12), and a linear RGBA color (offset 24). `#[repr(C)]` pins the exact
/// 40-byte stride the gbuffer raster pipeline's `VertexBufferLayout` declares
/// (position\@0 `Float32x3`, normal\@12 `Float32x3`, color\@24 `Float32x4`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    /// Model-space position (the instanced VS multiplies it by the per-instance affine).
    pub position: [f32; 3],
    /// Outward normal (model space; the VS transforms it by the affine's 3x3 part).
    pub normal: [f32; 3],
    /// Linear base color (passed through to the G-buffer albedo lane).
    pub color: [f32; 4],
}

/// The byte stride of one [`Vertex`] — the gbuffer raster pipeline's vertex stride.
pub const VERTEX_STRIDE: usize = core::mem::size_of::<Vertex>();
const _: () = assert!(VERTEX_STRIDE == 40, "Vertex must be tightly packed at 40 bytes");

/// The `u16` index-width crossover (O3): a mesh whose UNIQUE vertex count is at or below
/// this fits `Uint16` indices (halving the index buffer's footprint + bandwidth); above
/// it [`MeshAssetsExt::register_mesh`](crate::mesh_assets::MeshAssetsExt::register_mesh)
/// mints `Uint32` indices. `65_536 == u16::MAX + 1` (indices `0..=65535`).
pub const U16_INDEX_VERTEX_LIMIT: usize = u16::MAX as usize + 1;

/// One GPU-resident mesh asset: the OWNED vertex + index RHI buffers plus the draw
/// metadata the instanced gbuffer arm needs (`index_count`, `index_type`,
/// `vertex_count`). The buffers are host-visible coherent (seeded once at registration,
/// then read-only on the GPU), mirroring the test harness's vertex-buffer discipline.
///
/// `MeshGpu` does NOT implement `Drop`: an RHI [`BoundBuffer`] must be destroyed through
/// the owning [`VulkanContext`](boyko_rhi_vulkan::device::VulkanContext) (`destroy_buffer`)
/// AFTER the device is idle, which a blind `Drop` cannot guarantee.
/// [`MeshAssetsExt::destroy`](crate::mesh_assets::MeshAssetsExt::destroy) tears the
/// buffers down explicitly under the caller's idle contract (the same pattern as the
/// test harness's hand-rolled buffer teardown).
pub struct MeshGpu {
    /// The model-space vertex buffer (`BufferUsage::VERTEX`, host-visible coherent).
    pub vertex_buffer: BoundBuffer,
    /// The index buffer (`BufferUsage::INDEX`, host-visible coherent), `index_type`-wide.
    pub index_buffer: BoundBuffer,
    /// The number of indices to `draw_indexed` (`indices.len()`).
    pub index_count: u32,
    /// The bound index width (`Uint16` when `vertex_count <= U16_INDEX_VERTEX_LIMIT`).
    pub index_type: IndexType,
    /// The number of vertices in `vertex_buffer` (the unique-vertex count).
    pub vertex_count: u32,
    /// HW-RT rung R2a-3: this mesh's per-mesh BLAS — durable per-mesh data ON the record
    /// (Principle 0: NOT a parallel `Vec<BuiltBlas>`). Built EAGERLY in
    /// [`register_mesh`](crate::mesh_assets::MeshAssetsExt::register_mesh) under
    /// [`ray_query_enabled`](boyko_rhi_vulkan::device::VulkanContext::ray_query_enabled)
    /// (`None` on a non-RT device or hwrt OFF), read at its real index width from the mesh's
    /// existing index buffer (no duplicate `u32` buffer). Freed FIRST in
    /// [`destroy`](crate::mesh_assets::MeshAssetsExt::destroy) (AS before its backing,
    /// device-idle contract).
    #[cfg(feature = "hwrt")]
    pub blas: Option<boyko_rhi_vulkan::accel_build::BuiltBlas>,
}

impl Asset for MeshGpu {
    // See the module doc's "`Asset::Cpu` — a placeholder, not `MeshGpu` itself" section:
    // `MeshGpu` owns non-`Send` RHI buffers, so it cannot satisfy `Asset::Cpu: Send`
    // itself, and no mesh loader exists yet to decode into a real intermediate.
    type Cpu = ();
}

// `Assets<MeshGpu>` is `!Send` (each record owns RHI buffers, device-bound and
// single-thread-touch) — registered as a NonSend resource alongside
// `RhiContext`, mirroring the standalone `MeshRegistry`'s identical impl. The
// `NonSendResource` impl itself lives in `boyko_ecs` (`Assets<T>`'s own blanket
// `impl<T: Asset> NonSendResource for Assets<T>`) — a downstream crate cannot
// write it here (the orphan rule: neither `Assets` nor `NonSendResource` is
// local to `boyko_render`, and `Assets` is not a "fundamental" wrapper like
// `&`/`Box`, so nesting a local `MeshGpu` inside it does not satisfy coherence).
