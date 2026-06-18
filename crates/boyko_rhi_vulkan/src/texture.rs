//! Phase-6 S0 — the device-local image (`VkImage` + `VkImageView` + a dedicated
//! `VkDeviceMemory` allocation) backing [`RhiDevice::create_texture`](boyko_rhi::RhiDevice::create_texture).
//!
//! A texture is a 2D (rung 1) or 3D (deferred) `OPTIMAL`-tiling color image — or a
//! 2D depth image (rung 4, `DEPTH_STENCIL_ATTACHMENT` usage → DEPTH-aspect view) —
//! bound to its own `DEVICE_LOCAL` allocation, plus one full-subresource `VkImageView`.
//! Unlike a buffer (which sub-allocates from the shared block), each image gets a
//! **dedicated** `vkAllocateMemory` (S0 has a handful of attachments — a dedicated
//! allocation is the simplest sound binding, and `OPTIMAL`-tiling images have
//! their own alignment/`memory_type_bits` that the buffer sub-allocator was not
//! sized for). The image is never CPU-mapped; the only CPU touch is the fenced
//! test readback through a host-visible staging buffer + `vkCmdCopyImageToBuffer`.
//!
//! # Ownership & teardown (mirrors `BoundBuffer`, plan A5/D2)
//!
//! [`VulkanTexture`] is **not** `Copy`/`Clone`: destruction is by-value
//! ([`VulkanContext::destroy_texture`](crate::device::VulkanContext)) so the move
//! encodes "destroyed exactly once". Teardown is reverse creation order: view →
//! image → memory. The originating [`VulkanContext`](crate::device::VulkanContext) must still be alive when the
//! texture is destroyed (the destroy goes through the context's device fn-table).

use core::ptr;

use boyko_rhi::TextureDesc;

use crate::device::DeviceFns;
use crate::error::VulkanError;
use crate::ffi::*;
use crate::memory::select_memory_type;

/// An owned device-local image (color, or depth for a `DEPTH_STENCIL_ATTACHMENT`
/// usage) + its full-subresource view + the dedicated `VkDeviceMemory` it is bound
/// to ([`RhiApi::Texture`](boyko_rhi::RhiApi::Texture)).
///
/// # Safety
///
/// The originating [`VulkanContext`](crate::device::VulkanContext) MUST still be
/// alive when this texture is used (as a barrier/attachment/copy source) or
/// destroyed: each goes through the context's device fn-table. No compile-time
/// `'ctx` tie this phase (plan F1; the structural fix is deferred to Phase 2-3).
pub struct VulkanTexture {
    /// The `VkImage` handle; destroyed by `destroy_texture`. Read by the encoder's
    /// `image_barrier` / `copy_image_to_buffer`.
    pub(crate) image: VkImage,
    /// The full-subresource `VkImageView`; destroyed before the image. Read by the
    /// encoder's `begin_rendering` (the color attachment).
    pub(crate) view: VkImageView,
    /// The dedicated device-local allocation backing the image; freed last.
    pub(crate) memory: VkDeviceMemory,
}

impl VulkanTexture {
    /// Creates a 2D/3D image per `desc` (color, or depth when the usage carries
    /// `DEPTH_STENCIL_ATTACHMENT`), allocates + binds a dedicated device-local
    /// block, and creates one full-subresource view with the matching aspect.
    ///
    /// On any partial failure every object created so far is torn down in reverse
    /// order before the error returns (no leak on the error path).
    ///
    /// # Safety
    ///
    /// `device`/`fns` must be the live device + its command table; `mem_props`
    /// must be that physical device's memory properties.
    pub(crate) unsafe fn create(
        device: VkDevice,
        fns: &DeviceFns,
        mem_props: &VkPhysicalDeviceMemoryProperties,
        desc: &TextureDesc,
    ) -> Result<Self, VulkanError> {
        debug_assert!(
            desc.width > 0 && desc.height > 0 && desc.depth > 0,
            "invariant: texture extent must be non-zero in every dimension"
        );

        let image_type = desc.dimension.as_i32();
        let view_type = match image_type {
            VK_IMAGE_TYPE_3D => VK_IMAGE_VIEW_TYPE_3D,
            // `VK_IMAGE_TYPE_2D` (the rung-1 path) and any other 2D-shaped value.
            _ => VK_IMAGE_VIEW_TYPE_2D,
        };
        let format = desc.format.as_i32();
        // The agnostic `ImageUsage` bits equal the `VK_IMAGE_USAGE_*` bits (identity
        // cast, asserted in `abi_guard.rs`).
        let usage: VkFlags = desc.usage.bits();

        // The view aspect is DEPTH for a depth-stencil-attachment image (rung 4),
        // else COLOR (rungs 1..3 color images + the deferred D3 storage image). A
        // mismatched aspect makes `vkCreateImageView` fault under validation; this
        // routes the single new depth case while leaving the color path byte-identical.
        let is_depth = (usage & VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT) != 0;
        let aspect_mask = if is_depth {
            VK_IMAGE_ASPECT_DEPTH_BIT
        } else {
            VK_IMAGE_ASPECT_COLOR_BIT
        };

        let image_info = VkImageCreateInfo {
            s_type: VkStructureType::ImageCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            image_type,
            format,
            extent: VkExtent3D {
                width: desc.width,
                height: desc.height,
                depth: desc.depth,
            },
            mip_levels: 1,
            array_layers: 1,
            samples: VK_SAMPLE_COUNT_1_BIT,
            tiling: VK_IMAGE_TILING_OPTIMAL,
            usage,
            sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            p_queue_family_indices: ptr::null(),
            initial_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        };

        let mut image = VkImage::NULL;
        // SAFETY: `device` is live; `image_info` is a fully-initialized `#[repr(C)]`
        // struct whose only pointer (`p_queue_family_indices`) is null for count 0;
        // `&mut image` is a valid out-pointer; NULL allocator.
        let raw = unsafe { (fns.create_image)(device, &image_info, ptr::null(), &mut image) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreateImage", result));
        }

        // Dedicated device-local allocation sized to the image's requirements.
        let mut reqs = VkMemoryRequirements {
            size: 0,
            alignment: 1,
            memory_type_bits: 0,
        };
        // SAFETY: `image` was just created on `device`; `&mut reqs` is a valid
        // out-pointer for the `#[repr(C)]` `VkMemoryRequirements`.
        unsafe { (fns.get_image_memory_requirements)(device, image, &mut reqs) };

        let Some(memory_type_index) = select_memory_type(
            mem_props,
            VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
            reqs.memory_type_bits,
        ) else {
            // SAFETY: `image` was created above and is not yet bound; destroy it
            // once on this error path so it never leaks.
            unsafe { (fns.destroy_image)(device, image, ptr::null()) };
            return Err(VulkanError::NoSuitableMemoryType);
        };

        let alloc_info = VkMemoryAllocateInfo {
            s_type: VkStructureType::MemoryAllocateInfo,
            p_next: ptr::null(),
            allocation_size: reqs.size,
            memory_type_index,
        };
        let mut memory = VkDeviceMemory::NULL;
        // SAFETY: `device` is live; `alloc_info` is fully initialized for a
        // device-local type that satisfies the image's `memory_type_bits`;
        // `&mut memory` is a valid out-pointer; NULL allocator.
        let raw = unsafe { (fns.allocate_memory)(device, &alloc_info, ptr::null(), &mut memory) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: `image` is created-but-unbound; destroy it once before the
            // error returns.
            unsafe { (fns.destroy_image)(device, image, ptr::null()) };
            return Err(VulkanError::Vk("vkAllocateMemory(texture)", result));
        }

        // SAFETY: `image` is unbound; `memory` is a fresh dedicated allocation of
        // `reqs.size` bytes of a type in `reqs.memory_type_bits`; binding at
        // offset 0 satisfies the image's alignment. `vkBindImageMemory` binds once.
        let raw = unsafe { (fns.bind_image_memory)(device, image, memory, 0) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: bind failed; free the allocation then destroy the unbound
            // image, each once, in reverse order on this error path.
            unsafe {
                (fns.free_memory)(device, memory, ptr::null());
                (fns.destroy_image)(device, image, ptr::null());
            }
            return Err(VulkanError::Vk("vkBindImageMemory", result));
        }

        // One full-subresource view with the format's aspect (COLOR or DEPTH;
        // mirrors the swapchain image-view path for the color case).
        let view_info = VkImageViewCreateInfo {
            s_type: VkStructureType::ImageViewCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            image,
            view_type,
            format,
            components: VkComponentMapping {
                r: VK_COMPONENT_SWIZZLE_IDENTITY,
                g: VK_COMPONENT_SWIZZLE_IDENTITY,
                b: VK_COMPONENT_SWIZZLE_IDENTITY,
                a: VK_COMPONENT_SWIZZLE_IDENTITY,
            },
            subresource_range: VkImageSubresourceRange {
                aspect_mask,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            },
        };
        let mut view = VkImageView::NULL;
        // SAFETY: `device` is live; `view_info` names the live `image`; `&mut view`
        // is a valid out-pointer; NULL allocator.
        let raw = unsafe { (fns.create_image_view)(device, &view_info, ptr::null(), &mut view) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: the image + its memory are bound; tear them down in reverse
            // order (memory then image), each once, on this error path.
            unsafe {
                (fns.free_memory)(device, memory, ptr::null());
                (fns.destroy_image)(device, image, ptr::null());
            }
            return Err(VulkanError::Vk("vkCreateImageView(texture)", result));
        }

        Ok(Self {
            image,
            view,
            memory,
        })
    }

    /// Tears down the view, image, and dedicated allocation in reverse creation
    /// order, consuming `self`.
    ///
    /// # Safety
    ///
    /// `device`/`fns` must be the live device the texture was created on; no GPU
    /// work referencing the image is in flight (caller fence-waited / `wait_idle`);
    /// it is destroyed exactly once (the by-value `self` enforces the latter).
    pub(crate) unsafe fn destroy(self, device: VkDevice, fns: &DeviceFns) {
        // SAFETY: per the contract `device` is live and nothing references the
        // image; destroy the view, then the image, then free the dedicated
        // allocation — each exactly once in reverse creation order.
        unsafe {
            (fns.destroy_image_view)(device, self.view, ptr::null());
            (fns.destroy_image)(device, self.image, ptr::null());
            (fns.free_memory)(device, self.memory, ptr::null());
        }
    }
}
