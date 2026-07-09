//! `VulkanCommandEncoder`: its inherent FFI recording helpers plus the
//! `RhiCommandEncoder` trait implementation — the hot command-recording path.
//! Split out of the parent `rhi_impl` module; a pure structural move — every
//! impl body is unchanged.

// `use super::*` surfaces the types + module-level consts/free-fns DEFINED in `rhi_impl`
// (mod.rs) and, through the parent's `use crate::ffi::*` glob, the FFI names. A glob import
// does NOT re-export the parent's NAMED `use` bindings, so the `boyko_rhi` / `crate::*`
// items the moved impls reference are re-declared below, pruned to exactly what they use.
use super::*;

use core::ffi::c_void;
use core::ptr;

use boyko_rhi::{
    BarrierDesc, BufferBarrier, BufferCopy, BufferImageCopy, ImageBarrierDesc, ImageLayout,
    ImageSubresourceRange, IndexType, RenderArea, RenderingDesc, RhiCommandEncoder, ShaderStage,
    TimestampStage, Viewport,
};

use crate::device::DeviceFns;
use crate::error::VulkanError;
use crate::memory::BoundBuffer;
use crate::texture::VulkanTexture;

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
    //
    // `pub(super)`: constructed by `RhiDevice::create_command_encoder` in the sibling
    // `rhi_impl::device` module. Before the file split this was module-private within the
    // single `rhi_impl` module; `pub(super)` preserves the identical accessibility scope
    // (the `rhi_impl` subtree — mod.rs + device.rs + encoder.rs), exposing nothing outside it.
    pub(super) unsafe fn new(
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
            // HW-RT rung R2a-1: null until `create_command_encoder` wires the context's
            // `AccelFns` under `hwrt` (a non-RT device leaves it null → the AS verbs no-op).
            #[cfg(feature = "hwrt")]
            accel_fns: ptr::null(),
        })
    }

    /// HW-RT rung R2a-1: wires the owning context's `AccelFns` (the AS command table) into
    /// the encoder so its `cmd_build_acceleration_structures` can reach the `vkCmd*` FFI. A
    /// raw pointer into the context (which outlives the encoder, §5.3); `null` when ray query
    /// is off. Gated `hwrt`.
    #[cfg(feature = "hwrt")]
    pub(crate) fn set_accel_fns(&mut self, accel: *const crate::accel::AccelFns) {
        self.accel_fns = accel;
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
    //
    // `pub(super)`: invoked by `RhiDevice::destroy_command_encoder` in the sibling
    // `rhi_impl::device` module. `pub(super)` preserves the pre-split module-private scope
    // (the `rhi_impl` subtree), exposing nothing outside it.
    pub(super) unsafe fn destroy(self, device: VkDevice, fns: &DeviceFns) {
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
        // Plan B2 (ABI-1/TD-5): this encoder records exclusively against the shared
        // COMPUTE pipeline layout, whose push range is declared at offset 0 with
        // `COMPUTE_PUSH_CONSTANT_RANGE_BYTES` bytes (P0a widened it from 4 to the
        // 80-byte `sdf_depth_composite` marcher block). `offset + len` outside
        // `[0, COMPUTE_PUSH_CONSTANT_RANGE_BYTES]`, or a non-COMPUTE stage, is a
        // caller error against that layout. Bound derived from the same constant the
        // layout uses, never a magic literal, so a future widening re-sizes both.
        debug_assert!(
            offset as u64 + bytes.len() as u64 <= COMPUTE_PUSH_CONSTANT_RANGE_BYTES as u64,
            "invariant: push range within COMPUTE_PUSH_CONSTANT_RANGE_BYTES"
        );
        debug_assert!(
            stage.bits() == crate::ffi::VK_SHADER_STAGE_COMPUTE_BIT,
            "invariant: compute push stage"
        );
        // The agnostic `ShaderStage` bits equal `VK_SHADER_STAGE_*` (plan D5).
        let stage_flags: VkFlags = stage.bits();
        // SAFETY: recording is open; `self.pipeline_layout` declares a
        // `COMPUTE_PUSH_CONSTANT_RANGE_BYTES`-byte COMPUTE push range at offset 0;
        // `bytes.as_ptr()` points to `bytes.len()` bytes alive for the call; the
        // caller passes offset/size within the declared range (asserted above).
        // `self.fns` borrows the live device fn-table.
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
        // SAFETY: `self.fns` points into the owning context's boxed `DeviceFns` — a stable
        // heap address that outlives this encoder (context teardown order); deref is valid.
        let fns = unsafe { &*self.fns };
        // The packed-buffer path binds its STORAGE_BUFFER via `bind_storage_buffer`
        // (so `bound_buffer != NULL`) before every dispatch — for it the recorded
        // command stream is byte-identical to before: bind the fixed set against the
        // device-shared `pipeline_layout`, then dispatch. The Render P1a
        // vocabulary-compute path instead binds its set via
        // `bind_descriptor_set_compute` (against the pipeline's OWN layout) and never
        // calls `bind_storage_buffer` (`bound_buffer == NULL`), so the fixed-set rebind
        // is skipped — it would otherwise clobber the vocabulary set 0 and bind against
        // an incompatible layout. The two paths thus coexist without touching each
        // other's recorded commands.
        if self.bound_buffer != VkBuffer::NULL {
            // SAFETY: recording is open; the fixed STORAGE_BUFFER set was pointed at the
            // bound buffer by `bind_storage_buffer` and is bound at the cached set index
            // for the COMPUTE bind point against the live device-shared `pipeline_layout`;
            // zero dynamic offsets (null valid for count 0). `self.fns` borrows the live
            // device fn-table.
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
            }
        }
        // SAFETY: recording is open; the bound compute pipeline + its descriptor set
        // (the fixed set just bound above for the packed path, or the vocabulary set
        // bound earlier via `bind_descriptor_set_compute`) cover the dispatch.
        unsafe { (fns.cmd_dispatch)(self.command_buffer, gx, gy, gz) };
    }

    fn reset_query_pool(&mut self, pool: &VulkanQueryPool, first: u32, count: u32) {
        debug_assert!(
            first + count <= pool.count,
            "invariant: reset range must fit the pool's query count"
        );
        // SAFETY: `self.fns` points into the owning context's boxed `DeviceFns` — a stable
        // heap address that outlives this encoder (context teardown order); deref is valid.
        let fns = unsafe { &*self.fns };
        // SAFETY: recording is open; `pool.pool` is a live TIMESTAMP pool; `[first..first+count)`
        // is in bounds (asserted above). MUST be recorded OUTSIDE any render / dynamic-rendering
        // scope (caller contract — the collector resets at the compute-only frame top).
        unsafe { (fns.cmd_reset_query_pool)(self.command_buffer, pool.pool, first, count) };
    }

    fn write_timestamp(&mut self, pool: &VulkanQueryPool, stage: TimestampStage, index: u32) {
        debug_assert!(index < pool.count, "invariant: timestamp index must be in the pool");
        // Map the agnostic stage to a `VkPipelineStageFlagBits` via an identity cast — the
        // `TimestampStage` discriminants equal the `VK_PIPELINE_STAGE_*` bit values (asserted
        // in `abi_guard.rs`).
        let vk_stage: VkFlags = stage.as_i32() as VkFlags;
        // SAFETY: `self.fns` points into the owning context's boxed `DeviceFns` (alive per the
        // type contract); deref is valid.
        let fns = unsafe { &*self.fns };
        // SAFETY: recording is open; `pool.pool` is a live TIMESTAMP pool; `index < pool.count`
        // (asserted above) and was reset this frame via `reset_query_pool` (caller contract);
        // `vk_stage` is a single valid pipeline-stage bit (TOP/BOTTOM).
        unsafe { (fns.cmd_write_timestamp)(self.command_buffer, vk_stage, pool.pool, index) };
    }

    // ===== HW-RT ACCELERATION-STRUCTURE ENCODER VERBS (rung R2a-1; `hwrt` overrides) =====

    #[cfg(feature = "hwrt")]
    fn cmd_build_acceleration_structures(
        &mut self,
        entries: &[boyko_rhi::AsBuildEntry],
        dest: &[&crate::accel::BoundAccelStruct],
    ) {
        if self.accel_fns.is_null() || entries.is_empty() {
            // Ray query off (null table) or nothing to build → no-op (R2a-1 records nothing).
            return;
        }
        // SAFETY: `self.accel_fns` is a live pointer into the owning context's `AccelFns` (set
        // by `create_command_encoder`, non-null checked above; the context outlives the
        // encoder, §5.3); recording is open. `crate::accel::cmd_build_acceleration_structures`
        // upholds the per-build FFI invariants (documented at its definition): every device
        // address in `entries` + each `dest[i].handle` is a live, correctly-flagged resource
        // the caller pre-created, and `entries.len() == dest.len()`.
        let fns = unsafe { &*self.accel_fns };
        // SAFETY: as above — the command buffer is recording, `fns` is the live AS table.
        unsafe {
            crate::accel::cmd_build_acceleration_structures(
                fns,
                self.command_buffer,
                entries,
                dest,
            );
        }
    }

    #[cfg(feature = "hwrt")]
    fn cmd_acceleration_structure_barrier(&mut self) {
        // SAFETY: `self.fns` borrows the live device fn-table (context outlives the encoder);
        // recording is open. The helper records one AS write→read global memory barrier.
        let fns = unsafe { &*self.fns };
        // SAFETY: as above — the command buffer is recording, `fns` is the live device table.
        unsafe { crate::accel::cmd_acceleration_structure_barrier(fns, self.command_buffer) };
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

    fn copy_buffer(&mut self, src: &BoundBuffer, dst: &BoundBuffer, regions: &[BufferCopy]) {
        debug_assert!(!regions.is_empty(), "invariant: copy_buffer needs >= 1 region");
        // The agnostic `BufferCopy` is `#[repr(C)] { src_offset, dst_offset, size:
        // u64 }` — byte-identical to the Vulkan `VkBufferCopy` (same field order +
        // `u64` types), so a `&[BufferCopy]` reinterprets directly as a
        // `&[VkBufferCopy]` without a per-region copy. The size + alignment match
        // is enforced at build time here.
        const _: () = assert!(
            core::mem::size_of::<BufferCopy>() == core::mem::size_of::<VkBufferCopy>(),
            "BufferCopy and VkBufferCopy must share size for the slice reinterpret"
        );
        const _: () = assert!(
            core::mem::align_of::<BufferCopy>() == core::mem::align_of::<VkBufferCopy>(),
            "BufferCopy and VkBufferCopy must share alignment for the slice reinterpret"
        );
        // SAFETY: `BufferCopy` and `VkBufferCopy` are both `#[repr(C)]` with the
        // identical `(u64, u64, u64)` layout (size + align asserted above), so
        // casting the `*const BufferCopy` to `*const VkBufferCopy` and reading
        // `regions.len()` elements is in-bounds and ABI-valid — every field maps
        // 1:1. The slice is alive for the call.
        let vk_regions = regions.as_ptr().cast::<VkBufferCopy>();
        // SAFETY: recording is open; `src.buffer`/`dst.buffer` are live buffers
        // (created on this device, carrying the `TRANSFER_SRC`/`TRANSFER_DST` usage
        // the device-local path always adds); `vk_regions` points to `regions.len()`
        // fully-initialized `VkBufferCopy`s alive for the call. `self.fns` points
        // into the context's boxed fn-table (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_copy_buffer)(
                self.command_buffer,
                src.buffer,
                dst.buffer,
                regions.len() as u32,
                vk_regions,
            );
        }
    }

    fn image_barrier(&mut self, barrier: &ImageBarrierDesc<Vulkan>) {
        // Map the agnostic stage/access masks via identity casts (the
        // `BarrierStage`/`BarrierAccess` bit values equal the `VK_PIPELINE_STAGE_*`
        // / `VK_ACCESS_*` constants — asserted in `abi_guard.rs`); `ImageLayout` /
        // `ImageAspect` are the `i32`/`u32` FFI families mapped by `as_i32()`/
        // `bits()`. This abstracts the concrete `swapchain.rs::record_clear`
        // `VkImageMemoryBarrier`.
        let src_stage: VkFlags = barrier.src_stage.bits();
        let dst_stage: VkFlags = barrier.dst_stage.bits();
        debug_assert!(
            src_stage != 0 && dst_stage != 0,
            "invariant: an image barrier needs non-empty src+dst stages"
        );
        let image_barrier = VkImageMemoryBarrier {
            s_type: VkStructureType::ImageMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: barrier.src_access.bits(),
            dst_access_mask: barrier.dst_access.bits(),
            old_layout: barrier.old_layout.as_i32(),
            new_layout: barrier.new_layout.as_i32(),
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image: barrier.texture.image,
            subresource_range: VkImageSubresourceRange {
                aspect_mask: barrier.range.aspect.bits(),
                base_mip_level: barrier.range.base_mip_level,
                level_count: barrier.range.level_count,
                base_array_layer: barrier.range.base_array_layer,
                layer_count: barrier.range.layer_count,
            },
        };
        // SAFETY: recording is open; `src_stage`/`dst_stage` are the mapped stage
        // masks; one fully-initialized `VkImageMemoryBarrier` (the `image_barrier`
        // local, alive for the call) names the live `barrier.texture.image` with the
        // requested old→new layout + access scopes; zero global/buffer barriers
        // (null arrays valid for count 0). `self.fns` points into the context's
        // boxed fn-table (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_pipeline_barrier)(
                self.command_buffer,
                src_stage,
                dst_stage,
                0,
                0,
                ptr::null(),
                0,
                ptr::null(),
                1,
                (&image_barrier as *const VkImageMemoryBarrier).cast(),
            );
        }
    }

    fn begin_rendering(&mut self, desc: &RenderingDesc<Vulkan>) {
        // Abstracts the concrete `swapchain.rs::record_clear` `VkRenderingInfo`
        // begin (one color attachment, loadOp/storeOp/clear from the desc). The
        // basic slice's G-buffer has a small, fixed attachment count, so the
        // attachment array is a stack local sized for `MAX_RENDERING_COLOR_ATTACHMENTS`
        // — zero heap allocation on the record path.
        let count = desc.colors.len();
        debug_assert!(
            count <= MAX_RENDERING_COLOR_ATTACHMENTS,
            "invariant: begin_rendering color-attachment count exceeds the fixed cap"
        );
        let count = count.min(MAX_RENDERING_COLOR_ATTACHMENTS);

        // Build the fixed-capacity attachment array: the first `count` slots map the
        // caller's color attachments; the tail slots hold a neutral default that is
        // never read (only `count` entries are passed to the driver). `from_fn`
        // avoids requiring `Copy` on the raw-pointer-bearing `VkRenderingAttachmentInfo`.
        let attachments: [VkRenderingAttachmentInfo; MAX_RENDERING_COLOR_ATTACHMENTS] =
            core::array::from_fn(|i| {
                if i < count {
                    let color = &desc.colors[i];
                    VkRenderingAttachmentInfo {
                        s_type: VkStructureType::RenderingAttachmentInfo,
                        p_next: ptr::null(),
                        image_view: color.texture.view,
                        image_layout: color.layout.as_i32(),
                        resolve_mode: 0,
                        resolve_image_view: VkImageView::NULL,
                        resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                        load_op: color.load_op.as_i32(),
                        store_op: color.store_op.as_i32(),
                        clear_value: VkClearValue {
                            color: VkClearColorValue {
                                float32: color.clear_color,
                            },
                        },
                    }
                } else {
                    VkRenderingAttachmentInfo {
                        s_type: VkStructureType::RenderingAttachmentInfo,
                        p_next: ptr::null(),
                        image_view: VkImageView::NULL,
                        image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                        resolve_mode: 0,
                        resolve_image_view: VkImageView::NULL,
                        resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                        load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
                        store_op: VK_ATTACHMENT_STORE_OP_STORE,
                        clear_value: VkClearValue {
                            color: VkClearColorValue { float32: [0.0; 4] },
                        },
                    }
                }
            });

        // The optional depth attachment (Phase-6 S0 rung 4). When present, build one
        // `VkRenderingAttachmentInfo` whose clear value uses the depth-stencil variant
        // of the `VkClearValue` union (depth = `clear_depth`, e.g. 1.0; stencil unused).
        // The `depth_attachment` local must outlive the `cmd_begin_rendering` call, so
        // it is bound here and `p_depth_attachment` points at it; `None` leaves the
        // pointer null (the rungs-1..3 no-depth path). `as_i32()` lowerings equal the
        // `VkImageLayout`/`VkAttachmentLoadOp`/`VkAttachmentStoreOp` constants (asserted
        // in `abi_guard.rs`).
        let depth_attachment = desc.depth.as_ref().map(|d| VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: d.texture.view,
            image_layout: d.layout.as_i32(),
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: d.load_op.as_i32(),
            store_op: d.store_op.as_i32(),
            clear_value: VkClearValue {
                depth_stencil: VkClearDepthStencilValue {
                    depth: d.clear_depth,
                    stencil: 0,
                },
            },
        });
        let p_depth_attachment: *const c_void = match &depth_attachment {
            Some(att) => (att as *const VkRenderingAttachmentInfo).cast(),
            None => ptr::null(),
        };

        let rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: VkRect2D {
                offset: VkOffset2D {
                    x: desc.render_area.x,
                    y: desc.render_area.y,
                },
                extent: VkExtent2D {
                    width: desc.render_area.width,
                    height: desc.render_area.height,
                },
            },
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: count as u32,
            p_color_attachments: if count == 0 {
                ptr::null()
            } else {
                attachments.as_ptr()
            },
            p_depth_attachment,
            p_stencil_attachment: ptr::null(),
        };
        // SAFETY: recording is open; `rendering` is fully initialized and its
        // `p_color_attachments` points to the first `count` entries of the live
        // `attachments` stack array (each naming the caller's live image view, now
        // in the declared layout per a prior `image_barrier`). `p_depth_attachment`
        // points at the live `depth_attachment` local (alive for this call) naming the
        // caller's live DEPTH-aspect view in DEPTH_ATTACHMENT_OPTIMAL (per a prior
        // depth `image_barrier`) when a depth attachment is requested, else null. No
        // stencil (null). Dynamic rendering is enabled on the device (`dynamicRendering`
        // feature, Correction #1). All locals outlive the call. `self.fns` points into
        // the context's boxed fn-table (alive per the type contract).
        // SAFETY: `self.fns` points into the owning context's boxed `DeviceFns` — a stable
        // heap address that outlives this encoder (context teardown order); deref is valid.
        let fns = unsafe { &*self.fns };
        unsafe { (fns.cmd_begin_rendering)(self.command_buffer, &rendering) };
    }

    fn end_rendering(&mut self) {
        // SAFETY: recording is open and a `begin_rendering` opened the scope (caller
        // contract); `vkCmdEndRendering` is its matching close. `self.fns` points
        // into the context's boxed fn-table (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe { (fns.cmd_end_rendering)(self.command_buffer) };
    }

    fn bind_graphics_pipeline(&mut self, pipeline: &VulkanGraphicsPipeline) {
        // SAFETY: recording is open; `pipeline.pipeline` is a live graphics pipeline
        // (its declared color format must match the enclosing `begin_rendering`
        // scope — the W2-b draw-time contract); the GRAPHICS bind point matches its
        // creation. `self.fns` points into the context's boxed fn-table (alive per
        // the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_bind_pipeline)(
                self.command_buffer,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                pipeline.pipeline,
            );
        }
    }

    fn bind_descriptor_set(
        &mut self,
        group: &VulkanBindGroup,
        pipeline: &VulkanGraphicsPipeline,
    ) {
        // SAFETY: recording is open and inside a `begin_rendering` scope with the
        // matching graphics pipeline bound (caller contract); `pipeline.layout` is
        // that pipeline's own layout, built with the same bind-group set-layout at
        // `set 0` (`GraphicsPipelineDesc::bind_group_layout`), so binding
        // `group.descriptor_set` there for the GRAPHICS bind point is type-compatible.
        // `&group.descriptor_set` is a single-element local (alive for the call), so
        // `first_set = 0`, `descriptor_set_count = 1` matches it; zero dynamic offsets
        // (null valid for count 0). `self.fns` points into the context's boxed
        // fn-table (alive per the type contract).
        // SAFETY: `self.fns` points into the owning context's boxed `DeviceFns` — a stable
        // heap address that outlives this encoder (context teardown order); deref is valid.
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_bind_descriptor_sets)(
                self.command_buffer,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                pipeline.layout,
                0,
                1,
                &group.descriptor_set,
                0,
                ptr::null(),
            );
        }
    }

    fn bind_descriptor_set_compute(
        &mut self,
        group: &VulkanBindGroup,
        compute_pipeline: &ComputePipeline,
    ) {
        // SAFETY: recording is open with `compute_pipeline` bound (caller contract);
        // `compute_pipeline.layout` is that pipeline's own layout, built with the same
        // vocabulary bind-group set-layout at `set 0`
        // (`ComputePipelineDesc::bind_group_layout`), so binding `group.descriptor_set`
        // there for the COMPUTE bind point is type-compatible. `&group.descriptor_set`
        // is a single-element local (alive for the call), so `first_set = 0`,
        // `descriptor_set_count = 1` matches it; zero dynamic offsets (null valid for
        // count 0). This binds the vocabulary set ONLY — it does not touch the
        // encoder's fixed STORAGE_BUFFER set (`bind_storage_buffer`/`dispatch`), so the
        // packed-buffer offscreen path is unaffected. `self.fns` points into the
        // context's boxed fn-table (alive per the type contract).
        // SAFETY: `self.fns` points into the owning context's boxed `DeviceFns` — a stable
        // heap address that outlives this encoder (context teardown order); deref is valid.
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_bind_descriptor_sets)(
                self.command_buffer,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                compute_pipeline.layout,
                0,
                1,
                &group.descriptor_set,
                0,
                ptr::null(),
            );
        }
    }

    fn set_viewport(&mut self, viewport: &Viewport) {
        // The agnostic `Viewport` is `#[repr(C)] { x, y, width, height, min_depth,
        // max_depth: f32 }` — byte-identical to `VkViewport` (same field order +
        // `f32` types), so the `*const Viewport` casts directly to `*const VkViewport`
        // without a per-call copy. The size + align match is enforced at build time.
        const _: () = assert!(
            core::mem::size_of::<Viewport>() == core::mem::size_of::<VkViewport>(),
            "Viewport and VkViewport must share size for the pointer reinterpret"
        );
        const _: () = assert!(
            core::mem::align_of::<Viewport>() == core::mem::align_of::<VkViewport>(),
            "Viewport and VkViewport must share alignment for the pointer reinterpret"
        );
        let vk_viewport = (viewport as *const Viewport).cast::<VkViewport>();
        // SAFETY: recording is open; `Viewport`/`VkViewport` share layout (asserted),
        // so reading one `VkViewport` from `vk_viewport` (the live `viewport` borrow,
        // alive for the call) is ABI-valid; `first_viewport = 0`, `count = 1` matches
        // the pipeline's single dynamic viewport. `self.fns` points into the context's
        // boxed fn-table (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe { (fns.cmd_set_viewport)(self.command_buffer, 0, 1, vk_viewport) };
    }

    fn set_scissor(&mut self, scissor: &RenderArea) {
        let rect = VkRect2D {
            offset: VkOffset2D {
                x: scissor.x,
                y: scissor.y,
            },
            extent: VkExtent2D {
                width: scissor.width,
                height: scissor.height,
            },
        };
        // SAFETY: recording is open; one fully-initialized `VkRect2D` (the `rect`
        // local, alive for the call) describes the scissor; `first_scissor = 0`,
        // `count = 1` matches the pipeline's single dynamic scissor. `self.fns` points
        // into the context's boxed fn-table (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe { (fns.cmd_set_scissor)(self.command_buffer, 0, 1, &rect) };
    }

    fn bind_vertex_buffer(&mut self, buffer: &BoundBuffer, binding: u32, offset: u64) {
        let buffers = [buffer.buffer];
        let offsets = [offset as VkDeviceSize];
        // SAFETY: recording is open; `buffer.buffer` is a live buffer (created on this
        // device, carrying VERTEX usage); `buffers`/`offsets` are single-element stack
        // locals alive for the call, so `binding_count = 1` matches both array
        // pointers; `offset` is a byte offset within the bound buffer (the caller's
        // contract). `self.fns` points into the context's boxed fn-table (alive per
        // the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_bind_vertex_buffers)(
                self.command_buffer,
                binding,
                1,
                buffers.as_ptr(),
                offsets.as_ptr(),
            );
        }
    }

    fn bind_index_buffer(&mut self, buffer: &BoundBuffer, offset: u64, index_type: IndexType) {
        // The agnostic `IndexType` discriminant equals the `VkIndexType` constant
        // (asserted in `abi_guard.rs`).
        // SAFETY: recording is open; `buffer.buffer` is a live buffer (created on this
        // device, carrying INDEX usage); `offset` is a byte offset within it (the
        // caller's contract); `index_type` is a valid `VkIndexType`. `self.fns` points
        // into the context's boxed fn-table (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_bind_index_buffer)(
                self.command_buffer,
                buffer.buffer,
                offset as VkDeviceSize,
                index_type.as_i32(),
            );
        }
    }

    fn push_graphics_constants(
        &mut self,
        pipeline: &VulkanGraphicsPipeline,
        stage: ShaderStage,
        offset: u32,
        bytes: &[u8],
    ) {
        // The agnostic `ShaderStage` bits equal `VK_SHADER_STAGE_*` (plan D5,
        // asserted in `abi_guard.rs`).
        let stage_flags: VkFlags = stage.bits();
        // SAFETY: recording is open; `pipeline.layout` is the graphics pipeline's own
        // layout (created in `create_graphics_pipeline` with a VERTEX-stage push range
        // at offset 0). The encoder does NOT carry the layout's declared push size, so
        // it cannot statically bound `stage`/`offset`/`bytes` against it — an over-range
        // or wrong-stage push is caught at runtime by the Vulkan validation layer (the
        // GPU-half soundness oracle), not by a debug_assert here (contrast the compute
        // sibling, whose FIXED 4-byte/COMPUTE layout makes a static assert trivial).
        // `bytes.as_ptr()` points to `bytes.len()` bytes alive for the call; `self.fns`
        // points into the context's boxed fn-table (alive per the type contract).
        // SAFETY: `self.fns` points into the owning context's boxed `DeviceFns` — a stable
        // heap address that outlives this encoder (context teardown order); deref is valid.
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_push_constants)(
                self.command_buffer,
                pipeline.layout,
                stage_flags,
                offset,
                bytes.len() as u32,
                bytes.as_ptr().cast::<c_void>(),
            );
        }
    }

    fn push_compute_constants(
        &mut self,
        pipeline: &ComputePipeline,
        stage: ShaderStage,
        offset: u32,
        bytes: &[u8],
    ) {
        // Render P4b: the COMPUTE counterpart of `push_graphics_constants`. Pushes
        // against the passed pipeline's OWN layout — for a vocabulary pipeline
        // (`ComputePipelineDesc::bind_group_layout == Some`) that is the DEDICATED
        // layout its bind group was bound against (`bind_descriptor_set_compute`),
        // which declares a `COMPUTE_PUSH_CONSTANT_RANGE_BYTES` COMPUTE range at offset 0
        // (see `create_compute_pipeline`). The fine marcher pushes a 4-byte
        // `coarse_enabled` gate here.
        let stage_flags: VkFlags = stage.bits();
        debug_assert!(
            stage_flags == crate::ffi::VK_SHADER_STAGE_COMPUTE_BIT,
            "invariant: compute push stage"
        );
        debug_assert!(
            offset as u64 + bytes.len() as u64 <= COMPUTE_PUSH_CONSTANT_RANGE_BYTES as u64,
            "invariant: push range within the pipeline's COMPUTE push range"
        );
        // SAFETY: recording is open with `pipeline` bound (caller contract);
        // `pipeline.layout` is that compute pipeline's own layout, which declares a
        // `COMPUTE_PUSH_CONSTANT_RANGE_BYTES`-byte COMPUTE push range at offset 0
        // (created in `create_compute_pipeline`); `offset`/`bytes.len()` are within that
        // range (asserted above) at the COMPUTE stage; `bytes.as_ptr()` points to
        // `bytes.len()` bytes alive for the call. `self.fns` borrows the live device
        // fn-table.
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_push_constants)(
                self.command_buffer,
                pipeline.layout,
                stage_flags,
                offset,
                bytes.len() as u32,
                bytes.as_ptr().cast::<c_void>(),
            );
        }
    }

    fn draw(
        &mut self,
        vertex_count: u32,
        instance_count: u32,
        first_vertex: u32,
        first_instance: u32,
    ) {
        // A zero `vertex_count`/`instance_count` is a legal Vulkan no-op — a culled or
        // GPU-driven-indirect path may legitimately issue one — so the RHI deliberately
        // permits it rather than asserting non-zero (API-faithful for the future
        // indirect/culled draw rungs).
        // SAFETY: recording is open and inside a `begin_rendering` scope with a bound
        // graphics pipeline + a set dynamic viewport/scissor (caller contract);
        // `vkCmdDraw` issues the non-indexed draw. `self.fns` points into the
        // context's boxed fn-table (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_draw)(
                self.command_buffer,
                vertex_count,
                instance_count,
                first_vertex,
                first_instance,
            );
        }
    }

    fn draw_indexed(
        &mut self,
        index_count: u32,
        instance_count: u32,
        first_index: u32,
        vertex_offset: i32,
        first_instance: u32,
    ) {
        // A zero `index_count`/`instance_count` is a legal Vulkan no-op (as in `draw`)
        // — a culled or GPU-driven-indirect path may legitimately issue one — so the
        // RHI deliberately permits it rather than asserting non-zero.
        // SAFETY: recording is open and inside a `begin_rendering` scope with a bound
        // graphics pipeline + a set dynamic viewport/scissor, a bound index buffer
        // (`bind_index_buffer`) and the vertex buffer(s) the indices reference (caller
        // contract); `vkCmdDrawIndexed` reads `index_count` indices from `first_index`
        // in the bound index buffer, adds `vertex_offset` per index, and issues the
        // indexed draw. `self.fns` points into the context's boxed fn-table (alive per
        // the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_draw_indexed)(
                self.command_buffer,
                index_count,
                instance_count,
                first_index,
                vertex_offset,
                first_instance,
            );
        }
    }

    fn copy_image_to_buffer(
        &mut self,
        src: &VulkanTexture,
        src_layout: ImageLayout,
        dst: &BoundBuffer,
        regions: &[BufferImageCopy],
    ) {
        debug_assert!(
            !regions.is_empty(),
            "invariant: copy_image_to_buffer needs >= 1 region"
        );
        // The basic-slice readback uses a single full-image region; the inline cap
        // avoids any heap allocation on that path. A larger batch (never hit by S0)
        // falls into the cold heap helper, mirroring `pipeline_barrier_many`.
        if regions.len() <= MAX_IMAGE_COPY_REGIONS {
            // Invariant (mirrors `begin_rendering`'s belt-and-suspenders): inside this
            // branch the count is provably `<= MAX_IMAGE_COPY_REGIONS`, so the
            // `inline_regions[..regions.len()]` slice fill is in-bounds and the
            // `regions.len() as u32` region count handed to Vulkan is `<= CAP`. The
            // `> CAP` case is routed to the cold heap helper below, never here — this
            // assert traps any future refactor that loosens the branch condition.
            debug_assert!(regions.len() <= MAX_IMAGE_COPY_REGIONS);
            let mut inline_regions = [DEFAULT_BUFFER_IMAGE_COPY; MAX_IMAGE_COPY_REGIONS];
            for (slot, region) in inline_regions.iter_mut().zip(regions.iter()) {
                *slot = vk_buffer_image_copy(region);
            }
            // SAFETY: recording is open; `src.image` is a live image currently in
            // `src_layout` (the caller transitioned it via `image_barrier`);
            // `dst.buffer` is a live host-visible buffer carrying TRANSFER_DST usage;
            // `inline_regions[..regions.len()]` are fully-initialized `VkBufferImageCopy`s
            // (alive for the call) describing in-bounds sub-rects. `self.fns` points
            // into the context's boxed fn-table (alive per the type contract).
            let fns = unsafe { &*self.fns };
            unsafe {
                (fns.cmd_copy_image_to_buffer)(
                    self.command_buffer,
                    src.image,
                    src_layout.as_i32(),
                    dst.buffer,
                    regions.len() as u32,
                    inline_regions.as_ptr(),
                );
            }
            return;
        }
        self.copy_image_to_buffer_many(src.image, src_layout.as_i32(), dst.buffer, regions);
    }

    fn copy_buffer_to_image(
        &mut self,
        src: &BoundBuffer,
        dst: &VulkanTexture,
        dst_layout: ImageLayout,
        regions: &[BufferImageCopy],
    ) {
        debug_assert!(
            !regions.is_empty(),
            "invariant: copy_buffer_to_image needs >= 1 region"
        );
        // The rung-11 composite upload uses a single full-image region; the inline
        // cap avoids any heap allocation on that path. A larger batch (never hit by
        // S1) falls into the cold heap helper, mirroring `copy_image_to_buffer`.
        if regions.len() <= MAX_IMAGE_COPY_REGIONS {
            // Invariant (mirrors `copy_image_to_buffer`): inside this branch the count
            // is provably `<= MAX_IMAGE_COPY_REGIONS`, so the `inline_regions[..len]`
            // fill is in-bounds and the `len as u32` count handed to Vulkan is `<=
            // CAP`. The `> CAP` case is routed to the cold heap helper below — this
            // assert traps any future refactor that loosens the branch condition.
            debug_assert!(regions.len() <= MAX_IMAGE_COPY_REGIONS);
            let mut inline_regions = [DEFAULT_BUFFER_IMAGE_COPY; MAX_IMAGE_COPY_REGIONS];
            for (slot, region) in inline_regions.iter_mut().zip(regions.iter()) {
                *slot = vk_buffer_image_copy(region);
            }
            // SAFETY: recording is open; `src.buffer` is a live buffer carrying
            // TRANSFER_SRC usage (the host-coherent composite buffer); `dst.image` is
            // a live image currently in `dst_layout` (the caller transitioned it to
            // TRANSFER_DST_OPTIMAL via `image_barrier`);
            // `inline_regions[..regions.len()]` are fully-initialized
            // `VkBufferImageCopy`s (alive for the call) describing in-bounds sub-rects.
            // `self.fns` points into the context's boxed fn-table (alive per the type
            // contract).
            let fns = unsafe { &*self.fns };
            unsafe {
                (fns.cmd_copy_buffer_to_image)(
                    self.command_buffer,
                    src.buffer,
                    dst.image,
                    dst_layout.as_i32(),
                    regions.len() as u32,
                    inline_regions.as_ptr(),
                );
            }
            return;
        }
        self.copy_buffer_to_image_many(src.buffer, dst.image, dst_layout.as_i32(), regions);
    }
}

impl VulkanCommandEncoder {
    /// Records a `vkCmdClearColorImage` over `range` of `texture` (which the caller MUST
    /// have transitioned to `layout`, one of `GENERAL`/`TRANSFER_DST_OPTIMAL`), clearing
    /// every covered texel to `color` (SDFDDGI I1 boot-clear of the probe atlases). A
    /// crate-internal helper (not on the public `RhiCommandEncoder` trait) reaching the
    /// encoder's private `command_buffer`/`fns` the same way [`Self::image_barrier`] does.
    pub(crate) fn clear_color_image(
        &mut self,
        texture: &VulkanTexture,
        layout: ImageLayout,
        color: [f32; 4],
        range: ImageSubresourceRange,
    ) {
        let clear = VkClearColorValue { float32: color };
        let vk_range = VkImageSubresourceRange {
            aspect_mask: range.aspect.bits(),
            base_mip_level: range.base_mip_level,
            level_count: range.level_count,
            base_array_layer: range.base_array_layer,
            layer_count: range.layer_count,
        };
        // SAFETY: recording is open; `texture.image` is a live COLOR image the caller has
        // transitioned to `layout` (TRANSFER_DST_OPTIMAL per its clear boot path);
        // `&clear` + `&vk_range` are fully-initialized locals alive for the call, and
        // `vk_range` names an in-bounds subresource (the caller passes the image's own
        // `0..layer_count`). `self.fns` points into the context's boxed fn-table (alive
        // per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_clear_color_image)(
                self.command_buffer,
                texture.image,
                layout.as_i32(),
                &clear,
                1,
                &vk_range,
            );
        }
    }

    /// Records a `vkCmdFillBuffer` filling all `size` bytes of `buffer` from offset 0 with
    /// the 4-byte `pattern` (SDFDDGI I1 boot-clear of the per-probe classification buffer
    /// to 0 = unconverged). A crate-internal helper reaching the encoder's private
    /// `command_buffer`/`fns` directly, mirroring the gbuffer cull's `cmd_fill_buffer` reset.
    pub(crate) fn fill_buffer(&mut self, buffer: &BoundBuffer, pattern: u32) {
        // SAFETY: recording is open; `buffer.buffer` is a live buffer carrying TRANSFER_DST
        // usage (the classification buffer is created with it); `buffer.size` is its exact
        // byte size (a multiple of 4 — a `u8`-per-probe count rounded to a `u32` word).
        // `self.fns` points into the context's boxed fn-table (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_fill_buffer)(self.command_buffer, buffer.buffer, 0, buffer.size, pattern);
        }
    }

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

    /// The cold multi-region fallback for [`RhiCommandEncoder::copy_image_to_buffer`]:
    /// builds a heap `Vec<VkBufferImageCopy>` and records the copy. The basic-slice
    /// readback uses a single region, so this path (and its only allocation) is kept
    /// off the common path's I-cache via `#[cold] #[inline(never)]`.
    #[cold]
    #[inline(never)]
    fn copy_image_to_buffer_many(
        &mut self,
        src_image: VkImage,
        src_layout: i32,
        dst_buffer: VkBuffer,
        regions: &[BufferImageCopy],
    ) {
        let mut heap_regions: Vec<VkBufferImageCopy> = Vec::with_capacity(regions.len());
        for r in regions {
            heap_regions.push(vk_buffer_image_copy(r));
        }
        // SAFETY: recording is open; `src_image` is a live image in `src_layout`;
        // `dst_buffer` is a live TRANSFER_DST buffer; `heap_regions` holds
        // `regions.len()` fully-initialized `VkBufferImageCopy`s alive for the call.
        // `self.fns` points into the context's boxed fn-table (alive per the type
        // contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_copy_image_to_buffer)(
                self.command_buffer,
                src_image,
                src_layout,
                dst_buffer,
                heap_regions.len() as u32,
                heap_regions.as_ptr(),
            );
        }
    }

    /// The cold multi-region fallback for [`RhiCommandEncoder::copy_buffer_to_image`]:
    /// builds a heap `Vec<VkBufferImageCopy>` and records the copy. The rung-11
    /// composite upload uses a single region, so this path (and its only allocation)
    /// is kept off the common path's I-cache via `#[cold] #[inline(never)]`
    /// (mirrors [`Self::copy_image_to_buffer_many`]).
    #[cold]
    #[inline(never)]
    fn copy_buffer_to_image_many(
        &mut self,
        src_buffer: VkBuffer,
        dst_image: VkImage,
        dst_layout: i32,
        regions: &[BufferImageCopy],
    ) {
        let mut heap_regions: Vec<VkBufferImageCopy> = Vec::with_capacity(regions.len());
        for r in regions {
            heap_regions.push(vk_buffer_image_copy(r));
        }
        // SAFETY: recording is open; `src_buffer` is a live TRANSFER_SRC buffer;
        // `dst_image` is a live image in `dst_layout` (TRANSFER_DST_OPTIMAL);
        // `heap_regions` holds `regions.len()` fully-initialized `VkBufferImageCopy`s
        // alive for the call. `self.fns` points into the context's boxed fn-table
        // (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_copy_buffer_to_image)(
                self.command_buffer,
                src_buffer,
                dst_image,
                dst_layout,
                heap_regions.len() as u32,
                heap_regions.as_ptr(),
            );
        }
    }
}
