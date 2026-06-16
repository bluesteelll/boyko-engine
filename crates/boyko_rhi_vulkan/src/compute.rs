//! Slice-0 steps 0c + 0d — one compute dispatch, then a SECOND dispatch chained
//! through a `vkCmdPipelineBarrier`, with ZERO per-frame readback (the single
//! readback is the TEST ORACLE only). See `docs/RENDER-PHYSICS-GPU-PLAN.md` §11.
//!
//! # What this proves
//!
//! - **0c**: build a `VkShaderModule` from a committed `.spv` (no SDK at build
//!   time), a one-binding (STORAGE_BUFFER at COMPUTE) descriptor-set layout, a
//!   pipeline layout with a 4-byte push constant, and a compute pipeline; bind a
//!   host-visible storage buffer to a descriptor set; record begin → bind
//!   pipeline → bind set → push constant → `vkCmdDispatch(ceil(N/64))` → end;
//!   submit with a fence; wait; read back the persistent mapping; assert the
//!   shader's known pattern (`buffer[i] = i*2 + 1`).
//! - **0d**: a SECOND pipeline (`buffer[i] += 100`) recorded into the SAME
//!   command buffer AFTER a `VkBufferMemoryBarrier` (COMPUTE_SHADER/SHADER_WRITE
//!   → COMPUTE_SHADER/SHADER_READ on the buffer); one submit + fence; the result
//!   diffs bit-exact against a CPU-computed golden — the §5.5 edge→barrier
//!   lowering in miniature.
//!
//! # The shader contract (read from `shaders/{write_pattern,transform_add}.hlsl`)
//!
//! Both shaders declare `RWStructuredBuffer<uint> Data : register(u0)` (binding
//! 0, set 0, a single STORAGE_BUFFER) and a `[[vk::push_constant]] uint count`
//! (a 4-byte range at offset 0, visible to the COMPUTE stage), with
//! `[numthreads(64,1,1)]` and entry point `main`. Each invocation bounds-checks
//! `i < count` so a non-multiple-of-64 `N` never writes out of range. This
//! module's descriptor layout, push-constant range, local-size assumption and
//! dispatch group count are all derived from that contract.
//!
//! # Soundness (raw FFI → validation + golden are the oracle)
//!
//! Miri cannot run this (real driver FFI, VRAM mapping). The oracle is two-fold,
//! per plan §6: (a) the `VK_LAYER_KHRONOS_validation` messenger asserted to
//! `total() == 0` after the run (a validation fault FAILS the test), and (b) the
//! bit-exact golden-buffer diff. Every `unsafe` block states the invariant that
//! makes it sound (fence-before-destroy, barrier params, descriptor lifetimes).

use core::ffi::c_void;
use core::ptr::{self, NonNull};

use crate::device::DeviceFns;
use crate::ffi::*;
use crate::memory::{BoundBuffer, MemoryError};

/// The committed SPIR-V for step 0c (`buffer[i] = i*2 + 1`).
///
/// Wrapped in a `#[repr(C, align(4))]` newtype so the `include_bytes!` blob is
/// 4-byte aligned: `VkShaderModuleCreateInfo::p_code` is a `*const u32` and the
/// spec requires the SPIR-V word stream to be 4-byte aligned (a bare
/// `include_bytes!` is only `align(1)`).
static WRITE_PATTERN_SPV: SpirvBlob<988> = SpirvBlob(*include_bytes!(
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/write_pattern.comp.spv")
));

/// The committed SPIR-V for step 0d (`buffer[i] += 100`).
static TRANSFORM_ADD_SPV: SpirvBlob<968> = SpirvBlob(*include_bytes!(
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/transform_add.comp.spv")
));

/// A 4-byte-aligned wrapper around a committed SPIR-V byte blob so its address
/// is a valid `*const u32` for `VkShaderModuleCreateInfo::p_code`.
#[repr(C, align(4))]
struct SpirvBlob<const N: usize>([u8; N]);

impl<const N: usize> SpirvBlob<N> {
    /// SPIR-V byte length (a 4-byte multiple by construction — see the
    /// const-assert in [`ComputePipeline::new`]). `code_size` is in BYTES.
    #[inline]
    const fn byte_len(&self) -> usize {
        N
    }

    /// The blob's first byte as a `*const u32` SPIR-V word pointer. The
    /// `align(4)` wrapper guarantees the pointer is 4-byte aligned.
    #[inline]
    fn as_u32_ptr(&self) -> *const u32 {
        self.0.as_ptr().cast::<u32>()
    }
}

/// The `local_size_x` of both shaders (`[numthreads(64,1,1)]`). The dispatch
/// group count is `ceil(N / LOCAL_SIZE_X)`.
const LOCAL_SIZE_X: u32 = 64;

/// Errors from the compute-dispatch flow. `VkError` carries the failing command
/// name + the raw `VkResult`; `Memory` forwards a buffer/allocation failure.
#[derive(Debug)]
pub enum ComputeError {
    /// A Vulkan command returned a non-success `VkResult`.
    VkError(&'static str, VkResult),
    /// Buffer creation / sub-allocation failed.
    Memory(MemoryError),
}

impl From<MemoryError> for ComputeError {
    #[inline]
    fn from(e: MemoryError) -> Self {
        ComputeError::Memory(e)
    }
}

/// A compute pipeline built from one committed SPIR-V module, sharing the
/// single-STORAGE_BUFFER descriptor-set layout + 4-byte push-constant pipeline
/// layout owned by the [`ComputeHarness`].
///
/// Owns only the `VkShaderModule` + `VkPipeline`; the layouts are borrowed from
/// the harness (one layout serves every pipeline — both shaders share the
/// identical binding contract). Teardown is the harness's job, in reverse order.
struct ComputePipeline {
    module: VkShaderModule,
    pipeline: VkPipeline,
}

impl ComputePipeline {
    /// Builds a shader module + compute pipeline from `spv` against the harness's
    /// shared `pipeline_layout`. The caller owns teardown (`destroy_pipeline`
    /// then `destroy_shader_module`).
    fn new<const N: usize>(
        device: VkDevice,
        fns: &DeviceFns,
        pipeline_layout: VkPipelineLayout,
        spv: &SpirvBlob<N>,
    ) -> Result<Self, ComputeError> {
        // SPIR-V `code_size` must be a multiple of 4 (the spec); a committed blob
        // whose length is not a word multiple is a build-time mistake, caught
        // here rather than as a driver validation error.
        const { assert!(N.is_multiple_of(4), "SPIR-V byte length must be a multiple of 4") };

        let sm_info = VkShaderModuleCreateInfo {
            s_type: VkStructureType::ShaderModuleCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            code_size: spv.byte_len(),
            p_code: spv.as_u32_ptr(),
        };
        let mut module = VkShaderModule::NULL;
        // SAFETY: `device` is live; `sm_info` is a fully-initialized `#[repr(C)]`
        // struct whose `p_code` points to `code_size` bytes of 4-byte-aligned
        // SPIR-V (the `align(4)` `SpirvBlob` wrapper) that outlive the call (a
        // `'static` blob); `&mut module` is a valid out-pointer; NULL allocator.
        let raw = unsafe { (fns.create_shader_module)(device, &sm_info, ptr::null(), &mut module) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(ComputeError::VkError("vkCreateShaderModule", result));
        }

        let stage = VkPipelineShaderStageCreateInfo {
            s_type: VkStructureType::PipelineShaderStageCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            stage: VK_SHADER_STAGE_COMPUTE_BIT,
            module,
            // The SPIR-V entry point is `main` (DXC `-E main`); a `'static`
            // NUL-terminated literal read only during the create call.
            p_name: c"main".as_ptr(),
            p_specialization_info: ptr::null(),
        };
        let cp_info = VkComputePipelineCreateInfo {
            s_type: VkStructureType::ComputePipelineCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            stage,
            layout: pipeline_layout,
            base_pipeline_handle: VkPipeline::NULL,
            base_pipeline_index: -1,
        };
        let mut pipeline = VkPipeline::NULL;
        // SAFETY: `device` is live; null pipeline cache (`0`) is valid; one
        // create-info is fully initialized and references the just-created
        // `module` + the harness's live `pipeline_layout`; `&mut pipeline` is a
        // valid out-pointer for the single pipeline; NULL allocator. On failure
        // we destroy the orphaned module before returning so it never leaks.
        let raw = unsafe {
            (fns.create_compute_pipelines)(
                device,
                0,
                1,
                &cp_info,
                ptr::null(),
                &mut pipeline,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: `module` was created above and is not owned by any live
            // pipeline (creation failed); destroy it exactly once here.
            unsafe { (fns.destroy_shader_module)(device, module, ptr::null()) };
            return Err(ComputeError::VkError("vkCreateComputePipelines", result));
        }

        Ok(Self { module, pipeline })
    }
}

/// Owns every Vulkan object for the Slice-0 compute flow over one storage buffer
/// of `element_count` `u32`s: the shared descriptor-set layout + pipeline layout,
/// the descriptor pool + set, the command pool + buffer, the fence, and the
/// `write_pattern` (0c) + `transform_add` (0d) pipelines.
///
/// The storage buffer itself is borrowed: it lives in the caller's
/// [`HostVisibleBlock`](crate::memory::HostVisibleBlock) (so the caller controls
/// the buffer's lifetime + map),
/// and the harness only references its handle in the descriptor write and the
/// barrier. [`Drop`] tears down every owned object in strict reverse creation
/// order; the buffer is freed by the caller after the harness is dropped.
pub struct ComputeHarness<'d> {
    device: VkDevice,
    fns: &'d DeviceFns,
    element_count: u32,
    /// CPU pointer to the storage buffer's first byte (the persistent mapping).
    mapped: NonNull<u8>,
    set_layout: VkDescriptorSetLayout,
    pipeline_layout: VkPipelineLayout,
    descriptor_pool: VkDescriptorPool,
    /// Allocated FROM `descriptor_pool`; freed implicitly when the pool is
    /// destroyed (no explicit free needed for a non-FREE_DESCRIPTOR_SET pool).
    descriptor_set: VkDescriptorSet,
    command_pool: VkCommandPool,
    /// Allocated FROM `command_pool`; freed implicitly when the pool is
    /// destroyed (no explicit `vkFreeCommandBuffers` needed).
    command_buffer: VkCommandBuffer,
    fence: VkFence,
    write_pattern: ComputePipeline,
    transform_add: ComputePipeline,
}

impl<'d> ComputeHarness<'d> {
    /// Builds the full compute object graph for a storage buffer of
    /// `element_count` `u32`s already bound + mapped in `block`.
    ///
    /// `buffer` must be a [`BoundBuffer`] created from `block` with usage
    /// `VK_BUFFER_USAGE_STORAGE_BUFFER_BIT` and a size of at least
    /// `element_count * 4` bytes; the harness references its handle + mapping but
    /// does NOT own it (the caller destroys it after dropping the harness).
    ///
    /// On any partial failure, every object created so far is torn down before
    /// the error is returned, so a failed `new` leaks nothing.
    pub fn new(
        device: VkDevice,
        fns: &'d DeviceFns,
        queue_family_index: u32,
        buffer: &BoundBuffer,
        element_count: u32,
    ) -> Result<Self, ComputeError> {
        debug_assert!(element_count > 0, "element_count must be non-zero");
        debug_assert!(
            buffer.size >= (element_count as u64) * 4,
            "buffer too small for element_count u32s"
        );

        // --- 1. Descriptor-set layout: one STORAGE_BUFFER at binding 0, COMPUTE. ---
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
        let raw =
            unsafe { (fns.create_descriptor_set_layout)(device, &dsl_info, ptr::null(), &mut set_layout) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(ComputeError::VkError("vkCreateDescriptorSetLayout", result));
        }

        // A small RAII-on-error guard: from here, each fallible step that fails
        // tears down everything created so far in reverse order. Implemented as a
        // sequence of explicit destroys on the early-return paths.
        macro_rules! destroy_set_layout {
            () => {
                // SAFETY: `set_layout` was just created on `device` and is not yet
                // owned by any live pipeline layout; destroy it exactly once.
                unsafe { (fns.destroy_descriptor_set_layout)(device, set_layout, ptr::null()) }
            };
        }

        // --- 2. Pipeline layout: the set layout + a 4-byte COMPUTE push range. ---
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
        let raw =
            unsafe { (fns.create_pipeline_layout)(device, &pl_info, ptr::null(), &mut pipeline_layout) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            destroy_set_layout!();
            return Err(ComputeError::VkError("vkCreatePipelineLayout", result));
        }
        macro_rules! destroy_pipeline_layout {
            () => {{
                // SAFETY: `pipeline_layout` was created above on `device` and is
                // not owned by a live pipeline; destroy it once, then its set
                // layout.
                unsafe { (fns.destroy_pipeline_layout)(device, pipeline_layout, ptr::null()) };
                destroy_set_layout!();
            }};
        }

        // --- 3. The two compute pipelines (share the one pipeline layout). ---
        let write_pattern = match ComputePipeline::new(device, fns, pipeline_layout, &WRITE_PATTERN_SPV)
        {
            Ok(p) => p,
            Err(e) => {
                destroy_pipeline_layout!();
                return Err(e);
            }
        };
        macro_rules! destroy_write_pattern {
            () => {{
                // SAFETY: `write_pattern`'s pipeline + module were created above
                // and are not in use by any pending submission yet; destroy the
                // pipeline then its module, in reverse order.
                unsafe {
                    (fns.destroy_pipeline)(device, write_pattern.pipeline, ptr::null());
                    (fns.destroy_shader_module)(device, write_pattern.module, ptr::null());
                }
            }};
        }
        let transform_add = match ComputePipeline::new(device, fns, pipeline_layout, &TRANSFORM_ADD_SPV)
        {
            Ok(p) => p,
            Err(e) => {
                destroy_write_pattern!();
                destroy_pipeline_layout!();
                return Err(e);
            }
        };
        macro_rules! destroy_transform_add {
            () => {{
                // SAFETY: as `destroy_write_pattern`, for the second pipeline.
                unsafe {
                    (fns.destroy_pipeline)(device, transform_add.pipeline, ptr::null());
                    (fns.destroy_shader_module)(device, transform_add.module, ptr::null());
                }
            }};
        }

        // --- 4. Descriptor pool + set; bind the storage buffer. ---
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
        // SAFETY: `device` is live; `dp_info` is fully initialized and references
        // the `pool_size` local; `&mut descriptor_pool` is a valid out-pointer;
        // NULL allocator.
        let raw =
            unsafe { (fns.create_descriptor_pool)(device, &dp_info, ptr::null(), &mut descriptor_pool) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            destroy_transform_add!();
            destroy_write_pattern!();
            destroy_pipeline_layout!();
            return Err(ComputeError::VkError("vkCreateDescriptorPool", result));
        }
        macro_rules! destroy_descriptor_pool {
            () => {
                // SAFETY: `descriptor_pool` was created above on `device`;
                // destroying it also frees any set allocated from it.
                unsafe { (fns.destroy_descriptor_pool)(device, descriptor_pool, ptr::null()) }
            };
        }

        let ds_alloc = VkDescriptorSetAllocateInfo {
            s_type: VkStructureType::DescriptorSetAllocateInfo,
            p_next: ptr::null(),
            descriptor_pool,
            descriptor_set_count: 1,
            p_set_layouts: &set_layout,
        };
        let mut descriptor_set = VkDescriptorSet::NULL;
        // SAFETY: `device` is live; `ds_alloc` is fully initialized, names the
        // live pool, and references the live `set_layout`; `&mut descriptor_set`
        // is a valid out-pointer for the single requested set.
        let raw = unsafe { (fns.allocate_descriptor_sets)(device, &ds_alloc, &mut descriptor_set) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            destroy_descriptor_pool!();
            destroy_transform_add!();
            destroy_write_pattern!();
            destroy_pipeline_layout!();
            return Err(ComputeError::VkError("vkAllocateDescriptorSets", result));
        }

        // Point the descriptor at the caller's storage buffer (full range).
        let buffer_info = VkDescriptorBufferInfo {
            buffer: buffer.buffer,
            offset: 0,
            range: buffer.size,
        };
        let write = VkWriteDescriptorSet {
            s_type: VkStructureType::WriteDescriptorSet,
            p_next: ptr::null(),
            dst_set: descriptor_set,
            dst_binding: 0,
            dst_array_element: 0,
            descriptor_count: 1,
            descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
            p_image_info: ptr::null(),
            p_buffer_info: &buffer_info,
            p_texel_buffer_view: ptr::null(),
        };
        // SAFETY: `device` is live; one write referencing the live `descriptor_set`
        // and the `buffer_info` local (which names the caller's live buffer); zero
        // copies. The write is consumed entirely during the call (no retained
        // pointers), so `buffer_info` only needs to outlive this call.
        unsafe { (fns.update_descriptor_sets)(device, 1, &write, 0, ptr::null()) };

        // --- 5. Command pool + one primary command buffer. ---
        let cp_info = VkCommandPoolCreateInfo {
            s_type: VkStructureType::CommandPoolCreateInfo,
            p_next: ptr::null(),
            flags: VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT,
            queue_family_index,
        };
        let mut command_pool = VkCommandPool::NULL;
        // SAFETY: `device` is live; `cp_info` is fully initialized for the
        // graphics+compute family; `&mut command_pool` is a valid out-pointer.
        let raw = unsafe { (fns.create_command_pool)(device, &cp_info, ptr::null(), &mut command_pool) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            destroy_descriptor_pool!();
            destroy_transform_add!();
            destroy_write_pattern!();
            destroy_pipeline_layout!();
            return Err(ComputeError::VkError("vkCreateCommandPool", result));
        }
        macro_rules! destroy_command_pool {
            () => {
                // SAFETY: `command_pool` was created above; destroying it frees
                // any command buffer allocated from it.
                unsafe { (fns.destroy_command_pool)(device, command_pool, ptr::null()) }
            };
        }

        let cb_alloc = VkCommandBufferAllocateInfo {
            s_type: VkStructureType::CommandBufferAllocateInfo,
            p_next: ptr::null(),
            command_pool,
            level: VK_COMMAND_BUFFER_LEVEL_PRIMARY,
            command_buffer_count: 1,
        };
        let mut command_buffer = VkCommandBuffer::NULL;
        // SAFETY: `device` is live; `cb_alloc` names the live pool and requests
        // one primary buffer; `&mut command_buffer` is a valid out-pointer.
        let raw = unsafe { (fns.allocate_command_buffers)(device, &cb_alloc, &mut command_buffer) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            destroy_command_pool!();
            destroy_descriptor_pool!();
            destroy_transform_add!();
            destroy_write_pattern!();
            destroy_pipeline_layout!();
            return Err(ComputeError::VkError("vkAllocateCommandBuffers", result));
        }

        // --- 6. An unsignaled fence for the submit/wait. ---
        let fence_info = VkFenceCreateInfo {
            s_type: VkStructureType::FenceCreateInfo,
            p_next: ptr::null(),
            flags: 0,
        };
        let mut fence = VkFence::NULL;
        // SAFETY: `device` is live; `fence_info` is fully initialized (unsignaled);
        // `&mut fence` is a valid out-pointer; NULL allocator.
        let raw = unsafe { (fns.create_fence)(device, &fence_info, ptr::null(), &mut fence) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // Command buffer is freed with the pool.
            destroy_command_pool!();
            destroy_descriptor_pool!();
            destroy_transform_add!();
            destroy_write_pattern!();
            destroy_pipeline_layout!();
            return Err(ComputeError::VkError("vkCreateFence", result));
        }

        Ok(Self {
            device,
            fns,
            element_count,
            mapped: buffer.mapped,
            set_layout,
            pipeline_layout,
            descriptor_pool,
            descriptor_set,
            command_pool,
            command_buffer,
            fence,
            write_pattern,
            transform_add,
        })
    }

    /// The dispatch group count for the buffer's element count
    /// (`ceil(element_count / 64)`).
    #[inline]
    fn group_count_x(&self) -> u32 {
        self.element_count.div_ceil(LOCAL_SIZE_X)
    }

    /// Records the common preamble into the (reset) command buffer: begin,
    /// bind the descriptor set for COMPUTE.
    ///
    /// # Safety
    ///
    /// `self.command_buffer` must be in the initial/recordable state (freshly
    /// allocated or reset) and not pending on the GPU; the descriptor set must be
    /// validly bound to the buffer (it is, after `new`).
    unsafe fn begin_and_bind_set(&self) -> Result<(), ComputeError> {
        let begin = VkCommandBufferBeginInfo {
            s_type: VkStructureType::CommandBufferBeginInfo,
            p_next: ptr::null(),
            flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            p_inheritance_info: ptr::null(),
        };
        // SAFETY: per this fn's contract `command_buffer` is recordable and not
        // pending; `begin` is a fully-initialized one-time-submit begin-info.
        let raw = unsafe { (self.fns.begin_command_buffer)(self.command_buffer, &begin) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(ComputeError::VkError("vkBeginCommandBuffer", result));
        }

        // SAFETY: recording is open; `descriptor_set` is the live set bound to the
        // buffer; `pipeline_layout` is its layout; set index 0 matches the
        // shaders' `set 0`; zero dynamic offsets (null is valid for count 0).
        unsafe {
            (self.fns.cmd_bind_descriptor_sets)(
                self.command_buffer,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                self.pipeline_layout,
                0,
                1,
                &self.descriptor_set,
                0,
                ptr::null(),
            );
        }
        Ok(())
    }

    /// Records one pipeline's dispatch: bind the pipeline, push the element count,
    /// dispatch `ceil(N/64)` groups.
    ///
    /// # Safety
    ///
    /// Recording must be open on `self.command_buffer` and the descriptor set
    /// already bound (via [`Self::begin_and_bind_set`]); `pipeline` must be one of
    /// this harness's compute pipelines built against `self.pipeline_layout`.
    unsafe fn record_dispatch(&self, pipeline: VkPipeline) {
        // SAFETY: recording is open; `pipeline` is a live compute pipeline of this
        // harness; COMPUTE bind point matches its creation.
        unsafe {
            (self.fns.cmd_bind_pipeline)(
                self.command_buffer,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                pipeline,
            );
        }

        let count = self.element_count;
        // SAFETY: recording is open; `pipeline_layout` declares a 4-byte COMPUTE
        // push range at offset 0; `&count` points to exactly 4 bytes of a `u32`
        // local that outlives the call; size/offset (4/0) match the range.
        unsafe {
            (self.fns.cmd_push_constants)(
                self.command_buffer,
                self.pipeline_layout,
                VK_SHADER_STAGE_COMPUTE_BIT,
                0,
                4,
                (&count as *const u32).cast::<c_void>(),
            );
        }

        // SAFETY: recording is open; the bound pipeline + set cover the dispatch;
        // group count `ceil(N/64)` with the shader's `i < count` bounds check
        // never writes past element `N-1`.
        unsafe {
            (self.fns.cmd_dispatch)(self.command_buffer, self.group_count_x(), 1, 1);
        }
    }

    /// Records a buffer memory barrier between two COMPUTE dispatches on the
    /// storage buffer: SHADER_WRITE (the first pass) → SHADER_READ + SHADER_WRITE
    /// (the second pass reads then writes back). This is the §5.5 edge→barrier
    /// lowering in miniature.
    ///
    /// # Safety
    ///
    /// Recording must be open; `buffer` must be the storage buffer the bound
    /// descriptor set points at (so the barrier scopes the right resource).
    unsafe fn record_barrier(&self, buffer: VkBuffer) {
        let barrier = VkBufferMemoryBarrier {
            s_type: VkStructureType::BufferMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: VK_ACCESS_SHADER_WRITE_BIT,
            // The 0d shader reads then writes the same element, so the barrier
            // makes the prior writes visible to both subsequent reads and writes.
            dst_access_mask: VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            buffer,
            offset: 0,
            size: VK_WHOLE_SIZE,
        };
        // SAFETY: recording is open; the barrier is a fully-initialized
        // `#[repr(C)]` struct naming the live buffer; COMPUTE_SHADER→COMPUTE_SHADER
        // with WRITE→READ|WRITE on the whole buffer is the correct (and
        // superset-correct) write-before-read dependency; zero global/image
        // barriers (null arrays are valid for count 0); `&barrier` points to one
        // buffer barrier alive for the call.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                self.command_buffer,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                0,
                0,
                ptr::null(),
                1,
                &barrier,
                0,
                ptr::null(),
            );
        }
    }

    /// Ends recording and submits the command buffer with the harness's fence,
    /// then waits indefinitely for completion (the single sync point).
    ///
    /// # Safety
    ///
    /// Recording must be open on `self.command_buffer`; the fence must be
    /// unsignaled (it is freshly created, or reset is not needed for a single
    /// submit). After this returns `Ok`, the GPU work is complete and the buffer's
    /// mapped contents are coherent for CPU read.
    unsafe fn end_submit_wait(&self, queue: VkQueue) -> Result<(), ComputeError> {
        // SAFETY: recording is open per the contract; ending it is the matching
        // close of the `begin` in `begin_and_bind_set`.
        let raw = unsafe { (self.fns.end_command_buffer)(self.command_buffer) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(ComputeError::VkError("vkEndCommandBuffer", result));
        }

        let submit = VkSubmitInfo {
            s_type: VkStructureType::SubmitInfo,
            p_next: ptr::null(),
            wait_semaphore_count: 0,
            p_wait_semaphores: ptr::null(),
            p_wait_dst_stage_mask: ptr::null(),
            command_buffer_count: 1,
            p_command_buffers: &self.command_buffer,
            signal_semaphore_count: 0,
            p_signal_semaphores: ptr::null(),
        };
        // SAFETY: `queue` is the device's live graphics+compute queue; one submit
        // referencing the just-ended `command_buffer` (the `&self.command_buffer`
        // local outlives the call); no semaphores (null arrays valid for count 0);
        // `self.fence` is the live unsignaled fence to signal on completion.
        let raw = unsafe { (self.fns.queue_submit)(queue, 1, &submit, self.fence) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(ComputeError::VkError("vkQueueSubmit", result));
        }

        // SAFETY: `device` is live; `&self.fence` names the one submitted fence;
        // `wait_all = VK_TRUE` with an infinite timeout blocks until the GPU
        // signals it — the fence-before-readback discipline. After this the
        // command buffer is no longer pending, so it (and every object it
        // referenced) is safe to read back / destroy.
        let raw = unsafe {
            (self.fns.wait_for_fences)(
                self.device,
                1,
                &self.fence,
                VK_TRUE,
                VK_TIMEOUT_INFINITE,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(ComputeError::VkError("vkWaitForFences", result));
        }
        Ok(())
    }

    /// **Step 0c**: records + submits ONLY the `write_pattern` pass, waits on the
    /// fence, and returns the buffer's `element_count` `u32`s read back from the
    /// persistent mapping.
    ///
    /// The returned `Vec` is the TEST ORACLE readback (plan §11 step 3) — not a
    /// per-frame path. The caller asserts `out[i] == i*2 + 1`.
    pub fn run_write_pattern(&self, queue: VkQueue) -> Result<Vec<u32>, ComputeError> {
        // SAFETY: the command buffer is freshly allocated (or this is its first
        // use) and not pending; begin recording + bind the set.
        unsafe { self.begin_and_bind_set()? };
        // SAFETY: recording is open and the set bound; record the 0c pipeline.
        unsafe { self.record_dispatch(self.write_pattern.pipeline) };
        // SAFETY: recording is open; end + submit + fence-wait. On return the
        // GPU writes are complete and the mapping is coherent for CPU read.
        unsafe { self.end_submit_wait(queue)? };

        Ok(self.read_back())
    }

    /// **Step 0d**: records BOTH passes into ONE command buffer chained by a
    /// `vkCmdPipelineBarrier` (`write_pattern` → barrier → `transform_add`),
    /// submits once with the fence, waits, and returns the buffer read back.
    ///
    /// The caller diffs the result against the CPU golden `(i*2 + 1) + 100`.
    pub fn run_chained(&self, queue: VkQueue, buffer: VkBuffer) -> Result<Vec<u32>, ComputeError> {
        // SAFETY: the command buffer is recordable (RESET_COMMAND_BUFFER pool;
        // each `begin` implicitly resets it) and not pending; begin + bind set.
        unsafe { self.begin_and_bind_set()? };
        // SAFETY: recording is open; first pass writes the pattern.
        unsafe { self.record_dispatch(self.write_pattern.pipeline) };
        // SAFETY: recording is open; the barrier orders the first pass's writes
        // before the second pass's reads on the same `buffer`.
        unsafe { self.record_barrier(buffer) };
        // SAFETY: recording is open; second pass reads + transforms.
        unsafe { self.record_dispatch(self.transform_add.pipeline) };
        // SAFETY: recording is open; end + submit + fence-wait (single sync).
        unsafe { self.end_submit_wait(queue)? };

        Ok(self.read_back())
    }

    /// Reads `element_count` `u32`s from the persistent host-coherent mapping.
    ///
    /// Only valid AFTER a fence-waited submit (the caller's `run_*` ensures this);
    /// host-coherent memory makes the GPU writes visible without an invalidate.
    fn read_back(&self) -> Vec<u32> {
        let n = self.element_count as usize;
        let mut out = Vec::with_capacity(n);
        let base = self.mapped.as_ptr().cast::<u32>();
        for i in 0..n {
            // SAFETY: the buffer is `element_count * 4` bytes inside the persistent
            // host-coherent mapping (the caller sized it so in `new`'s
            // debug_assert); `base + i` for `i < n` is in-bounds. A fence wait
            // preceded this read, so the GPU writes are complete + coherent.
            // `read_unaligned` tolerates any alignment of the sub-allocated offset.
            let v = unsafe { base.add(i).read_unaligned() };
            out.push(v);
        }
        out
    }
}

impl Drop for ComputeHarness<'_> {
    fn drop(&mut self) {
        // SAFETY: every object was created on `self.device` in `new` and is
        // destroyed here exactly once in strict reverse creation order. By the
        // time a harness is dropped its last submission has been fence-waited
        // (the `run_*` methods block on the fence before returning), so no object
        // is in use by a pending command buffer. `vkDeviceWaitIdle` is a belt-and-
        // braces barrier in case the harness is dropped without a completed run
        // (e.g. an early `?` after `new`): it guarantees the device is idle before
        // any destroy. The descriptor set + command buffer are freed implicitly by
        // destroying their pools (no FREE_DESCRIPTOR_SET / explicit free needed).
        // The storage buffer is NOT owned here — the caller frees it after.
        unsafe {
            (self.fns.device_wait_idle)(self.device);
            (self.fns.destroy_fence)(self.device, self.fence, ptr::null());
            (self.fns.destroy_command_pool)(self.device, self.command_pool, ptr::null());
            (self.fns.destroy_descriptor_pool)(self.device, self.descriptor_pool, ptr::null());
            (self.fns.destroy_pipeline)(self.device, self.transform_add.pipeline, ptr::null());
            (self.fns.destroy_shader_module)(self.device, self.transform_add.module, ptr::null());
            (self.fns.destroy_pipeline)(self.device, self.write_pattern.pipeline, ptr::null());
            (self.fns.destroy_shader_module)(self.device, self.write_pattern.module, ptr::null());
            (self.fns.destroy_pipeline_layout)(self.device, self.pipeline_layout, ptr::null());
            (self.fns.destroy_descriptor_set_layout)(self.device, self.set_layout, ptr::null());
        }
    }
}

/// The CPU golden for step 0c: `out[i] == i*2 + 1` (the `write_pattern` shader).
///
/// Exposed so the test (and any later golden harness) shares ONE definition of
/// the shader's contract rather than duplicating the arithmetic.
#[inline]
pub fn golden_write_pattern(i: u32) -> u32 {
    i.wrapping_mul(2).wrapping_add(1)
}

/// The CPU golden for the chained 0c→0d result: `(i*2 + 1) + 100` (the
/// `transform_add` shader applied on top of `write_pattern`).
#[inline]
pub fn golden_chained(i: u32) -> u32 {
    golden_write_pattern(i).wrapping_add(100)
}
