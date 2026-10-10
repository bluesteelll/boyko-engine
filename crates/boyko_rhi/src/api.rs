//! The umbrella [`RhiApi`] trait collecting every backend resource type.
//!
//! Modeled on `wgpu-hal`'s `Api`: one marker trait gathers all associated
//! resource types; the operational traits ([`RhiDevice`], [`RhiQueue`],
//! [`RhiCommandEncoder`]) are separate and reference `A: RhiApi`. A backend is a
//! zero-sized marker (`struct Vulkan;`) implementing `RhiApi`. Static dispatch +
//! monomorphization means every call lowers to a direct, non-virtual call —
//! `RhiApi` is intentionally **not** object-safe.

use crate::device::RhiDevice;
use crate::encoder::RhiCommandEncoder;
use crate::queue::RhiQueue;

/// Umbrella trait collecting every backend resource type (wgpu-hal `Api` shape).
///
/// **Not object-safe. Static dispatch only.** The bound `Sized + 'static`
/// reflects that backends are zero-sized markers with no borrowed state.
///
/// The foundation-now associated types ([`RhiApi::Device`], [`RhiApi::Queue`],
/// [`RhiApi::CommandEncoder`], plus the owned-resource types) carry their
/// operational-trait bounds. The deferred-seam types (`Surface`, `Swapchain`,
/// `Semaphore`, textures, graphics pipelines, bind groups) are **plain
/// unbounded** associated types: declared now so the trait surface stays stable
/// across phases, but with no trait bound and no impl until the relevant phase
/// (Phase 2-3 for on-screen, Phase 6+ for SDF/graphics) lands.
pub trait RhiApi: Sized + 'static {
    // ===== FOUNDATION-NOW (headless compute) =====

    /// The logical device: creates/destroys resources, maps buffers, waits
    /// fences (see [`RhiDevice`]).
    type Device: RhiDevice<Self>;
    /// The submission queue (see [`RhiQueue`]).
    type Queue: RhiQueue<Self>;
    /// The command-recording encoder on the hot path (see [`RhiCommandEncoder`]).
    type CommandEncoder: RhiCommandEncoder<Self>;

    /// An owned GPU buffer (Vulkan backend: `BoundBuffer`).
    type Buffer;
    /// An owned compiled shader module.
    type ShaderModule;
    /// An owned compute pipeline (Vulkan backend: `ComputePipeline`).
    type ComputePipeline;
    /// An owned fence for CPU↔GPU submission synchronization.
    type Fence;
    /// An owned GPU timestamp-query pool (HW-RT rung R0: the per-pass GPU
    /// wall-clock bracket resource — Vulkan backend: `VulkanQueryPool`).
    type QueryPool;

    // ===== DEFERRED SEAM — plain unbounded associated types =====
    // Declared now (cheap; avoids a later ABA/ABI break) but with no operational
    // trait bound and no backend impl until the named phase lands.

    /// Window/surface handle. Seam: Phase 2-3 (the concrete `Surface` is used
    /// directly meanwhile).
    type Surface;
    /// Swapchain. Seam: Phase 2-3.
    type Swapchain;
    /// Per-frame acquire / render-finished semaphore. Seam: Phase 2-3.
    type Semaphore;
    /// SDF 3D storage image / texture. Seam: Phase 6+.
    type Texture;
    /// An owned EXPLICIT image view over a sub-range of a [`Self::Texture`] — a
    /// single mip level, a layer slice, or a format reinterpretation (VG R3 step S1;
    /// Vulkan backend: `VulkanTextureView`).
    ///
    /// Distinct from the views a texture creates for ITSELF (which the backend keeps
    /// inside the `Texture` and never hands out as owned values): those all start at
    /// mip 0, so none of them can name mip `k`. This type is what a caller owns, and it
    /// is owned by whichever struct owns the texture — see the ownership rule on
    /// [`RhiDevice::create_texture_view`].
    type TextureView;
    /// Texture sampler. Seam: Phase 6+.
    type Sampler;
    /// Dynamic-rendering graphics pipeline. Seam: Phase 6+.
    type GraphicsPipeline;
    /// A bound descriptor set. Seam: Phase 6+ (supersedes the fixed compute layout).
    type BindGroup;
    /// A descriptor-set layout. Seam: Phase 6+.
    type BindGroupLayout;
    /// A ray-tracing acceleration structure (BLAS/TLAS). Seam: HW-RT rung R2a.
    ///
    /// Declared now (HW-RT rung R1) as a BARE unbounded associated type — no
    /// operational-trait bound, no backend verbs, no FFI — so the R2a create /
    /// build / refit verbs (and the `BoundAccelStruct` binding) land without an
    /// [`RhiApi`] ABI break, exactly how `Surface`/`Swapchain`/`Texture` were
    /// declared phases before their verbs. In R1 both backends bind it to `()`.
    type AccelerationStructure;
}
