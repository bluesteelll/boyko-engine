//! The renderer-owned mesh asset table (mesh foundation M2).
//!
//! [`MeshRegistry`] is a DENSE, renderer-owned table of GPU mesh assets keyed by
//! [`MeshHandle`](boyko_scene::render_caps::MeshHandle)`.0` (a `u32` index). Each
//! [`MeshGpu`] OWNS the RHI vertex + index buffers a mesh is drawn from. This is a
//! legitimate Principle-0 FFI/GPU exception (the same class as `RhiContext`'s owned
//! device buffers and the swapchain's images), NOT a parallel ECS data system: the
//! durable *per-entity* render state (which mesh an entity uses, its transform, its
//! visibility) lives in ECS `ComponentPool` columns; `MeshHandle` was DESIGNED to be a
//! small dense INDEX into THIS table of immutable, shared GPU assets. An entity-keyed
//! store of vertex buffers would be the violation; a small handle-indexed asset table is
//! the asset cache every renderer needs and the ECS columns point into.
//!
//! `MeshRegistry` is `!Send` (it owns RHI buffers, which are device-bound and
//! single-thread-touch), so it is registered as a
//! [`NonSendResource`](boyko_ecs::ecs::core::resources::resource::NonSendResource)
//! alongside [`RhiContext`](crate::RhiContext). The registry is the immutable, SHARED
//! ASSET table: it is populated once at setup via [`register_mesh`](Self::register_mesh) /
//! [`cube`](Self::cube) / [`plane`](Self::plane) — assets are shared across entities, NOT
//! auto-created per spawned handle (that would re-upload geometry per instance). The M3
//! ECS gather that reads spawned `(MeshHandle, InstanceModelCol)` entities and buckets
//! them into per-mesh draw batches + the shared instance ring is
//! [`gather_mesh_draws`](crate::mesh_draw::gather_mesh_draws) (`crate::mesh_draw`, SHIPPED);
//! it resolves each bucket's `mesh_id` to THIS table for the draw. The gather also emits a
//! parallel per-instance mesh-id lane so the instance ring is directly TLAS-consumable
//! (mesh foundation M3 → HW-RT).
//!
//! # The vertex contract
//!
//! [`Vertex`] mirrors the `gbuffer_mrt.vs` vertex input EXACTLY: `position` @0,
//! `normal` @12, `color` @24, a 40-byte `#[repr(C)]` stride. The instanced arm reads
//! these as MODEL-SPACE positions and transforms them by the per-instance 3x4 affine
//! the instance SSBO carries; the registry stores the model-space mesh once and every
//! instance reuses it.

use boyko_ecs::ecs::core::resources::resource::NonSendResource;
use boyko_rhi::enums::IndexType;
use boyko_rhi::{BufferDesc, BufferUsage, MemoryLocation, RhiDevice};
#[cfg(feature = "hwrt")]
use boyko_rhi::AsIndexType;
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_scene::render_caps::MeshHandle;

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
/// it the registry mints `Uint32` indices. `65_536 == u16::MAX + 1` (indices `0..=65535`).
pub const U16_INDEX_VERTEX_LIMIT: usize = u16::MAX as usize + 1;

/// One GPU-resident mesh asset: the OWNED vertex + index RHI buffers plus the draw
/// metadata the instanced gbuffer arm needs (`index_count`, `index_type`,
/// `vertex_count`). The buffers are host-visible coherent (seeded once at registration,
/// then read-only on the GPU), mirroring the test harness's vertex-buffer discipline.
///
/// `MeshGpu` does NOT implement `Drop`: an RHI [`BoundBuffer`] must be destroyed through
/// the owning [`VulkanContext`] (`destroy_buffer`) AFTER the device is idle, which a
/// blind `Drop` cannot guarantee. [`MeshRegistry::destroy`] tears the buffers down
/// explicitly under the caller's idle contract (the same pattern as the test harness's
/// hand-rolled buffer teardown).
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
    /// [`register_mesh`](MeshRegistry::register_mesh) under
    /// [`ray_query_enabled`](boyko_rhi_vulkan::device::VulkanContext::ray_query_enabled)
    /// (`None` on a non-RT device or hwrt OFF), read at its real index width from the mesh's
    /// existing index buffer (no duplicate `u32` buffer). Freed FIRST in
    /// [`destroy`](MeshRegistry::destroy) (AS before its backing, device-idle contract).
    #[cfg(feature = "hwrt")]
    pub blas: Option<boyko_rhi_vulkan::accel_build::BuiltBlas>,
}

/// A DENSE, renderer-owned table of [`MeshGpu`] assets keyed by `MeshHandle.0`.
///
/// `meshes[h.0]` is the asset a [`MeshHandle`] `h` resolves to; handles are minted in
/// registration order (`0, 1, 2, …`), so the table never has gaps. Registered as a
/// [`NonSendResource`] alongside [`RhiContext`](crate::RhiContext).
pub struct MeshRegistry {
    /// The dense asset column, `meshes[MeshHandle.0]`. Address stability is NOT required
    /// (handles are integer indices, never pointers), so a plain `Vec` is correct here.
    meshes: Vec<MeshGpu>,
    /// HW-RT rung R2a-3: bumped once per [`register_mesh`](Self::register_mesh) that built a
    /// BLAS. The host's per-frame TLAS-instance packer reads a per-mesh BLAS-address table;
    /// this generation lets it rewrite that (frame-invariant) table ONLY when a new mesh
    /// registered, not every frame (BLASes never move — spec).
    #[cfg(feature = "hwrt")]
    blas_generation: u64,
}

impl NonSendResource for MeshRegistry {}

impl MeshRegistry {
    /// An empty registry with NO preallocation. Prefer [`with_reserved`](Self::with_reserved)
    /// when the mesh count is known at setup (Principle 5 — preallocate, no gameplay-time
    /// realloc).
    #[inline]
    pub fn new() -> Self {
        Self {
            meshes: Vec::new(),
            #[cfg(feature = "hwrt")]
            blas_generation: 0,
        }
    }

    /// An empty registry preallocated for `capacity` meshes (Principle 5: the setup path
    /// reserves the known asset count once so registration never reallocates the column).
    #[inline]
    pub fn with_reserved(capacity: usize) -> Self {
        Self {
            meshes: Vec::with_capacity(capacity),
            #[cfg(feature = "hwrt")]
            blas_generation: 0,
        }
    }

    /// The number of registered meshes (the next handle's index).
    #[inline]
    pub fn len(&self) -> usize {
        self.meshes.len()
    }

    /// Whether no mesh is registered yet.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty()
    }

    /// Uploads a model-space mesh (`vertices` + triangle `indices`) into a fresh GPU
    /// asset and returns its [`MeshHandle`].
    ///
    /// The index width is chosen by O3: `Uint16` when the unique vertex count is at or
    /// below [`U16_INDEX_VERTEX_LIMIT`] (every index fits a `u16`), else `Uint32`. The
    /// `u32` `indices` are narrowed to `u16` on the `Uint16` path; the caller's indices
    /// MUST be in `0..vertices.len()` (a `debug_assert!` catches an out-of-range index,
    /// which would be a `u16` truncation bug on the narrow path).
    ///
    /// Both buffers are `HostVisibleCoherent` and seeded once here (read-only on the GPU
    /// afterward), reusing the RHI's `create_buffer` + `buffer_mapped_ptr` upload helpers
    /// — the SAME host-coherent staging discipline the UI / vertex-buffer paths use; no
    /// hand-rolled Vulkan.
    ///
    /// # Panics
    /// Panics (`expect`) if either buffer create or its host mapping fails — a device
    /// out-of-memory at asset-registration time is a setup failure, not a recoverable
    /// per-frame error.
    pub fn register_mesh(
        &mut self,
        ctx: &VulkanContext,
        vertices: &[Vertex],
        indices: &[u32],
    ) -> MeshHandle {
        debug_assert!(!vertices.is_empty(), "invariant: a mesh has at least one vertex");
        debug_assert!(!indices.is_empty(), "invariant: an indexed mesh has at least one index");
        let vertex_count = vertices.len();
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
        let vertex_usage = BufferUsage::VERTEX | as_bits;
        let index_usage = BufferUsage::INDEX | as_bits;

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
        // non-RT GPU ⇒ no BLAS, no generation bump (byte-identical to the pre-R2a registry).
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
                self.blas_generation += 1;
                Some(built)
            } else {
                None
            }
        };

        let handle = MeshHandle(self.meshes.len() as u32);
        self.meshes.push(MeshGpu {
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            index_type,
            vertex_count: vertex_count as u32,
            #[cfg(feature = "hwrt")]
            blas,
        });
        handle
    }

    /// Registers an axis-aligned CUBE of edge length `size`, centered at the
    /// model-space origin, with per-face outward normals (24 unique vertices,
    /// 36 indices — `Uint16` by O3) and a neutral light-gray base color. The
    /// canonical primitive for a first scene (host plan R3); place it with the
    /// entity's `Transform`.
    ///
    /// # Panics
    /// Same contract as [`register_mesh`](Self::register_mesh): a buffer create
    /// / map failure at asset-registration time is a setup failure.
    pub fn cube(&mut self, ctx: &VulkanContext, size: f32) -> MeshHandle {
        const COLOR: [f32; 4] = [0.82, 0.82, 0.82, 1.0];
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
        let mut vertices = [Vertex {
            position: [0.0; 3],
            normal: [0.0; 3],
            color: COLOR,
        }; 24];
        let mut indices = [0u32; 36];
        for (f, (normal, corners)) in faces.iter().enumerate() {
            for (c, corner) in corners.iter().enumerate() {
                vertices[f * 4 + c] = Vertex {
                    position: *corner,
                    normal: *normal,
                    color: COLOR,
                };
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
        self.register_mesh(ctx, &vertices, &indices)
    }

    /// Registers a flat XZ-plane quad of side length `size`, centered at the
    /// model-space origin at `y == 0`, normal `+Y` (4 vertices, 6 indices —
    /// `Uint16` by O3), with a neutral mid-gray base color — the canonical
    /// floor/receiver primitive (host plan R3).
    ///
    /// # Panics
    /// Same contract as [`register_mesh`](Self::register_mesh).
    pub fn plane(&mut self, ctx: &VulkanContext, size: f32) -> MeshHandle {
        const COLOR: [f32; 4] = [0.62, 0.62, 0.62, 1.0];
        const NORMAL: [f32; 3] = [0.0, 1.0, 0.0];
        let h = size * 0.5;
        let vertices = [
            Vertex { position: [-h, 0.0, -h], normal: NORMAL, color: COLOR },
            Vertex { position: [-h, 0.0, h], normal: NORMAL, color: COLOR },
            Vertex { position: [h, 0.0, h], normal: NORMAL, color: COLOR },
            Vertex { position: [h, 0.0, -h], normal: NORMAL, color: COLOR },
        ];
        let indices = [0u32, 1, 2, 0, 2, 3];
        self.register_mesh(ctx, &vertices, &indices)
    }

    /// Resolves a [`MeshHandle`] to its GPU asset.
    ///
    /// # Panics
    /// Panics if `h` is out of range — a handle the registry never minted is a
    /// caller/asset-binding bug, not a recoverable error (the ECS gather only emits
    /// handles this registry returned).
    #[inline]
    pub fn get(&self, h: MeshHandle) -> &MeshGpu {
        &self.meshes[h.0 as usize]
    }

    /// Resolves a [`MeshHandle`] to its GPU asset, or `None` if the handle is out of
    /// range (the fallible counterpart of [`get`](Self::get) for a gather that may hold a
    /// not-yet-registered handle).
    #[inline]
    pub fn try_get(&self, h: MeshHandle) -> Option<&MeshGpu> {
        self.meshes.get(h.0 as usize)
    }

    /// Destroys every registered mesh's RHI buffers through `ctx` and empties the table.
    ///
    /// # Safety
    /// The caller MUST have made the device idle (e.g. via the renderer's `Drop` /
    /// `wait_idle`) so no in-flight submit still references any mesh buffer; each buffer
    /// is destroyed exactly once. Mirrors the test harness's explicit buffer teardown.
    pub unsafe fn destroy(&mut self, ctx: &VulkanContext) {
        #[cfg(feature = "hwrt")]
        for mesh in self.meshes.iter_mut() {
            // R2a-3 (P0-3): free the AS FIRST — the AS's memory lives in its backing buffer,
            // which MUST outlive it. `destroy_blas` frees the AS then its backing.
            // SAFETY: the device is idle (caller contract), so no submit builds/traces this
            // BLAS; `take` ensures it is destroyed exactly once (subsequent iterations see `None`).
            if let Some(b) = mesh.blas.take() {
                unsafe { boyko_rhi_vulkan::accel_build::destroy_blas(ctx, b) };
            }
        }
        for mesh in self.meshes.drain(..) {
            // SAFETY: each buffer was created by `register_mesh` on `ctx`; the device is
            // idle (caller contract), so no submit references it; the by-value move
            // destroys it exactly once. Any per-mesh BLAS was already freed above (R2a-3).
            unsafe {
                ctx.destroy_buffer(mesh.vertex_buffer);
                ctx.destroy_buffer(mesh.index_buffer);
            }
        }
    }

    /// HW-RT rung R2a-3: the current BLAS-address generation — bumped once per
    /// [`register_mesh`](Self::register_mesh) that built a BLAS. The host's per-frame
    /// TLAS-instance packer rewrites its (frame-invariant) per-mesh BLAS-address table ONLY when
    /// this advances (BLASes never move — spec), never per frame.
    #[cfg(feature = "hwrt")]
    #[inline]
    pub fn blas_generation(&self) -> u64 {
        self.blas_generation
    }

    /// HW-RT rung R2a-3: mesh `h`'s BLAS device address (a TLAS instance's
    /// `accelerationStructureReference`), or `0` if the handle has no BLAS (a non-RT device, or a
    /// handle the registry never minted). Non-zero for every mesh registered on an RT device.
    #[cfg(feature = "hwrt")]
    #[inline]
    pub fn blas_address(&self, h: MeshHandle) -> u64 {
        self.meshes
            .get(h.0 as usize)
            .and_then(|m| m.blas.as_ref())
            .map_or(0, |b| b.device_address)
    }
}

impl Default for MeshRegistry {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}
