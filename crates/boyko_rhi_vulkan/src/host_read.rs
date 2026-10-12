//! OBS (render-wave plan H0 Scope 7; perf O2's class, closed once): the HOST-read barrier every
//! GPU→host copy needs before the host reads the destination.
//!
//! A fence wait makes the device's writes AVAILABLE in the device domain; it does not make them
//! VISIBLE to the host. The Vulkan memory model requires a memory dependency whose destination is
//! `VK_PIPELINE_STAGE_HOST_BIT` / `VK_ACCESS_HOST_READ_BIT` (the host side of the "fence then
//! read" pattern every readback in this tree uses). Before OBS no such dependency was recorded
//! anywhere (`rg 'HOST_READ|PIPELINE_STAGE_HOST' crates/` returned 0): the readbacks worked on
//! this box because its host-visible memory is coherent and the driver happens to flush, which is
//! not a guarantee. One helper, recorded right after every copy whose destination the host reads:
//!
//! - the raw recorder sites in `present/passes/*` (swapchain readbacks, the HZB dump, the VG
//!   census, the VB cull readback) call [`record_host_read_barrier`] after their copies;
//! - `rhi_impl/encoder.rs` records it after `copy_buffer` / `copy_image_to_buffer` whenever the
//!   destination is MAPPED (`BoundBuffer::mapped.is_some()`), which closes the RHI-encoder family
//!   (`particle_readback`, the shadow-poison probe, every test harness) in one place.
//!
//! `tests/host_read_barrier_census.rs` keeps it that way: a readback copy site with no helper call
//! after it, or an unclassified new `cmd_copy_buffer` site, is red.

use core::ptr;

use crate::device::DeviceFns;
use crate::ffi::{
    VkBuffer, VkBufferMemoryBarrier, VkCommandBuffer, VkStructureType, VK_ACCESS_HOST_READ_BIT,
    VK_ACCESS_TRANSFER_WRITE_BIT, VK_PIPELINE_STAGE_HOST_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT,
    VK_QUEUE_FAMILY_IGNORED, VK_WHOLE_SIZE,
};

/// The most destination buffers one [`record_host_read_barrier`] call names (the barrier array is
/// built on the stack).
pub(crate) const MAX_HOST_READ_BUFFERS: usize = 4;

/// Records ONE `vkCmdPipelineBarrier(TRANSFER → HOST)` carrying a whole-range
/// `TRANSFER_WRITE → HOST_READ` buffer barrier for every buffer in `buffers` — the dependency that
/// makes a transfer's writes visible to a host read after the fence wait.
///
/// # Safety
///
/// The caller guarantees that `cmd` is a command buffer in the recording state, OUTSIDE any render
/// pass or dynamic-rendering scope (a pipeline barrier with a `HOST` destination is illegal
/// inside one), that `fns` is the function table of the device `cmd` was allocated from, and that
/// every entry of `buffers` is a live buffer created on that device. `buffers.len()` must not
/// exceed [`MAX_HOST_READ_BUFFERS`] (asserted in every profile).
#[inline]
pub(crate) unsafe fn record_host_read_barrier(
    fns: &DeviceFns,
    cmd: VkCommandBuffer,
    buffers: &[VkBuffer],
) {
    assert!(
        !buffers.is_empty() && buffers.len() <= MAX_HOST_READ_BUFFERS,
        "invariant: record_host_read_barrier names 1..={MAX_HOST_READ_BUFFERS} buffers"
    );
    let n = buffers.len();
    let barriers: [VkBufferMemoryBarrier; MAX_HOST_READ_BUFFERS] = core::array::from_fn(|i| {
        VkBufferMemoryBarrier {
            s_type: VkStructureType::BufferMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: VK_ACCESS_TRANSFER_WRITE_BIT,
            dst_access_mask: VK_ACCESS_HOST_READ_BIT,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            buffer: if i < n { buffers[i] } else { VkBuffer::NULL },
            offset: 0,
            size: VK_WHOLE_SIZE,
        }
    });
    // SAFETY: the caller guarantees `cmd` is recording outside any render scope and that every
    // `buffers[i]` is live on `fns`'s device. `barriers[..n]` are fully initialised
    // `VkBufferMemoryBarrier`s (whole range, `QUEUE_FAMILY_IGNORED` on both sides — no ownership
    // transfer), the stack array outlives the call, and the count passed is `n`, so the
    // `NULL`-buffer tail slots are never read by the driver. No memory or image barriers.
    unsafe {
        (fns.cmd_pipeline_barrier)(
            cmd,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_PIPELINE_STAGE_HOST_BIT,
            0,
            0,
            ptr::null(),
            n as u32,
            barriers.as_ptr(),
            0,
            ptr::null(),
        );
    }
}
