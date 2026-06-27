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
//! alongside [`RhiContext`](crate::RhiContext). M2 builds the registry directly in the
//! test harness and drives ONE registered mesh through the instanced gbuffer arm; the
//! ECS gather that fills it from spawned `MeshHandle` components is M3.
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
}

impl NonSendResource for MeshRegistry {}

impl MeshRegistry {
    /// An empty registry with NO preallocation. Prefer [`with_reserved`](Self::with_reserved)
    /// when the mesh count is known at setup (Principle 5 — preallocate, no gameplay-time
    /// realloc).
    #[inline]
    pub fn new() -> Self {
        Self { meshes: Vec::new() }
    }

    /// An empty registry preallocated for `capacity` meshes (Principle 5: the setup path
    /// reserves the known asset count once so registration never reallocates the column).
    #[inline]
    pub fn with_reserved(capacity: usize) -> Self {
        Self {
            meshes: Vec::with_capacity(capacity),
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

        // --- Vertex buffer: copy the model-space vertices in once. ---
        let vertex_bytes = core::mem::size_of_val(vertices) as u64;
        let vertex_buffer = ctx
            .create_buffer(&BufferDesc {
                size: vertex_bytes,
                usage: BufferUsage::VERTEX,
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
                usage: BufferUsage::INDEX,
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

        let handle = MeshHandle(self.meshes.len() as u32);
        self.meshes.push(MeshGpu {
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            index_type,
            vertex_count: vertex_count as u32,
        });
        handle
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
        for mesh in self.meshes.drain(..) {
            // SAFETY: each buffer was created by `register_mesh` on `ctx`; the device is
            // idle (caller contract), so no submit references it; the by-value move
            // destroys it exactly once.
            unsafe {
                ctx.destroy_buffer(mesh.vertex_buffer);
                ctx.destroy_buffer(mesh.index_buffer);
            }
        }
    }
}

impl Default for MeshRegistry {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}
