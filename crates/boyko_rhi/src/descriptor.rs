//! POD descriptor structs passed into the RHI create/record calls.
//!
//! Each foundation descriptor fits within a cache line; barriers are stack
//! locals the backend walks once. Recording is single-threaded on the dispatcher
//! during the apply-window (plan §5.3), so false-sharing is not a concern.

use crate::api::RhiApi;
use crate::enums::{BarrierAccess, BarrierStage, BufferUsage, MemoryLocation};

/// Parameters for [`crate::device::RhiDevice::create_buffer`].
///
/// `#[repr(C)]` so the field layout is stable (size + usage + location) — a
/// backend can read it without depending on Rust's default field reordering.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferDesc {
    /// Size of the buffer in bytes.
    pub size: u64,
    /// Usage bits the buffer must support.
    pub usage: BufferUsage,
    /// Where the backing memory lives.
    pub location: MemoryLocation,
}

/// One buffer-to-buffer copy region for
/// [`crate::encoder::RhiCommandEncoder::copy_buffer`].
///
/// `#[repr(C)]` with the exact `(src_offset, dst_offset, size)` field order and
/// `u64` types of Vulkan's `VkBufferCopy`, so a Vulkan backend can reinterpret a
/// `&[BufferCopy]` as a `&[VkBufferCopy]` without a per-region copy (the layout
/// match is asserted backend-side, plan MF-8). Used for the Phase-5 staging
/// upload + the test-only readback.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferCopy {
    /// Byte offset within the source buffer.
    pub src_offset: u64,
    /// Byte offset within the destination buffer.
    pub dst_offset: u64,
    /// Number of bytes to copy.
    pub size: u64,
}

/// Parameters for [`crate::device::RhiDevice::create_compute_pipeline`].
///
/// Generic over the backend `A` because it borrows that backend's owned shader
/// module by reference. The `'a` lifetime ties the descriptor to the borrowed
/// module + entry name for the duration of the create call (the backend copies
/// what it needs; nothing is retained past the call).
pub struct ComputePipelineDesc<'a, A: RhiApi> {
    /// The compiled shader module the pipeline's compute stage is built from.
    pub module: &'a A::ShaderModule,
    /// The shader entry-point name (today always `c"main"`).
    pub entry: &'a core::ffi::CStr,
    /// Size in bytes of the push-constant range bound at pipeline-layout time
    /// (4, today — the foundation's single u32 push constant).
    pub push_constant_bytes: u32,
}

/// A single buffer's access transition inside a [`BarrierDesc`].
///
/// `#[repr(C)]` for a stable, backend-readable layout. The `'a` lifetime borrows
/// the buffer for the barrier-record call only.
#[repr(C)]
pub struct BufferBarrier<'a, A: RhiApi> {
    /// The buffer whose access is transitioning.
    pub buffer: &'a A::Buffer,
    /// Access scope before the barrier (e.g. a prior shader write).
    pub src_access: BarrierAccess,
    /// Access scope after the barrier (e.g. a subsequent shader read).
    pub dst_access: BarrierAccess,
}

/// Parameters for [`crate::encoder::RhiCommandEncoder::pipeline_barrier`].
///
/// **Buffer-only** in Phase 1 (plan D3): the only image-layout transitions that
/// exist today live in the concrete `Renderer` and are not routed through the
/// trait. `ImageBarrier`/`images` is a genuine Phase-2-3 seam — intentionally
/// absent here.
///
/// The `buffers` slice is a stack local walked once by the backend; the
/// foundation chained-barrier path supplies 0 or 1 entries.
pub struct BarrierDesc<'a, A: RhiApi> {
    /// Pipeline stage(s) that must complete before the barrier.
    pub src_stage: BarrierStage,
    /// Pipeline stage(s) that wait on the barrier.
    pub dst_stage: BarrierStage,
    /// The buffer transitions covered by this barrier (foundation: 0 or 1).
    pub buffers: &'a [BufferBarrier<'a, A>],
}
