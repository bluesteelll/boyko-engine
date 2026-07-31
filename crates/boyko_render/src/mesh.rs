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
//! # `Asset::Cpu` — [`MeshData`], not `MeshGpu` itself
//!
//! Unlike `MaterialGpu` (`type Cpu = MaterialGpu`, since its GPU layout doubles
//! as its own decoded form — it owns no device handle), [`MeshGpu`] owns
//! non-`Send` RHI buffers and therefore CANNOT itself satisfy `Asset::Cpu: Send`.
//! Asset-system rung A3b adds the first mesh loader
//! ([`ObjMeshLoader`](crate::loaders::ObjMeshLoader)), so `Cpu = `[`MeshData`]
//! (a plain, `Send`-safe `Vec<Vertex>` + `Vec<u32>` pair) replaces the pre-A3b
//! `()` placeholder — `register_mesh` still mints `MeshGpu` directly from
//! host-provided vertex/index slices for the host-authored primitives (`cube`,
//! `plane`), sharing the same device-upload path
//! ([`build_mesh_gpu`](crate::mesh_assets::build_mesh_gpu)) a decoded
//! [`MeshData`] uses.
//!
//! # The vertex contract
//!
//! [`Vertex`] mirrors the `gbuffer_mrt.vs` vertex input EXACTLY on its first three
//! fields: `position` @0, `normal` @12, `color` @24 — FROZEN offsets, since every
//! existing gbuffer pipeline (base/mv/pm/mvpm) declares its `VertexAttribute` array
//! against them. The instanced arm reads `position` as a MODEL-SPACE position and
//! transforms it by the per-instance 3x4 affine the instance SSBO carries; the table
//! stores the model-space mesh once and every instance reuses it.
//!
//! Two fields were appended for normal mapping (a future textured pipeline, asset-
//! streaming rung T6): `uv` @40 (`Float32x2`) and `tangent` @48 (`Float32x4`: the unit
//! tangent `xyz` + the bitangent handedness sign `w`), for a 64-byte (one cache line)
//! `#[repr(C)]` stride. NO existing pipeline declares these two attributes, so a mesh
//! carrying `uv`/`tangent` renders BYTE-IDENTICALLY through the base/mv/pm/mvpm VS —
//! the two trailing fields simply ride along, unread, in the wider stride.

use boyko_ecs::ecs::core::asset::{Asset, AssetBacking, HasLoaders, LoaderEntry, register_asset_layout};
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_rhi::enums::IndexType;
use boyko_rhi_vulkan::memory::BoundBuffer;

use crate::loaders::{GlbMeshLoader, ObjMeshLoader};
use crate::mesh_data::MeshData;

/// The `gbuffer_mrt.vs` vertex: a model-space position (offset 0), an outward world
/// normal (offset 12), a linear RGBA color (offset 24), a texture coordinate (offset
/// 40), and a tangent-space basis (offset 48). `#[repr(C)]` pins the exact 64-byte
/// stride (one cache line) the gbuffer raster pipeline's `VertexBufferLayout`
/// declares. Every EXISTING pipeline (base/mv/pm/mvpm) only declares the first three
/// attributes (position\@0 `Float32x3`, normal\@12 `Float32x3`, color\@24
/// `Float32x4`) — `uv`/`tangent` are read by no shader yet (asset-streaming rung T6),
/// so they do not change how any current mesh renders.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    /// Model-space position (the instanced VS multiplies it by the per-instance affine).
    pub position: [f32; 3],
    /// Outward normal (model space; the VS transforms it by the affine's 3x3 part).
    pub normal: [f32; 3],
    /// Linear base color (passed through to the G-buffer albedo lane).
    pub color: [f32; 4],
    /// Texture coordinates. `[0.0, 0.0]` on a mesh with no real UV data (a host
    /// primitive built via [`Vertex::new`], or a `.obj` with no `vt` lines) — inert
    /// until a textured pipeline (T6) declares this attribute.
    pub uv: [f32; 2],
    /// The tangent-space basis: `xyz` is the unit surface tangent, `w` is the
    /// bitangent handedness sign (`±1`, `bitangent = cross(normal, tangent) * w`).
    /// Identity `[1.0, 0.0, 0.0, 1.0]` on a mesh with no generated tangent basis —
    /// see [`generate_tangents`](crate::tangent::generate_tangents). Inert until a
    /// normal-mapped pipeline (T6) declares this attribute.
    pub tangent: [f32; 4],
}

/// The byte stride of one [`Vertex`] — the gbuffer raster pipeline's vertex stride.
pub const VERTEX_STRIDE: usize = core::mem::size_of::<Vertex>();
const _: () = assert!(VERTEX_STRIDE == 64, "Vertex must be tightly packed at 64 bytes (one cache line)");

impl Vertex {
    /// Builds a vertex with the pre-T6 placeholder UV/tangent (`uv: [0.0, 0.0]`,
    /// `tangent: [1.0, 0.0, 0.0, 1.0]` — the identity basis) — the shape every
    /// call site without real texture data (host primitives, the legacy degenerate
    /// vertex, a `.obj` with no `vt`) wants. A generator with real UV data sets
    /// `.uv` on the result and runs
    /// [`generate_tangents`](crate::tangent::generate_tangents) over the whole mesh
    /// afterward (tangent generation needs the full triangle list, not one vertex).
    #[inline]
    pub const fn new(position: [f32; 3], normal: [f32; 3], color: [f32; 4]) -> Self {
        Self { position, normal, color, uv: [0.0, 0.0], tangent: [1.0, 0.0, 0.0, 1.0] }
    }
}

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
    /// Multi-paradigm render-path plan, rung R-VBGEO (Decision 0): this mesh's slot in
    /// the bindless [`MeshGeometryTable`](crate::mesh_geometry_table::MeshGeometryTable)
    /// — the `mesh_id` the VB instance gather (R8/R9) reads to key
    /// `gMeshVerts[]`/`gMeshIndices[]`/`gMeshMeta[]`. Mirrors how a texture's bindless
    /// slot lives on [`MaterialTextures`](crate::material::MaterialTextures) at the
    /// owning asset record, not in a separate handle→slot side table.
    /// [`VB_GEOMETRY_RESERVED_SLOT`](crate::mesh_geometry_table::VB_GEOMETRY_RESERVED_SLOT)
    /// (`0`) when the table is absent (every non-VB boot, or a VB boot whose device lacks
    /// the descriptor-indexing prerequisite) — and, TRANSIENTLY, for a host-authored
    /// `cube`/`plane`/`register_mesh` mesh between registration and the boot back-fill
    /// (`backfill_vb_geometry_slots`), which is the only writer of this field outside
    /// [`build_mesh_gpu`](crate::mesh_assets::build_mesh_gpu).
    pub geometry_slot: u32,
    /// CSM auto-fit plan, rung C0: this mesh's model-space AABB minimum, folded over
    /// `vertices[].position` (mesh.rs:85). Durable per-mesh data ON THE RECORD — the same
    /// shape of datum as `blas` above (Principle 0: NOT a parallel `HashMap<MeshHandle,
    /// Aabb>` side table). Minted once in
    /// [`build_mesh_gpu`](crate::mesh_assets::build_mesh_gpu); CPU-only, never uploaded to
    /// the GPU. Dark this rung — nothing reads it yet (the caster-bounds fold, rung C2,
    /// is the first consumer).
    ///
    /// An empty vertex slice (never legal — `build_mesh_gpu` debug-asserts non-empty) would
    /// fold to `[f32::INFINITY; 3]`, an INVERTED box paired with `local_max`'s
    /// `[f32::NEG_INFINITY; 3]`: `local_min[i] > local_max[i]` on every axis, which cannot
    /// be mistaken for a real (possibly degenerate point-sized) AABB — see
    /// `build_mesh_gpu`'s doc for why a zeroed box was rejected instead.
    pub local_min: [f32; 3],
    /// Model-space AABB maximum. See [`Self::local_min`] for the fold + degenerate case.
    pub local_max: [f32; 3],
}

impl Asset for MeshGpu {
    // See the module doc's "`Asset::Cpu` — `MeshData`, not `MeshGpu` itself" section:
    // `MeshGpu` owns non-`Send` RHI buffers, so it cannot satisfy `Asset::Cpu: Send`
    // itself; `MeshData` is the `Send`-safe decoded intermediate `ObjMeshLoader`
    // produces (asset-system rung A3b).
    type Cpu = MeshData;
}

impl MeshGpu {
    /// The [`AssetBacking::register_layout`] drop glue for `MeshGpu` (asset-streaming
    /// plan F1). `MeshGpu` implements no `Drop` (see the struct doc: device buffers are
    /// torn down explicitly via [`MeshAssetsExt::destroy`](crate::mesh_assets::MeshAssetsExt::destroy),
    /// under the caller's device-idle contract, never from a destructor) — this glue is
    /// therefore DEVICE-INERT BY DESIGN: it moves the value out and lets Rust's ordinary
    /// field-drop glue run (`BoundBuffer`/`BuiltBlas`/`IndexType`/integers all have
    /// trivial drop), freeing NO device memory. It exists only so `Assets<MeshGpu>`'s
    /// store-owned `ComponentPool` has a registered drop_fn to invoke on a live row
    /// (`col.drop_at` / the terminal `Drop for Assets<T>`), matching `MeshGpu`'s existing
    /// contract exactly (a bare `Drop for Assets<MeshGpu>` under the old `Vec<Slot<T>>`
    /// storage would have run precisely this same trivial field-drop, nothing more). The
    /// real, fence-gated device teardown (`destroy_buffer`/`destroy_blas`) arrives with
    /// the streaming take-at-retire path at F6.
    ///
    /// # Safety
    /// The caller (`ComponentPool::drop_at` / the terminal `Drop for Assets<MeshGpu>`)
    /// guarantees `ptr` points at a valid, aligned, fully-initialized `MeshGpu`,
    /// exclusively owned, not accessed again after this call — the standard `DropFn`
    /// contract (`boyko_ecs`'s `component_registry::DropFn`).
    unsafe fn drop_glue(ptr: *mut u8) {
        // SAFETY: see this function's own doc — the caller upholds the `DropFn`
        // contract. `drop_in_place` runs `MeshGpu`'s (trivial, field-wise) drop glue
        // exactly once; no device call is made here (see the doc above).
        unsafe { core::ptr::drop_in_place(ptr.cast::<MeshGpu>()) }
    }
}

impl AssetBacking for MeshGpu {
    // Resident asset record: teardown is manual (device-idle-gated), never a bare
    // `Drop` — see `drop_glue`'s doc.
    const NEEDS_TEARDOWN: bool = true;

    fn register_layout() -> ComponentId {
        register_asset_layout::<MeshGpu>(Some(MeshGpu::drop_glue))
    }
}

impl HasLoaders for MeshGpu {
    /// Two entries: the in-house Wavefront `.obj` loader and the in-house glTF
    /// 2.0 binary (`.glb`) loader VG-R0 rung R0b added for the high-poly corpus.
    /// Asset-streaming plan F3 — a compile-time-static table, no runtime
    /// registration; the extension picks the entry.
    const LOADERS: &'static [LoaderEntry<Self>] =
        &[LoaderEntry::of::<ObjMeshLoader>(), LoaderEntry::of::<GlbMeshLoader>()];
}

// `Assets<MeshGpu>` is `!Send` (each record owns RHI buffers, device-bound and
// single-thread-touch) — registered as a NonSend resource alongside
// `RhiContext`, mirroring the standalone `MeshRegistry`'s identical impl. The
// `NonSendResource` impl itself lives in `boyko_ecs` (`Assets<T>`'s own blanket
// `impl<T: Asset> NonSendResource for Assets<T>`) — a downstream crate cannot
// write it here (the orphan rule: neither `Assets` nor `NonSendResource` is
// local to `boyko_render`, and `Assets` is not a "fundamental" wrapper like
// `&`/`Box`, so nesting a local `MeshGpu` inside it does not satisfy coherence).
