//! `VkSurfaceKHR` wrapper + surface-capability helpers (format / present-mode /
//! extent selection). Split out of the former monolithic `swapchain.rs` (audit W4).

use core::ptr;

use crate::device::{SurfaceInstanceFns, VulkanContext};
use crate::ffi::*;

use super::SwapchainError;

// Doc-link scope: `Swapchain` is referenced only from this module's doc-comments (the
// surface documents that the caller destroys any `Swapchain` built from it first).
#[allow(unused_imports)]
use super::swapchain::Swapchain;

/// A `VkSurfaceKHR` over a Win32 window plus the present queue family + chosen
/// color format selected from the surface's capabilities.
///
/// Borrows the surface/swapchain command tables from the owning [`VulkanContext`]
/// (its `'ctx` lifetime), so a [`Surface`] cannot outlive its context. `Drop`
/// destroys the surface with `vkDestroySurfaceKHR`; the caller destroys any
/// [`Swapchain`] built from it first.
pub struct Surface<'ctx> {
    pub(crate) instance: VkInstance,
    pub(crate) physical_device: VkPhysicalDevice,
    pub(crate) surface: VkSurfaceKHR,
    pub(crate) present_family: u32,
    pub(crate) format: i32,
    pub(crate) color_space: i32,
    pub(crate) surface_fns: &'ctx SurfaceInstanceFns,
}

impl<'ctx> Surface<'ctx> {
    /// Creates a Win32 surface over `hwnd` / `hinstance`, confirms the context's
    /// graphics+compute queue family also supports presentation, and picks a
    /// color format.
    ///
    /// # Safety
    ///
    /// `hwnd` / `hinstance` must be a live Win32 window + its instance (from
    /// [`crate::window::Window::hwnd`] / [`crate::window::Window::hinstance`])
    /// that outlives the returned [`Surface`] — the surface borrows the OS window
    /// and is invalid once the window is destroyed.
    pub unsafe fn new(
        ctx: &'ctx VulkanContext,
        hinstance: *mut core::ffi::c_void,
        hwnd: *mut core::ffi::c_void,
    ) -> Result<Self, SwapchainError> {
        let surface_fns = ctx.surface_fns().ok_or(SwapchainError::NotWindowed)?;

        let ci = VkWin32SurfaceCreateInfoKhr {
            s_type: VkStructureType::Win32SurfaceCreateInfoKhr,
            p_next: ptr::null(),
            flags: 0,
            hinstance,
            hwnd,
        };
        let mut surface = VkSurfaceKHR::NULL;
        // SAFETY: `ctx.instance()` is the live windowed instance with
        // `VK_KHR_win32_surface` enabled; `ci` is a fully-initialized `#[repr(C)]`
        // struct whose `hinstance`/`hwnd` name the caller's live window (its
        // outlives-the-surface contract is this fn's safety precondition); `&mut
        // surface` is a valid out-pointer; NULL allocator.
        let raw = unsafe {
            (surface_fns.create_win32_surface)(ctx.instance(), &ci, ptr::null(), &mut surface)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkCreateWin32SurfaceKHR", result));
        }

        // Confirm the context's queue family can present to this surface.
        let mut supported: VkBool32 = VK_FALSE;
        // SAFETY: `physical_device` + the just-created `surface` are live; the
        // queue family index is the one the device's queue belongs to; `&mut
        // supported` is a valid out-pointer.
        let raw = unsafe {
            (surface_fns.get_surface_support)(
                ctx.physical_device(),
                ctx.queue_family_index(),
                surface,
                &mut supported,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: `surface` was just created and is not yet owned by a
            // swapchain; destroy it once on this error path.
            unsafe { (surface_fns.destroy_surface)(ctx.instance(), surface, ptr::null()) };
            return Err(SwapchainError::VkError(
                "vkGetPhysicalDeviceSurfaceSupportKHR",
                result,
            ));
        }
        if supported == VK_FALSE {
            // SAFETY: as above — the unsupported surface is destroyed once.
            unsafe { (surface_fns.destroy_surface)(ctx.instance(), surface, ptr::null()) };
            return Err(SwapchainError::NoPresentQueue);
        }

        // Pick a present-capable color format.
        let (format, color_space) =
            match pick_surface_format(surface_fns, ctx.physical_device(), surface) {
                Ok(f) => f,
                Err(e) => {
                    // SAFETY: the surface is created-but-unused; destroy it once.
                    unsafe {
                        (surface_fns.destroy_surface)(ctx.instance(), surface, ptr::null())
                    };
                    return Err(e);
                }
            };

        Ok(Self {
            instance: ctx.instance(),
            physical_device: ctx.physical_device(),
            surface,
            present_family: ctx.queue_family_index(),
            format,
            color_space,
            surface_fns,
        })
    }

    /// The raw `VkSurfaceKHR` handle.
    #[inline]
    pub fn handle(&self) -> VkSurfaceKHR {
        self.surface
    }

    /// The chosen swapchain color format (`VkFormat`).
    #[inline]
    pub fn format(&self) -> i32 {
        self.format
    }

    /// The present queue family index (== the context's graphics+compute family).
    #[inline]
    pub fn present_family(&self) -> u32 {
        self.present_family
    }
}

impl Drop for Surface<'_> {
    fn drop(&mut self) {
        // SAFETY: `surface` was created on `instance` in `new`; any swapchain
        // built from it has already been destroyed by the caller (teardown
        // order). `vkDestroySurfaceKHR` releases it exactly once.
        unsafe { (self.surface_fns.destroy_surface)(self.instance, self.surface, ptr::null()) };
    }
}

// ---------------------------------------------------------------------------
// Surface-capability helpers.
// ---------------------------------------------------------------------------

/// Picks a surface format: prefer `B8G8R8A8_UNORM`/`_SRGB` (or RGBA equivalents)
/// in SRGB_NONLINEAR space; else the first advertised format. The
/// `current_extent == u32::MAX` special case (any extent) is `0xFFFFFFFF`; a
/// single advertised `{UNDEFINED, SRGB_NONLINEAR}` means "any format".
pub(crate) fn pick_surface_format(
    fns: &SurfaceInstanceFns,
    physical_device: VkPhysicalDevice,
    surface: VkSurfaceKHR,
) -> Result<(i32, i32), SwapchainError> {
    let mut count: u32 = 0;
    // SAFETY: live device + surface; count query with a null array.
    let raw = unsafe { (fns.get_surface_formats)(physical_device, surface, &mut count, ptr::null_mut()) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() && result != VkResult::INCOMPLETE {
        return Err(SwapchainError::VkError(
            "vkGetPhysicalDeviceSurfaceFormatsKHR(count)",
            result,
        ));
    }
    if count == 0 {
        return Err(SwapchainError::NoSuitableFormat);
    }

    let mut formats = vec![VkSurfaceFormatKhr { format: 0, color_space: 0 }; count as usize];
    // SAFETY: `formats` has exactly `count` slots; the array pointer is valid for
    // `count` writes of the driver-written `#[repr(C)]` `VkSurfaceFormatKHR`.
    let raw =
        unsafe { (fns.get_surface_formats)(physical_device, surface, &mut count, formats.as_mut_ptr()) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() && result != VkResult::INCOMPLETE {
        return Err(SwapchainError::VkError(
            "vkGetPhysicalDeviceSurfaceFormatsKHR(fill)",
            result,
        ));
    }
    formats.truncate(count as usize);

    // The "any format" sentinel: a single entry with format == UNDEFINED.
    if formats.len() == 1 && formats[0].format == VK_FORMAT_UNDEFINED {
        return Ok((VK_FORMAT_B8G8R8A8_UNORM, VK_COLOR_SPACE_SRGB_NONLINEAR_KHR));
    }

    let preferred = [
        VK_FORMAT_B8G8R8A8_UNORM,
        VK_FORMAT_B8G8R8A8_SRGB,
        VK_FORMAT_R8G8B8A8_UNORM,
        VK_FORMAT_R8G8B8A8_SRGB,
    ];
    for &want in &preferred {
        if let Some(f) = formats
            .iter()
            .find(|f| f.format == want && f.color_space == VK_COLOR_SPACE_SRGB_NONLINEAR_KHR)
        {
            return Ok((f.format, f.color_space));
        }
    }
    // Fall back to the first advertised format (always present + valid).
    Ok((formats[0].format, formats[0].color_space))
}

/// Whether `want` (a `VkPresentModeKHR`) is advertised for the surface.
pub(crate) fn present_mode_supported(
    fns: &SurfaceInstanceFns,
    physical_device: VkPhysicalDevice,
    surface: VkSurfaceKHR,
    want: i32,
) -> Result<bool, SwapchainError> {
    let mut count: u32 = 0;
    // SAFETY: live device + surface; count query with a null array.
    let raw =
        unsafe { (fns.get_surface_present_modes)(physical_device, surface, &mut count, ptr::null_mut()) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() && result != VkResult::INCOMPLETE {
        return Err(SwapchainError::VkError(
            "vkGetPhysicalDeviceSurfacePresentModesKHR(count)",
            result,
        ));
    }
    if count == 0 {
        return Ok(false);
    }
    let mut modes = vec![0i32; count as usize];
    // SAFETY: `modes` has exactly `count` slots; the array pointer is valid for
    // `count` writes of the driver-written `VkPresentModeKHR` (an `i32` C enum).
    let raw = unsafe {
        (fns.get_surface_present_modes)(physical_device, surface, &mut count, modes.as_mut_ptr())
    };
    let result = VkResult::from_raw(raw);
    if !result.is_success() && result != VkResult::INCOMPLETE {
        return Err(SwapchainError::VkError(
            "vkGetPhysicalDeviceSurfacePresentModesKHR(fill)",
            result,
        ));
    }
    modes.truncate(count as usize);
    Ok(modes.contains(&want))
}

/// Resolves the swapchain extent from the surface caps + the requested size.
/// `current_extent.width == u32::MAX` means the surface defers to the swapchain
/// extent, so we clamp the request to `[min, max]`; otherwise the surface's
/// current extent is authoritative.
pub(crate) fn resolve_extent(caps: &VkSurfaceCapabilitiesKhr, width: u32, height: u32) -> VkExtent2D {
    if caps.current_extent.width != u32::MAX {
        return caps.current_extent;
    }
    VkExtent2D {
        width: width.clamp(caps.min_image_extent.width, caps.max_image_extent.width),
        height: height.clamp(caps.min_image_extent.height, caps.max_image_extent.height),
    }
}
