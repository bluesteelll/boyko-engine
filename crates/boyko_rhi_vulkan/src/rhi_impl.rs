//! The [`boyko_rhi`] trait implementation for the Vulkan backend — compute path
//! only (Phase 1, plan Waves C+D).
//!
//! [`Vulkan`] is the zero-sized [`RhiApi`] marker. [`VulkanContext`] implements
//! [`RhiDevice`]; a thin [`VulkanQueue`] implements [`RhiQueue`] (plan O1/Q2);
//! [`VulkanCommandEncoder`] implements [`RhiCommandEncoder`] (the hot recording
//! path). The fixed compute descriptor-set + pipeline layouts that once lived in
//! the dissolved `ComputeHarness` are now cached on the device ([`ComputeLayouts`],
//! plan Q1/W2), and the command pool + buffer + descriptor pool + set move onto
//! the encoder.
//!
//! # Ownership & teardown (preserved 1:1 from `ComputeHarness`)
//!
//! Every owned RHI resource ([`VulkanShaderModule`], [`ComputePipeline`],
//! [`VulkanFence`], [`VulkanCommandEncoder`]) is destroyed by value through the
//! matching `unsafe destroy_*` (plan D2): the move encodes "destroyed exactly
//! once" in the type system. The shared [`ComputeLayouts`] are torn down in
//! [`VulkanContext`]'s `Drop`, after the resources that reference them, mirroring
//! the original reverse-order discipline.
//!
//! # Single-thread / `!Send + !Sync`
//!
//! Per plan §5.3 the RHI boundary is touched only by the dispatcher in the
//! apply-window. [`VulkanQueue`] / [`VulkanCommandEncoder`] hold a raw
//! `*const DeviceFns` into the owning [`VulkanContext`] (which outlives them);
//! they are neither `Send` nor `Sync`, so the borrow can never cross a thread.
//!
//! # Seam associated-type bindings
//!
//! The deferred-seam types (`Surface`/`Swapchain`/`Texture`/…) are unbounded, so
//! any type satisfies them. The concrete on-screen `Surface<'ctx>` /
//! `Swapchain<'ctx>` / `Renderer<'ctx>` are lifetime-parameterized (not
//! `'static`) and stay used directly this phase (scope decision), so the seam
//! types bind to the cheapest sensible placeholder: `()` for the type-less ones
//! and the `'static` [`VkSemaphore`](crate::ffi::VkSemaphore) for `Semaphore`.

use core::ffi::c_void;
use core::ptr::{self, NonNull};

use boyko_rhi::{
    BarrierDesc, BufferBarrier, BufferDesc, ComputePipelineDesc, MemoryLocation, RhiApi,
    RhiCommandEncoder, RhiDevice, RhiQueue, ShaderStage,
};

use crate::compute::ComputeError;
use crate::device::{DeviceFns, VulkanContext};
use crate::error::VulkanError;
use crate::ffi::*;
use crate::memory::BoundBuffer;

/// The zero-sized Vulkan backend marker implementing [`RhiApi`] (plan D1).
///
/// Static dispatch only: every trait call monomorphizes to a direct
/// `(fns.cmd_*)` indirect call, byte-identical to the inherent FFI methods.
pub struct Vulkan;

impl RhiApi for Vulkan {
    type Device = VulkanContext;
    type Queue = VulkanQueue;
    type CommandEncoder = VulkanCommandEncoder;
    type Buffer = BoundBuffer;
    type ShaderModule = VulkanShaderModule;
    type ComputePipeline = ComputePipeline;
    type Fence = VulkanFence;

    // ===== DEFERRED SEAM — bound to the cheapest placeholder this phase. =====
    // The concrete on-screen `Surface`/`Swapchain`/`Renderer` are lifetime-bound
    // and used directly (scope decision); they cannot satisfy a `'static`
    // associated type, so `()` stands in until the Phase-2-3 on-screen-in-trait
    // surface lands. `Semaphore` binds to the concrete `VkSemaphore` (a `'static`
    // `u64` newtype) since that one fits.
    type Surface = ();
    type Swapchain = ();
    type Semaphore = VkSemaphore;
    type Texture = ();
    type Sampler = ();
    type GraphicsPipeline = ();
    type BindGroup = ();
    type BindGroupLayout = ();
}

/// The fixed Slice-0 compute layouts shared by every compute pipeline + command
/// encoder: one STORAGE_BUFFER @ set0/binding0 (COMPUTE) descriptor-set layout +
/// a pipeline layout with that set + a 4-byte COMPUTE push-constant range.
///
/// Cached on [`VulkanContext`] (plan Q1/W2): created once on first
/// `create_compute_pipeline` / `create_command_encoder`, destroyed in the
/// context's `Drop` before `vkDestroyDevice`. The Phase-6 bind-group seam
/// supersedes this fixed layout.
pub struct ComputeLayouts {
    /// One STORAGE_BUFFER @ binding 0, COMPUTE stage.
    pub(crate) set_layout: VkDescriptorSetLayout,
    /// The set layout + a 4-byte COMPUTE push range.
    pub(crate) pipeline_layout: VkPipelineLayout,
}

impl ComputeLayouts {
    /// Builds the shared descriptor-set + pipeline layouts on `device`.
    ///
    /// On a partial failure the set layout is torn down before the error returns,
    /// so a failed `new` leaks nothing (the original `ComputeHarness::new`
    /// rollback, narrowed to just the layouts).
    pub(crate) fn new(device: VkDevice, fns: &DeviceFns) -> Result<Self, VulkanError> {
        let binding = VkDescriptorSetLayoutBinding {
            binding: 0,
            descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
            descriptor_count: 1,
            stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
            p_immutable_samplers: ptr::null(),
        };
        let dsl_info = VkDescriptorSetLayoutCreateInfo {
            s_type: VkStructureType::DescriptorSetLayoutCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            binding_count: 1,
            p_bindings: &binding,
        };
        let mut set_layout = VkDescriptorSetLayout::NULL;
        // SAFETY: `device` is live; `dsl_info` is fully initialized and its
        // `p_bindings` points to the single `binding` local (alive for the call);
        // `&mut set_layout` is a valid out-pointer; NULL allocator.
        let raw = unsafe {
            (fns.create_descriptor_set_layout)(device, &dsl_info, ptr::null(), &mut set_layout)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreateDescriptorSetLayout", result));
        }

        let push_range = VkPushConstantRange {
            stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
            offset: 0,
            // The shaders' push constant is a single `uint count` (4 bytes).
            size: 4,
        };
        let pl_info = VkPipelineLayoutCreateInfo {
            s_type: VkStructureType::PipelineLayoutCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            set_layout_count: 1,
            p_set_layouts: &set_layout,
            push_constant_range_count: 1,
            p_push_constant_ranges: &push_range,
        };
        let mut pipeline_layout = VkPipelineLayout::NULL;
        // SAFETY: `device` is live; `pl_info` is fully initialized and references
        // the live `set_layout` + the `push_range` local; `&mut pipeline_layout`
        // is a valid out-pointer; NULL allocator.
        let raw = unsafe {
            (fns.create_pipeline_layout)(device, &pl_info, ptr::null(), &mut pipeline_layout)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: `set_layout` was just created on `device` and is not yet
            // owned by any pipeline layout; destroy it exactly once on this error
            // path so it never leaks.
            unsafe { (fns.destroy_descriptor_set_layout)(device, set_layout, ptr::null()) };
            return Err(VulkanError::Vk("vkCreatePipelineLayout", result));
        }

        Ok(Self {
            set_layout,
            pipeline_layout,
        })
    }

    /// Destroys both layouts in reverse creation order, consuming `self`.
    ///
    /// # Safety
    ///
    /// `device`/`fns` must be the live device the layouts were created on; no
    /// compute pipeline or command encoder still referencing them is in flight;
    /// they are destroyed exactly once (the by-value `self` enforces the latter).
    pub(crate) unsafe fn destroy(self, device: VkDevice, fns: &DeviceFns) {
        // SAFETY: per the contract `device` is live and nothing references the
        // layouts; `vkDestroyPipelineLayout` then `vkDestroyDescriptorSetLayout`
        // are their matching teardown in reverse creation order, each once.
        unsafe {
            (fns.destroy_pipeline_layout)(device, self.pipeline_layout, ptr::null());
            (fns.destroy_descriptor_set_layout)(device, self.set_layout, ptr::null());
        }
    }
}

/// An owned compiled shader module ([`RhiApi::ShaderModule`]).
///
/// # Safety
///
/// The originating [`VulkanContext`] MUST still be alive when this module is
/// destroyed (via `destroy_shader_module`): the destroy goes through the context's
/// device fn-table. No compile-time `'ctx` tie this phase (plan F1; structural fix
/// deferred to Phase 2-3).
pub struct VulkanShaderModule {
    /// The `VkShaderModule` handle; destroyed by `destroy_shader_module`.
    pub(crate) module: VkShaderModule,
}

/// An owned compute pipeline ([`RhiApi::ComputePipeline`]).
///
/// Holds only the `VkPipeline`; its shader module is a separate owned
/// [`VulkanShaderModule`] (the trait splits module + pipeline creation), and the
/// pipeline layout is the device's shared [`ComputeLayouts::pipeline_layout`].
///
/// # Safety
///
/// The originating [`VulkanContext`] MUST still be alive when this pipeline is
/// destroyed (via `destroy_compute_pipeline`): the destroy goes through the
/// context's device fn-table, and the pipeline references the context's shared
/// layouts. No compile-time `'ctx` tie this phase (plan F1; deferred to Phase 2-3).
pub struct ComputePipeline {
    /// The `VkPipeline` handle; destroyed by `destroy_compute_pipeline`.
    pub(crate) pipeline: VkPipeline,
}

/// An owned fence ([`RhiApi::Fence`]).
///
/// # Safety
///
/// The originating [`VulkanContext`] MUST still be alive when this fence is waited
/// on, reset, or destroyed: each goes through the context's device fn-table. No
/// compile-time `'ctx` tie this phase (plan F1; deferred to Phase 2-3).
pub struct VulkanFence {
    /// The `VkFence` handle; destroyed by `destroy_fence`.
    pub(crate) fence: VkFence,
}

/// A thin submission-queue wrapper ([`RhiQueue`], plan O1/Q2).
///
/// Holds the device's graphics+compute `VkQueue` + a raw `*const DeviceFns`
/// pointing into the owning context's **boxed** fn-table (plan A1). Because the
/// fn-table lives behind a stable heap address owned by the context, a context
/// move does not invalidate the pointer. `!Send + !Sync`: the pointer is
/// dereferenced only on the owning thread, and the context (hence the box)
/// outlives the queue wrapper; the RHI is single-threaded (§5.3).
///
/// # Safety
///
/// The originating [`VulkanContext`] MUST still be alive whenever this queue is
/// used (e.g. [`submit`](RhiQueue::submit)): submitting after the context is
/// dropped dangles the cached fn-table pointer = undefined behavior. There is no
/// compile-time `'ctx` tie this phase (the accepted plan-D2 trade-off; the
/// structural `'ctx` fix is deferred to Phase 2-3, plan F1).
pub struct VulkanQueue {
    queue: VkQueue,
    /// Raw pointer into the owning [`VulkanContext`]'s boxed [`DeviceFns`] — a
    /// stable heap address that survives context moves (plan A1) and outlives this
    /// wrapper (enforced by teardown order in the context's `Drop`).
    fns: *const DeviceFns,
}

/// The hot command-recording encoder ([`RhiCommandEncoder`]).
///
/// Owns its command pool + primary command buffer + descriptor pool + the one
/// fixed compute descriptor set (allocated ONCE here at
/// `create_command_encoder`, plan Q1 — no per-record `vkUpdateDescriptorSets`
/// regression). References the device's shared [`ComputeLayouts`] (cached copies
/// of the `Copy` layout handles) + a raw `*const DeviceFns` into the context's
/// **boxed** fn-table (stable across context moves, plan A1). `!Send + !Sync`.
///
/// # Safety
///
/// The originating [`VulkanContext`] MUST still be alive whenever this encoder is
/// used or destroyed: recording / destroying after the context is dropped dangles
/// the cached fn-table pointer = undefined behavior. There is no compile-time
/// `'ctx` tie this phase (the accepted plan-D2 trade-off; the structural `'ctx`
/// fix is deferred to Phase 2-3, plan F1).
pub struct VulkanCommandEncoder {
    /// Borrowed device handle (for `vkUpdateDescriptorSets`).
    device: VkDevice,
    /// Raw pointer into the owning [`VulkanContext`]'s boxed [`DeviceFns`] — a
    /// stable heap address that survives context moves (plan A1) and outlives this
    /// encoder (enforced by teardown order in the context's `Drop`).
    fns: *const DeviceFns,
    command_pool: VkCommandPool,
    /// Allocated FROM `command_pool`; freed implicitly when the pool is
    /// destroyed (a RESET_COMMAND_BUFFER pool, no explicit free needed).
    command_buffer: VkCommandBuffer,
    descriptor_pool: VkDescriptorPool,
    /// Allocated FROM `descriptor_pool`; freed implicitly with the pool.
    descriptor_set: VkDescriptorSet,
    /// Cached copy of the device's shared pipeline layout (for bind-set + push).
    pipeline_layout: VkPipelineLayout,
    /// The buffer the descriptor set currently points at — the set is updated
    /// only when a `bind_storage_buffer` names a different one (plan Q1).
    bound_buffer: VkBuffer,
    /// The descriptor-set index the next `dispatch` binds at (set by
    /// `bind_storage_buffer`; the Slice-0 contract is set 0).
    bound_set_index: u32,
}

// SAFETY: the encoder is a single-thread-only resource (plan §5.3): it is NOT
// `Send`/`Sync`. The `*const DeviceFns` points into the owning `VulkanContext`'s
// boxed fn-table — a stable heap address that a context move does not invalidate
// (plan A1) and that the context (hence the box) outlives, enforced by teardown
// order in the context's `Drop`; it is dereferenced only on the owning thread.
// The raw pointer makes the type `!Send + !Sync` by default (no auto-impl), which
// is the discipline we want — no explicit `unsafe impl` is added.

impl VulkanQueue {
    /// Wraps a context's queue + device fn-table into the thin RHI queue.
    ///
    /// `fns` must point to the owning context's [`DeviceFns`] (which outlives the
    /// returned queue).
    pub(crate) fn new(queue: VkQueue, fns: *const DeviceFns) -> Self {
        Self { queue, fns }
    }
}

impl VulkanContext {
    /// The thin RHI submission queue over this context's graphics+compute queue
    /// (plan O1/Q2). The returned [`VulkanQueue`] borrows this context's device
    /// fn-table and must not outlive the context.
    #[inline]
    pub fn rhi_queue(&self) -> VulkanQueue {
        VulkanQueue::new(self.queue(), self.device_fns() as *const DeviceFns)
    }
}

impl RhiDevice<Vulkan> for VulkanContext {
    type Error = VulkanError;

    fn create_buffer(&self, desc: &BufferDesc) -> Result<BoundBuffer, VulkanError> {
        // Plan A2: a zero-size buffer is an invalid request — it would yield a
        // `VkDescriptorBufferInfo.range == 0`. Reject it loud in debug; there is no
        // silent 0→1 size divergence (the created size, the stored
        // `BoundBuffer.size`, and the later descriptor `range` are all `desc.size`).
        debug_assert!(desc.size > 0, "invariant: zero-size buffer");
        // The foundation routes every buffer through the device's one shared
        // host-visible+coherent block (plan Q1). `DeviceLocal` is a Phase-5 seam
        // (staging + flush) — surface it as Unsupported rather than silently
        // mapping host memory.
        if desc.location != MemoryLocation::HostVisibleCoherent {
            return Err(VulkanError::Unsupported("create_buffer(DeviceLocal)"));
        }
        // The agnostic `BufferUsage` bits equal the Vulkan `VK_BUFFER_USAGE_*`
        // bits (plan D5), so the projection is an identity cast on the u32 family.
        let usage: VkFlags = desc.usage.bits();
        let block = self.host_block()?;
        let bound = block.borrow_mut().create_bound_buffer(desc.size, usage)?;
        Ok(bound)
    }

    unsafe fn destroy_buffer(&self, buffer: BoundBuffer) {
        // Plan A3: if a `BoundBuffer` exists, it was sub-allocated from the shared
        // block, so the block MUST already be initialized. A silent early-return on
        // `Err` here would drop the owned `buffer` WITHOUT destroying its `VkBuffer`
        // / returning its sub-allocation — a leak. `host_block()` only fails on the
        // first-ever allocation (which already happened to mint `buffer`), so this
        // `expect` is unreachable by construction.
        let block = self
            .host_block()
            .expect("invariant: host block initialized when a BoundBuffer exists");
        // SAFETY: `buffer` was produced by `create_buffer` on this device's shared
        // block, the GPU is no longer using it (caller fence-waited per the trait
        // contract), and the by-value move destroys it exactly once (the buffer
        // handle is destroyed once + its sub-region returned to the allocator
        // once). The block is borrowed `&mut` single-threaded.
        unsafe { block.borrow_mut().destroy_bound_buffer(buffer) };
    }

    fn buffer_mapped_ptr(&self, buffer: &BoundBuffer) -> Option<NonNull<u8>> {
        // The foundation's buffers live in a host-visible-coherent block, so the
        // persistent mapping always exists.
        Some(buffer.mapped)
    }

    fn create_shader_module(&self, spirv: &[u32]) -> Result<VulkanShaderModule, VulkanError> {
        debug_assert!(!spirv.is_empty(), "invariant: SPIR-V word slice is non-empty");
        let sm_info = VkShaderModuleCreateInfo {
            s_type: VkStructureType::ShaderModuleCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            // `code_size` is in BYTES.
            code_size: spirv.len() * 4,
            p_code: spirv.as_ptr(),
        };
        let mut module = VkShaderModule::NULL;
        // SAFETY: `device` is live; `sm_info` is a fully-initialized `#[repr(C)]`
        // struct whose `p_code` points to `code_size` bytes of 4-byte-aligned
        // SPIR-V (`&[u32]` is word-aligned) alive for the call; `&mut module` is a
        // valid out-pointer; NULL allocator.
        let raw = unsafe {
            (self.device_fns().create_shader_module)(
                self.device(),
                &sm_info,
                ptr::null(),
                &mut module,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreateShaderModule", result));
        }
        Ok(VulkanShaderModule { module })
    }

    unsafe fn destroy_shader_module(&self, module: VulkanShaderModule) {
        // SAFETY: `module.module` was created on this device by
        // `create_shader_module`, no pipeline referencing it is in flight (caller
        // contract), and the by-value move destroys it exactly once.
        unsafe {
            (self.device_fns().destroy_shader_module)(self.device(), module.module, ptr::null())
        };
    }

    fn create_compute_pipeline(
        &self,
        desc: &ComputePipelineDesc<Vulkan>,
    ) -> Result<ComputePipeline, VulkanError> {
        // Plan B3 (ABI-2): the fixed Slice-0 pipeline layout has exactly a 4-byte
        // push range. `desc.push_constant_bytes` is otherwise a dead knob — surface
        // a mismatch as `Unsupported` rather than silently building a pipeline whose
        // declared push size disagrees with the shared layout.
        if desc.push_constant_bytes != 4 {
            return Err(VulkanError::Unsupported("push_constant_bytes != 4"));
        }
        // The shared pipeline layout is needed at pipeline-create time (plan Q1).
        let layouts = self.compute_layouts()?;

        let stage = VkPipelineShaderStageCreateInfo {
            s_type: VkStructureType::PipelineShaderStageCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            stage: VK_SHADER_STAGE_COMPUTE_BIT,
            module: desc.module.module,
            p_name: desc.entry.as_ptr(),
            p_specialization_info: ptr::null(),
        };
        let cp_info = VkComputePipelineCreateInfo {
            s_type: VkStructureType::ComputePipelineCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            stage,
            layout: layouts.pipeline_layout,
            base_pipeline_handle: VkPipeline::NULL,
            base_pipeline_index: -1,
        };
        let mut pipeline = VkPipeline::NULL;
        // SAFETY: `device` is live; null pipeline cache (`0`) is valid; one
        // create-info is fully initialized, referencing the live shader module +
        // the device's shared `pipeline_layout`; `&mut pipeline` is a valid
        // out-pointer for the single pipeline; NULL allocator. The module is owned
        // by the caller's `VulkanShaderModule`, alive for this call.
        let raw = unsafe {
            (self.device_fns().create_compute_pipelines)(
                self.device(),
                0,
                1,
                &cp_info,
                ptr::null(),
                &mut pipeline,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::from(ComputeError::VkError(
                "vkCreateComputePipelines",
                result,
            )));
        }
        Ok(ComputePipeline { pipeline })
    }

    unsafe fn destroy_compute_pipeline(&self, pipeline: ComputePipeline) {
        // SAFETY: `pipeline.pipeline` was created on this device, no submission
        // using it is pending (caller contract), and the by-value move destroys it
        // exactly once.
        unsafe {
            (self.device_fns().destroy_pipeline)(self.device(), pipeline.pipeline, ptr::null())
        };
    }

    fn create_fence(&self, signaled: bool) -> Result<VulkanFence, VulkanError> {
        let fence_info = VkFenceCreateInfo {
            s_type: VkStructureType::FenceCreateInfo,
            p_next: ptr::null(),
            // `VK_FENCE_CREATE_SIGNALED_BIT` == 0x1.
            flags: if signaled { 0x0000_0001 } else { 0 },
        };
        let mut fence = VkFence::NULL;
        // SAFETY: `device` is live; `fence_info` is fully initialized; `&mut
        // fence` is a valid out-pointer; NULL allocator.
        let raw = unsafe {
            (self.device_fns().create_fence)(self.device(), &fence_info, ptr::null(), &mut fence)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreateFence", result));
        }
        Ok(VulkanFence { fence })
    }

    unsafe fn destroy_fence(&self, fence: VulkanFence) {
        // SAFETY: `fence.fence` was created on this device, is not pending (caller
        // contract), and the by-value move destroys it exactly once.
        unsafe { (self.device_fns().destroy_fence)(self.device(), fence.fence, ptr::null()) };
    }

    fn wait_fence(&self, fence: &VulkanFence, timeout_ns: u64) -> Result<(), VulkanError> {
        // SAFETY: `device` is live; `&fence.fence` names one live fence;
        // `wait_all = VK_TRUE` blocks until it is signaled (or the timeout
        // elapses). After this returns `Ok` the submission that signals it has
        // completed — the fence-before-readback discipline.
        let raw = unsafe {
            (self.device_fns().wait_for_fences)(
                self.device(),
                1,
                &fence.fence,
                VK_TRUE,
                timeout_ns,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkWaitForFences", result));
        }
        Ok(())
    }

    fn reset_fence(&self, fence: &VulkanFence) -> Result<(), VulkanError> {
        // SAFETY: `device` is live; `&fence.fence` names one live fence to reset
        // to unsignaled (no submission referencing it is pending — caller resets
        // only after a `wait_fence`).
        let raw =
            unsafe { (self.device_fns().reset_fences)(self.device(), 1, &fence.fence) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkResetFences", result));
        }
        Ok(())
    }

    fn create_command_encoder(&self) -> Result<VulkanCommandEncoder, VulkanError> {
        let layouts = self.compute_layouts()?;
        // SAFETY: the device is live; `layouts` are this device's shared compute
        // layouts; the encoder takes a raw pointer to this context's `DeviceFns`
        // (which outlives any encoder built from `&self`).
        unsafe {
            VulkanCommandEncoder::new(
                self.device(),
                self.device_fns() as *const DeviceFns,
                self.queue_family_index(),
                layouts.set_layout,
                layouts.pipeline_layout,
            )
        }
    }

    unsafe fn destroy_command_encoder(&self, enc: VulkanCommandEncoder) {
        // SAFETY: `enc` was created on this device, its last submission has
        // completed (caller contract), and the by-value move destroys it exactly
        // once. `destroy` tears down the descriptor pool + command pool (which
        // frees the set + command buffer) in reverse order.
        unsafe { enc.destroy(self.device(), self.device_fns()) };
    }

    fn wait_idle(&self) -> Result<(), VulkanError> {
        // SAFETY: `device` is live; `vkDeviceWaitIdle` blocks until every queue is
        // idle — the belt-and-braces teardown sync (plan W4).
        let raw = unsafe { (self.device_fns().device_wait_idle)(self.device()) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkDeviceWaitIdle", result));
        }
        Ok(())
    }
}

impl RhiQueue<Vulkan> for VulkanQueue {
    type Error = VulkanError;

    fn submit(
        &self,
        encoder: &VulkanCommandEncoder,
        signal_fence: &VulkanFence,
    ) -> Result<(), VulkanError> {
        let submit = VkSubmitInfo {
            s_type: VkStructureType::SubmitInfo,
            p_next: ptr::null(),
            wait_semaphore_count: 0,
            p_wait_semaphores: ptr::null(),
            p_wait_dst_stage_mask: ptr::null(),
            command_buffer_count: 1,
            p_command_buffers: &encoder.command_buffer,
            signal_semaphore_count: 0,
            p_signal_semaphores: ptr::null(),
        };
        // SAFETY: `self.fns` points into the owning context's boxed `DeviceFns`
        // (stable heap address, context-outlives-queue — plan A1); the caller
        // upholds the type-level "context still alive" contract. `self.queue` is
        // its graphics+compute queue; one submit references the encoder's ended
        // `command_buffer` (the `&encoder.command_buffer` local outlives the call);
        // no semaphores (null arrays valid for count 0); `signal_fence.fence` is
        // the live fence signaled on completion. The headless path's only sync is
        // this fence.
        let fns = unsafe { &*self.fns };
        let raw = unsafe { (fns.queue_submit)(self.queue, 1, &submit, signal_fence.fence) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkQueueSubmit", result));
        }
        Ok(())
    }
}

impl VulkanCommandEncoder {
    /// Allocates the encoder's command pool + buffer + descriptor pool + the one
    /// fixed compute descriptor set (built ONCE here, plan Q1).
    ///
    /// On any partial failure every object created so far is torn down in reverse
    /// order before the error returns (the `ComputeHarness::new` rollback,
    /// narrowed to the per-encoder objects).
    ///
    /// # Safety
    ///
    /// `device`/`fns` must be the live device the layouts belong to; `set_layout`
    /// / `pipeline_layout` must be that device's shared compute layouts; `fns`
    /// must outlive the returned encoder.
    unsafe fn new(
        device: VkDevice,
        fns: *const DeviceFns,
        queue_family_index: u32,
        set_layout: VkDescriptorSetLayout,
        pipeline_layout: VkPipelineLayout,
    ) -> Result<Self, VulkanError> {
        // SAFETY (whole fn): `fns` is a live `DeviceFns` borrowed from the owning
        // context (caller contract); dereferencing it here is sound on the owning
        // thread. Each create call below mirrors `ComputeHarness::new`'s sound
        // usage with the same `// SAFETY:` invariants.
        let fns_ref = unsafe { &*fns };

        // --- Descriptor pool + set (one STORAGE_BUFFER). ---
        let pool_size = VkDescriptorPoolSize {
            descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
            descriptor_count: 1,
        };
        let dp_info = VkDescriptorPoolCreateInfo {
            s_type: VkStructureType::DescriptorPoolCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            max_sets: 1,
            pool_size_count: 1,
            p_pool_sizes: &pool_size,
        };
        let mut descriptor_pool = VkDescriptorPool::NULL;
        // SAFETY: `device` is live; `dp_info` is fully initialized referencing the
        // `pool_size` local; `&mut descriptor_pool` is a valid out-pointer.
        let raw = unsafe {
            (fns_ref.create_descriptor_pool)(device, &dp_info, ptr::null(), &mut descriptor_pool)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreateDescriptorPool", result));
        }

        let ds_alloc = VkDescriptorSetAllocateInfo {
            s_type: VkStructureType::DescriptorSetAllocateInfo,
            p_next: ptr::null(),
            descriptor_pool,
            descriptor_set_count: 1,
            p_set_layouts: &set_layout,
        };
        let mut descriptor_set = VkDescriptorSet::NULL;
        // SAFETY: `device` is live; `ds_alloc` names the live pool + references
        // the live `set_layout`; `&mut descriptor_set` is a valid out-pointer.
        let raw =
            unsafe { (fns_ref.allocate_descriptor_sets)(device, &ds_alloc, &mut descriptor_set) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: `descriptor_pool` was just created; destroying it frees any
            // partially-allocated set and releases the pool exactly once.
            unsafe { (fns_ref.destroy_descriptor_pool)(device, descriptor_pool, ptr::null()) };
            return Err(VulkanError::Vk("vkAllocateDescriptorSets", result));
        }

        // --- Command pool + one primary command buffer. ---
        let cp_info = VkCommandPoolCreateInfo {
            s_type: VkStructureType::CommandPoolCreateInfo,
            p_next: ptr::null(),
            flags: VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT,
            queue_family_index,
        };
        let mut command_pool = VkCommandPool::NULL;
        // SAFETY: `device` is live; `cp_info` is fully initialized for the
        // graphics+compute family; `&mut command_pool` is a valid out-pointer.
        let raw = unsafe {
            (fns_ref.create_command_pool)(device, &cp_info, ptr::null(), &mut command_pool)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: descriptor pool was created above; destroy it once on this
            // error path before returning.
            unsafe { (fns_ref.destroy_descriptor_pool)(device, descriptor_pool, ptr::null()) };
            return Err(VulkanError::Vk("vkCreateCommandPool", result));
        }

        let cb_alloc = VkCommandBufferAllocateInfo {
            s_type: VkStructureType::CommandBufferAllocateInfo,
            p_next: ptr::null(),
            command_pool,
            level: VK_COMMAND_BUFFER_LEVEL_PRIMARY,
            command_buffer_count: 1,
        };
        let mut command_buffer = VkCommandBuffer::NULL;
        // SAFETY: `device` is live; `cb_alloc` names the live pool + requests one
        // primary buffer; `&mut command_buffer` is a valid out-pointer.
        let raw =
            unsafe { (fns_ref.allocate_command_buffers)(device, &cb_alloc, &mut command_buffer) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: both pools were created above; destroy them once each in
            // reverse order on this error path.
            unsafe {
                (fns_ref.destroy_command_pool)(device, command_pool, ptr::null());
                (fns_ref.destroy_descriptor_pool)(device, descriptor_pool, ptr::null());
            }
            return Err(VulkanError::Vk("vkAllocateCommandBuffers", result));
        }

        Ok(Self {
            device,
            fns,
            command_pool,
            command_buffer,
            descriptor_pool,
            descriptor_set,
            pipeline_layout,
            bound_buffer: VkBuffer::NULL,
            bound_set_index: 0,
        })
    }

    /// Tears down the encoder's command pool + descriptor pool in reverse creation
    /// order, consuming `self`. The command buffer + descriptor set are freed
    /// implicitly by destroying their pools.
    ///
    /// # Safety
    ///
    /// `device`/`fns` must be the live device the encoder was created on; the
    /// encoder's last submission has completed (not pending); it is destroyed
    /// exactly once (the by-value `self` enforces the latter).
    unsafe fn destroy(self, device: VkDevice, fns: &DeviceFns) {
        // SAFETY: per the contract `device` is live and nothing is pending;
        // destroying the command pool frees its command buffer, and destroying the
        // descriptor pool frees its set — each pool destroyed exactly once in
        // reverse creation order.
        unsafe {
            (fns.destroy_command_pool)(device, self.command_pool, ptr::null());
            (fns.destroy_descriptor_pool)(device, self.descriptor_pool, ptr::null());
        }
    }
}

impl RhiCommandEncoder<Vulkan> for VulkanCommandEncoder {
    type Error = VulkanError;

    fn begin(&mut self) -> Result<(), VulkanError> {
        // Plan C1 (TD-1 ABA): reset the cached binding so every fresh recording
        // re-binds. `vkBeginCommandBuffer` resets the command buffer (so the prior
        // recording's `vkCmdBindDescriptorSets` is gone), and the descriptor set
        // itself may have been left pointing at a now-destroyed buffer whose
        // `VkBuffer` handle value a recreate could reuse — clearing the cache to
        // NULL forces a `vkUpdateDescriptorSets` on the next `bind_storage_buffer`,
        // closing the ABA while keeping the "at most one update per recording"
        // property (NULL never equals a real buffer handle).
        self.bound_buffer = VkBuffer::NULL;
        let begin = VkCommandBufferBeginInfo {
            s_type: VkStructureType::CommandBufferBeginInfo,
            p_next: ptr::null(),
            flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            p_inheritance_info: ptr::null(),
        };
        // SAFETY: `self.fns` borrows the live device fn-table; the command buffer
        // is from a RESET_COMMAND_BUFFER pool, so `vkBeginCommandBuffer` implicitly
        // resets it (it is not pending — the caller fence-waits before reusing an
        // encoder); `begin` is a fully-initialized one-time-submit begin-info.
        let fns = unsafe { &*self.fns };
        let raw = unsafe { (fns.begin_command_buffer)(self.command_buffer, &begin) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkBeginCommandBuffer", result));
        }
        Ok(())
    }

    fn end(&mut self) -> Result<(), VulkanError> {
        // SAFETY: `self.fns` borrows the live device fn-table; recording was
        // opened by `begin`; `vkEndCommandBuffer` is its matching close.
        let fns = unsafe { &*self.fns };
        let raw = unsafe { (fns.end_command_buffer)(self.command_buffer) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkEndCommandBuffer", result));
        }
        Ok(())
    }

    fn bind_compute_pipeline(&mut self, pipeline: &ComputePipeline) {
        // SAFETY: recording is open; `pipeline.pipeline` is a live compute pipeline
        // built against this encoder's `pipeline_layout`; COMPUTE bind point
        // matches its creation. `self.fns` borrows the live device fn-table.
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_bind_pipeline)(
                self.command_buffer,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                pipeline.pipeline,
            );
        }
    }

    fn bind_storage_buffer(&mut self, buffer: &BoundBuffer, set: u32, binding: u32) {
        // Plan B4 (ABI-4): the Slice-0 fixed layout is one STORAGE_BUFFER at
        // set0/binding0; any other `(set, binding)` is a caller error against the
        // fixed compute layout (Phase-6 bind groups supersede this).
        debug_assert!(
            set == 0 && binding == 0,
            "invariant: Slice-0 fixed set0/binding0"
        );
        self.bound_set_index = set;
        // Update the descriptor set ONLY when the bound buffer changes (plan Q1);
        // the foundation binds one buffer per recording, so the update fires at
        // most once. The actual `vkCmdBindDescriptorSets` is recorded at dispatch.
        if self.bound_buffer == buffer.buffer {
            return;
        }
        self.bound_buffer = buffer.buffer;

        let buffer_info = VkDescriptorBufferInfo {
            buffer: buffer.buffer,
            offset: 0,
            range: buffer.size,
        };
        let write = VkWriteDescriptorSet {
            s_type: VkStructureType::WriteDescriptorSet,
            p_next: ptr::null(),
            dst_set: self.descriptor_set,
            dst_binding: 0,
            dst_array_element: 0,
            descriptor_count: 1,
            descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
            p_image_info: ptr::null(),
            p_buffer_info: &buffer_info,
            p_texel_buffer_view: ptr::null(),
        };
        // SAFETY: `self.fns` borrows the live device fn-table; one write references
        // the live `descriptor_set` + the `buffer_info` local (which names the
        // caller's live buffer); zero copies; the write is consumed entirely during
        // the call. The set is not bound to any pending command buffer (the caller
        // fence-waits before reusing the encoder), so updating it is sound.
        let fns = unsafe { &*self.fns };
        unsafe { (fns.update_descriptor_sets)(self.device, 1, &write, 0, ptr::null()) };
    }

    fn push_constants(&mut self, stage: ShaderStage, offset: u32, bytes: &[u8]) {
        // Plan B2 (ABI-1/TD-5): the fixed Slice-0 pipeline layout declares a single
        // 4-byte COMPUTE push range at offset 0. `offset + len` outside `[0, 4]`, or
        // a non-COMPUTE stage, is a caller error against that fixed layout.
        debug_assert!(
            offset as u64 + bytes.len() as u64 <= 4,
            "invariant: push range"
        );
        debug_assert!(
            stage.bits() == crate::ffi::VK_SHADER_STAGE_COMPUTE_BIT,
            "invariant: compute push stage"
        );
        // The agnostic `ShaderStage` bits equal `VK_SHADER_STAGE_*` (plan D5).
        let stage_flags: VkFlags = stage.bits();
        // SAFETY: recording is open; `self.pipeline_layout` declares a 4-byte
        // COMPUTE push range at offset 0; `bytes.as_ptr()` points to `bytes.len()`
        // bytes alive for the call; the caller passes offset/size within the
        // declared range. `self.fns` borrows the live device fn-table.
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_push_constants)(
                self.command_buffer,
                self.pipeline_layout,
                stage_flags,
                offset,
                bytes.len() as u32,
                bytes.as_ptr().cast::<c_void>(),
            );
        }
    }

    fn dispatch(&mut self, gx: u32, gy: u32, gz: u32) {
        debug_assert!(
            gx > 0 && gy > 0 && gz > 0,
            "invariant: dispatch group counts must be non-zero"
        );
        // SAFETY: recording is open; binding the descriptor set at the cached set
        // index for the COMPUTE bind point uses the live `pipeline_layout` + the
        // live `descriptor_set` (pointed at the bound buffer by
        // `bind_storage_buffer`); zero dynamic offsets (null valid for count 0).
        // Then the dispatch runs with the bound pipeline + set covering it.
        // `self.fns` borrows the live device fn-table.
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_bind_descriptor_sets)(
                self.command_buffer,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                self.pipeline_layout,
                self.bound_set_index,
                1,
                &self.descriptor_set,
                0,
                ptr::null(),
            );
            (fns.cmd_dispatch)(self.command_buffer, gx, gy, gz);
        }
    }

    fn pipeline_barrier(&mut self, barrier: &BarrierDesc<Vulkan>) {
        // Map the agnostic stage/access masks via identity casts — the
        // `BarrierStage`/`BarrierAccess` bit values equal the `VK_PIPELINE_STAGE_*`
        // / `VK_ACCESS_*` constants (plan D3/D5).
        let src_stage: VkFlags = barrier.src_stage.bits();
        let dst_stage: VkFlags = barrier.dst_stage.bits();
        debug_assert!(
            barrier.buffers.is_empty() || (src_stage != 0 && dst_stage != 0),
            "invariant: a buffer barrier needs non-empty src+dst stages"
        );

        // The foundation supplies 0 or 1 buffer barriers — the common, hot path.
        // Plan D1 (TD-3/UB-4): the multi-barrier heap fallback (never hit on the
        // headless compute path) is factored into a `#[cold]` helper so this path
        // never even names a `Vec`.
        let count = barrier.buffers.len();
        if count <= 1 {
            let mut inline_buf = VkBufferMemoryBarrier {
                s_type: VkStructureType::BufferMemoryBarrier,
                p_next: ptr::null(),
                src_access_mask: 0,
                dst_access_mask: 0,
                src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                buffer: VkBuffer::NULL,
                offset: 0,
                size: VK_WHOLE_SIZE,
            };
            let vk_barriers: *const VkBufferMemoryBarrier = if count == 0 {
                ptr::null()
            } else {
                let b = &barrier.buffers[0];
                inline_buf.src_access_mask = b.src_access.bits();
                inline_buf.dst_access_mask = b.dst_access.bits();
                inline_buf.buffer = b.buffer.buffer;
                &inline_buf
            };
            // SAFETY: recording is open; `src_stage`/`dst_stage` are the mapped
            // stage masks; `vk_barriers` points to `count` (0 or 1) fully-
            // initialized `VkBufferMemoryBarrier`s in the live `inline_buf` local,
            // naming a live buffer with WRITE→READ|WRITE-style scopes; zero
            // global/image barriers (null arrays valid for count 0). `self.fns`
            // points into the context's boxed fn-table (alive per the type
            // contract).
            let fns = unsafe { &*self.fns };
            unsafe {
                (fns.cmd_pipeline_barrier)(
                    self.command_buffer,
                    src_stage,
                    dst_stage,
                    0,
                    0,
                    ptr::null(),
                    count as u32,
                    vk_barriers,
                    0,
                    ptr::null(),
                );
            }
            return;
        }

        self.pipeline_barrier_many(src_stage, dst_stage, barrier.buffers);
    }
}

impl VulkanCommandEncoder {
    /// The cold multi-buffer-barrier fallback for [`RhiCommandEncoder::pipeline_barrier`]
    /// (plan D1): builds a heap `Vec<VkBufferMemoryBarrier>` and records the
    /// barrier. The headless compute path never reaches this (it supplies 0 or 1
    /// buffer barriers), so the only allocation is kept off the common path's
    /// I-cache via `#[cold] #[inline(never)]`.
    #[cold]
    #[inline(never)]
    fn pipeline_barrier_many(
        &mut self,
        src_stage: VkFlags,
        dst_stage: VkFlags,
        buffers: &[BufferBarrier<Vulkan>],
    ) {
        let mut heap_buf: Vec<VkBufferMemoryBarrier> = Vec::with_capacity(buffers.len());
        for b in buffers {
            heap_buf.push(VkBufferMemoryBarrier {
                s_type: VkStructureType::BufferMemoryBarrier,
                p_next: ptr::null(),
                src_access_mask: b.src_access.bits(),
                dst_access_mask: b.dst_access.bits(),
                src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                buffer: b.buffer.buffer,
                offset: 0,
                size: VK_WHOLE_SIZE,
            });
        }
        // SAFETY: recording is open; `src_stage`/`dst_stage` are the mapped stage
        // masks; `heap_buf` holds `buffers.len()` fully-initialized
        // `VkBufferMemoryBarrier`s (alive for the call), each naming a live buffer;
        // zero global/image barriers (null arrays valid for count 0). `self.fns`
        // points into the context's boxed fn-table (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_pipeline_barrier)(
                self.command_buffer,
                src_stage,
                dst_stage,
                0,
                0,
                ptr::null(),
                heap_buf.len() as u32,
                heap_buf.as_ptr(),
                0,
                ptr::null(),
            );
        }
    }
}
