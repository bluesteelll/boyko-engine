//! One `VkDeviceMemory` block + the [`SubAllocator`] over it, plus the buffer
//! create / bind / map helpers the Slice-0 round-trip needs.
//!
//! [`HostVisibleBlock`] allocates ONE large host-visible + host-coherent
//! `VkDeviceMemory` allocation (memory-type index selected from
//! `VkPhysicalDeviceMemoryProperties` by required property flags), maps it
//! persistently for the block's lifetime, and hands out aligned byte offsets
//! through the pure-logic [`SubAllocator`]. This is the §4 "free-list with
//! coalescing for long-lived resources" pool in its minimal form — the
//! streaming ring pool is a separate later structure.
//!
//! Mapping is persistent (mapped once at creation, unmapped in `Drop`): a
//! host-coherent allocation needs no flush/invalidate, so a stable CPU pointer
//! plus a sub-allocated offset is a complete CPU-side write/read path.

use core::ffi::c_void;
use core::ptr::{self, NonNull};

use crate::device::DeviceFns;
use crate::ffi::*;
use crate::suballocator::SubAllocator;

/// Errors from device-memory allocation / buffer binding.
#[derive(Debug)]
pub enum MemoryError {
    /// No memory type satisfied the required property flags + type-bits mask.
    NoSuitableMemoryType,
    /// A Vulkan command failed.
    VkError(&'static str, VkResult),
    /// The sub-allocator could not satisfy the request (exhaustion).
    SubAllocExhausted,
}

/// A bound, sub-allocated buffer: the `VkBuffer`, its block offset, and — for a
/// host-visible buffer — the CPU pointer to its first byte inside the
/// persistently-mapped block (`None` for a device-local buffer, which is never
/// mapped, plan D3/MF-8).
///
/// Deliberately **not** `Copy`/`Clone` (plan A5/SEAM-2): destruction is by-value
/// (`destroy_bound_buffer` consumes it) so the move encodes "destroyed exactly
/// once" in the type system. A `Copy`/`Clone` would let a `BoundBuffer` be
/// duplicated and freed twice (double-free of the `VkBuffer` + double-return of
/// the sub-allocation), defeating that guarantee.
///
/// `mapped` is `Option<NonNull<u8>>` (the null-pointer niche makes it the same
/// 8 bytes as the bare `NonNull<u8>` it replaced — layout-neutral): a host-visible
/// `BoundBuffer` carries `Some(ptr)`, a device-local one carries `None`, so
/// `buffer_mapped_ptr` can honor the device.rs:91 "`None` if not host-mappable"
/// contract by returning the field verbatim.
pub struct BoundBuffer {
    /// The Vulkan buffer handle.
    pub buffer: VkBuffer,
    /// The byte offset within the block where the buffer's memory is bound.
    pub offset: u64,
    /// The buffer's requested size in bytes.
    pub size: u64,
    /// CPU pointer to the buffer's first byte (block map base + offset) for a
    /// host-visible buffer; `None` for a device-local (never-mapped) buffer.
    pub mapped: Option<NonNull<u8>>,
}

/// One host-visible + host-coherent `VkDeviceMemory` block with a sub-allocator
/// and a persistent CPU mapping.
///
/// # Address-stability contract (plan A1)
///
/// `fns` is a raw `*const DeviceFns`, **not** a `&'static DeviceFns`. The block is
/// owned by a [`VulkanContext`](crate::device::VulkanContext) whose `device_fns`
/// is heap-boxed: the pointer therefore targets a stable heap address that a
/// context move does not invalidate, and the context tears the block down in its
/// `Drop` BEFORE `vkDestroyDevice` (so the fn-table is alive for every block use).
/// No `'static` lifetime is fabricated — the raw pointer states the real,
/// non-`'static` invariant. The block is `!Send + !Sync` (the raw pointer makes
/// it so) and is touched single-threaded (plan §5.3).
pub struct HostVisibleBlock {
    device: VkDevice,
    /// Raw pointer into the owning context's boxed [`DeviceFns`] (stable address,
    /// outlives the block per the context's reverse-order `Drop`). See the type
    /// docs for the full invariant; not `&'static` — no false lifetime claim.
    fns: *const DeviceFns,
    memory: VkDeviceMemory,
    /// Persistent CPU mapping of `[0, capacity)`.
    map_base: NonNull<u8>,
    capacity: u64,
    suballoc: SubAllocator,
}

impl HostVisibleBlock {
    /// Allocates and persistently maps a `capacity`-byte host-visible +
    /// host-coherent block.
    ///
    /// The memory type is chosen from `mem_props` as the first type carrying
    /// both `HOST_VISIBLE` and `HOST_COHERENT`. `capacity` must be non-zero.
    ///
    /// `fns` is captured as a raw `*const DeviceFns`; the caller must guarantee it
    /// targets a stable address (the context's boxed fn-table) that outlives the
    /// returned block (plan A1).
    pub fn new(
        device: VkDevice,
        fns: &DeviceFns,
        mem_props: &VkPhysicalDeviceMemoryProperties,
        capacity: u64,
    ) -> Result<Self, MemoryError> {
        debug_assert!(capacity > 0, "block capacity must be non-zero");

        let required =
            VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
        // All types are acceptable for a raw allocation not yet tied to a
        // resource (`memory_type_bits = !0`); buffer binding later re-checks the
        // per-buffer `memory_type_bits` against this chosen type.
        let memory_type_index = select_memory_type(mem_props, required, u32::MAX)
            .ok_or(MemoryError::NoSuitableMemoryType)?;

        let alloc_info = VkMemoryAllocateInfo {
            s_type: VkStructureType::MemoryAllocateInfo,
            p_next: ptr::null(),
            allocation_size: capacity,
            memory_type_index,
        };

        let mut memory = VkDeviceMemory::NULL;
        // SAFETY: `device` is a live logical device; `alloc_info` is a
        // fully-initialized `#[repr(C)]` struct; `&mut memory` is a valid
        // out-pointer; NULL allocator selects the default host allocator.
        let raw = unsafe { (fns.allocate_memory)(device, &alloc_info, ptr::null(), &mut memory) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(MemoryError::VkError("vkAllocateMemory", result));
        }

        // Persistently map the whole block.
        let mut mapped_ptr: *mut c_void = ptr::null_mut();
        // SAFETY: `memory` was just allocated from a host-visible type, so it is
        // mappable; offset 0 + `VK_WHOLE_SIZE` maps the entire allocation; the
        // out-pointer `&mut mapped_ptr` receives the CPU address. flags must be
        // 0 (reserved).
        let raw = unsafe {
            (fns.map_memory)(device, memory, 0, VK_WHOLE_SIZE, 0, &mut mapped_ptr)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: `memory` is the just-allocated, not-yet-mapped block;
            // freeing it on this error path matches the single `allocate_memory`
            // above (no double free — we return before storing it).
            unsafe { (fns.free_memory)(device, memory, ptr::null()) };
            return Err(MemoryError::VkError("vkMapMemory", result));
        }
        let map_base = NonNull::new(mapped_ptr as *mut u8).ok_or_else(|| {
            // A success result with a null pointer is a broken driver; free and
            // surface a loud error rather than handing out a null base.
            // SAFETY: `memory` is allocated and (claimed) mapped; freeing is the
            // matching teardown on this error path.
            unsafe { (fns.free_memory)(device, memory, ptr::null()) };
            MemoryError::VkError("vkMapMemory(null base)", VkResult::ERROR_INITIALIZATION_FAILED)
        })?;

        Ok(Self {
            device,
            // Store the borrow as a raw pointer (plan A1): the caller guarantees a
            // stable, block-outliving address (the context's boxed fn-table).
            fns: fns as *const DeviceFns,
            memory,
            map_base,
            capacity,
            suballoc: SubAllocator::new(capacity),
        })
    }

    /// The block's total capacity in bytes.
    #[inline]
    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// The persistent CPU map base of the whole block.
    #[inline]
    pub fn map_base(&self) -> NonNull<u8> {
        self.map_base
    }

    /// Creates a `VkBuffer`, sub-allocates an aligned region of this block that
    /// satisfies its `VkMemoryRequirements`, binds the buffer at that offset,
    /// and returns the [`BoundBuffer`] (handle + offset + mapped pointer).
    ///
    /// `usage` is the `VkBufferUsageFlags` for the buffer (any valid usage is
    /// fine for a host-visible map round-trip).
    pub fn create_bound_buffer(
        &mut self,
        size: u64,
        usage: VkFlags,
    ) -> Result<BoundBuffer, MemoryError> {
        debug_assert!(size > 0, "invariant: zero-size buffer");
        // SAFETY (plan A1): `self.fns` targets the owning context's boxed
        // `DeviceFns` — a stable heap address that outlives this block (the
        // context drops the block before `vkDestroyDevice`). Single-threaded use
        // (`!Send + !Sync`). The borrow is live only for this call.
        let fns = unsafe { &*self.fns };
        let create_info = VkBufferCreateInfo {
            s_type: VkStructureType::BufferCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            size,
            usage,
            sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            p_queue_family_indices: ptr::null(),
        };

        let mut buffer = VkBuffer::NULL;
        // SAFETY: `device` is live; `create_info` is a fully-initialized
        // `#[repr(C)]` struct; `&mut buffer` is a valid out-pointer.
        let raw =
            unsafe { (fns.create_buffer)(self.device, &create_info, ptr::null(), &mut buffer) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(MemoryError::VkError("vkCreateBuffer", result));
        }

        // Query the buffer's memory requirements (size may exceed `size`,
        // alignment is driver-dictated).
        let mut reqs = VkMemoryRequirements { size: 0, alignment: 1, memory_type_bits: 0 };
        // SAFETY: `buffer` was just created on `device`; `&mut reqs` is a valid
        // out-pointer for the `#[repr(C)]` `VkMemoryRequirements`.
        unsafe { (fns.get_buffer_memory_requirements)(self.device, buffer, &mut reqs) };

        // Sub-allocate honoring the driver's alignment + size.
        let align = reqs.alignment.max(1);
        let Some(offset) = self.suballoc.alloc(reqs.size, align) else {
            // SAFETY: `buffer` was created above and is not yet bound; destroying
            // it here is the matching teardown on this error path.
            unsafe { (fns.destroy_buffer)(self.device, buffer, ptr::null()) };
            return Err(MemoryError::SubAllocExhausted);
        };

        // SAFETY: `buffer` is unbound; `memory` is the block's allocation;
        // `offset` is sub-allocated to satisfy `reqs.alignment` and lies within
        // `[0, capacity)` with `reqs.size` bytes free (the sub-allocator
        // guarantees both). vkBindBufferMemory binds it exactly once.
        let raw = unsafe {
            (fns.bind_buffer_memory)(self.device, buffer, self.memory, offset)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            self.suballoc.free(offset);
            // SAFETY: bind failed, the buffer is created-but-unbound; destroy it
            // once on this error path.
            unsafe { (fns.destroy_buffer)(self.device, buffer, ptr::null()) };
            return Err(MemoryError::VkError("vkBindBufferMemory", result));
        }

        // SAFETY: `offset < capacity` and `offset + reqs.size <= capacity` (the
        // sub-allocator guarantees the region lies inside the mapped block), so
        // `map_base + offset` is in-bounds of the persistent mapping.
        let mapped = unsafe { NonNull::new_unchecked(self.map_base.as_ptr().add(offset as usize)) };

        Ok(BoundBuffer { buffer, offset, size, mapped: Some(mapped) })
    }

    /// Destroys a previously-created [`BoundBuffer`] and frees its sub-region.
    ///
    /// # Safety
    ///
    /// `bound` must have been produced by [`Self::create_bound_buffer`] on THIS
    /// block and not already destroyed (its `VkBuffer` is destroyed exactly
    /// once and its offset is returned to the sub-allocator exactly once).
    pub unsafe fn destroy_bound_buffer(&mut self, bound: BoundBuffer) {
        // SAFETY (plan A1): `self.fns` targets the context's boxed `DeviceFns`,
        // alive for this call (see the type docs); single-threaded.
        let fns = unsafe { &*self.fns };
        // SAFETY: by the function contract `bound.buffer` was created on
        // `self.device` and not yet destroyed; `vkDestroyBuffer` releases it
        // exactly once. The mapping stays valid (the buffer's backing is the
        // block, which outlives the buffer).
        unsafe { (fns.destroy_buffer)(self.device, bound.buffer, ptr::null()) };
        // Plan A5: `free` returns whether `offset` named a live allocation. A
        // `false` here means a double-free or an unknown offset (a violated
        // by-value-destroy contract) — trip it in debug. `BoundBuffer` is not
        // `Copy`/`Clone`, so the only way to reach this twice is a contract
        // breach the caller's `unsafe` accepted responsibility for.
        let freed = self.suballoc.free(bound.offset);
        debug_assert!(freed, "invariant: freeing a live sub-allocation");
    }
}

impl Drop for HostVisibleBlock {
    fn drop(&mut self) {
        // SAFETY (plan A1): `self.fns` targets the context's boxed `DeviceFns`.
        // `HostVisibleBlock` is dropped in the context's `Drop` BEFORE
        // `vkDestroyDevice` and before the boxed fn-table is freed, so the
        // pointer is still live here; single-threaded.
        let fns = unsafe { &*self.fns };
        // SAFETY: `memory` is the block's allocation, mapped once at creation;
        // `vkUnmapMemory` then `vkFreeMemory` are its matching teardown, each
        // called exactly once in reverse order. Any buffers bound into the
        // block must already be destroyed by the caller (the `&mut self`
        // enforces single-ownership). NULL allocator matches the allocation's
        // NULL allocator.
        unsafe {
            (fns.unmap_memory)(self.device, self.memory);
            (fns.free_memory)(self.device, self.memory, ptr::null());
        }
    }
}

/// One device-local (VRAM) `VkDeviceMemory` block with a sub-allocator,
/// **never mapped** (plan D3/MF-8).
///
/// Mirrors [`HostVisibleBlock`]'s allocate + sub-allocate + bind discipline and
/// reuses the proven pure-logic [`SubAllocator`], but selects a `DEVICE_LOCAL`
/// (NOT `HOST_VISIBLE`) memory type and **never calls `vkMapMemory`** — so no CPU
/// read/write path to the column ever exists (zero-readback by construction;
/// the only CPU touch is the test readback, which goes through a separate
/// host-visible staging buffer + `vkCmdCopyBuffer`). Buffers created here carry
/// `BoundBuffer.mapped == None`.
///
/// # Address-stability contract (plan A1)
///
/// Identical to [`HostVisibleBlock`]: `fns` is a raw `*const DeviceFns` into the
/// owning context's boxed fn-table (a stable heap address that a context move
/// does not invalidate), and the context tears this block down in its `Drop`
/// BEFORE `vkDestroyDevice`. The block is `!Send + !Sync` and touched
/// single-threaded (plan §5.3).
pub struct DeviceLocalBlock {
    device: VkDevice,
    /// Raw pointer into the owning context's boxed [`DeviceFns`] — see the type
    /// docs for the full invariant; not `&'static`.
    fns: *const DeviceFns,
    memory: VkDeviceMemory,
    capacity: u64,
    suballoc: SubAllocator,
}

impl DeviceLocalBlock {
    /// Allocates a `capacity`-byte device-local block. The memory type is the
    /// first one carrying `DEVICE_LOCAL`; the block is **never mapped**.
    ///
    /// `fns` is captured as a raw `*const DeviceFns`; the caller must guarantee it
    /// targets a stable address (the context's boxed fn-table) that outlives the
    /// returned block (plan A1).
    pub fn new(
        device: VkDevice,
        fns: &DeviceFns,
        mem_props: &VkPhysicalDeviceMemoryProperties,
        capacity: u64,
    ) -> Result<Self, MemoryError> {
        debug_assert!(capacity > 0, "block capacity must be non-zero");

        // Device-local: GPU-fast VRAM, NOT host-visible (so it is never mapped).
        // All types are acceptable for a raw allocation not yet tied to a resource
        // (`memory_type_bits = !0`); per-buffer binding later re-checks the
        // buffer's `memory_type_bits` against this chosen type.
        let memory_type_index =
            select_memory_type(mem_props, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT, u32::MAX)
                .ok_or(MemoryError::NoSuitableMemoryType)?;

        let alloc_info = VkMemoryAllocateInfo {
            s_type: VkStructureType::MemoryAllocateInfo,
            p_next: ptr::null(),
            allocation_size: capacity,
            memory_type_index,
        };

        let mut memory = VkDeviceMemory::NULL;
        // SAFETY: `device` is a live logical device; `alloc_info` is a
        // fully-initialized `#[repr(C)]` struct naming a device-local type;
        // `&mut memory` is a valid out-pointer; NULL allocator. No `vkMapMemory`
        // follows — device-local memory is not host-mappable.
        let raw = unsafe { (fns.allocate_memory)(device, &alloc_info, ptr::null(), &mut memory) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(MemoryError::VkError("vkAllocateMemory(device-local)", result));
        }

        Ok(Self {
            device,
            // Plan A1: store the borrow as a raw pointer into the context's boxed
            // fn-table (a stable, block-outliving address).
            fns: fns as *const DeviceFns,
            memory,
            capacity,
            suballoc: SubAllocator::new(capacity),
        })
    }

    /// The block's total capacity in bytes.
    #[inline]
    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Creates a `VkBuffer`, sub-allocates an aligned region of this block that
    /// satisfies its `VkMemoryRequirements`, binds the buffer at that offset, and
    /// returns the [`BoundBuffer`] with `mapped == None` (device-local memory is
    /// never mapped).
    ///
    /// `usage` is the `VkBufferUsageFlags` for the buffer; the caller adds the
    /// `TRANSFER_SRC`/`TRANSFER_DST` bits the staging copy needs.
    pub fn create_bound_buffer(
        &mut self,
        size: u64,
        usage: VkFlags,
    ) -> Result<BoundBuffer, MemoryError> {
        debug_assert!(size > 0, "invariant: zero-size buffer");
        // SAFETY (plan A1): `self.fns` targets the owning context's boxed
        // `DeviceFns` — a stable heap address that outlives this block (the context
        // drops the block before `vkDestroyDevice`). Single-threaded use
        // (`!Send + !Sync`). The borrow is live only for this call.
        let fns = unsafe { &*self.fns };
        let create_info = VkBufferCreateInfo {
            s_type: VkStructureType::BufferCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            size,
            usage,
            sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            p_queue_family_indices: ptr::null(),
        };

        let mut buffer = VkBuffer::NULL;
        // SAFETY: `device` is live; `create_info` is a fully-initialized
        // `#[repr(C)]` struct; `&mut buffer` is a valid out-pointer.
        let raw =
            unsafe { (fns.create_buffer)(self.device, &create_info, ptr::null(), &mut buffer) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(MemoryError::VkError("vkCreateBuffer(device-local)", result));
        }

        let mut reqs = VkMemoryRequirements { size: 0, alignment: 1, memory_type_bits: 0 };
        // SAFETY: `buffer` was just created on `device`; `&mut reqs` is a valid
        // out-pointer for the `#[repr(C)]` `VkMemoryRequirements`.
        unsafe { (fns.get_buffer_memory_requirements)(self.device, buffer, &mut reqs) };

        let align = reqs.alignment.max(1);
        let Some(offset) = self.suballoc.alloc(reqs.size, align) else {
            // SAFETY: `buffer` was created above and is not yet bound; destroying it
            // here is the matching teardown on this error path.
            unsafe { (fns.destroy_buffer)(self.device, buffer, ptr::null()) };
            return Err(MemoryError::SubAllocExhausted);
        };

        // SAFETY: `buffer` is unbound; `memory` is the block's device-local
        // allocation; `offset` is sub-allocated to satisfy `reqs.alignment` and
        // lies within `[0, capacity)` with `reqs.size` bytes free (the
        // sub-allocator guarantees both). `vkBindBufferMemory` binds it once.
        let raw = unsafe { (fns.bind_buffer_memory)(self.device, buffer, self.memory, offset) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            self.suballoc.free(offset);
            // SAFETY: bind failed, the buffer is created-but-unbound; destroy it
            // once on this error path.
            unsafe { (fns.destroy_buffer)(self.device, buffer, ptr::null()) };
            return Err(MemoryError::VkError("vkBindBufferMemory(device-local)", result));
        }

        // No `mapped` pointer: device-local memory is never mapped (plan D3/MF-8).
        Ok(BoundBuffer { buffer, offset, size, mapped: None })
    }

    /// Destroys a previously-created [`BoundBuffer`] and frees its sub-region.
    ///
    /// # Safety
    ///
    /// `bound` must have been produced by [`Self::create_bound_buffer`] on THIS
    /// block and not already destroyed (its `VkBuffer` is destroyed exactly once
    /// and its offset returned to the sub-allocator exactly once).
    pub unsafe fn destroy_bound_buffer(&mut self, bound: BoundBuffer) {
        // SAFETY (plan A1): `self.fns` targets the context's boxed `DeviceFns`,
        // alive for this call (see the type docs); single-threaded.
        let fns = unsafe { &*self.fns };
        // SAFETY: by the function contract `bound.buffer` was created on
        // `self.device` and not yet destroyed; `vkDestroyBuffer` releases it
        // exactly once.
        unsafe { (fns.destroy_buffer)(self.device, bound.buffer, ptr::null()) };
        // Plan A5: a `false` here means a double-free or unknown offset (a violated
        // by-value-destroy contract) — trip it in debug. `BoundBuffer` is not
        // `Copy`/`Clone`, so reaching this twice is a contract breach the caller's
        // `unsafe` accepted responsibility for.
        let freed = self.suballoc.free(bound.offset);
        debug_assert!(freed, "invariant: freeing a live sub-allocation");
    }
}

impl Drop for DeviceLocalBlock {
    fn drop(&mut self) {
        // SAFETY (plan A1): `self.fns` targets the context's boxed `DeviceFns`.
        // `DeviceLocalBlock` is dropped in the context's `Drop` BEFORE
        // `vkDestroyDevice` and before the boxed fn-table is freed, so the pointer
        // is still live here; single-threaded.
        let fns = unsafe { &*self.fns };
        // SAFETY: `memory` is the block's device-local allocation; it was never
        // mapped (no `vkUnmapMemory` to pair), so `vkFreeMemory` alone is its
        // matching teardown, called exactly once. Any buffers bound into the block
        // must already be destroyed by the caller (the `&mut self` enforces
        // single-ownership). NULL allocator matches the allocation's NULL allocator.
        unsafe {
            (fns.free_memory)(self.device, self.memory, ptr::null());
        }
    }
}

/// Selects the first memory type index whose property flags include all of
/// `required` and whose bit is set in `type_bits` (pass `u32::MAX` to accept
/// any type).
///
/// Standalone + pure so the selection logic is unit-testable without Vulkan.
pub fn select_memory_type(
    mem_props: &VkPhysicalDeviceMemoryProperties,
    required: VkFlags,
    type_bits: u32,
) -> Option<u32> {
    let count = (mem_props.memory_type_count as usize).min(VK_MAX_MEMORY_TYPES);
    for i in 0..count {
        let ty = mem_props.memory_types[i];
        let bit_ok = (type_bits & (1u32 << i)) != 0;
        let flags_ok = (ty.property_flags & required) == required;
        if bit_ok && flags_ok {
            return Some(i as u32);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props_with(types: &[(VkFlags, u32)]) -> VkPhysicalDeviceMemoryProperties {
        let mut p = VkPhysicalDeviceMemoryProperties {
            memory_type_count: types.len() as u32,
            memory_types: [VkMemoryType { property_flags: 0, heap_index: 0 }; VK_MAX_MEMORY_TYPES],
            memory_heap_count: 0,
            memory_heaps: [VkMemoryHeap { size: 0, flags: 0 }; VK_MAX_MEMORY_HEAPS],
        };
        for (i, &(flags, heap)) in types.iter().enumerate() {
            p.memory_types[i] = VkMemoryType { property_flags: flags, heap_index: heap };
        }
        p
    }

    /// M1 — the first type matching all required flags is chosen.
    #[test]
    fn selects_first_matching_type() {
        let p = props_with(&[
            (VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT, 0),
            (
                VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
                1,
            ),
        ]);
        let required =
            VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
        assert_eq!(select_memory_type(&p, required, u32::MAX), Some(1));
    }

    /// M2 — a type missing one required flag is skipped.
    #[test]
    fn skips_partial_match() {
        let p = props_with(&[
            (VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT, 0), // visible but not coherent
            (
                VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
                1,
            ),
        ]);
        let required =
            VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
        assert_eq!(select_memory_type(&p, required, u32::MAX), Some(1));
    }

    /// M3 — `type_bits` masks out otherwise-matching types.
    #[test]
    fn respects_type_bits_mask() {
        let p = props_with(&[
            (
                VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
                0,
            ),
            (
                VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
                1,
            ),
        ]);
        let required =
            VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
        // Mask excludes index 0 → index 1 is chosen.
        assert_eq!(select_memory_type(&p, required, 0b10), Some(1));
        // Mask excludes both → None.
        assert_eq!(select_memory_type(&p, required, 0b0), None);
    }

    /// M4 — no matching type returns None.
    #[test]
    fn no_match_returns_none() {
        let p = props_with(&[(VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT, 0)]);
        let required = VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT;
        assert_eq!(select_memory_type(&p, required, u32::MAX), None);
    }
}
