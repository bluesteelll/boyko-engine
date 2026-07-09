//! `VkSwapchainKHR` wrapper: image/view acquisition, recreation on resize /
//! out-of-date, and the `swapchain_image_for` accessor. Split out of the former
//! monolithic `swapchain.rs` (audit W4).

use core::ptr;

use crate::device::{DeviceFns, SwapchainDeviceFns, VulkanContext};
use crate::ffi::*;

use super::surface::{present_mode_supported, resolve_extent};
use super::{COLOR_SUBRESOURCE_RANGE, Surface, SwapchainError};

// Doc-link scope: `Renderer` is referenced only from this module's doc-comments (the
// swapchain documents that the owning `Renderer` waits the device idle before drop).
#[allow(unused_imports)]
use super::frame_driver::Renderer;

/// A `VkSwapchainKHR` plus its color images, one `VkImageView` per image, and the
/// chosen extent — recreatable on resize / out-of-date.
///
/// Borrows the device + the surface/swapchain command tables; `Drop` destroys the
/// image views then the swapchain (device-idle is the caller's responsibility
/// before drop / recreate — [`Renderer`] waits idle).
pub struct Swapchain<'ctx> {
    pub(crate) device: VkDevice,
    pub(crate) fns: &'ctx DeviceFns,
    pub(crate) swap_fns: &'ctx SwapchainDeviceFns,
    pub(crate) swapchain: VkSwapchainKHR,
    pub(crate) format: i32,
    pub(crate) color_space: i32,
    pub(crate) extent: VkExtent2D,
    /// The swapchain's color images (owned by the swapchain; not destroyed
    /// individually — `vkDestroySwapchainKHR` reclaims them). Retained so the
    /// renderer's image-memory barriers can name the `VkImage` behind each view.
    pub(crate) images: Vec<VkImage>,
    /// One view per swapchain image (parallel to `images`).
    pub(crate) image_views: Vec<VkImageView>,
}

impl<'ctx> Swapchain<'ctx> {
    /// Creates a swapchain over `surface` sized to `width` × `height` (clamped to
    /// the surface caps), FIFO present mode, `COLOR_ATTACHMENT` usage, and one
    /// image view per image.
    pub fn new(
        ctx: &'ctx VulkanContext,
        surface: &Surface<'_>,
        width: u32,
        height: u32,
    ) -> Result<Self, SwapchainError> {
        let swap_fns = ctx.swapchain_fns().ok_or(SwapchainError::NotWindowed)?;
        Self::build(
            ctx.device(),
            ctx.device_fns(),
            swap_fns,
            surface,
            width,
            height,
            VkSwapchainKHR::NULL,
        )
    }

    /// Recreates the swapchain in place for a new `width` × `height` (after a
    /// resize or an out-of-date acquire/present). The caller MUST have made the
    /// device idle first (no image view / swapchain may be in use). The old
    /// swapchain is passed as `old_swapchain` to let the driver retire it, then
    /// the old views + old swapchain are destroyed.
    pub fn recreate(
        &mut self,
        surface: &Surface<'_>,
        width: u32,
        height: u32,
    ) -> Result<(), SwapchainError> {
        let rebuilt = Self::build(
            self.device,
            self.fns,
            self.swap_fns,
            surface,
            width,
            height,
            self.swapchain,
        )?;

        // Destroy the OLD views + swapchain (the device is idle per the contract;
        // the driver retired the old swapchain via `old_swapchain` in `build`).
        // SAFETY: every old view was created on `self.device` in the previous
        // `build` and is not in use (device idle); each is destroyed once. The old
        // swapchain is then destroyed once with the matching destroyer.
        unsafe {
            for &view in &self.image_views {
                (self.fns.destroy_image_view)(self.device, view, ptr::null());
            }
            (self.swap_fns.destroy_swapchain)(self.device, self.swapchain, ptr::null());
        }

        // Adopt the rebuilt swapchain's state. `rebuilt` is wrapped in
        // `ManuallyDrop` so its `Drop` does NOT destroy the objects we are moving
        // into `self` (a double-free); the `Vec`s are moved out via `take` and the
        // scalar handles copied, after which the husk is forgotten.
        let mut rebuilt = core::mem::ManuallyDrop::new(rebuilt);
        self.swapchain = rebuilt.swapchain;
        self.format = rebuilt.format;
        self.color_space = rebuilt.color_space;
        self.extent = rebuilt.extent;
        self.images = core::mem::take(&mut rebuilt.images);
        self.image_views = core::mem::take(&mut rebuilt.image_views);
        Ok(())
    }

    /// The shared construction path for `new` + `recreate`.
    #[allow(clippy::too_many_arguments)]
    fn build(
        device: VkDevice,
        fns: &'ctx DeviceFns,
        swap_fns: &'ctx SwapchainDeviceFns,
        surface: &Surface<'_>,
        width: u32,
        height: u32,
        old_swapchain: VkSwapchainKHR,
    ) -> Result<Self, SwapchainError> {
        let surface_fns = surface.surface_fns;

        // --- Query the surface capabilities. ---
        // SAFETY: `physical_device` + `surface` are live; `&mut caps` is a valid
        // out-pointer for the driver-written `#[repr(C)]` `VkSurfaceCapabilitiesKHR`
        // (ABI size-asserted in ffi.rs).
        let mut caps = VkSurfaceCapabilitiesKhr {
            min_image_count: 0,
            max_image_count: 0,
            current_extent: VkExtent2D::default(),
            min_image_extent: VkExtent2D::default(),
            max_image_extent: VkExtent2D::default(),
            max_image_array_layers: 0,
            supported_transforms: 0,
            current_transform: 0,
            supported_composite_alpha: 0,
            supported_usage_flags: 0,
        };
        let raw = unsafe {
            (surface_fns.get_surface_capabilities)(
                surface.physical_device,
                surface.surface,
                &mut caps,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError(
                "vkGetPhysicalDeviceSurfaceCapabilitiesKHR",
                result,
            ));
        }

        // Resolve the extent: `current_extent.width == u32::MAX` means "pick any"
        // → clamp the requested size to the caps; else use the surface's current
        // extent. A zero extent (minimized) is a defer-rendering signal.
        let extent = resolve_extent(&caps, width, height);
        if extent.width == 0 || extent.height == 0 {
            return Err(SwapchainError::ZeroExtent);
        }

        // Confirm FIFO present mode is advertised. The spec guarantees FIFO is
        // always supported, so this is a belt-and-braces check (and exercises the
        // present-mode query the plan §7 lists) rather than a real choice.
        if !present_mode_supported(surface_fns, surface.physical_device, surface.surface, VK_PRESENT_MODE_FIFO_KHR)? {
            // A spec-impossible corner; surface as a format error rather than
            // silently presenting with an unsupported mode.
            return Err(SwapchainError::NoSuitableFormat);
        }

        // Min image count: caps.min + 1 (so the CPU is never blocked on a single
        // in-use image), clamped to caps.max (0 == unlimited).
        let mut min_image_count = caps.min_image_count + 1;
        if caps.max_image_count > 0 && min_image_count > caps.max_image_count {
            min_image_count = caps.max_image_count;
        }

        let ci = VkSwapchainCreateInfoKhr {
            s_type: VkStructureType::SwapchainCreateInfoKhr,
            p_next: ptr::null(),
            flags: 0,
            surface: surface.surface,
            min_image_count,
            image_format: surface.format,
            image_color_space: surface.color_space,
            image_extent: extent,
            image_array_layers: 1,
            // `COLOR_ATTACHMENT` for the clear (Slice 1) + scene draw (rung 7);
            // `TRANSFER_SRC` so the rung-7 acceptance test can `vkCmdCopyImageToBuffer`
            // ONE rendered swapchain image into a host-visible staging buffer for a
            // golden readback (the on-screen-render proof). `TRANSFER_SRC` is a
            // caps-universal swapchain usage and is never used on the steady present
            // path, only on the test's flagged frame.
            image_usage: VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
            image_sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            p_queue_family_indices: ptr::null(),
            pre_transform: VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR,
            composite_alpha: VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR,
            present_mode: VK_PRESENT_MODE_FIFO_KHR,
            clipped: VK_TRUE,
            old_swapchain,
        };
        let mut swapchain = VkSwapchainKHR::NULL;
        // SAFETY: `device` is the live windowed device with `VK_KHR_swapchain`
        // enabled; `ci` is a fully-initialized `#[repr(C)]` struct naming the live
        // surface + (possibly null) old swapchain; FIFO present mode + the
        // identity transform + opaque composite-alpha are all caps-guaranteed
        // present; `&mut swapchain` is a valid out-pointer; NULL allocator.
        let raw = unsafe { (swap_fns.create_swapchain)(device, &ci, ptr::null(), &mut swapchain) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkCreateSwapchainKHR", result));
        }

        // --- Fetch the swapchain images (count query, then fill). ---
        let mut count: u32 = 0;
        // SAFETY: live device + swapchain; count query with a null array.
        let raw = unsafe { (swap_fns.get_swapchain_images)(device, swapchain, &mut count, ptr::null_mut()) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() && result != VkResult::INCOMPLETE {
            // SAFETY: the swapchain was created above and has no views yet;
            // destroy it once on this error path.
            unsafe { (swap_fns.destroy_swapchain)(device, swapchain, ptr::null()) };
            return Err(SwapchainError::VkError("vkGetSwapchainImagesKHR(count)", result));
        }
        let mut images = vec![VkImage::NULL; count as usize];
        // SAFETY: `images` has exactly `count` slots; the array pointer is valid
        // for `count` writes of the swapchain images the driver owns.
        let raw =
            unsafe { (swap_fns.get_swapchain_images)(device, swapchain, &mut count, images.as_mut_ptr()) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() && result != VkResult::INCOMPLETE {
            unsafe { (swap_fns.destroy_swapchain)(device, swapchain, ptr::null()) };
            return Err(SwapchainError::VkError("vkGetSwapchainImagesKHR(fill)", result));
        }
        images.truncate(count as usize);

        // --- Create one image view per image. ---
        let mut image_views = Vec::with_capacity(images.len());
        for &image in &images {
            let view_ci = VkImageViewCreateInfo {
                s_type: VkStructureType::ImageViewCreateInfo,
                p_next: ptr::null(),
                flags: 0,
                image,
                view_type: VK_IMAGE_VIEW_TYPE_2D,
                format: surface.format,
                components: VkComponentMapping {
                    r: VK_COMPONENT_SWIZZLE_IDENTITY,
                    g: VK_COMPONENT_SWIZZLE_IDENTITY,
                    b: VK_COMPONENT_SWIZZLE_IDENTITY,
                    a: VK_COMPONENT_SWIZZLE_IDENTITY,
                },
                subresource_range: COLOR_SUBRESOURCE_RANGE,
            };
            let mut view = VkImageView::NULL;
            // SAFETY: `device` is live; `view_ci` is fully initialized and names a
            // live swapchain image with the swapchain's format + a single color
            // mip/layer; `&mut view` is a valid out-pointer.
            let raw = unsafe { (fns.create_image_view)(device, &view_ci, ptr::null(), &mut view) };
            let result = VkResult::from_raw(raw);
            if !result.is_success() {
                // Destroy the views created so far + the swapchain, in reverse.
                // SAFETY: each prior view was created on `device` above and is not
                // in use; the swapchain is created-but-now-being-torn-down; each
                // is destroyed exactly once.
                unsafe {
                    for &v in &image_views {
                        (fns.destroy_image_view)(device, v, ptr::null());
                    }
                    (swap_fns.destroy_swapchain)(device, swapchain, ptr::null());
                }
                return Err(SwapchainError::VkError("vkCreateImageView", result));
            }
            image_views.push(view);
        }

        Ok(Self {
            device,
            fns,
            swap_fns,
            swapchain,
            format: surface.format,
            color_space: surface.color_space,
            extent,
            images,
            image_views,
        })
    }

    /// The swapchain extent in pixels.
    #[inline]
    pub fn extent(&self) -> VkExtent2D {
        self.extent
    }

    /// The number of swapchain images / views.
    #[inline]
    pub fn image_count(&self) -> usize {
        self.image_views.len()
    }

    /// The swapchain's color `VkFormat` (the `i32` family). A rung-7 scene
    /// graphics pipeline MUST declare this as its single color-attachment format
    /// (the W2-b format-matching contract — the pipeline's declared format must
    /// equal the `begin_rendering` color attachment's, here the swapchain image).
    #[inline]
    pub fn format(&self) -> i32 {
        self.format
    }
}

impl Drop for Swapchain<'_> {
    fn drop(&mut self) {
        // SAFETY: every view was created on `self.device` in `build` and is not in
        // use (the caller waited the device idle before dropping the renderer →
        // swapchain). Views are destroyed before the swapchain that owns their
        // images, each exactly once.
        unsafe {
            for &view in &self.image_views {
                (self.fns.destroy_image_view)(self.device, view, ptr::null());
            }
            (self.swap_fns.destroy_swapchain)(self.device, self.swapchain, ptr::null());
        }
    }
}


/// The swapchain image handle at `index`. `Swapchain` retains the `VkImage`
/// handles (in `images`) alongside their views; the per-frame barriers operate
/// on the handle returned here. The images are owned by the swapchain object
/// and are not destroyed individually (destroying the swapchain frees them).
#[inline]
pub(crate) fn swapchain_image_for(swapchain: &Swapchain<'_>, index: usize) -> VkImage {
    debug_assert!(index < swapchain.images.len(), "image index out of range");
    swapchain.images[index]
}
