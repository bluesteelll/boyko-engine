//! [`MeshAssetsExt`] — the mesh-domain API over `Assets<MeshGpu>` (asset-system
//! rung A2: the `MeshRegistry` fold).
//!
//! Rung A2 replaces the standalone `MeshRegistry` (mesh foundation M2) with the
//! world-global [`Assets<MeshGpu>`](boyko_ecs::ecs::core::asset::Assets)
//! `NonSendResource` — the SAME generic asset-kernel table
//! [`Assets<MaterialGpu>`](crate::material::MaterialGpu) shares (rung A1), this
//! time in its "Resource residency" flavor: EACH RECORD OWNS its own GPU
//! buffers, so there is no separate GPU-mirror table like
//! [`MaterialTable`](crate::material_table::MaterialTable) — a draw binds
//! straight from the resolved record.
//!
//! `Assets<T>` is a bare generic kernel type in `boyko_ecs` (it cannot carry
//! mesh-specific methods), so the mint/resolve/teardown domain API
//! (`register_mesh`, `cube`, `plane`, `mesh`, `try_get`, `destroy`, plus the
//! HW-RT `blas_address`/`blas_generation`) is attached via this extension
//! trait, `impl`ed once for `Assets<MeshGpu>`. A consumer brings it into scope
//! with `use boyko_render::MeshAssetsExt;` (or `boyko_app::prelude::*`) and
//! calls the same method names `MeshRegistry` exposed, on an `Assets<MeshGpu>`
//! receiver.
//!
//! # Why the panicking accessor is named `mesh`, not `get`
//!
//! `Assets<T>` already declares an INHERENT `get(&self, handle: Handle<T>) ->
//! Option<&T>` (the generation-checked core accessor). Rust's method-call
//! resolution always prefers an inherent method over a trait method of the
//! SAME name — an extension-trait `get(&self, h: MeshHandle) -> &MeshGpu`
//! would be permanently shadowed by that inherent method and never reachable
//! via `assets.get(mesh_handle)` (it would resolve to the inherent `get`,
//! then fail to type-check `MeshHandle` against `Handle<MeshGpu>`). The
//! panicking single-mesh accessor is therefore named
//! [`mesh`](MeshAssetsExt::mesh) instead — the one deliberate naming deviation
//! from `MeshRegistry`'s original API; every other method name is unchanged.
//!
//! # `MeshHandle` stays a raw dense index (unchanged, the P1-3 caveat)
//!
//! [`MeshHandle`](boyko_scene::render_caps::MeshHandle) is NOT widened to carry
//! an `Assets` [`Handle<MeshGpu>`](boyko_ecs::ecs::core::asset::Handle)'s
//! generation — it stays the plain 4-byte `u32` row index it always was.
//! [`register_mesh`](MeshAssetsExt::register_mesh) mints via
//! [`Assets::add`](boyko_ecs::ecs::core::asset::Assets::add) and truncates the
//! returned `Handle`'s index into a `MeshHandle` (mirrors
//! [`MaterialId::from_handle`](crate::material::MaterialId::from_handle)'s
//! carrier truncation); [`mesh`](MeshAssetsExt::mesh) /
//! [`try_get`](MeshAssetsExt::try_get) resolve it back via
//! [`Assets::get_by_index`](boyko_ecs::ecs::core::asset::Assets::get_by_index) —
//! the generation-AGNOSTIC accessor added specifically for this render-carrier
//! shape. Three per-frame consumers
//! ([`gather_mesh_draws`](crate::mesh_draw::gather_mesh_draws),
//! [`gather_shadow_casters`](crate::csm_caster::gather_shadow_casters), and the
//! host's per-draw / TLAS BLAS-address resolution) fabricate a `MeshHandle`
//! from a bare dense loop counter (never a stored generational `Handle`), so
//! `get_by_index` is the only resolution mechanism that fits their call
//! sites. Sound ONLY under the append-only invariant meshes hold at setup
//! (documented on [`Handle`](boyko_ecs::ecs::core::asset::Handle)'s own doc,
//! the same P1-3 caveat `MaterialId` carries) — no caller ever removes a live
//! mesh handle.
//!
//! # BLAS generation, without a bespoke counter field
//!
//! The old `MeshRegistry::blas_generation` was a dedicated `u64` field bumped
//! once per [`register_mesh`](MeshAssetsExt::register_mesh) call that built a
//! BLAS. `Assets<T>` has no room for mesh-specific extra state, so
//! `MeshAssetsExt::blas_generation` (feature = "hwrt") is derived instead from
//! [`Assets::high_water`](boyko_ecs::ecs::core::asset::Assets::high_water) — the
//! existing O(1) monotonic row-count high-water mark. This stays a correct,
//! useful metric for a scene that only ever GROWS (an RT device builds a BLAS
//! on every registration — the two counters advance in lockstep, 1:1).
//!
//! It is, however, NOT the signal `TlasResources::sync_blas_addr`
//! (`boyko_app::gpu_scene::tlas`) gates its `blas_addr` table refresh on
//! (asset-streaming plan F6): a fence-gated retire-then-reuse installs a
//! DIFFERENT mesh's BLAS at the SAME slot without advancing `high_water` (a
//! free-list reuse overwrites a hole in place), which `blas_generation` alone
//! cannot detect. `sync_blas_addr` instead gates on
//! [`Assets::install_epoch`](boyko_ecs::ecs::core::asset::Assets::install_epoch),
//! which advances on every `add`/`fill`, reuse included — see that fn's doc
//! for the full argument.

use boyko_ecs::ecs::core::asset::assets::Assets;
use boyko_ecs::ecs::core::asset::handle::Handle;
use boyko_ecs::ecs::core::resources::resource::NonSendResource;
#[cfg(feature = "hwrt")]
use boyko_rhi::AsIndexType;
use boyko_rhi::enums::IndexType;
use boyko_rhi::{BufferDesc, BufferUsage, MemoryLocation, RhiDevice};
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_scene::render_caps::MeshHandle;

use crate::mesh::{MeshGpu, U16_INDEX_VERTEX_LIMIT, Vertex};
#[cfg(feature = "hwrt")]
use crate::mesh::VERTEX_STRIDE;
use crate::mesh_geometry_table::{MeshGeometryTable, VB_GEOMETRY_RESERVED_SLOT, mesh_buffer_usage};
use crate::tangent::generate_tangents;

/// The mesh-domain API over the world's [`Assets<MeshGpu>`] table (asset-system
/// rung A2). See the module doc for the fold's shape, the `mesh`-vs-`get`
/// naming note, the dense-index resolution, and the BLAS-generation
/// derivation.
pub trait MeshAssetsExt {
    /// Uploads a model-space mesh (`vertices` + triangle `indices`) into a fresh GPU
    /// asset and returns its [`MeshHandle`].
    ///
    /// The index width is chosen by O3: `Uint16` when the unique vertex count is at or
    /// below [`U16_INDEX_VERTEX_LIMIT`], else `Uint32`. The `u32` `indices` are narrowed
    /// to `u16` on the `Uint16` path; the caller's indices MUST be in `0..vertices.len()`
    /// (a `debug_assert!` catches an out-of-range index, which would be a `u16`
    /// truncation bug on the narrow path).
    ///
    /// Both buffers are `HostVisibleCoherent` and seeded once here, reusing the RHI's
    /// `create_buffer` + `buffer_mapped_ptr` upload helpers — the SAME host-coherent
    /// staging discipline the UI / vertex-buffer paths use; no hand-rolled Vulkan.
    ///
    /// # Panics
    /// Panics (`expect`) if either buffer create or its host mapping fails — a device
    /// out-of-memory at asset-registration time is a setup failure, not a recoverable
    /// per-frame error.
    fn register_mesh(
        &mut self,
        ctx: &VulkanContext,
        vertices: &[Vertex],
        indices: &[u32],
    ) -> MeshHandle;

    /// Registers an axis-aligned CUBE of edge length `size`, centered at the
    /// model-space origin, with per-face outward normals (24 unique vertices,
    /// 36 indices — `Uint16` by O3) and a neutral light-gray base color. The
    /// canonical primitive for a first scene (host plan R3); place it with the
    /// entity's `Transform`.
    ///
    /// # Panics
    /// Same contract as [`register_mesh`](Self::register_mesh): a buffer create
    /// / map failure at asset-registration time is a setup failure.
    fn cube(&mut self, ctx: &VulkanContext, size: f32) -> MeshHandle;

    /// Registers a flat XZ-plane quad of side length `size`, centered at the
    /// model-space origin at `y == 0`, normal `+Y` (4 vertices, 6 indices —
    /// `Uint16` by O3), with a neutral mid-gray base color — the canonical
    /// floor/receiver primitive (host plan R3).
    ///
    /// # Panics
    /// Same contract as [`register_mesh`](Self::register_mesh).
    fn plane(&mut self, ctx: &VulkanContext, size: f32) -> MeshHandle;

    /// Resolves a [`MeshHandle`] to its GPU asset.
    ///
    /// # Panics
    /// Panics if `h` is out of range or was never registered — a handle no
    /// live row resolves to is a caller/asset-binding bug, not a recoverable
    /// error (the ECS gather only emits handles this table returned).
    fn mesh(&self, h: MeshHandle) -> &MeshGpu;

    /// Resolves a [`MeshHandle`] to its GPU asset, or `None` if the handle is
    /// out of range or was never registered (the fallible counterpart of
    /// [`mesh`](Self::mesh) for a gather that may hold a not-yet-registered
    /// handle).
    fn try_get(&self, h: MeshHandle) -> Option<&MeshGpu>;

    /// Destroys every registered mesh's RHI buffers through `ctx` and empties
    /// the table.
    ///
    /// # Safety
    /// The caller MUST have made the device idle (e.g. via the renderer's `Drop` /
    /// `wait_idle`) so no in-flight submit still references any mesh buffer; each buffer
    /// is destroyed exactly once. Mirrors the test harness's explicit buffer teardown.
    unsafe fn destroy(&mut self, ctx: &VulkanContext);

    /// HW-RT rung R2a-3: the current BLAS-address generation. The host's per-frame
    /// TLAS-instance packer rewrites its (frame-invariant) per-mesh BLAS-address table
    /// ONLY when this advances (BLASes never move — spec), never every frame. See the
    /// module doc's "BLAS generation, without a bespoke counter field" section.
    #[cfg(feature = "hwrt")]
    fn blas_generation(&self) -> u64;

    /// HW-RT rung R2a-3: mesh `h`'s BLAS device address (a TLAS instance's
    /// `accelerationStructureReference`), or `0` if the handle has no BLAS (a non-RT
    /// device, or a handle the table never minted). Non-zero for every mesh registered
    /// on an RT device.
    #[cfg(feature = "hwrt")]
    fn blas_address(&self, h: MeshHandle) -> u64;
}

/// The pure (host-only) model-space AABB fold over `vertices[].position`
/// ([`MeshGpu::local_min`]/[`MeshGpu::local_max`], CSM auto-fit plan rung C0).
/// Factored out of [`build_mesh_gpu`] for the same reason [`cube_geometry`] is
/// factored out of `cube` — unit-testable without a `VulkanContext`.
///
/// An empty slice folds to an INVERTED box (`min > max` on every axis): a zeroed box
/// at the origin would silently read as a valid degenerate point-sized mesh, whereas
/// an inverted box cannot be mistaken for any real AABB. `build_mesh_gpu`'s own
/// `debug_assert!(!vertices.is_empty(), ..)` already rejects this case in debug
/// builds; this representation is the release-mode backstop.
fn local_aabb(vertices: &[Vertex]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for v in vertices {
        for axis in 0..3 {
            min[axis] = min[axis].min(v.position[axis]);
            max[axis] = max[axis].max(v.position[axis]);
        }
    }
    (min, max)
}

/// The device work behind [`MeshAssetsExt::register_mesh`]: creates + fills the
/// vertex/index RHI buffers (index width chosen by O3), and — on an RT device —
/// builds the mesh's BLAS eagerly, returning the assembled [`MeshGpu`] (NOT yet
/// inserted into any [`Assets<MeshGpu>`] table — the caller mints the row).
///
/// Factored out of `register_mesh` (asset-system rung A3b) so a loaded mesh's
/// GPU-upload pass ([`GpuUpload`](crate::gpu_upload::GpuUpload) for [`MeshGpu`])
/// can build a resident mesh from a decoded
/// [`MeshData`](crate::mesh_data::MeshData) through the EXACT SAME device path a
/// host-authored `register_mesh` call uses — a pure refactor of
/// `register_mesh`'s prior inline body, byte-identical to the pre-A3b behavior.
///
/// # Rung R-VBGEO — `geometry_table` (Decision 0 / P2-b)
///
/// `geometry_table` is the [`MeshGeometryTable`] to ALSO claim a slot in, when armed —
/// threaded so the STREAMED mesh-upload drain
/// ([`GpuUpload for MeshGpu`](crate::gpu_upload::GpuUpload)) can pass
/// `Some(&mut table)` (the Rev-5 "every runtime-streamed mesh" invariant). The
/// STORAGE-usage-bit decision itself ([`mesh_buffer_usage`]) reads
/// `ctx.vb_geometry_table_armed()` — a flag threaded via `ctx` (already a parameter at
/// EVERY registration call site), so it applies UNIVERSALLY regardless of whether this
/// call passes a table. Host-authored primitives (`register_mesh`/`cube`/`plane`) pass
/// `None` this rung: threading `Option<&mut MeshGeometryTable>` through THEIR public
/// signatures would touch every one of their ~20 call sites across `boyko_app`
/// (showcases/tests/examples), well beyond this rung's "data layer only" scope —
/// `VB_IMPLEMENTED == false` keeps `vb_geometry_table_armed()` `false` on every boot
/// today regardless, so this is a documented, currently-inert scope cut, not a
/// correctness gap; a later rung that actually needs host-authored meshes IN the VB
/// geometry table can widen `register_mesh`'s signature then.
///
/// # Panics
/// Same contract as [`MeshAssetsExt::register_mesh`]: a buffer create / map
/// failure at asset-registration time is a setup failure.
pub fn build_mesh_gpu(
    ctx: &VulkanContext,
    vertices: &[Vertex],
    indices: &[u32],
    geometry_table: Option<&mut MeshGeometryTable>,
) -> MeshGpu {
    debug_assert!(!vertices.is_empty(), "invariant: a mesh has at least one vertex");
    debug_assert!(!indices.is_empty(), "invariant: an indexed mesh has at least one index");
    let vertex_count = vertices.len();

    // CSM auto-fit plan, rung C0: the model-space AABB fold. ONE pass over `vertices`
    // (not a second one layered onto the buffer-fill copy below, which is a bulk
    // `copy_nonoverlapping`, not a loop) — `local_aabb` is factored into its own fn
    // purely so it is unit-testable without a `VulkanContext` (mirrors `cube_geometry`'s
    // factoring above the trait impl).
    let (local_min, local_max) = local_aabb(vertices);
    debug_assert!(
        (0..3).all(|axis| local_min[axis] <= local_max[axis]),
        "invariant: build_mesh_gpu is called with a non-empty vertex slice \
         (see the debug_assert above) — an inverted box means that invariant broke"
    );
    let index_type = if vertex_count <= U16_INDEX_VERTEX_LIMIT {
        IndexType::Uint16
    } else {
        IndexType::Uint32
    };

    // HW-RT rung R2a-2: on an RT device the mesh is a BLAS build input, so its vertex +
    // index buffers must carry `ACCEL_BUILD_INPUT | SHADER_DEVICE_ADDRESS` (the shared
    // host block carries `VK_MEMORY_ALLOCATE_DEVICE_ADDRESS_BIT` — device.rs
    // `rt_buffer_device_address` — so the device-address usage is valid). hwrt-off OR a
    // non-RT GPU ⇒ `as_bits` is `NONE`, so the usage is unchanged (byte-identical to the
    // pre-R2a mesh buffers).
    let as_bits = {
        #[cfg(feature = "hwrt")]
        {
            if ctx.ray_query_enabled() {
                BufferUsage::ACCEL_BUILD_INPUT | BufferUsage::SHADER_DEVICE_ADDRESS
            } else {
                BufferUsage::NONE
            }
        }
        #[cfg(not(feature = "hwrt"))]
        {
            BufferUsage::NONE
        }
    };
    // Multi-paradigm render-path plan, rung R-VBGEO (Decision 0 / P2-b): the
    // STORAGE_BUFFER usage bit is ONLY added when the boot-committed
    // `vb_geometry_table` flag is armed (read through `ctx`, the channel already
    // present at every registration call site) — otherwise `vb_bits == BufferUsage::NONE`
    // and the usage is EXACTLY today's `VERTEX`/`INDEX`-only (byte-identical
    // registration). `VB_IMPLEMENTED == false` keeps this flag `false` on every boot
    // today (see `mesh_geometry_table`'s module doc), so `vb_bits` is always `NONE` in
    // practice — this is the pure fn `mesh_buffer_usage` proves, unit-tested without a
    // device.
    let vb_bits = mesh_buffer_usage(ctx.vb_geometry_table_armed());
    let vertex_usage = BufferUsage::VERTEX | as_bits | vb_bits;
    let index_usage = BufferUsage::INDEX | as_bits | vb_bits;

    // --- Vertex buffer: copy the model-space vertices in once. ---
    let vertex_bytes = core::mem::size_of_val(vertices) as u64;
    let vertex_buffer = ctx
        .create_buffer(&BufferDesc {
            size: vertex_bytes,
            usage: vertex_usage,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("invariant: mesh vertex buffer create");
    let vb_ptr = ctx
        .buffer_mapped_ptr(&vertex_buffer)
        .expect("invariant: host-visible vertex buffer is mapped");
    // SAFETY: `vb_ptr` points to `vertex_bytes` mapped host-coherent bytes; `vertices`
    // is a distinct `vertex_bytes`-byte slice (`#[repr(C)]`, tightly packed); the two
    // regions do not overlap (a fresh device allocation vs the caller's slice). The
    // copy completes before any GPU submit references the buffer.
    unsafe {
        core::ptr::copy_nonoverlapping(
            vertices.as_ptr().cast::<u8>(),
            vb_ptr.as_ptr(),
            vertex_bytes as usize,
        );
    }

    // --- Index buffer: width chosen above; the bytes are built host-side then copied. ---
    let index_bytes: Vec<u8> = match index_type {
        IndexType::Uint16 => {
            let mut bytes = Vec::with_capacity(indices.len() * 2);
            for &i in indices {
                debug_assert!(
                    (i as usize) < vertex_count,
                    "invariant: index in range for the u16 narrow path"
                );
                bytes.extend_from_slice(&(i as u16).to_le_bytes());
            }
            // Code review P2-1 (Decision 0 / VB geometry-table fetch): a `Uint16` index buffer
            // with an ODD `index_count` (a real, common case — every mesh's `index_count` is a
            // multiple of 3, which is frequently odd) leaves `bytes.len()` a multiple of 2 but
            // NOT of 4. `vb_geom_fetch.hlsli`'s `vb_load_index` reads the LAST index via one
            // 4-byte-aligned `ByteAddressBuffer::Load` (loading both 16-bit halves of the
            // containing word, then masking to the half it needs) — for the last index of an
            // odd-count buffer, that 4-byte load's upper half falls 2 bytes past
            // `bytes.len()`. This engine does not enable `robustBufferAccess` (`device.rs`'s
            // `enabled_features`), so that would be a genuine OOB `STORAGE_BUFFER` read past this
            // buffer's allocated end. Pad the ALLOCATION (not the index count/`MeshGpu::
            // index_count`, which stays the real, unpadded value — `vkCmdDrawIndexed`'s
            // fixed-function index fetch never reads past `index_count` regardless of the
            // underlying buffer's byte size) to the next 4-byte multiple; the pad byte's VALUE is
            // irrelevant (the fetch masks it away), zeroed here only for cleanliness (no
            // genuinely-uninitialized host memory left mapped). Unconditional (not gated on
            // `ctx.vb_geometry_table_armed()`): two harmless trailing zero bytes in a
            // `VERTEX|INDEX`-only buffer are never read by the fixed-function index fetch on ANY
            // path, so padding costs nothing even when the VB table is never used.
            if !bytes.len().is_multiple_of(4) {
                bytes.extend_from_slice(&[0u8, 0u8]);
            }
            bytes
        }
        IndexType::Uint32 => {
            let mut bytes = Vec::with_capacity(indices.len() * 4);
            for &i in indices {
                bytes.extend_from_slice(&i.to_le_bytes());
            }
            bytes
        }
    };
    let index_buffer = ctx
        .create_buffer(&BufferDesc {
            size: index_bytes.len() as u64,
            usage: index_usage,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("invariant: mesh index buffer create");
    let ib_ptr = ctx
        .buffer_mapped_ptr(&index_buffer)
        .expect("invariant: host-visible index buffer is mapped");
    // SAFETY: `ib_ptr` points to `index_bytes.len()` mapped host-coherent bytes;
    // `index_bytes` is a distinct, equally-sized owned allocation (no overlap with the
    // device buffer). The copy completes before any GPU submit references the buffer.
    unsafe {
        core::ptr::copy_nonoverlapping(
            index_bytes.as_ptr(),
            ib_ptr.as_ptr(),
            index_bytes.len(),
        );
    }

    // HW-RT rung R2a-3: build this mesh's per-mesh BLAS EAGERLY on an RT device (Principle 0
    // — durable per-mesh data ON the record). The BLAS reads the vertex + index buffers just
    // created (at the mesh's REAL index width — no duplicate `u32` buffer), so it must build
    // BEFORE they move into `MeshGpu`; `build_blas` submits + fence-waits synchronously and
    // caches only the device addresses (it keeps no reference to these buffers). hwrt-off OR a
    // non-RT GPU ⇒ no BLAS (byte-identical to the pre-R2a registry).
    #[cfg(feature = "hwrt")]
    let blas = {
        if ctx.ray_query_enabled() {
            let as_index_type = match index_type {
                IndexType::Uint16 => AsIndexType::Uint16,
                IndexType::Uint32 => AsIndexType::Uint32,
            };
            let built = boyko_rhi_vulkan::accel_build::build_blas(
                ctx,
                &ctx.rhi_queue(),
                &boyko_rhi_vulkan::accel_build::BlasBuildInput {
                    vertex_buffer: &vertex_buffer,
                    index_buffer: &index_buffer,
                    vertex_count: vertex_count as u32,
                    index_count: indices.len() as u32,
                    vertex_stride: VERTEX_STRIDE as u64,
                    index_type: as_index_type,
                },
            )
            .expect("invariant: mesh BLAS build on an RT device");
            Some(built)
        } else {
            None
        }
    };

    // Multi-paradigm render-path plan, rung R-VBGEO (Decision 0 / Rev-5 streaming
    // invariant): claim a geometry-table slot for this mesh IFF the caller threaded a
    // live table AND the boot-committed flag is armed — the two checks are threaded
    // independently (see this fn's doc), but every REAL caller keeps them in lockstep
    // (`GpuUpload for MeshGpu::upload` only ever passes `Some` when the SAME
    // `ctx.vb_geometry_table_armed()` is `true` — see that impl's doc), so a mesh's
    // buffers and its geometry-table registration are never inconsistent in practice.
    // `register_mesh`/`cube`/`plane` always pass `None` this rung (documented scope
    // cut above), so `geometry_slot` stays `VB_GEOMETRY_RESERVED_SLOT` for every
    // host-authored mesh today.
    let geometry_slot = match geometry_table {
        Some(table) if ctx.vb_geometry_table_armed() => table.register(
            ctx,
            &vertex_buffer,
            vertex_count as u32,
            &index_buffer,
            indices.len() as u32,
            index_type,
        ),
        _ => VB_GEOMETRY_RESERVED_SLOT,
    };

    MeshGpu {
        vertex_buffer,
        index_buffer,
        index_count: indices.len() as u32,
        index_type,
        vertex_count: vertex_count as u32,
        #[cfg(feature = "hwrt")]
        blas,
        geometry_slot,
        local_min,
        local_max,
    }
}

/// The pure (host-only) geometry for [`MeshAssetsExt::cube`]: 24 unique vertices
/// (per-face outward normals + planar UVs + a Lengyel-generated tangent basis —
/// exact/analytic here, since every face is a single planar quad) and 36 indices.
/// Factored out of `cube` so the vertex/UV/tangent math is unit-testable without a
/// `VulkanContext` — the GPU upload stays in `cube` itself.
pub(crate) fn cube_geometry(size: f32) -> ([Vertex; 24], [u32; 36]) {
    const COLOR: [f32; 4] = [0.82, 0.82, 0.82, 1.0];
    // Corner-order-matched planar UV: corner `c` of every face gets `QUAD_UV[c]`,
    // consistent with the `(0,1,2)`+`(0,2,3)` two-triangle fan below.
    const QUAD_UV: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let h = size * 0.5;
    // Six faces × four corners; each face carries its own outward normal so
    // the G-buffer normal lane is face-correct (no vertex-normal averaging).
    // Faces: +X, -X, +Y, -Y, +Z, -Z. Corner order is consistent per face so
    // one index pattern (two triangles per quad) covers all six.
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        ([1.0, 0.0, 0.0], [[h, -h, -h], [h, h, -h], [h, h, h], [h, -h, h]]),
        ([-1.0, 0.0, 0.0], [[-h, -h, h], [-h, h, h], [-h, h, -h], [-h, -h, -h]]),
        ([0.0, 1.0, 0.0], [[-h, h, -h], [-h, h, h], [h, h, h], [h, h, -h]]),
        ([0.0, -1.0, 0.0], [[-h, -h, h], [-h, -h, -h], [h, -h, -h], [h, -h, h]]),
        ([0.0, 0.0, 1.0], [[-h, -h, h], [h, -h, h], [h, h, h], [-h, h, h]]),
        ([0.0, 0.0, -1.0], [[h, -h, -h], [-h, -h, -h], [-h, h, -h], [h, h, -h]]),
    ];
    let mut vertices = [Vertex::new([0.0; 3], [0.0; 3], COLOR); 24];
    let mut indices = [0u32; 36];
    for (f, (normal, corners)) in faces.iter().enumerate() {
        for (c, corner) in corners.iter().enumerate() {
            let mut v = Vertex::new(*corner, *normal, COLOR);
            v.uv = QUAD_UV[c];
            vertices[f * 4 + c] = v;
        }
        let base = (f * 4) as u32;
        indices[f * 6..f * 6 + 6].copy_from_slice(&[
            base,
            base + 1,
            base + 2,
            base,
            base + 2,
            base + 3,
        ]);
    }
    generate_tangents(&mut vertices, &indices);
    (vertices, indices)
}

/// The pure (host-only) geometry for [`MeshAssetsExt::plane`]: 4 vertices (a flat
/// `+Y`-normal quad with a planar UV) + 6 indices, tangent-generated the same way
/// as [`cube_geometry`]. Factored out for the same testability reason.
pub(crate) fn plane_geometry(size: f32) -> ([Vertex; 4], [u32; 6]) {
    const COLOR: [f32; 4] = [0.62, 0.62, 0.62, 1.0];
    const NORMAL: [f32; 3] = [0.0, 1.0, 0.0];
    let h = size * 0.5;
    let mut vertices = [
        Vertex::new([-h, 0.0, -h], NORMAL, COLOR),
        Vertex::new([-h, 0.0, h], NORMAL, COLOR),
        Vertex::new([h, 0.0, h], NORMAL, COLOR),
        Vertex::new([h, 0.0, -h], NORMAL, COLOR),
    ];
    vertices[0].uv = [0.0, 0.0];
    vertices[1].uv = [0.0, 1.0];
    vertices[2].uv = [1.0, 1.0];
    vertices[3].uv = [1.0, 0.0];
    let indices = [0u32, 1, 2, 0, 2, 3];
    generate_tangents(&mut vertices, &indices);
    (vertices, indices)
}

impl MeshAssetsExt for Assets<MeshGpu> {
    fn register_mesh(
        &mut self,
        ctx: &VulkanContext,
        vertices: &[Vertex],
        indices: &[u32],
    ) -> MeshHandle {
        // `None`: host-authored registration does not thread the VB geometry table
        // this rung — see `build_mesh_gpu`'s doc for the scope cut.
        let mesh = build_mesh_gpu(ctx, vertices, indices, None);
        MeshHandle(self.add(mesh).index())
    }

    fn cube(&mut self, ctx: &VulkanContext, size: f32) -> MeshHandle {
        let (vertices, indices) = cube_geometry(size);
        self.register_mesh(ctx, &vertices, &indices)
    }

    fn plane(&mut self, ctx: &VulkanContext, size: f32) -> MeshHandle {
        let (vertices, indices) = plane_geometry(size);
        self.register_mesh(ctx, &vertices, &indices)
    }

    #[inline]
    fn mesh(&self, h: MeshHandle) -> &MeshGpu {
        self.get_by_index(h.0)
            .expect("invariant: MeshHandle resolves to a registered mesh")
    }

    #[inline]
    fn try_get(&self, h: MeshHandle) -> Option<&MeshGpu> {
        self.get_by_index(h.0)
    }

    unsafe fn destroy(&mut self, ctx: &VulkanContext) {
        // `Assets<T>` exposes no owned/mutable whole-table iteration (only the
        // borrowed `iter()` and the handle-keyed `remove()`), so every live handle is
        // collected FIRST (a small, one-shot teardown allocation — never on the
        // gameplay hot path, Principle 5's actual scope) and then removed by value,
        // one mesh at a time.
        let handles: Vec<Handle<MeshGpu>> = self.iter().map(|(h, _)| h).collect();
        for h in handles {
            #[cfg_attr(not(feature = "hwrt"), allow(unused_mut))]
            let mut mesh = self
                .remove(h)
                .expect("invariant: a handle collected from iter() resolves via remove()");
            // R2a-3 (P0-3): free the AS FIRST — the AS's memory lives in its backing buffer,
            // which MUST outlive it. `destroy_blas` frees the AS then its backing.
            #[cfg(feature = "hwrt")]
            if let Some(b) = mesh.blas.take() {
                // SAFETY: the device is idle (caller contract), so no submit builds/traces this
                // BLAS; `take` ensures it is destroyed exactly once (a repeat `destroy` call sees
                // `None`).
                unsafe { boyko_rhi_vulkan::accel_build::destroy_blas(ctx, b) };
            }
            // SAFETY: `mesh.vertex_buffer` / `mesh.index_buffer` were created by
            // `register_mesh` on this same `ctx`; the device is idle (caller contract), so
            // no submit references them; the by-value move destroys each exactly once. Any
            // per-mesh BLAS was already freed above (R2a-3).
            unsafe {
                ctx.destroy_buffer(mesh.vertex_buffer);
                ctx.destroy_buffer(mesh.index_buffer);
            }
        }
    }

    #[cfg(feature = "hwrt")]
    #[inline]
    fn blas_generation(&self) -> u64 {
        self.high_water() as u64
    }

    #[cfg(feature = "hwrt")]
    #[inline]
    fn blas_address(&self, h: MeshHandle) -> u64 {
        self.get_by_index(h.0)
            .and_then(|m| m.blas.as_ref())
            .map_or(0, |b| b.device_address)
    }
}

/// Multi-paradigm render-path plan, rung R8 — the host-authored-registration half of the
/// register_mesh geometry-table-slot gap [`build_mesh_gpu`]'s own doc flags: `register_mesh`/
/// `cube`/`plane` ([`MeshAssetsExt`]) always thread `None` for [`build_mesh_gpu`]'s
/// `geometry_table` parameter, so a host-authored mesh (spawned directly by app/test code, not
/// streamed through [`GpuUpload`](crate::gpu_upload::GpuUpload)) never claims a VB
/// geometry-table slot — its `geometry_slot` stays [`VB_GEOMETRY_RESERVED_SLOT`] forever, which
/// would make it unresolvable through `gMeshVerts[]`/`gMeshIndices[]` under a VB-resolved boot.
///
/// A SEPARATE extension trait (not a widened [`MeshAssetsExt`]) so this rung's fix stays
/// ADDITIVE: widening `register_mesh`'s own signature would force a `None`/`Option` argument
/// onto every one of its ~20 existing call sites across unrelated examples/tests/showcases,
/// well outside this rung's scope. `register_mesh_vb`/`cube_vb`/`plane_vb` mirror their
/// [`MeshAssetsExt`] counterparts exactly, with ONE extra parameter: the live
/// [`MeshGeometryTable`] to claim a slot in (typically `NonSendResMut<MeshGeometryTableSlot>`'s
/// `.0.as_mut()` in the caller's own startup system — `boyko_app::runner` inserts that resource,
/// `Some(...)`-armed, BEFORE `app.finish()` drains any startup system, so it is always available
/// to a `setup` fn that wants it, on EVERY boot — `None` on a non-VB boot, in which case a
/// caller falls back to the plain [`MeshAssetsExt`] method).
pub trait MeshAssetsVbExt {
    /// [`MeshAssetsExt::register_mesh`], but ALSO claims `geometry_table`'s next slot for this
    /// mesh (`build_mesh_gpu`'s `Some(geometry_table)` arm) — the VB-aware sibling.
    ///
    /// # Panics
    /// Same contract as [`MeshAssetsExt::register_mesh`].
    fn register_mesh_vb(
        &mut self,
        ctx: &VulkanContext,
        vertices: &[Vertex],
        indices: &[u32],
        geometry_table: &mut MeshGeometryTable,
    ) -> MeshHandle;

    /// [`MeshAssetsExt::cube`], VB-aware (see [`Self::register_mesh_vb`]).
    ///
    /// # Panics
    /// Same contract as [`MeshAssetsExt::cube`].
    fn cube_vb(&mut self, ctx: &VulkanContext, size: f32, geometry_table: &mut MeshGeometryTable) -> MeshHandle;

    /// [`MeshAssetsExt::plane`], VB-aware (see [`Self::register_mesh_vb`]).
    ///
    /// # Panics
    /// Same contract as [`MeshAssetsExt::plane`].
    fn plane_vb(&mut self, ctx: &VulkanContext, size: f32, geometry_table: &mut MeshGeometryTable) -> MeshHandle;
}

impl MeshAssetsVbExt for Assets<MeshGpu> {
    fn register_mesh_vb(
        &mut self,
        ctx: &VulkanContext,
        vertices: &[Vertex],
        indices: &[u32],
        geometry_table: &mut MeshGeometryTable,
    ) -> MeshHandle {
        let mesh = build_mesh_gpu(ctx, vertices, indices, Some(geometry_table));
        MeshHandle(self.add(mesh).index())
    }

    fn cube_vb(&mut self, ctx: &VulkanContext, size: f32, geometry_table: &mut MeshGeometryTable) -> MeshHandle {
        let (vertices, indices) = cube_geometry(size);
        self.register_mesh_vb(ctx, &vertices, &indices, geometry_table)
    }

    fn plane_vb(&mut self, ctx: &VulkanContext, size: f32, geometry_table: &mut MeshGeometryTable) -> MeshHandle {
        let (vertices, indices) = plane_geometry(size);
        self.register_mesh_vb(ctx, &vertices, &indices, geometry_table)
    }
}

/// Fill-reject `MeshGpu` values awaiting the fence-gated device-free pass
/// (asset-streaming plan F6 Decision 4).
///
/// `Assets::fill`'s `Err((_, MeshGpu))` arm returns a value the store never
/// took ownership of (a stale/double-fill target) — it still holds LIVE
/// device buffers (and, under hwrt, a BLAS) with no `Drop` to free them.
/// [`DeferredFree`](boyko_scene::DeferredFree) cannot carry it: a
/// `FreeEntry` is a `slot: u32` into a store-owned row, and this value was
/// never stored at all; `DeferredFree` itself is `Send + Sync` POD in
/// `boyko_scene`, which cannot depend on the `!Send` `MeshGpu` (wrong crate
/// direction) or hold a non-POD payload. This dedicated `!Send`
/// `NonSendResource` is therefore the only correct home — a caller pushes
/// the rejected value here with its own fence-gate stamp, and
/// `retire_deferred_frees` (`boyko_render::asset_refcount`, F6) drains it on
/// the same `epoch` gate as every store-owned retire.
#[derive(Default)]
pub struct OrphanedMeshGpu {
    orphans: Vec<(MeshGpu, u64)>,
}

impl NonSendResource for OrphanedMeshGpu {}

impl OrphanedMeshGpu {
    /// Queues a fill-rejected `MeshGpu` value for teardown once every submit
    /// that could reference it is fence-complete. `retire_frame` is the
    /// caller-computed gate (`RenderEpoch` at reject time `+ RETIRE_DELAY`) —
    /// strictly conservative here, since a value `fill` rejected was never
    /// submitted at all (see the F6 design's proof C).
    #[inline]
    pub fn push(&mut self, mesh: MeshGpu, retire_frame: u64) {
        self.orphans.push((mesh, retire_frame));
    }

    /// `true` if no orphan is awaiting teardown — the O(1) golden early-out
    /// (no `fill` caller exists in-tree yet, so this is always `true` today).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.orphans.is_empty()
    }

    /// Tears down (BLAS before its backing buffer, then the vertex/index
    /// buffers — mirrors [`MeshAssetsExt::destroy`]'s ordering) every orphan
    /// whose `retire_frame <= epoch`, retaining the rest in enqueue order.
    /// Called ONLY by `retire_deferred_frees` (`boyko_render::asset_refcount`,
    /// F6) AFTER `wait_frame_in_flight` for this `epoch` — the same
    /// per-resource fence precondition every other F6 destroy call relies on.
    pub fn drain_ready(&mut self, epoch: u64, ctx: &VulkanContext) {
        let mut i = 0;
        while i < self.orphans.len() {
            if self.orphans[i].1 > epoch {
                i += 1;
                continue;
            }
            let (mesh, _) = self.orphans.remove(i);
            // R2a-3 (P0-3): free the AS FIRST — its memory lives in its backing
            // buffer, which must outlive it (mirrors `MeshAssetsExt::destroy`).
            #[cfg(feature = "hwrt")]
            if let Some(b) = mesh.blas {
                // SAFETY: this value was NEVER submitted (the store rejected it
                // before any draw could reference it — F6 design proof C), so
                // no GPU work reads its AS/buffers; `remove` above guarantees
                // it is destroyed exactly once.
                unsafe { boyko_rhi_vulkan::accel_build::destroy_blas(ctx, b) };
            }
            // SAFETY: `mesh.vertex_buffer` / `mesh.index_buffer` were created
            // by the failed upload attempt on this same `ctx`; the value was
            // never submitted (see above), so no GPU work references them;
            // the by-value move destroys each exactly once.
            unsafe {
                ctx.destroy_buffer(mesh.vertex_buffer);
                ctx.destroy_buffer(mesh.index_buffer);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CSM auto-fit plan T16: `cube_geometry`'s 24 vertices span `[-h, h]` on every axis
    /// (`h = size * 0.5`, see the faces table in `cube_geometry`) — `local_aabb` must
    /// recover exactly that half-edge box, without a `VulkanContext`.
    #[test]
    fn local_aabb_of_cube_is_half_edge() {
        let size = 2.0_f32;
        let (vertices, _indices) = cube_geometry(size);
        let (min, max) = local_aabb(&vertices);
        let h = size * 0.5;
        assert_eq!(min, [-h, -h, -h]);
        assert_eq!(max, [h, h, h]);
    }
}
