//! Slice-1 — Vulkan surface + swapchain + present, rendering a cleared frame via
//! **Vulkan 1.3 dynamic rendering** (no `VkRenderPass` / `VkFramebuffer`).
//!
//! Per `docs/RENDER-PHYSICS-GPU-PLAN.md` §7 (Phase 1-3 on-screen path), this
//! completes the on-screen seam over the raw Win32 [`crate::window::Window`] and
//! the windowed [`crate::device::VulkanContext`]:
//!
//! - [`Surface`] wraps `vkCreateWin32SurfaceKHR` over an `HWND`/`HINSTANCE`,
//!   confirms a present-capable queue family via
//!   `vkGetPhysicalDeviceSurfaceSupportKHR`, and selects a present-capable color
//!   format (preferring `B8G8R8A8_UNORM` / `_SRGB`).
//! - [`Swapchain`] queries surface caps/formats/present-modes, creates a
//!   FIFO-present-mode `VkSwapchainKHR` with `COLOR_ATTACHMENT` images, fetches
//!   the images and a `VkImageView` per image, and recreates itself on resize /
//!   `VK_ERROR_OUT_OF_DATE_KHR` / `VK_SUBOPTIMAL_KHR`.
//! - [`Renderer`] owns the per-frame sync (2 frames in flight) and runs the
//!   acquire → record (barrier → `vkCmdBeginRendering` clear → `vkCmdEndRendering`
//!   → barrier) → submit → present loop, recreating the swapchain when needed.
//!
//! # Soundness oracle (raw FFI → no Miri)
//!
//! Raw driver FFI cannot run under Miri; the oracle (plan §6) is the
//! `VK_LAYER_KHRONOS_validation` messenger asserted to `total() == 0` plus clean
//! reverse-order teardown (no leaked-object validation reports). Every `unsafe`
//! states the invariant that makes it sound (sync ordering, barrier params,
//! handle lifetimes, fence-before-destroy).
//!
//! # Teardown order
//!
//! Reverse of creation, device-idle first: per-frame sync → image views →
//! swapchain → surface → (window, destroyed by the caller after). Each owner's
//! `Drop` handles its own objects; the caller drops `Renderer` → `Swapchain` →
//! `Surface` before the [`crate::window::Window`].

use core::ptr;

use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, Format, ImageUsage, RhiDevice, TextureDesc, TextureDimension,
};

use crate::compute::{
    CoarseMode, DEFAULT_MARCHER_OMEGA, FineMarcherPush, LOCAL_SIZE_X, tile_grid_extent,
};
use crate::device::{DeviceFns, SurfaceInstanceFns, SwapchainDeviceFns, VulkanContext};
use crate::ffi::*;
use crate::memory::BoundBuffer;
use crate::rhi_impl::{
    ComputePipeline, Vulkan, VulkanBindGroup, VulkanBindGroupLayout, VulkanGraphicsPipeline,
    VulkanSampler,
};
use crate::texture::VulkanTexture;

/// The number of frames the [`Renderer`] keeps in flight (double-buffered CPU↔GPU
/// overlap). Per-frame: an acquire semaphore + an in-flight fence; render-finished
/// semaphores are per swapchain image (so a present is never signalled by a
/// semaphore still pending another image's present).
const FRAMES_IN_FLIGHT: usize = 2;

/// Errors from surface / swapchain / present operations.
#[derive(Debug)]
pub enum SwapchainError {
    /// The context was not built windowed ([`crate::device::InstanceConfig::windowed`]
    /// was `false`), so the surface/swapchain command tables are absent.
    NotWindowed,
    /// No queue family supports presentation to this surface.
    NoPresentQueue,
    /// The surface advertised no usable color format.
    NoSuitableFormat,
    /// The surface reported a zero extent (e.g. a minimized window) — defer
    /// rendering until it is non-zero again.
    ZeroExtent,
    /// A Vulkan command returned a non-success `VkResult`.
    VkError(&'static str, VkResult),
    /// The rung-7 scene's per-extent depth image could not be (re)created (resource
    /// creation through the RHI texture path failed).
    DepthImage(crate::error::VulkanError),
}

/// A `VkSurfaceKHR` over a Win32 window plus the present queue family + chosen
/// color format selected from the surface's capabilities.
///
/// Borrows the surface/swapchain command tables from the owning [`VulkanContext`]
/// (its `'ctx` lifetime), so a [`Surface`] cannot outlive its context. `Drop`
/// destroys the surface with `vkDestroySurfaceKHR`; the caller destroys any
/// [`Swapchain`] built from it first.
pub struct Surface<'ctx> {
    instance: VkInstance,
    physical_device: VkPhysicalDevice,
    surface: VkSurfaceKHR,
    present_family: u32,
    format: i32,
    color_space: i32,
    surface_fns: &'ctx SurfaceInstanceFns,
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

/// A `VkSwapchainKHR` plus its color images, one `VkImageView` per image, and the
/// chosen extent — recreatable on resize / out-of-date.
///
/// Borrows the device + the surface/swapchain command tables; `Drop` destroys the
/// image views then the swapchain (device-idle is the caller's responsibility
/// before drop / recreate — [`Renderer`] waits idle).
pub struct Swapchain<'ctx> {
    device: VkDevice,
    fns: &'ctx DeviceFns,
    swap_fns: &'ctx SwapchainDeviceFns,
    swapchain: VkSwapchainKHR,
    format: i32,
    color_space: i32,
    extent: VkExtent2D,
    /// The swapchain's color images (owned by the swapchain; not destroyed
    /// individually — `vkDestroySwapchainKHR` reclaims them). Retained so the
    /// renderer's image-memory barriers can name the `VkImage` behind each view.
    images: Vec<VkImage>,
    /// One view per swapchain image (parallel to `images`).
    image_views: Vec<VkImageView>,
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

/// Per-frame-in-flight CPU↔GPU sync: an acquire semaphore + an in-flight fence.
struct FrameSync {
    /// Signalled by `vkAcquireNextImageKHR`, waited at COLOR_ATTACHMENT_OUTPUT.
    acquire: VkSemaphore,
    /// Signalled by the submit, waited by the CPU before reusing this frame slot.
    in_flight: VkFence,
}

/// Owns the command pool + one command buffer per frame, the per-frame sync, and
/// a render-finished semaphore per swapchain image, and drives the
/// acquire→record→submit→present loop with dynamic rendering + out-of-date
/// recreation.
///
/// Borrows the device tables (`'ctx`); `Drop` waits the device idle then tears
/// down all sync + the command pool in reverse order.
pub struct Renderer<'ctx> {
    device: VkDevice,
    fns: &'ctx DeviceFns,
    swap_fns: &'ctx SwapchainDeviceFns,
    queue: VkQueue,
    command_pool: VkCommandPool,
    /// One command buffer per frame in flight (allocated from `command_pool`,
    /// freed implicitly when the pool is destroyed).
    command_buffers: [VkCommandBuffer; FRAMES_IN_FLIGHT],
    frames: [FrameSync; FRAMES_IN_FLIGHT],
    /// One render-finished semaphore per swapchain image (sized to the swapchain;
    /// rebuilt when the swapchain is recreated).
    render_finished: Vec<VkSemaphore>,
    /// The current frame-in-flight slot (round-robin).
    frame_index: usize,
}

impl<'ctx> Renderer<'ctx> {
    /// Builds the command pool + per-frame command buffers + per-frame sync + one
    /// render-finished semaphore per image of `swapchain`.
    pub fn new(
        ctx: &'ctx VulkanContext,
        surface: &Surface<'_>,
        swapchain: &Swapchain<'_>,
    ) -> Result<Self, SwapchainError> {
        let fns = ctx.device_fns();
        let swap_fns = ctx.swapchain_fns().ok_or(SwapchainError::NotWindowed)?;
        let device = ctx.device();

        // --- Command pool (RESET_COMMAND_BUFFER so each frame can re-record). ---
        let cp_info = VkCommandPoolCreateInfo {
            s_type: VkStructureType::CommandPoolCreateInfo,
            p_next: ptr::null(),
            flags: VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT,
            queue_family_index: surface.present_family,
        };
        let mut command_pool = VkCommandPool::NULL;
        // SAFETY: `device` is live; `cp_info` is fully initialized for the present
        // family; `&mut command_pool` is a valid out-pointer.
        let raw = unsafe { (fns.create_command_pool)(device, &cp_info, ptr::null(), &mut command_pool) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkCreateCommandPool", result));
        }

        // --- One primary command buffer per frame in flight. ---
        let cb_alloc = VkCommandBufferAllocateInfo {
            s_type: VkStructureType::CommandBufferAllocateInfo,
            p_next: ptr::null(),
            command_pool,
            level: VK_COMMAND_BUFFER_LEVEL_PRIMARY,
            command_buffer_count: FRAMES_IN_FLIGHT as u32,
        };
        let mut command_buffers = [VkCommandBuffer::NULL; FRAMES_IN_FLIGHT];
        // SAFETY: `device` is live; `cb_alloc` names the live pool and requests
        // `FRAMES_IN_FLIGHT` buffers; `command_buffers.as_mut_ptr()` is a valid
        // out-pointer for exactly that many primary buffers.
        let raw = unsafe {
            (fns.allocate_command_buffers)(device, &cb_alloc, command_buffers.as_mut_ptr())
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: the pool was created above; destroying it frees the partial
            // buffers and the pool, once.
            unsafe { (fns.destroy_command_pool)(device, command_pool, ptr::null()) };
            return Err(SwapchainError::VkError("vkAllocateCommandBuffers", result));
        }

        // --- Per-frame acquire semaphore + signalled in-flight fence. ---
        // Fences start SIGNALLED so the first frame's wait returns immediately.
        let mut frames: [FrameSync; FRAMES_IN_FLIGHT] = [
            FrameSync { acquire: VkSemaphore::NULL, in_flight: VkFence::NULL },
            FrameSync { acquire: VkSemaphore::NULL, in_flight: VkFence::NULL },
        ];
        for slot in frames.iter_mut() {
            match create_semaphore(fns, device) {
                Ok(s) => slot.acquire = s,
                Err(e) => {
                    // SAFETY: tear down whatever was created so far + the pool.
                    unsafe { destroy_partial_frames(fns, device, &frames, &[]); };
                    unsafe { (fns.destroy_command_pool)(device, command_pool, ptr::null()) };
                    return Err(e);
                }
            }
            match create_fence_signalled(fns, device) {
                Ok(f) => slot.in_flight = f,
                Err(e) => {
                    unsafe { destroy_partial_frames(fns, device, &frames, &[]); };
                    unsafe { (fns.destroy_command_pool)(device, command_pool, ptr::null()) };
                    return Err(e);
                }
            }
        }

        // --- One render-finished semaphore per swapchain image. ---
        let mut render_finished = Vec::with_capacity(swapchain.image_count());
        for _ in 0..swapchain.image_count() {
            match create_semaphore(fns, device) {
                Ok(s) => render_finished.push(s),
                Err(e) => {
                    unsafe { destroy_partial_frames(fns, device, &frames, &render_finished); };
                    unsafe { (fns.destroy_command_pool)(device, command_pool, ptr::null()) };
                    return Err(e);
                }
            }
        }

        Ok(Self {
            device,
            fns,
            swap_fns,
            queue: ctx.queue(),
            command_pool,
            command_buffers,
            frames,
            render_finished,
            frame_index: 0,
        })
    }

    /// The frame-in-flight slot index the NEXT [`present_sampled`](Self::present_sampled)
    /// / [`render_frame`](Self::render_frame) call will use (round-robin in
    /// `0..FRAMES_IN_FLIGHT`).
    ///
    /// The render host reads this to select WHICH per-frame UI ring slot + bind-group
    /// to upload into and re-resolve for the [`UiPass`] (GUI P5a MF-7): the host must
    /// pass this exact index to `RhiContext::ui_upload` / `ui_handles` so the slot it
    /// writes + binds matches the swapchain's in-flight fence this present waits on.
    #[inline]
    pub fn frame_index(&self) -> usize {
        self.frame_index
    }

    /// Blocks until the CURRENT [`frame_index`](Self::frame_index) slot's in-flight
    /// fence is signalled — i.e. the GPU has finished the submit two frames back that
    /// last used this slot's command buffer + per-frame resources.
    ///
    /// # The UI-ring write-after-read hazard this closes (GUI P5a)
    ///
    /// `present_sampled` ALSO waits this fence, but only at its START — AFTER the host
    /// has already memcpy'd this frame's instances into the per-frame UI ring slot.
    /// With `FRAMES_IN_FLIGHT == 2` that ring slot was last READ by the GPU in the
    /// submit two presents ago, whose fence is exactly this slot's in-flight fence; a
    /// host upload before that fence signals is a write-after-read race on a
    /// persistently-mapped, host-coherent buffer the GPU may still be reading.
    ///
    /// The host therefore calls this IMMEDIATELY BEFORE `RhiContext::ui_upload` for the
    /// SAME `frame_index`, so the prior GPU read of that ring slot is complete before
    /// the memcpy. The fence is left SIGNALLED (not reset) — `present_sampled` resets
    /// it itself once it commits to a submit, so this extra wait is a pure no-op for
    /// `present_sampled`'s own discipline (an already-signalled fence wait returns
    /// immediately).
    ///
    /// # Errors
    /// [`SwapchainError::VkError`] if `vkWaitForFences` fails.
    pub fn wait_frame_in_flight(&self) -> Result<(), SwapchainError> {
        let fence = self.frames[self.frame_index].in_flight;
        // SAFETY: `device` is live for `'ctx`; `&fence` names the current frame slot's
        // in-flight fence (created signalled in `new`, kept signalled between presents);
        // an infinite wait blocks until the last submit on this slot completed. The
        // fence is NOT reset here — `present_sampled` owns the reset on its commit path.
        let raw = unsafe {
            (self.fns.wait_for_fences)(self.device, 1, &fence, VK_TRUE, VK_TIMEOUT_INFINITE)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkWaitForFences", result));
        }
        Ok(())
    }

    /// Renders + presents ONE cleared frame in `clear` (RGBA, 0..=1) to
    /// `swapchain`, recreating it on resize / out-of-date / suboptimal.
    ///
    /// Returns `Ok(true)` if the frame presented normally, `Ok(false)` if the
    /// swapchain was (re)created this call and the frame was skipped (the caller
    /// simply tries again next frame). A `ZeroExtent` (minimized window) is also
    /// reported as `Ok(false)`.
    ///
    /// An `Err` return is TERMINAL: drop the `Renderer` (recreate from scratch),
    /// do not call `render_frame` again. A failure *after* the image is acquired
    /// (record / submit error) leaves this frame slot's acquire semaphore
    /// signalled and its in-flight fence unsignalled — reuse would deadlock the
    /// next `vkWaitForFences` on that slot and trip the acquire-semaphore VUID.
    /// The reset placement above only protects the *out-of-date* early return,
    /// which has not yet acquired; it cannot rescue a post-acquire failure. The
    /// `window_clear` example and the integration test both treat `Err` as
    /// terminal.
    ///
    /// `width`/`height` are the window's current client size, used when a
    /// recreate is triggered.
    pub fn render_frame(
        &mut self,
        surface: &Surface<'_>,
        swapchain: &mut Swapchain<'ctx>,
        width: u32,
        height: u32,
        clear: [f32; 4],
    ) -> Result<bool, SwapchainError> {
        let frame = &self.frames[self.frame_index];

        // --- Wait + reset this frame slot's in-flight fence. ---
        // SAFETY: `device` is live; `&frame.in_flight` names this slot's fence;
        // an infinite wait blocks until this slot's previous submit completed, so
        // its command buffer + acquire semaphore are free to reuse.
        let raw = unsafe {
            (self.fns.wait_for_fences)(self.device, 1, &frame.in_flight, VK_TRUE, VK_TIMEOUT_INFINITE)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkWaitForFences", result));
        }

        // --- Acquire the next image (signals this frame's acquire semaphore). ---
        let mut image_index: u32 = 0;
        // SAFETY: `device` + `swapchain` are live; an infinite timeout + this
        // slot's acquire semaphore (and null fence) is the standard acquire; `&mut
        // image_index` is a valid out-pointer.
        let raw = unsafe {
            (self.swap_fns.acquire_next_image)(
                self.device,
                swapchain.swapchain,
                VK_TIMEOUT_INFINITE,
                frame.acquire,
                VkFence::NULL,
                &mut image_index,
            )
        };
        let acquire_result = VkResult::from_raw(raw);
        if acquire_result == VkResult::ERROR_OUT_OF_DATE_KHR {
            self.recreate(surface, swapchain, width, height)?;
            return Ok(false);
        }
        if !acquire_result.is_success() && acquire_result != VkResult::SUBOPTIMAL_KHR {
            return Err(SwapchainError::VkError("vkAcquireNextImageKHR", acquire_result));
        }

        // Only reset the fence once we are committing to a submit (so an
        // out-of-date early return above does not leave the fence unsignalled,
        // which would deadlock the next wait).
        // SAFETY: `device` is live; `&frame.in_flight` names this slot's fence;
        // resetting an unsubmitted (already-waited) fence is valid.
        let raw = unsafe { (self.fns.reset_fences)(self.device, 1, &frame.in_flight) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkResetFences", result));
        }

        let cmd = self.command_buffers[self.frame_index];
        let image = swapchain_image_for(swapchain, image_index as usize);
        let view = swapchain.image_views[image_index as usize];
        let render_finished = self.render_finished[image_index as usize];

        // SAFETY: this slot's fence was just waited, so `cmd` is no longer pending
        // and is recordable (RESET_COMMAND_BUFFER pool); the image/view belong to
        // `swapchain`; `clear` is a finite RGBA; the recorded barriers + dynamic
        // rendering are the UNDEFINED→COLOR→PRESENT clear path.
        unsafe { self.record_clear(cmd, image, view, swapchain.extent, clear)? };

        // --- Submit: wait acquire @ COLOR_ATTACHMENT_OUTPUT, signal render-finished + fence. ---
        let wait_stage: VkFlags = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
        let submit = VkSubmitInfo {
            s_type: VkStructureType::SubmitInfo,
            p_next: ptr::null(),
            wait_semaphore_count: 1,
            p_wait_semaphores: (&frame.acquire as *const VkSemaphore).cast(),
            p_wait_dst_stage_mask: &wait_stage,
            command_buffer_count: 1,
            p_command_buffers: &cmd,
            signal_semaphore_count: 1,
            p_signal_semaphores: (&render_finished as *const VkSemaphore).cast(),
        };
        // SAFETY: `queue` is the live present/graphics queue; one submit naming the
        // recorded `cmd`, waiting this frame's acquire semaphore at
        // COLOR_ATTACHMENT_OUTPUT, signalling this image's render-finished
        // semaphore + this frame's in-flight fence; all referenced locals outlive
        // the call.
        let raw = unsafe { (self.fns.queue_submit)(self.queue, 1, &submit, frame.in_flight) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkQueueSubmit", result));
        }

        // --- Present: wait render-finished. ---
        let present = VkPresentInfoKhr {
            s_type: VkStructureType::PresentInfoKhr,
            p_next: ptr::null(),
            wait_semaphore_count: 1,
            p_wait_semaphores: &render_finished,
            swapchain_count: 1,
            p_swapchains: &swapchain.swapchain,
            p_image_indices: &image_index,
            p_results: ptr::null_mut(),
        };
        // SAFETY: `queue` supports present (confirmed in `Surface::new`); the
        // present-info names the live swapchain + acquired `image_index`, waiting
        // this image's render-finished semaphore; all locals outlive the call.
        let raw = unsafe { (self.swap_fns.queue_present)(self.queue, &present) };
        let present_result = VkResult::from_raw(raw);

        self.frame_index = (self.frame_index + 1) % FRAMES_IN_FLIGHT;

        if present_result == VkResult::ERROR_OUT_OF_DATE_KHR
            || present_result == VkResult::SUBOPTIMAL_KHR
        {
            self.recreate(surface, swapchain, width, height)?;
            return Ok(false);
        }
        if !present_result.is_success() {
            return Err(SwapchainError::VkError("vkQueuePresentKHR", present_result));
        }
        Ok(true)
    }

    /// Records the clear into `cmd`: barrier UNDEFINED→COLOR_ATTACHMENT_OPTIMAL,
    /// `vkCmdBeginRendering` (loadOp=CLEAR), `vkCmdEndRendering`, barrier
    /// COLOR_ATTACHMENT_OPTIMAL→PRESENT_SRC_KHR.
    ///
    /// # Safety
    ///
    /// `cmd` must be recordable (waited free) and `image`/`view` must belong to
    /// the swapchain image being rendered this frame.
    unsafe fn record_clear(
        &self,
        cmd: VkCommandBuffer,
        image: VkImage,
        view: VkImageView,
        extent: VkExtent2D,
        clear: [f32; 4],
    ) -> Result<(), SwapchainError> {
        let begin = VkCommandBufferBeginInfo {
            s_type: VkStructureType::CommandBufferBeginInfo,
            p_next: ptr::null(),
            flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            p_inheritance_info: ptr::null(),
        };
        // SAFETY: `cmd` is recordable per this fn's contract; `begin` is a
        // fully-initialized one-time-submit begin-info.
        let raw = unsafe { (self.fns.begin_command_buffer)(cmd, &begin) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkBeginCommandBuffer", result));
        }

        // Barrier 1: UNDEFINED → COLOR_ATTACHMENT_OPTIMAL (TOP_OF_PIPE → COLOR_OUT).
        let to_color = VkImageMemoryBarrier {
            s_type: VkStructureType::ImageMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: 0,
            dst_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image,
            subresource_range: COLOR_SUBRESOURCE_RANGE,
        };
        // SAFETY: recording is open; one image barrier naming the live `image`;
        // TOP_OF_PIPE→COLOR_ATTACHMENT_OUTPUT with UNDEFINED→COLOR layout is the
        // correct (superset-correct) acquire→render transition; null global/buffer
        // arrays are valid for count 0; `&to_color` outlives the call.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                cmd,
                VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                0,
                0,
                ptr::null(),
                0,
                ptr::null(),
                1,
                (&to_color as *const VkImageMemoryBarrier).cast(),
            );
        }

        // Dynamic rendering: one color attachment, loadOp=CLEAR, storeOp=STORE.
        let attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue {
                color: VkClearColorValue { float32: clear },
            },
        };
        let rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: VkRect2D {
                offset: VkOffset2D { x: 0, y: 0 },
                extent,
            },
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: 1,
            p_color_attachments: &attachment,
            p_depth_attachment: ptr::null(),
            p_stencil_attachment: ptr::null(),
        };
        // SAFETY: recording is open; `rendering` is fully initialized and its
        // single attachment names the live `view` (now in COLOR_ATTACHMENT_OPTIMAL
        // per the barrier above); dynamic rendering is enabled on the device
        // (`dynamicRendering` feature). Begin/End bracket the clear exactly.
        unsafe {
            (self.fns.cmd_begin_rendering)(cmd, &rendering);
            (self.fns.cmd_end_rendering)(cmd);
        }

        // Barrier 2: COLOR_ATTACHMENT_OPTIMAL → PRESENT_SRC_KHR (COLOR_OUT → BOTTOM).
        let to_present = VkImageMemoryBarrier {
            s_type: VkStructureType::ImageMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            dst_access_mask: 0,
            old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            new_layout: VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image,
            subresource_range: COLOR_SUBRESOURCE_RANGE,
        };
        // SAFETY: recording is open; the COLOR_ATTACHMENT_OUTPUT→BOTTOM_OF_PIPE
        // barrier with COLOR→PRESENT layout makes the clear's writes visible to
        // the presentation engine; `&to_present` outlives the call.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                cmd,
                VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
                0,
                0,
                ptr::null(),
                0,
                ptr::null(),
                1,
                (&to_present as *const VkImageMemoryBarrier).cast(),
            );
        }

        // SAFETY: recording is open; ending it matches the `begin` above.
        let raw = unsafe { (self.fns.end_command_buffer)(cmd) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkEndCommandBuffer", result));
        }
        Ok(())
    }

    /// Renders + presents ONE depth-tested SCENE frame (Phase-6 S1 rung 7 — the
    /// first real 3D geometry ON SCREEN) into `swapchain`, recreating it on resize /
    /// out-of-date / suboptimal.
    ///
    /// Unlike [`render_frame`](Self::render_frame) (which only clears), this binds
    /// `scene`'s graphics pipeline + vertex buffer + MVP push constant and draws a
    /// depth-tested mesh into the swapchain image against `scene`'s depth attachment
    /// (recreated to match the swapchain extent on resize, see [`Scene`]`::sync_depth`).
    ///
    /// `clear` is the background color the draw composites over.
    ///
    /// If `readback` is `Some`, on THIS frame — after the draw, before present — the
    /// rendered swapchain image is `vkCmdCopyImageToBuffer`'d into the supplied
    /// host-visible staging buffer (transitioning COLOR → TRANSFER_SRC → PRESENT
    /// instead of COLOR → PRESENT). This is the rung-7 acceptance test's golden
    /// readback path (proving real geometry reached the swapchain image, not just a
    /// clear); the steady present path passes `None` and pays nothing for it.
    ///
    /// Return / error semantics are identical to [`render_frame`](Self::render_frame):
    /// `Ok(true)` presented, `Ok(false)` swapchain (re)created this call, `Err`
    /// terminal.
    ///
    /// # Safety
    ///
    /// `scene`'s pipeline / vertex buffer were created on the same device as this
    /// renderer and outlive the call; `scene.depth` has been synced to `swapchain`'s
    /// current extent (the call does this via [`Scene`]`::sync_depth` when needed). A
    /// `Some(readback)` buffer must be a host-visible buffer of at least
    /// `extent.width * extent.height * 4` bytes (R8G8B8A8/B8G8R8A8 is 4 B/texel).
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn render_scene_frame(
        &mut self,
        ctx: &VulkanContext,
        surface: &Surface<'_>,
        swapchain: &mut Swapchain<'ctx>,
        scene: &mut Scene,
        width: u32,
        height: u32,
        clear: [f32; 4],
        readback: Option<&BoundBuffer>,
    ) -> Result<bool, SwapchainError> {
        let frame = &self.frames[self.frame_index];

        // --- Wait + (later) reset this frame slot's in-flight fence. ---
        // SAFETY: `device` is live; `&frame.in_flight` names this slot's fence; an
        // infinite wait blocks until this slot's previous submit completed, so its
        // command buffer + acquire semaphore (and the depth image it used) are free.
        let raw = unsafe {
            (self.fns.wait_for_fences)(self.device, 1, &frame.in_flight, VK_TRUE, VK_TIMEOUT_INFINITE)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkWaitForFences", result));
        }

        // Ensure the depth image matches the current swapchain extent. The fence
        // wait above guarantees no in-flight frame still references the old depth
        // image, so recreating it here is safe. (The first call creates it.)
        scene.sync_depth(ctx, swapchain.extent)?;

        // --- Acquire the next image (signals this frame's acquire semaphore). ---
        let mut image_index: u32 = 0;
        // SAFETY: `device` + `swapchain` are live; an infinite timeout + this slot's
        // acquire semaphore (null fence) is the standard acquire; `&mut image_index`
        // is a valid out-pointer.
        let raw = unsafe {
            (self.swap_fns.acquire_next_image)(
                self.device,
                swapchain.swapchain,
                VK_TIMEOUT_INFINITE,
                frame.acquire,
                VkFence::NULL,
                &mut image_index,
            )
        };
        let acquire_result = VkResult::from_raw(raw);
        if acquire_result == VkResult::ERROR_OUT_OF_DATE_KHR {
            self.recreate(surface, swapchain, width, height)?;
            return Ok(false);
        }
        if !acquire_result.is_success() && acquire_result != VkResult::SUBOPTIMAL_KHR {
            return Err(SwapchainError::VkError("vkAcquireNextImageKHR", acquire_result));
        }

        // Only reset the fence once we are committing to a submit (mirrors the
        // `render_frame` out-of-date discipline so an early return never leaves the
        // fence unsignalled).
        // SAFETY: `device` is live; `&frame.in_flight` names this slot's
        // already-waited fence; resetting it is valid.
        let raw = unsafe { (self.fns.reset_fences)(self.device, 1, &frame.in_flight) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkResetFences", result));
        }

        let cmd = self.command_buffers[self.frame_index];
        let image = swapchain_image_for(swapchain, image_index as usize);
        let view = swapchain.image_views[image_index as usize];
        let render_finished = self.render_finished[image_index as usize];

        // SAFETY: this slot's fence was just waited so `cmd` is recordable; the
        // image/view belong to `swapchain`; `scene` was created on this device and
        // its depth is synced to `swapchain.extent`; the recorded path is the
        // UNDEFINED→COLOR(+DEPTH)→draw→PRESENT (or →TRANSFER_SRC→readback→PRESENT)
        // scene path.
        unsafe {
            self.record_scene(
                cmd,
                image,
                view,
                swapchain.extent,
                clear,
                scene,
                readback,
            )?
        };

        // --- Submit: wait acquire @ COLOR_ATTACHMENT_OUTPUT, signal render-finished + fence. ---
        let wait_stage: VkFlags = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
        let submit = VkSubmitInfo {
            s_type: VkStructureType::SubmitInfo,
            p_next: ptr::null(),
            wait_semaphore_count: 1,
            p_wait_semaphores: (&frame.acquire as *const VkSemaphore).cast(),
            p_wait_dst_stage_mask: &wait_stage,
            command_buffer_count: 1,
            p_command_buffers: &cmd,
            signal_semaphore_count: 1,
            p_signal_semaphores: (&render_finished as *const VkSemaphore).cast(),
        };
        // SAFETY: `queue` is the live present/graphics queue; one submit naming the
        // recorded `cmd`, waiting this frame's acquire semaphore at
        // COLOR_ATTACHMENT_OUTPUT, signalling this image's render-finished semaphore
        // + this frame's in-flight fence; all referenced locals outlive the call.
        let raw = unsafe { (self.fns.queue_submit)(self.queue, 1, &submit, frame.in_flight) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkQueueSubmit", result));
        }

        // --- Present: wait render-finished. ---
        let present = VkPresentInfoKhr {
            s_type: VkStructureType::PresentInfoKhr,
            p_next: ptr::null(),
            wait_semaphore_count: 1,
            p_wait_semaphores: &render_finished,
            swapchain_count: 1,
            p_swapchains: &swapchain.swapchain,
            p_image_indices: &image_index,
            p_results: ptr::null_mut(),
        };
        // SAFETY: `queue` supports present (confirmed in `Surface::new`); the
        // present-info names the live swapchain + acquired `image_index`, waiting
        // this image's render-finished semaphore; all locals outlive the call.
        let raw = unsafe { (self.swap_fns.queue_present)(self.queue, &present) };
        let present_result = VkResult::from_raw(raw);

        self.frame_index = (self.frame_index + 1) % FRAMES_IN_FLIGHT;

        if present_result == VkResult::ERROR_OUT_OF_DATE_KHR
            || present_result == VkResult::SUBOPTIMAL_KHR
        {
            self.recreate(surface, swapchain, width, height)?;
            return Ok(false);
        }
        if !present_result.is_success() {
            return Err(SwapchainError::VkError("vkQueuePresentKHR", present_result));
        }
        Ok(true)
    }

    /// Records the rung-7 scene into `cmd`: barriers (color UNDEFINED→COLOR + depth
    /// UNDEFINED→DEPTH), `vkCmdBeginRendering` (color CLEAR + depth CLEAR to the far
    /// plane), bind the pipeline + push the MVP + bind the vertex buffer + dynamic
    /// viewport/scissor, `vkCmdDraw`, `vkCmdEndRendering`, then either color
    /// COLOR→PRESENT (steady path) or color COLOR→TRANSFER_SRC, copy-to-buffer,
    /// COLOR (TRANSFER_SRC)→PRESENT (the test readback path).
    ///
    /// # Safety
    ///
    /// `cmd` must be recordable (waited free); `image`/`view` must belong to the
    /// swapchain image rendered this frame; `scene.depth` must be `Some` and sized to
    /// `extent` (the caller syncs it); `scene`'s pipeline / vertex buffer are live on
    /// this device; a `Some(readback)` buffer is host-visible and ≥ the image's byte
    /// size.
    #[allow(clippy::too_many_arguments)]
    unsafe fn record_scene(
        &self,
        cmd: VkCommandBuffer,
        image: VkImage,
        view: VkImageView,
        extent: VkExtent2D,
        clear: [f32; 4],
        scene: &Scene,
        readback: Option<&BoundBuffer>,
    ) -> Result<(), SwapchainError> {
        let depth = scene
            .depth
            .as_ref()
            .expect("invariant: Scene::sync_depth made the depth image present before record");

        let begin = VkCommandBufferBeginInfo {
            s_type: VkStructureType::CommandBufferBeginInfo,
            p_next: ptr::null(),
            flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            p_inheritance_info: ptr::null(),
        };
        // SAFETY: `cmd` is recordable per this fn's contract; `begin` is a
        // fully-initialized one-time-submit begin-info.
        let raw = unsafe { (self.fns.begin_command_buffer)(cmd, &begin) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkBeginCommandBuffer", result));
        }

        // Barrier (color): UNDEFINED → COLOR_ATTACHMENT_OPTIMAL.
        let to_color = VkImageMemoryBarrier {
            s_type: VkStructureType::ImageMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: 0,
            dst_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image,
            subresource_range: COLOR_SUBRESOURCE_RANGE,
        };
        // SAFETY: recording is open; one image barrier on the live `image`;
        // TOP_OF_PIPE→COLOR_ATTACHMENT_OUTPUT with UNDEFINED→COLOR is the
        // superset-correct acquire→render transition; `&to_color` outlives the call.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                cmd,
                VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                0,
                0,
                ptr::null(),
                0,
                ptr::null(),
                1,
                (&to_color as *const VkImageMemoryBarrier).cast(),
            );
        }

        // Barrier (depth): UNDEFINED → DEPTH_ATTACHMENT_OPTIMAL at the
        // early/late-fragment-test stage (the depth-write access, DEPTH aspect).
        let to_depth = VkImageMemoryBarrier {
            s_type: VkStructureType::ImageMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: 0,
            dst_access_mask: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
            old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image: depth.texture.image,
            subresource_range: DEPTH_SUBRESOURCE_RANGE,
        };
        // SAFETY: recording is open; one image barrier on the live depth image;
        // TOP_OF_PIPE→(EARLY|LATE)_FRAGMENT_TESTS with UNDEFINED→DEPTH is the
        // superset-correct first depth transition; `&to_depth` outlives the call.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                cmd,
                VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT
                    | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
                0,
                0,
                ptr::null(),
                0,
                ptr::null(),
                1,
                (&to_depth as *const VkImageMemoryBarrier).cast(),
            );
        }

        // Dynamic rendering: one color attachment (CLEAR/STORE) + the depth
        // attachment (CLEAR to the far plane 1.0 / STORE). The pipeline's declared
        // color format equals the swapchain format and its depth format equals the
        // depth image's (the W2-b contract is upheld at `Scene::new` / `sync_depth`).
        let color_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue {
                color: VkClearColorValue { float32: clear },
            },
        };
        let depth_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: depth.texture.view,
            image_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue {
                depth_stencil: VkClearDepthStencilValue {
                    depth: 1.0,
                    stencil: 0,
                },
            },
        };
        let rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: VkRect2D {
                offset: VkOffset2D { x: 0, y: 0 },
                extent,
            },
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: 1,
            p_color_attachments: &color_attachment,
            p_depth_attachment: (&depth_attachment as *const VkRenderingAttachmentInfo).cast(),
            p_stencil_attachment: ptr::null(),
        };
        let viewport = VkViewport {
            x: 0.0,
            y: 0.0,
            width: extent.width as f32,
            height: extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let scissor = VkRect2D {
            offset: VkOffset2D { x: 0, y: 0 },
            extent,
        };
        let vertex_offset: VkDeviceSize = 0;
        // SAFETY: recording is open; `rendering` is fully initialized — its color
        // attachment names the live `view` (now COLOR_ATTACHMENT_OPTIMAL) and its
        // depth attachment names the live depth view (now DEPTH_ATTACHMENT_OPTIMAL);
        // dynamic rendering is enabled on this device. The pipeline + push range +
        // vertex buffer all belong to this device (caller contract) and the
        // pipeline's declared formats equal the bound attachments' (W2-b). The MVP
        // push is `MVP_BYTES` bytes at offset 0 into the pipeline's VERTEX range;
        // `vertex_offset`/`viewport`/`scissor` locals outlive the bracketed calls;
        // `draw(3, 1, 0, 0)` reads the three bound vertices. Begin/End bracket the
        // scene exactly.
        unsafe {
            (self.fns.cmd_begin_rendering)(cmd, &rendering);
            (self.fns.cmd_bind_pipeline)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                scene.pipeline.pipeline,
            );
            (self.fns.cmd_push_constants)(
                cmd,
                scene.pipeline.layout,
                VK_SHADER_STAGE_VERTEX_BIT,
                0,
                scene.mvp.len() as u32,
                scene.mvp.as_ptr().cast(),
            );
            (self.fns.cmd_bind_vertex_buffers)(
                cmd,
                0,
                1,
                &scene.vertex_buffer.buffer,
                &vertex_offset,
            );
            (self.fns.cmd_set_viewport)(cmd, 0, 1, &viewport);
            (self.fns.cmd_set_scissor)(cmd, 0, 1, &scissor);
            (self.fns.cmd_draw)(cmd, scene.vertex_count, 1, 0, 0);
            (self.fns.cmd_end_rendering)(cmd);
        }

        // The post-draw color transition depends on whether a readback is requested.
        match readback {
            // Steady present path: COLOR_ATTACHMENT_OPTIMAL → PRESENT_SRC_KHR.
            None => {
                let to_present = VkImageMemoryBarrier {
                    s_type: VkStructureType::ImageMemoryBarrier,
                    p_next: ptr::null(),
                    src_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                    dst_access_mask: 0,
                    old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    new_layout: VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
                    src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    image,
                    subresource_range: COLOR_SUBRESOURCE_RANGE,
                };
                // SAFETY: recording is open; COLOR_ATTACHMENT_OUTPUT→BOTTOM_OF_PIPE
                // with COLOR→PRESENT makes the draw's writes visible to the present
                // engine; `&to_present` outlives the call.
                unsafe {
                    (self.fns.cmd_pipeline_barrier)(
                        cmd,
                        VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                        VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
                        0,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        1,
                        (&to_present as *const VkImageMemoryBarrier).cast(),
                    );
                }
            }
            // Test readback path: COLOR → TRANSFER_SRC, copy image → buffer, then
            // TRANSFER_SRC → PRESENT (the image is still presented after the copy).
            Some(staging) => {
                let to_transfer = VkImageMemoryBarrier {
                    s_type: VkStructureType::ImageMemoryBarrier,
                    p_next: ptr::null(),
                    src_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                    dst_access_mask: VK_ACCESS_TRANSFER_READ_BIT,
                    old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    new_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    image,
                    subresource_range: COLOR_SUBRESOURCE_RANGE,
                };
                // SAFETY: recording is open; COLOR_ATTACHMENT_OUTPUT→TRANSFER with
                // COLOR→TRANSFER_SRC makes the draw's writes available to the copy;
                // `&to_transfer` outlives the call.
                unsafe {
                    (self.fns.cmd_pipeline_barrier)(
                        cmd,
                        VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                        VK_PIPELINE_STAGE_TRANSFER_BIT,
                        0,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        1,
                        (&to_transfer as *const VkImageMemoryBarrier).cast(),
                    );
                }

                let region = VkBufferImageCopy {
                    buffer_offset: 0,
                    buffer_row_length: 0,
                    buffer_image_height: 0,
                    image_subresource: VkImageSubresourceLayers {
                        aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    },
                    image_offset: VkOffset3D { x: 0, y: 0, z: 0 },
                    image_extent: VkExtent3D {
                        width: extent.width,
                        height: extent.height,
                        depth: 1,
                    },
                };
                // SAFETY: recording is open; the image is TRANSFER_SRC_OPTIMAL per the
                // barrier above; one full-image tightly-packed color region copies
                // into the live host-visible `staging.buffer` (≥ the image's byte size
                // per this fn's contract); `&region` outlives the call.
                unsafe {
                    (self.fns.cmd_copy_image_to_buffer)(
                        cmd,
                        image,
                        VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                        staging.buffer,
                        1,
                        &region,
                    );
                }

                let to_present = VkImageMemoryBarrier {
                    s_type: VkStructureType::ImageMemoryBarrier,
                    p_next: ptr::null(),
                    src_access_mask: VK_ACCESS_TRANSFER_READ_BIT,
                    dst_access_mask: 0,
                    old_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    new_layout: VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
                    src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    image,
                    subresource_range: COLOR_SUBRESOURCE_RANGE,
                };
                // SAFETY: recording is open; TRANSFER→BOTTOM_OF_PIPE with
                // TRANSFER_SRC→PRESENT releases the image to the present engine after
                // the readback copy; `&to_present` outlives the call.
                unsafe {
                    (self.fns.cmd_pipeline_barrier)(
                        cmd,
                        VK_PIPELINE_STAGE_TRANSFER_BIT,
                        VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
                        0,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        1,
                        (&to_present as *const VkImageMemoryBarrier).cast(),
                    );
                }
            }
        }

        // SAFETY: recording is open; ending it matches the `begin` above.
        let raw = unsafe { (self.fns.end_command_buffer)(cmd) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkEndCommandBuffer", result));
        }
        Ok(())
    }

    /// Presents the rung-11 SDF/mesh HYBRID COMPOSITE to the swapchain — the FIRST
    /// HYBRID FRAME ON SCREEN. The compute composite has already been uploaded into
    /// `composite.texture` (a SAMPLED `R8G8B8A8_UNORM` image left in
    /// `SHADER_READ_ONLY_OPTIMAL` by the caller's pre-loop one-time submit); this call
    /// only SAMPLES that resident texture in a fullscreen-sample graphics pass writing
    /// into the acquired swapchain image, so the GPU converts RGBA → the swapchain's
    /// format on the attachment write and the on-screen colors are correct on any
    /// swapchain format. There is no per-frame upload or per-frame transition of the
    /// composite texture — it is a pure read.
    ///
    /// The composite is presented at its NATIVE size
    /// ([`SampledComposite::texture_extent`]) in the TOP-LEFT of the swapchain image —
    /// the present pass's viewport/scissor are clamped to
    /// `min(swapchain_extent, texture_extent)`, so the composite maps 1:1 and is never
    /// stretched to a (possibly WSI-clamped) wider swapchain extent; the rest of the
    /// swapchain image stays `clear`. A scale-to-fill mode is a future addition.
    ///
    /// Because the composite texture is uploaded once and only ever read here, ALL
    /// frames-in-flight may sample it concurrently with no write-after-read hazard and
    /// no cross-frame fence/barrier on the texture (the per-frame sync below covers
    /// only the per-frame swapchain image + command buffer, exactly as the other
    /// present paths).
    ///
    /// Synchronization / recreate semantics are IDENTICAL to
    /// [`render_scene_frame`](Self::render_scene_frame): `Ok(true)` presented,
    /// `Ok(false)` swapchain (re)created this call (frame skipped), `Err` terminal.
    ///
    /// If `readback` is `Some`, on THIS frame — after the fullscreen draw, before
    /// present — the presented swapchain image is `vkCmdCopyImageToBuffer`'d into the
    /// supplied host-visible staging buffer (the rung-11 golden path, proving the
    /// hybrid composite reached the swapchain image); the steady path passes `None`.
    ///
    /// # Safety
    ///
    /// Every resource borrowed by `composite` (texture / sampler / bind group /
    /// fullscreen pipeline) was created on the same device as this renderer and
    /// outlives the call; `composite.texture` is a SAMPLED image the caller has
    /// already uploaded the composite into and transitioned to
    /// `SHADER_READ_ONLY_OPTIMAL` (and never writes again); `composite.pipeline`'s
    /// `color_formats[0]` equals the swapchain surface format (W2-b) and its layout
    /// declares `composite.bind_group`'s set-0 layout; a `Some(readback)` buffer is
    /// host-visible and at least `extent.width * extent.height * 4` bytes (the
    /// swapchain image's size).
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn present_sampled(
        &mut self,
        surface: &Surface<'_>,
        swapchain: &mut Swapchain<'ctx>,
        composite: &SampledComposite<'_>,
        width: u32,
        height: u32,
        clear: [f32; 4],
        readback: Option<&BoundBuffer>,
        ui: Option<&UiPass<'_>>,
    ) -> Result<bool, SwapchainError> {
        let frame = &self.frames[self.frame_index];

        // --- Wait + (later) reset this frame slot's in-flight fence. ---
        // SAFETY: `device` is live; `&frame.in_flight` names this slot's fence; an
        // infinite wait blocks until this slot's previous submit completed, so its
        // command buffer + acquire semaphore (and the composite texture it sampled)
        // are free to reuse.
        let raw = unsafe {
            (self.fns.wait_for_fences)(self.device, 1, &frame.in_flight, VK_TRUE, VK_TIMEOUT_INFINITE)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkWaitForFences", result));
        }

        // --- Acquire the next image (signals this frame's acquire semaphore). ---
        let mut image_index: u32 = 0;
        // SAFETY: `device` + `swapchain` are live; an infinite timeout + this slot's
        // acquire semaphore (null fence) is the standard acquire; `&mut image_index`
        // is a valid out-pointer.
        let raw = unsafe {
            (self.swap_fns.acquire_next_image)(
                self.device,
                swapchain.swapchain,
                VK_TIMEOUT_INFINITE,
                frame.acquire,
                VkFence::NULL,
                &mut image_index,
            )
        };
        let acquire_result = VkResult::from_raw(raw);
        if acquire_result == VkResult::ERROR_OUT_OF_DATE_KHR {
            self.recreate(surface, swapchain, width, height)?;
            return Ok(false);
        }
        if !acquire_result.is_success() && acquire_result != VkResult::SUBOPTIMAL_KHR {
            return Err(SwapchainError::VkError("vkAcquireNextImageKHR", acquire_result));
        }

        // Only reset the fence once we are committing to a submit (mirrors the
        // `render_scene_frame` out-of-date discipline so an early return never leaves
        // the fence unsignalled).
        // SAFETY: `device` is live; `&frame.in_flight` names this slot's
        // already-waited fence; resetting it is valid.
        let raw = unsafe { (self.fns.reset_fences)(self.device, 1, &frame.in_flight) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkResetFences", result));
        }

        let cmd = self.command_buffers[self.frame_index];
        let image = swapchain_image_for(swapchain, image_index as usize);
        let view = swapchain.image_views[image_index as usize];
        let render_finished = self.render_finished[image_index as usize];

        // SAFETY: this slot's fence was just waited so `cmd` is recordable; the
        // image/view belong to `swapchain`; `composite`'s resources were created on
        // this device (caller contract) and its texture is already resident in
        // SHADER_READ_ONLY_OPTIMAL; the recorded path only samples that texture into
        // the swapchain (UNDEFINED → COLOR → PRESENT, or → TRANSFER_SRC → readback →
        // PRESENT) — no composite-texture write.
        unsafe {
            self.record_present_sampled(
                cmd,
                image,
                view,
                swapchain.extent,
                clear,
                composite,
                readback,
                ui,
            )?
        };

        // --- Submit: wait acquire @ COLOR_ATTACHMENT_OUTPUT, signal render-finished + fence. ---
        let wait_stage: VkFlags = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
        let submit = VkSubmitInfo {
            s_type: VkStructureType::SubmitInfo,
            p_next: ptr::null(),
            wait_semaphore_count: 1,
            p_wait_semaphores: (&frame.acquire as *const VkSemaphore).cast(),
            p_wait_dst_stage_mask: &wait_stage,
            command_buffer_count: 1,
            p_command_buffers: &cmd,
            signal_semaphore_count: 1,
            p_signal_semaphores: (&render_finished as *const VkSemaphore).cast(),
        };
        // SAFETY: `queue` is the live present/graphics queue; one submit naming the
        // recorded `cmd`, waiting this frame's acquire semaphore at
        // COLOR_ATTACHMENT_OUTPUT, signalling this image's render-finished semaphore
        // + this frame's in-flight fence; all referenced locals outlive the call.
        let raw = unsafe { (self.fns.queue_submit)(self.queue, 1, &submit, frame.in_flight) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkQueueSubmit", result));
        }

        // --- Present: wait render-finished. ---
        let present = VkPresentInfoKhr {
            s_type: VkStructureType::PresentInfoKhr,
            p_next: ptr::null(),
            wait_semaphore_count: 1,
            p_wait_semaphores: &render_finished,
            swapchain_count: 1,
            p_swapchains: &swapchain.swapchain,
            p_image_indices: &image_index,
            p_results: ptr::null_mut(),
        };
        // SAFETY: `queue` supports present (confirmed in `Surface::new`); the
        // present-info names the live swapchain + acquired `image_index`, waiting
        // this image's render-finished semaphore; all locals outlive the call.
        let raw = unsafe { (self.swap_fns.queue_present)(self.queue, &present) };
        let present_result = VkResult::from_raw(raw);

        self.frame_index = (self.frame_index + 1) % FRAMES_IN_FLIGHT;

        if present_result == VkResult::ERROR_OUT_OF_DATE_KHR
            || present_result == VkResult::SUBOPTIMAL_KHR
        {
            self.recreate(surface, swapchain, width, height)?;
            return Ok(false);
        }
        if !present_result.is_success() {
            return Err(SwapchainError::VkError("vkQueuePresentKHR", present_result));
        }
        Ok(true)
    }

    /// Records the rung-11 fullscreen-sample present into `cmd`: barrier the swapchain
    /// image (UNDEFINED → COLOR), `vkCmdBeginRendering` (color CLEAR), bind the
    /// fullscreen pipeline + the composite-texture bind group + dynamic
    /// viewport/scissor, `vkCmdDraw(3, 1, 0, 0)`, `vkCmdEndRendering`, then either
    /// color COLOR → PRESENT (steady) or color COLOR → TRANSFER_SRC, copy-to-buffer,
    /// TRANSFER_SRC → PRESENT (the test readback path).
    ///
    /// The composite texture is NOT touched as a write target here: the caller
    /// uploaded it once before the present loop and left it in
    /// `SHADER_READ_ONLY_OPTIMAL`, so this records only a `FRAGMENT_SHADER` sample of
    /// it (no upload copy, no composite-texture barrier). That is what keeps the
    /// multi-frame-in-flight present loop free of any write-after-read hazard on the
    /// shared composite texture.
    ///
    /// # Safety
    ///
    /// `cmd` must be recordable (waited free); `image`/`view` must belong to the
    /// swapchain image presented this frame; every `composite` resource is live on
    /// this device and `composite.texture` is already resident in
    /// `SHADER_READ_ONLY_OPTIMAL` (uploaded once by the caller, never written again);
    /// the pipeline's declared color format equals the swapchain image's (W2-b); a
    /// `Some(readback)` buffer is host-visible and ≥ the swapchain image's byte size.
    #[allow(clippy::too_many_arguments)]
    unsafe fn record_present_sampled(
        &self,
        cmd: VkCommandBuffer,
        image: VkImage,
        view: VkImageView,
        extent: VkExtent2D,
        clear: [f32; 4],
        composite: &SampledComposite<'_>,
        readback: Option<&BoundBuffer>,
        ui: Option<&UiPass<'_>>,
    ) -> Result<(), SwapchainError> {
        let begin = VkCommandBufferBeginInfo {
            s_type: VkStructureType::CommandBufferBeginInfo,
            p_next: ptr::null(),
            flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            p_inheritance_info: ptr::null(),
        };
        // SAFETY: `cmd` is recordable per this fn's contract; `begin` is a
        // fully-initialized one-time-submit begin-info.
        let raw = unsafe { (self.fns.begin_command_buffer)(cmd, &begin) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkBeginCommandBuffer", result));
        }

        // The composite texture is already resident in SHADER_READ_ONLY_OPTIMAL (the
        // caller's pre-loop one-time upload). This path only SAMPLES it, so it records
        // no barrier on the composite texture — a read-only image shared across
        // frames-in-flight needs none, and re-uploading/re-transitioning it per frame
        // would be the cross-frame write-after-read hazard this restructure removes.

        // --- Barrier (swapchain color): UNDEFINED → COLOR_ATTACHMENT_OPTIMAL. ---
        let to_color = VkImageMemoryBarrier {
            s_type: VkStructureType::ImageMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: 0,
            dst_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image,
            subresource_range: COLOR_SUBRESOURCE_RANGE,
        };
        // SAFETY: recording is open; one image barrier on the live swapchain `image`;
        // TOP_OF_PIPE→COLOR_ATTACHMENT_OUTPUT with UNDEFINED→COLOR is the
        // superset-correct acquire→render transition; `&to_color` outlives the call.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                cmd,
                VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                0,
                0,
                ptr::null(),
                0,
                ptr::null(),
                1,
                (&to_color as *const VkImageMemoryBarrier).cast(),
            );
        }

        // Dynamic rendering: one color attachment (the swapchain image, CLEAR/STORE),
        // no depth (the fullscreen triangle is depth-less). The pipeline's declared
        // color format equals the swapchain format (W2-b, upheld by the caller).
        let color_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue {
                color: VkClearColorValue { float32: clear },
            },
        };
        let rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: VkRect2D {
                offset: VkOffset2D { x: 0, y: 0 },
                extent,
            },
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: 1,
            p_color_attachments: &color_attachment,
            p_depth_attachment: ptr::null(),
            p_stencil_attachment: ptr::null(),
        };
        // Present the composite at its NATIVE size in the TOP-LEFT of the swapchain
        // image, NOT stretched to the full swapchain extent. The viewport/scissor are
        // clamped to `min(swapchain_extent, texture_extent)` at origin (0, 0): the
        // fullscreen triangle then writes exactly the composite's pixels 1:1, and the
        // rest of a wider WSI-clamped swapchain image keeps the clear color (the
        // begin-rendering `render_area` above stays the full swapchain extent so the
        // CLEAR covers it). A 1:1 top-left mapping makes a per-texel golden exact
        // regardless of any `current_extent` clamp.
        let present_extent = VkExtent2D {
            width: extent.width.min(composite.texture_extent.width),
            height: extent.height.min(composite.texture_extent.height),
        };
        let viewport = VkViewport {
            x: 0.0,
            y: 0.0,
            width: present_extent.width as f32,
            height: present_extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let scissor = VkRect2D {
            offset: VkOffset2D { x: 0, y: 0 },
            extent: present_extent,
        };
        // SAFETY: recording is open; `rendering` is fully initialized — its color
        // attachment names the live swapchain `view` (now COLOR_ATTACHMENT_OPTIMAL);
        // dynamic rendering is enabled on this device. The pipeline + its bind-group
        // layout belong to this device (caller contract) and the pipeline's declared
        // color format equals the swapchain image's (W2-b). The bind group binds the
        // composite texture (now SHADER_READ_ONLY_OPTIMAL) + sampler at set 0 of the
        // pipeline's layout; `viewport`/`scissor` locals outlive the bracketed calls;
        // `draw(3, 1, 0, 0)` is the `SV_VertexID` fullscreen triangle (no vertex
        // buffer). Begin/End bracket the pass exactly.
        unsafe {
            (self.fns.cmd_begin_rendering)(cmd, &rendering);
            (self.fns.cmd_bind_pipeline)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                composite.pipeline.pipeline,
            );
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                composite.pipeline.layout,
                0,
                1,
                &composite.bind_group.descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_set_viewport)(cmd, 0, 1, &viewport);
            (self.fns.cmd_set_scissor)(cmd, 0, 1, &scissor);
            (self.fns.cmd_draw)(cmd, 3, 1, 0, 0);
            (self.fns.cmd_end_rendering)(cmd);
        }

        // --- GUI P5a Rung 5 / Decision 9: the UI rect sub-pass. After the composite
        //     scope ENDED above, open a FRESH `begin_rendering(LoadOp::Load)` at the
        //     FULL swapchain extent (preserve the composite, do NOT re-clear) and
        //     record ONE instanced draw of the current frame's UI rects. The image is
        //     still COLOR_ATTACHMENT_OPTIMAL (set by the to_color barrier above; the
        //     composite scope only ended the render pass, not the layout), so no
        //     barrier is needed between the two color passes — both are
        //     COLOR_ATTACHMENT_OUTPUT writes to the same image, ordered by the render-
        //     pass boundary. The COLOR→PRESENT/TRANSFER transition below then covers
        //     BOTH passes' writes. A pass with `instance_count == 0` records nothing. ---
        if let Some(ui) = ui
            && ui.instance_count > 0
        {
            let ui_color = VkRenderingAttachmentInfo {
                s_type: VkStructureType::RenderingAttachmentInfo,
                p_next: ptr::null(),
                image_view: view,
                image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                resolve_mode: 0,
                resolve_image_view: VkImageView::NULL,
                resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                // LOAD preserves the composited scene; STORE keeps the UI result.
                load_op: VK_ATTACHMENT_LOAD_OP_LOAD,
                store_op: VK_ATTACHMENT_STORE_OP_STORE,
                clear_value: VkClearValue {
                    color: VkClearColorValue { float32: [0.0; 4] },
                },
            };
            // The UI pass covers the FULL swapchain extent (NOT `present_extent`): the
            // ortho denominator the host computed is the swapchain extent, so a rect at
            // the bottom-right corner must reach the bottom-right swapchain texel.
            let ui_rendering = VkRenderingInfo {
                s_type: VkStructureType::RenderingInfo,
                p_next: ptr::null(),
                flags: 0,
                render_area: VkRect2D {
                    offset: VkOffset2D { x: 0, y: 0 },
                    extent,
                },
                layer_count: 1,
                view_mask: 0,
                color_attachment_count: 1,
                p_color_attachments: &ui_color,
                p_depth_attachment: ptr::null(),
                p_stencil_attachment: ptr::null(),
            };
            let ui_viewport = VkViewport {
                x: 0.0,
                y: 0.0,
                width: extent.width as f32,
                height: extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            let ui_scissor = VkRect2D {
                offset: VkOffset2D { x: 0, y: 0 },
                extent,
            };
            debug_assert_eq!(
                ui.ortho_bytes.len(),
                16,
                "invariant: the UI ortho push block is 16 bytes (UiOrtho)"
            );
            // SAFETY: recording is open; `ui_rendering` is fully initialized — its color
            // attachment names the live swapchain `view` (still COLOR_ATTACHMENT_OPTIMAL
            // from the to_color barrier) with LoadOp::LOAD (preserving the composite).
            // `ui.pipeline`/`ui.bind_group` are the caller's live, current-frame-
            // re-resolved (MF-7) UI handles (their `RhiContext` outlives this submit per
            // the caller contract); the pipeline's `color_formats[0]` equals the
            // swapchain format (W2-b). The ortho is pushed to the pipeline's VERTEX range
            // (16 B, asserted); the bind-group's STORAGE ring holds `instance_count`
            // valid records uploaded for this frame index. `ui_viewport`/`ui_scissor`
            // span the full swapchain extent and outlive the bracketed calls; the
            // vertexless `draw(6, N, 0, 0)` reads the SSBO by `SV_InstanceID`. Begin/End
            // bracket the pass exactly.
            unsafe {
                (self.fns.cmd_begin_rendering)(cmd, &ui_rendering);
                (self.fns.cmd_bind_pipeline)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_GRAPHICS,
                    ui.pipeline.pipeline,
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_GRAPHICS,
                    ui.pipeline.layout,
                    0,
                    1,
                    &ui.bind_group.descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_push_constants)(
                    cmd,
                    ui.pipeline.layout,
                    VK_SHADER_STAGE_VERTEX_BIT,
                    0,
                    ui.ortho_bytes.len() as u32,
                    ui.ortho_bytes.as_ptr().cast(),
                );
                (self.fns.cmd_set_viewport)(cmd, 0, 1, &ui_viewport);
                (self.fns.cmd_set_scissor)(cmd, 0, 1, &ui_scissor);
                (self.fns.cmd_draw)(cmd, 6, ui.instance_count, 0, 0);
                (self.fns.cmd_end_rendering)(cmd);
            }
        }

        // The post-draw color transition depends on whether a readback is requested
        // (identical to `record_scene`'s branch).
        match readback {
            // Steady present path: COLOR_ATTACHMENT_OPTIMAL → PRESENT_SRC_KHR.
            None => {
                let to_present = VkImageMemoryBarrier {
                    s_type: VkStructureType::ImageMemoryBarrier,
                    p_next: ptr::null(),
                    src_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                    dst_access_mask: 0,
                    old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    new_layout: VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
                    src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    image,
                    subresource_range: COLOR_SUBRESOURCE_RANGE,
                };
                // SAFETY: recording is open; COLOR_ATTACHMENT_OUTPUT→BOTTOM_OF_PIPE
                // with COLOR→PRESENT makes the draw's writes visible to the present
                // engine; `&to_present` outlives the call.
                unsafe {
                    (self.fns.cmd_pipeline_barrier)(
                        cmd,
                        VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                        VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
                        0,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        1,
                        (&to_present as *const VkImageMemoryBarrier).cast(),
                    );
                }
            }
            // Test readback path: COLOR → TRANSFER_SRC, copy image → buffer, then
            // TRANSFER_SRC → PRESENT (the image is still presented after the copy).
            Some(staging) => {
                let to_transfer = VkImageMemoryBarrier {
                    s_type: VkStructureType::ImageMemoryBarrier,
                    p_next: ptr::null(),
                    src_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                    dst_access_mask: VK_ACCESS_TRANSFER_READ_BIT,
                    old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    new_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    image,
                    subresource_range: COLOR_SUBRESOURCE_RANGE,
                };
                // SAFETY: recording is open; COLOR_ATTACHMENT_OUTPUT→TRANSFER with
                // COLOR→TRANSFER_SRC makes the draw's writes available to the copy;
                // `&to_transfer` outlives the call.
                unsafe {
                    (self.fns.cmd_pipeline_barrier)(
                        cmd,
                        VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                        VK_PIPELINE_STAGE_TRANSFER_BIT,
                        0,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        1,
                        (&to_transfer as *const VkImageMemoryBarrier).cast(),
                    );
                }

                let region = VkBufferImageCopy {
                    buffer_offset: 0,
                    buffer_row_length: 0,
                    buffer_image_height: 0,
                    image_subresource: VkImageSubresourceLayers {
                        aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    },
                    image_offset: VkOffset3D { x: 0, y: 0, z: 0 },
                    image_extent: VkExtent3D {
                        width: extent.width,
                        height: extent.height,
                        depth: 1,
                    },
                };
                // SAFETY: recording is open; the image is TRANSFER_SRC_OPTIMAL per the
                // barrier above; one full-image tightly-packed color region copies
                // into the live host-visible `staging.buffer` (≥ the image's byte size
                // per this fn's contract); `&region` outlives the call.
                unsafe {
                    (self.fns.cmd_copy_image_to_buffer)(
                        cmd,
                        image,
                        VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                        staging.buffer,
                        1,
                        &region,
                    );
                }

                let to_present = VkImageMemoryBarrier {
                    s_type: VkStructureType::ImageMemoryBarrier,
                    p_next: ptr::null(),
                    src_access_mask: VK_ACCESS_TRANSFER_READ_BIT,
                    dst_access_mask: 0,
                    old_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    new_layout: VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
                    src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    image,
                    subresource_range: COLOR_SUBRESOURCE_RANGE,
                };
                // SAFETY: recording is open; TRANSFER→BOTTOM_OF_PIPE with
                // TRANSFER_SRC→PRESENT releases the image to the present engine after
                // the readback copy; `&to_present` outlives the call.
                unsafe {
                    (self.fns.cmd_pipeline_barrier)(
                        cmd,
                        VK_PIPELINE_STAGE_TRANSFER_BIT,
                        VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
                        0,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        1,
                        (&to_present as *const VkImageMemoryBarrier).cast(),
                    );
                }
            }
        }

        // SAFETY: recording is open; ending it matches the `begin` above.
        let raw = unsafe { (self.fns.end_command_buffer)(cmd) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkEndCommandBuffer", result));
        }
        Ok(())
    }

    /// Renders + presents ONE on-screen Render-P1c **image-based** G-buffer frame: the
    /// P1b shared-depth marcher (the depth IMAGE source + the MRT G-buffer sink) driven
    /// ON SCREEN, killing the packed path's per-frame depth→buffer copy. Recreates the
    /// swapchain on resize / out-of-date / suboptimal (identical return semantics to
    /// [`render_scene_frame`](Self::render_scene_frame)).
    ///
    /// # The 3-pass on-screen frame (one command buffer, fence-only submit, §1b model)
    ///
    /// (A) raster the mesh quad → D32 depth IMAGE, (B) the SDF compute marcher samples
    /// that depth image + writes the FINAL composite into the ALBEDO storage image
    /// (byte-untouched from P1b), (C) present-blit: fullscreen-sample the ALBEDO into
    /// the acquired swapchain image, present. The deferred-lighting split (the marcher
    /// writing UNLIT attributes + a separate lighting pass) is DEFERRED to P7
    /// (multi-light/clustered) — P1b's marcher already writes the lit composite, so a
    /// P1c lighting pass would be a no-op passthrough that breaks the golden.
    ///
    /// There is NO `copy_image_to_buffer(depth)` and NO per-frame
    /// `vkUpdateDescriptorSets`: the marcher SAMPLES the depth image, and both
    /// descriptor sets are written ONCE per composite extent by
    /// [`GBufferTargets::sync_gbuffer`]. The G-buffer targets + the marcher's
    /// raster/dispatch are sized to `present_extent` (the composite), NOT the swapchain
    /// extent: a P0a/rung-11 WSI-clamped (wider) swapchain image never resizes the
    /// G-buffer — the present-blit maps the composite 1:1 into the swapchain's top-left.
    ///
    /// If `readback` is `Some`, on THIS frame the presented swapchain image is
    /// `vkCmdCopyImageToBuffer`'d into the supplied host-visible staging buffer (the
    /// on-screen golden readback path — proving the image-based composite reached the
    /// swapchain); the steady present path passes `None`.
    ///
    /// `present_extent` is the composite's native size for the top-left 1:1 present
    /// (`min(swapchain_extent, present_extent)` clamps the present viewport/scissor, so
    /// the per-texel golden is exact regardless of the WSI extent clamp — the same
    /// 1:1-top-left contract [`SampledComposite`] uses). Pass the extent the marcher
    /// dispatched at (the clamped swapchain extent the caller sized `frame`'s targets
    /// + `scene.camera_uniform` + `scene.dispatch_group_count_x` to).
    ///
    /// # Safety
    ///
    /// Every `scene` resource was created on the same device as this renderer and
    /// outlives the call; `scene.edit_list` / `scene.camera_uniform` were host-seeded
    /// once before the present loop and are NEVER written again (the marcher only reads
    /// them — frames-in-flight dispatch against them with no host write-after-read);
    /// `frame`'s targets were synced to the swapchain extent (the call does this via
    /// [`GBufferTargets::sync_gbuffer`] when needed), and both
    /// `scene.dispatch_group_count_x` and `scene.camera_uniform`'s `count` were sized to
    /// that extent. Any readback buffer is host-visible and at least
    /// `swapchain.extent` * 4 bytes (4 B/texel).
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn render_gbuffer_frame(
        &mut self,
        ctx: &VulkanContext,
        surface: &Surface<'_>,
        swapchain: &mut Swapchain<'ctx>,
        scene: &GBufferScene<'_>,
        frame: &mut GBufferFrame,
        width: u32,
        height: u32,
        clear: [f32; 4],
        present_extent: VkExtent2D,
        readback: Option<&BoundBuffer>,
    ) -> Result<bool, SwapchainError> {
        let slot = &self.frames[self.frame_index];

        // --- Wait this frame slot's in-flight fence (free its cmd buffer + targets). ---
        // SAFETY: `device` is live; `&slot.in_flight` names this slot's fence; an
        // infinite wait blocks until this slot's previous submit completed, so its
        // command buffer + acquire semaphore (and the G-buffer targets it used) are free.
        let raw = unsafe {
            (self.fns.wait_for_fences)(self.device, 1, &slot.in_flight, VK_TRUE, VK_TIMEOUT_INFINITE)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkWaitForFences", result));
        }

        // Ensure the G-buffer targets (+ descriptor sets) match the COMPOSITE
        // (`present_extent`) — NOT the swapchain extent. The marcher dispatches +
        // rasterizes at `present_extent` (the golden composite size); the present-blit
        // maps that 1:1 into the swapchain's top-left, so a WSI-clamped (wider) swapchain
        // never resizes the G-buffer. The fence wait above frees THIS slot; a REPLACE
        // additionally waits idle for sibling slots. (The first call creates them.) The
        // descriptor sets are written ONCE per composite extent.
        GBufferTargets::sync_gbuffer(&mut frame.targets, ctx, scene, present_extent)?;

        // --- Acquire the next image (signals this frame's acquire semaphore). ---
        let mut image_index: u32 = 0;
        // SAFETY: `device` + `swapchain` are live; an infinite timeout + this slot's
        // acquire semaphore (null fence) is the standard acquire; `&mut image_index`
        // is a valid out-pointer.
        let raw = unsafe {
            (self.swap_fns.acquire_next_image)(
                self.device,
                swapchain.swapchain,
                VK_TIMEOUT_INFINITE,
                self.frames[self.frame_index].acquire,
                VkFence::NULL,
                &mut image_index,
            )
        };
        let acquire_result = VkResult::from_raw(raw);
        if acquire_result == VkResult::ERROR_OUT_OF_DATE_KHR {
            self.recreate(surface, swapchain, width, height)?;
            return Ok(false);
        }
        if !acquire_result.is_success() && acquire_result != VkResult::SUBOPTIMAL_KHR {
            return Err(SwapchainError::VkError("vkAcquireNextImageKHR", acquire_result));
        }

        // Only reset the fence once we are committing to a submit (mirrors the
        // `render_scene_frame` out-of-date discipline so an early return never leaves
        // the fence unsignalled).
        // SAFETY: `device` is live; `&...in_flight` names this slot's already-waited
        // fence; resetting it is valid.
        let in_flight = self.frames[self.frame_index].in_flight;
        let raw = unsafe { (self.fns.reset_fences)(self.device, 1, &in_flight) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkResetFences", result));
        }

        let cmd = self.command_buffers[self.frame_index];
        let image = swapchain_image_for(swapchain, image_index as usize);
        let view = swapchain.image_views[image_index as usize];
        let render_finished = self.render_finished[image_index as usize];
        let acquire = self.frames[self.frame_index].acquire;
        let targets = frame
            .targets
            .as_ref()
            .expect("invariant: sync_gbuffer made the targets present before record");

        // SAFETY: this slot's fence was just waited so `cmd` is recordable; the
        // image/view belong to `swapchain`; `scene`'s resources + `targets` were created
        // on this device and `targets` is synced to `swapchain.extent`; the recorded
        // path is the raster→depth-sample→march→present-blit (or →readback) 3-pass.
        unsafe {
            self.record_gbuffer(
                cmd,
                image,
                view,
                swapchain.extent,
                present_extent,
                clear,
                scene,
                targets,
                readback,
            )?
        };

        // --- Submit: wait acquire @ COLOR_ATTACHMENT_OUTPUT, signal render-finished + fence. ---
        let wait_stage: VkFlags = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
        let submit = VkSubmitInfo {
            s_type: VkStructureType::SubmitInfo,
            p_next: ptr::null(),
            wait_semaphore_count: 1,
            p_wait_semaphores: (&acquire as *const VkSemaphore).cast(),
            p_wait_dst_stage_mask: &wait_stage,
            command_buffer_count: 1,
            p_command_buffers: &cmd,
            signal_semaphore_count: 1,
            p_signal_semaphores: (&render_finished as *const VkSemaphore).cast(),
        };
        // SAFETY: `queue` is the live present/graphics queue; one submit naming the
        // recorded `cmd`, waiting this frame's acquire semaphore at
        // COLOR_ATTACHMENT_OUTPUT, signalling this image's render-finished semaphore
        // + this frame's in-flight fence; all referenced locals outlive the call.
        let raw = unsafe { (self.fns.queue_submit)(self.queue, 1, &submit, in_flight) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkQueueSubmit", result));
        }

        // --- Present: wait render-finished. ---
        let present = VkPresentInfoKhr {
            s_type: VkStructureType::PresentInfoKhr,
            p_next: ptr::null(),
            wait_semaphore_count: 1,
            p_wait_semaphores: &render_finished,
            swapchain_count: 1,
            p_swapchains: &swapchain.swapchain,
            p_image_indices: &image_index,
            p_results: ptr::null_mut(),
        };
        // SAFETY: `queue` supports present (confirmed in `Surface::new`); the
        // present-info names the live swapchain + acquired `image_index`, waiting
        // this image's render-finished semaphore; all locals outlive the call.
        let raw = unsafe { (self.swap_fns.queue_present)(self.queue, &present) };
        let present_result = VkResult::from_raw(raw);

        self.frame_index = (self.frame_index + 1) % FRAMES_IN_FLIGHT;

        if present_result == VkResult::ERROR_OUT_OF_DATE_KHR
            || present_result == VkResult::SUBOPTIMAL_KHR
        {
            self.recreate(surface, swapchain, width, height)?;
            return Ok(false);
        }
        if !present_result.is_success() {
            return Err(SwapchainError::VkError("vkQueuePresentKHR", present_result));
        }
        Ok(true)
    }

    /// Records the Render-P1c on-screen 3-pass G-buffer frame into `cmd`. The barrier
    /// sequence (one hand-FFI barrier per transition — correct-but-unbatched; P3a
    /// batches later):
    ///
    /// 0. throwaway raster color `UNDEFINED → COLOR_ATTACHMENT_OPTIMAL` (the raster
    ///    pipeline declares one color format, so the prepass binds a format-compatible
    ///    throwaway color attachment whose result is discarded — only the depth matters)
    /// 1. depth `UNDEFINED → DEPTH_ATTACHMENT_OPTIMAL` (TOP_OF_PIPE → (EARLY|LATE)_FRAGMENT_TESTS)
    /// 2. **(pass A)** `vkCmdBeginRendering` (throwaway color CLEAR/STORE + depth CLEAR
    ///    to the far plane / STORE), draw the mesh quad — the depth prepass (the
    ///    swapchain image becomes a color attachment only at pass C)
    /// 3. depth `DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL` (DEPTH aspect,
    ///    (EARLY|LATE)_FRAGMENT_TESTS → COMPUTE_SHADER) — the single dual-use depth
    ///    barrier (REPLACES the packed path's depth copy + its two transfer barriers)
    /// 4. the 3 G-buffer images `UNDEFINED → GENERAL` (TOP_OF_PIPE → COMPUTE_SHADER)
    /// 5. **(P0 coarse cull, OPTIONAL — only when `scene.coarse` is `Some`)** bind the
    ///    coarse-cull pipeline + the vocabulary set, dispatch one group per `LOCAL_SIZE_X`
    ///    tiles (each invocation writes a `TileBound` into binding 6), then a COMPUTE→COMPUTE
    ///    buffer barrier on `tiles_buffer` (SHADER_WRITE → SHADER_READ); the marcher then runs
    ///    with `coarse_enabled == scene.coarse_mode` (`1` = full / `2` = empty-skip-only). When
    ///    `scene.coarse` is `None` this step records NOTHING (`coarse_enabled == 0`).
    /// 6. **(pass B)** bind the marcher + the vocabulary set, dispatch (the marcher
    ///    SAMPLES the depth image, STORES the final composite into ALBEDO)
    /// 7. ALBEDO `GENERAL → SHADER_READ_ONLY_OPTIMAL` (COMPUTE_SHADER → FRAGMENT_SHADER)
    /// 8. swapchain `UNDEFINED → COLOR_ATTACHMENT_OPTIMAL` (TOP_OF_PIPE → COLOR_ATTACHMENT_OUTPUT)
    /// 9. **(pass C)** `vkCmdBeginRendering` (swapchain color CLEAR), fullscreen-sample
    ///    the ALBEDO 1:1 in the top-left, end
    /// 10. swapchain `COLOR_ATTACHMENT_OPTIMAL → PRESENT_SRC_KHR` (steady) or
    ///     `→ TRANSFER_SRC`, copy-to-buffer, `→ PRESENT` (the readback path)
    ///
    /// NO `copy_image_to_buffer(depth)` (step 3 replaces it) and NO
    /// `vkUpdateDescriptorSets` (both sets were written once at `sync_gbuffer`).
    ///
    /// Extents: passes A (prepass raster/depth) and B (the marcher dispatch → composite)
    /// run at `present_extent` (the composite size the G-buffer/depth images, the dispatch
    /// grid, and the camera UBO `count` were all sized to in `sync_gbuffer`). `extent` is
    /// the swapchain extent and governs ONLY pass C's clear render-area (step 8) and the
    /// readback region (step 9); the present-blit viewport is `min(extent, present_extent)`
    /// at the origin for the exact 1:1 top-left composite present.
    ///
    /// # Safety
    ///
    /// `cmd` must be recordable (waited free); `image`/`view` must belong to the
    /// swapchain image presented this frame; `scene`'s pipelines / buffers / samplers
    /// are live on this device; `targets` was synced to `present_extent` (the composite
    /// size — its descriptor sets bind `scene`'s SSBO/UBO + its own images, and its
    /// G-buffer/depth images are allocated at `present_extent`); `scene.dispatch_group_count_x`
    /// (and `scene.camera_uniform`'s `count`) cover `present_extent`'s pixel count.
    /// `extent` is the swapchain extent and governs ONLY pass C's clear render-area and the
    /// readback region; a `Some(readback)` buffer is host-visible and ≥ the swapchain
    /// image's (`extent`-sized) byte size.
    #[allow(clippy::too_many_arguments)]
    unsafe fn record_gbuffer(
        &self,
        cmd: VkCommandBuffer,
        image: VkImage,
        view: VkImageView,
        extent: VkExtent2D,
        present_extent: VkExtent2D,
        clear: [f32; 4],
        scene: &GBufferScene<'_>,
        targets: &GBufferTargets,
        readback: Option<&BoundBuffer>,
    ) -> Result<(), SwapchainError> {
        let begin = VkCommandBufferBeginInfo {
            s_type: VkStructureType::CommandBufferBeginInfo,
            p_next: ptr::null(),
            flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            p_inheritance_info: ptr::null(),
        };
        // SAFETY: `cmd` is recordable per this fn's contract; `begin` is a
        // fully-initialized one-time-submit begin-info.
        let raw = unsafe { (self.fns.begin_command_buffer)(cmd, &begin) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkBeginCommandBuffer", result));
        }

        // === Pass A (Render P5-r0): rasterize the mesh quad as a 3-MRT G-buffer PRODUCER
        // (albedo@0, normal@1, material@2) + the D32 depth. The marcher's attribute
        // encoding is the contract; pass A writes mesh fragments in it (mask=1) so the
        // deferred resolve lights mesh pixels first-class and the r1 ownership gate yields
        // to them. gViewT is UNTOUCHED by r0 (still wholly marcher-produced). ===

        // (0) Barrier-in: the 3 RGBA8 G-buffer images UNDEFINED → COLOR_ATTACHMENT_OPTIMAL.
        // Replaces the deleted throwaway-color barrier, now on the three real images.
        // `src=0`/`TOP_OF_PIPE` is the superset-correct FIRST transition for a freshly
        // re-`UNDEFINED`'d image (no prior content to make available).
        for tex in [&targets.albedo, &targets.normal, &targets.material] {
            let to_color = VkImageMemoryBarrier {
                s_type: VkStructureType::ImageMemoryBarrier,
                p_next: ptr::null(),
                src_access_mask: 0,
                dst_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                image: tex.image,
                subresource_range: COLOR_SUBRESOURCE_RANGE,
            };
            // SAFETY: recording is open; one image barrier on a live G-buffer image;
            // TOP_OF_PIPE→COLOR_ATTACHMENT_OUTPUT with UNDEFINED→COLOR is the
            // superset-correct first transition; `&to_color` outlives the iteration.
            unsafe {
                (self.fns.cmd_pipeline_barrier)(
                    cmd,
                    VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                    VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                    0,
                    0,
                    ptr::null(),
                    0,
                    ptr::null(),
                    1,
                    (&to_color as *const VkImageMemoryBarrier).cast(),
                );
            }
        }

        // (1) Barrier (depth): UNDEFINED → DEPTH_ATTACHMENT_OPTIMAL at the
        // early/late-fragment-test stage (the depth-write access, DEPTH aspect).
        let to_depth = VkImageMemoryBarrier {
            s_type: VkStructureType::ImageMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: 0,
            dst_access_mask: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
            old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image: targets.depth.image,
            subresource_range: DEPTH_SUBRESOURCE_RANGE,
        };
        // SAFETY: recording is open; one image barrier on the live depth image;
        // TOP_OF_PIPE→(EARLY|LATE)_FRAGMENT_TESTS with UNDEFINED→DEPTH is the
        // superset-correct first depth transition; `&to_depth` outlives the call.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                cmd,
                VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT
                    | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
                0,
                0,
                ptr::null(),
                0,
                ptr::null(),
                1,
                (&to_depth as *const VkImageMemoryBarrier).cast(),
            );
        }

        // (2) Dynamic rendering at the marcher's extent: 3 MRT color attachments
        // (albedo@0, normal@1, material@2; CLEAR/STORE) + the depth attachment (CLEAR to
        // the far plane / STORE). The render area is the marcher's extent so the
        // rasterized fragments cover exactly the dispatched pixels; the swapchain may be
        // WSI-clamped wider (the present-blit handles that).
        //
        // Render P5-r0 / Decision r0-2: each color clear IS the marcher's mask=0 neutral
        // G-buffer, so a pixel with NO mesh fragment holds the cleared neutral, which the
        // marcher (owning that pixel) overwrites anyway — making the no-mesh 0%-gate
        // trivial AND a depth-failed/missed mesh fragment fall back to a valid mask=0
        // neutral. The clears pass through the SAME float→UNORM8 `round(c*255)` quantizer
        // the marcher store uses; 0.05/0.10/0.5/1.0/0.0 are all exact, so the cleared
        // neutral is bit-identical to a marcher-written neutral.
        //   albedo  clear = (BACKGROUND.rgb, 1.0)  — the marcher's background base.
        //   normal  clear = (0.5, 0.5, 0.0, 0.0)   — neutral oct + id=0.
        //   material clear = (1.0, 1.0, 0.0, 1.0)  — shadow=1, ao=1, mask=0, 1.
        // These MUST equal the marcher's background-arm constants (sdf_gbuffer_composite.hlsl:
        // BACKGROUND = (0.05, 0.05, 0.1); the Site-A/B mask=0 neutrals).
        let albedo_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: targets.albedo.view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue {
                color: VkClearColorValue {
                    float32: [0.05, 0.05, 0.1, 1.0],
                },
            },
        };
        let normal_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: targets.normal.view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue {
                color: VkClearColorValue {
                    float32: [0.5, 0.5, 0.0, 0.0],
                },
            },
        };
        let material_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: targets.material.view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue {
                color: VkClearColorValue {
                    float32: [1.0, 1.0, 0.0, 1.0],
                },
            },
        };
        let raster_color_attachments =
            [albedo_attachment, normal_attachment, material_attachment];
        let depth_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: targets.depth.view,
            image_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue {
                depth_stencil: VkClearDepthStencilValue {
                    depth: 1.0,
                    stencil: 0,
                },
            },
        };
        let raster_area = VkRect2D {
            offset: VkOffset2D { x: 0, y: 0 },
            extent: present_extent,
        };
        let raster_rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: raster_area,
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: raster_color_attachments.len() as u32,
            p_color_attachments: raster_color_attachments.as_ptr(),
            p_depth_attachment: (&depth_attachment as *const VkRenderingAttachmentInfo).cast(),
            p_stencil_attachment: ptr::null(),
        };
        let raster_viewport = VkViewport {
            x: 0.0,
            y: 0.0,
            width: present_extent.width as f32,
            height: present_extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let vertex_offset: VkDeviceSize = 0;
        // SAFETY: recording is open; `raster_rendering` is fully initialized — its 3 color
        // attachments name the live albedo/normal/material views (now
        // COLOR_ATTACHMENT_OPTIMAL) and its depth attachment the live depth view (now
        // DEPTH_ATTACHMENT_OPTIMAL); `raster_color_attachments` outlives the bracketed
        // calls; dynamic rendering is enabled on this device. The raster pipeline (declaring
        // 3 matching color formats + 3 blend states, P5-r0) + its VERTEX push range + the
        // vertex buffer all belong to this device (caller contract) and the pipeline's
        // declared color/depth formats equal the bound attachments'. The MVP push is
        // `GBUFFER_MVP_BYTES` at offset 0 into the VERTEX range;
        // `vertex_offset`/`raster_viewport`/`raster_area` locals outlive the bracketed
        // calls; `draw(vertex_count, 1, 0, 0)` reads the bound vertices. Begin/End
        // bracket pass A exactly.
        unsafe {
            (self.fns.cmd_begin_rendering)(cmd, &raster_rendering);
            (self.fns.cmd_bind_pipeline)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                scene.raster_pipeline.pipeline,
            );
            (self.fns.cmd_push_constants)(
                cmd,
                scene.raster_pipeline.layout,
                VK_SHADER_STAGE_VERTEX_BIT,
                0,
                scene.mvp.len() as u32,
                scene.mvp.as_ptr().cast(),
            );
            (self.fns.cmd_bind_vertex_buffers)(
                cmd,
                0,
                1,
                &scene.vertex_buffer.buffer,
                &vertex_offset,
            );
            (self.fns.cmd_set_viewport)(cmd, 0, 1, &raster_viewport);
            (self.fns.cmd_set_scissor)(cmd, 0, 1, &raster_area);
            (self.fns.cmd_draw)(cmd, scene.vertex_count, 1, 0, 0);
            (self.fns.cmd_end_rendering)(cmd);
        }

        // (3) THE single depth dual-use barrier: DEPTH_ATTACHMENT_OPTIMAL →
        // SHADER_READ_ONLY_OPTIMAL. Depth WRITES happen at LATE_FRAGMENT_TESTS; the
        // marcher SAMPLES at COMPUTE_SHADER. This one barrier (DEPTH aspect) makes the
        // write available + visible to the shader-read and transitions the layout for
        // sampling. It REPLACES the packed path's depth→buffer copy + its two transfer
        // barriers — there is NO copy_image_to_buffer(depth) here.
        let depth_to_sampled = VkImageMemoryBarrier {
            s_type: VkStructureType::ImageMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
            dst_access_mask: VK_ACCESS_SHADER_READ_BIT,
            old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
            new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image: targets.depth.image,
            subresource_range: DEPTH_SUBRESOURCE_RANGE,
        };
        // SAFETY: recording is open; (EARLY|LATE)_FRAGMENT_TESTS→COMPUTE_SHADER with
        // DEPTH_WRITE→SHADER_READ and DEPTH→SHADER_READ_ONLY makes the rasterized depth
        // available + visible to the marcher's sample; `&depth_to_sampled` outlives the
        // call.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                cmd,
                VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT
                    | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                0,
                0,
                ptr::null(),
                0,
                ptr::null(),
                1,
                (&depth_to_sampled as *const VkImageMemoryBarrier).cast(),
            );
        }

        // (3b) Render P5-r0 barrier-out: the 3 RGBA8 G-buffer images
        // COLOR_ATTACHMENT_OPTIMAL → GENERAL, handing pass A's rasterized mesh fragments to
        // the marcher. This is a GENUINE raster-write hand-off (NOT the old throwaway
        // UNDEFINED→GENERAL shape): `src=COLOR_ATTACHMENT_OUTPUT/COLOR_ATTACHMENT_WRITE`
        // makes the raster's color writes AVAILABLE, `dst=COMPUTE_SHADER/SHADER_READ|
        // SHADER_WRITE` makes them VISIBLE to the marcher's reads/writes (and, post-r1, the
        // resolve's reads). The marcher then (under the r1 ownership gate) does NOT write a
        // mesh-owned texel, so the raster's value survives to the resolve — single producer
        // per texel across the barrier. On a no-mesh / clear-only pass a CLEAR is a color
        // write, correctly made available by the same source half.
        for tex in [&targets.albedo, &targets.normal, &targets.material] {
            let color_to_general = VkImageMemoryBarrier {
                s_type: VkStructureType::ImageMemoryBarrier,
                p_next: ptr::null(),
                src_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                dst_access_mask: VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT,
                old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                new_layout: VK_IMAGE_LAYOUT_GENERAL,
                src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                image: tex.image,
                subresource_range: COLOR_SUBRESOURCE_RANGE,
            };
            // SAFETY: recording is open; one image barrier on a live G-buffer image;
            // COLOR_ATTACHMENT_OUTPUT→COMPUTE_SHADER with COLOR_ATTACHMENT_WRITE→
            // SHADER_READ|SHADER_WRITE and COLOR_ATTACHMENT_OPTIMAL→GENERAL makes pass A's
            // rasterized fragments available + visible to the marcher; `&color_to_general`
            // outlives the iteration.
            unsafe {
                (self.fns.cmd_pipeline_barrier)(
                    cmd,
                    VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    0,
                    0,
                    ptr::null(),
                    0,
                    ptr::null(),
                    1,
                    (&color_to_general as *const VkImageMemoryBarrier).cast(),
                );
            }
        }

        // (4) The lit output + the Lighting-L0b gViewT lane: UNDEFINED → GENERAL. The
        // marcher stores gViewT and the resolve stores lit — both in GENERAL. (Render P5-r0:
        // albedo/normal/material moved to the COLOR_ATTACHMENT_OPTIMAL→GENERAL barrier-out
        // (3b) above, since pass A now rasterizes into them; gViewT stays UNDEFINED→GENERAL
        // because r0 does NOT rasterize into it — it is still wholly marcher-produced.)
        // Render P7: `targets.ssao` (R8_UNORM) joins this batch — it lives in GENERAL its whole
        // life like `viewt` (no SSAO pass writes it yet, C2 adds that; the resolve reads it only
        // under `ssao_mode != 0`, so on every pre-P7 scene this is a valid GENERAL image the
        // resolve never reads — byte-identical PIXELS, one extra harmless transition that does
        // NOT alter the OFF arithmetic).
        for tex in [&targets.lit, &targets.viewt, &targets.ssao] {
            let to_general = VkImageMemoryBarrier {
                s_type: VkStructureType::ImageMemoryBarrier,
                p_next: ptr::null(),
                src_access_mask: 0,
                dst_access_mask: VK_ACCESS_SHADER_WRITE_BIT,
                old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                new_layout: VK_IMAGE_LAYOUT_GENERAL,
                src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                image: tex.image,
                subresource_range: COLOR_SUBRESOURCE_RANGE,
            };
            // SAFETY: recording is open; one image barrier on the live G-buffer image;
            // TOP_OF_PIPE→COMPUTE_SHADER with UNDEFINED→GENERAL is the superset-correct
            // first storage-image transition; `&to_general` outlives the iteration.
            unsafe {
                (self.fns.cmd_pipeline_barrier)(
                    cmd,
                    VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    0,
                    0,
                    ptr::null(),
                    0,
                    ptr::null(),
                    1,
                    (&to_general as *const VkImageMemoryBarrier).cast(),
                );
            }
        }

        // === Lighting L0-r0: ASYNC light-table re-upload (C3), recorded only on a dirty
        // frame, BEFORE the marcher/resolve reads. A staging→device `cmd_copy_buffer` +
        // a TRANSFER_WRITE→SHADER_READ buffer barrier (TRANSFER→COMPUTE_SHADER) into the
        // SAME `cmd` — fence-free, no readback (mirroring the store-to-load image barrier
        // below). An idle (non-dirty) frame records NOTHING — byte-identical command
        // stream to before (the rung L0-r0 0%-gate). The collection system wrote the new
        // table into `light_staging`'s mapped bytes and set `light_dirty`. ===
        if scene.light_dirty && scene.light_upload_bytes > 0 {
            let region = VkBufferCopy {
                src_offset: 0,
                dst_offset: 0,
                size: scene.light_upload_bytes,
            };
            let to_shader_read = VkBufferMemoryBarrier {
                s_type: VkStructureType::BufferMemoryBarrier,
                p_next: ptr::null(),
                src_access_mask: VK_ACCESS_TRANSFER_WRITE_BIT,
                dst_access_mask: VK_ACCESS_SHADER_READ_BIT,
                src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                buffer: scene.light_table.buffer,
                offset: 0,
                size: scene.light_upload_bytes,
            };
            // SAFETY: recording is open; `light_staging` (host-coherent, the collection
            // wrote `light_upload_bytes` into its mapped bytes this frame) and
            // `light_table` (device-local, TRANSFER_DST | STORAGE) are live buffers on
            // this device; the copy region + barrier span `[0, light_upload_bytes)` ≤ both
            // buffer sizes (caller contract — the table is sized for MAX_LIGHTS); the
            // barrier orders the TRANSFER write before the COMPUTE_SHADER reads (the
            // marcher/resolve) on the GPU timeline, fence-free; `&region`/`&to_shader_read`
            // outlive the calls.
            unsafe {
                (self.fns.cmd_copy_buffer)(
                    cmd,
                    scene.light_staging.buffer,
                    scene.light_table.buffer,
                    1,
                    &region,
                );
                (self.fns.cmd_pipeline_barrier)(
                    cmd,
                    VK_PIPELINE_STAGE_TRANSFER_BIT,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    0,
                    0,
                    ptr::null(),
                    1,
                    (&to_shader_read as *const VkBufferMemoryBarrier).cast(),
                    0,
                    ptr::null(),
                );
            }
        }

        // === Render P0: the P4b COARSE-CULL pass (Decision: mirror the offscreen
        // `run_gbuffer_hybrid_ex` coarse dispatch + the `cluster_cull` optional-compute
        // recorder shape). Recorded ONLY when the scene wires the coarse pipeline; otherwise
        // skipped entirely — NO dispatch, NO barrier — so the command stream is byte-identical
        // to the pre-P0 windowed path (the 0%-gate). The coarse pass binds the SAME vocabulary
        // set (the cull shader declares only a subset — valid), SAMPLES the depth (already
        // SHADER_READ from barrier 3, which it shares with the marcher), and WRITES one
        // `TileBound` per 8×8 tile into vocab binding 6. The fine marcher then READS those
        // bounds (gated by `coarse_enabled == 1` in its push) to skip empty / cone-rejected
        // tiles — the SAME pixels, fewer marches. A COMPUTE→COMPUTE buffer barrier on
        // `tiles_buffer` orders the cull WRITE before the marcher READ. ===
        let coarse_enabled = scene.coarse.is_some();
        if let Some(coarse_pipeline) = scene.coarse {
            // The 1D coarse dispatch element count = the full tile grid at the COMPOSITE
            // extent (the marcher dispatches + the camera UBO `count` are sized to it). One
            // group per `LOCAL_SIZE_X` tiles, mirroring the offscreen `coarse_group_count_x`.
            let (tw, th) = tile_grid_extent(present_extent.width, present_extent.height);
            let coarse_groups = (tw * th).div_ceil(LOCAL_SIZE_X);
            // SAFETY: recording is open; the coarse pipeline + its layout (declaring
            // `vocab_layout` at set 0 + the shared COMPUTE push range) are live on this device
            // (caller contract); the vocabulary set binds the SSBO/UBO + the now-transitioned
            // depth (SHADER_READ) + a valid Tiles SSBO @6 (the cull's write target) + the valid
            // brick descriptors @9..=14; the cull shader uses only a subset of those bindings
            // (valid); `coarse_groups` covers the full tile grid at the 64-wide group;
            // `&...vocab_set.descriptor_set` is a single-element local alive for the call
            // (first_set 0, count 1, zero dynamic offsets). The cull declares no push it reads,
            // but the layout's push range matches the marcher's, so no constant is pushed here.
            unsafe {
                (self.fns.cmd_bind_pipeline)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    coarse_pipeline.pipeline,
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    coarse_pipeline.layout,
                    0,
                    1,
                    &targets.vocab_set.descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_dispatch)(cmd, coarse_groups, 1, 1);
            }

            // Order the coarse pass's `TileBound` WRITES (binding 6, COMPUTE/SHADER_WRITE)
            // before the fine marcher's READS (COMPUTE/SHADER_READ) — a COMPUTE→COMPUTE buffer
            // barrier on `tiles_buffer` (mirrors the offscreen inter-dispatch barrier).
            let tiles_barrier = VkBufferMemoryBarrier {
                s_type: VkStructureType::BufferMemoryBarrier,
                p_next: ptr::null(),
                src_access_mask: VK_ACCESS_SHADER_WRITE_BIT,
                dst_access_mask: VK_ACCESS_SHADER_READ_BIT,
                src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                buffer: scene.tiles_buffer.buffer,
                offset: 0,
                size: VK_WHOLE_SIZE,
            };
            // SAFETY: recording is open; `tiles_buffer` is a live STORAGE buffer on this device
            // (the cull just wrote it); COMPUTE_SHADER→COMPUTE_SHADER with
            // SHADER_WRITE→SHADER_READ makes the cull's tile-bound writes available + visible to
            // the marcher's reads on the GPU timeline; `&tiles_barrier` outlives the call.
            unsafe {
                (self.fns.cmd_pipeline_barrier)(
                    cmd,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    0,
                    0,
                    ptr::null(),
                    1,
                    (&tiles_barrier as *const VkBufferMemoryBarrier).cast(),
                    0,
                    ptr::null(),
                );
            }
        }

        // === Pass B: the marcher SAMPLES the depth image, STORES the G-buffer. ===
        // (5) Bind the marcher + the vocabulary set (written ONCE at sync_gbuffer; NO
        // per-frame update) against the marcher's OWN dedicated layout, push the 32-byte
        // P4b/B1 constants, dispatch.
        //
        // The marcher's 32-byte compute push range is `FineMarcherPush`
        // `{ coarse_enabled: u32 @0, omega: f32 @4, lighting_flags: u32 @8, light_dir: float3 @16 }`.
        // Render P0: `coarse_enabled` is a 3-value `CoarseMode` — `0` on the OFF path (no coarse
        // dispatch, the tile read is gated off), else `scene.coarse_mode`: `1` (full = EMPTY-skip +
        // `near_t` seed) or `2` (empty-skip-only = EMPTY-skip, NO seed → lit-transparent, no rim).
        // When the cull pass above ran the marcher reads the per-tile bounds it wrote into binding 6
        // (skipping empty tiles). Either way the marcher DECLARES binding 6, so the (valid) Tiles descriptor
        // is always bound in the vocabulary set. `omega` carries the B1 over-relaxation
        // factor (`DEFAULT_MARCHER_OMEGA`, the provably hole-free speedup). Render A1/A2:
        // the on-screen demo turns lighting ON (A1 soft shadows + A2 AO) with the default
        // directional light.
        // SDF brick-cache activation (campaign M1/M2/M4): the empty-skip + trilinear/cubic surface
        // cache + clip-map LOD gates live ENTIRELY in this per-frame push (the bound descriptors at
        // 9..=14 are static), so `scene.brick` selects ON/OFF at runtime with no re-record — the
        // owner's A/B toggle.
        //
        // - `None` (the default / OFF path): `brick_enabled == 0` / `brick_trilinear == 0` /
        //   `brick_levels == 1` — the marcher's `select_level` loops once over level 0 and never reads
        //   the brick grids/atlas, byte-identical to the pre-brick M2 marcher. `with_brick_levels(1)`
        //   is REQUIRED (the recompiled shader treats `brick_levels == 0` as no-level).
        // - `Some(a)` (the ON path): `with_brick(a.grid_origin, a.grid_dims, a.brick_world)` stamps the
        //   level-0 empty-skip grid uniforms (the `lvl == 0` arm indexes binding 9 with them),
        //   `with_brick_trilinear(true)` turns on the surface-brick cubic, and `with_brick_levels(a.levels)`
        //   loops the clip-map ladder. The caller MUST have bound the real BrickClipmap per-level
        //   resources at 9..=14 + written its `M4GridParams` tail into the b5 UBO. This mirrors the
        //   offscreen RTX-verified `run_gbuffer_hybrid_m4` push exactly.
        // Render P0: the marcher's coarse-cull mode. OFF (no `coarse` pipeline ⇒ no dispatch) forces
        // `CoarseMode::Off` so the push byte is 0 and the marcher never reads the (un-dispatched)
        // tile bounds — byte-identical to the pre-P0 stream. ON uses `scene.coarse_mode`: `Full`
        // keeps the historical EMPTY-skip + `near_t` seed (the offscreen goldens' mode);
        // `EmptySkipOnly` is the LIT-TRANSPARENT on-screen cull (EMPTY-skip only, no seed → no
        // grazing-silhouette AO/shadow rim).
        let coarse_mode = if coarse_enabled { scene.coarse_mode } else { CoarseMode::Off };
        let base = FineMarcherPush::new_mode(
            coarse_mode,
            DEFAULT_MARCHER_OMEGA,
            scene.lighting_flags,
            // The marcher marches the A1 soft shadow toward the SCENE's primary directional `L`
            // (NOT a hardcoded head-on `[0,0,1]`), so an angled sun casts a real shadow that the
            // resolve's primary directional then consumes via `gMaterial.r`. See `light_dir`.
            scene.light_dir,
        );
        let marcher_push = match scene.brick {
            Some(a) => base
                .with_brick(a.grid_origin, a.grid_dims, a.brick_world)
                .with_brick_trilinear(true)
                .with_brick_levels(a.levels),
            None => base.with_brick_levels(1),
        };
        // SAFETY: recording is open; the marcher pipeline + its layout (declaring
        // `vocab_layout` at set 0 AND the 80-byte COMPUTE push range) are live on this
        // device (caller contract); the vocabulary set binds the SSBO/UBO + the
        // now-transitioned depth (SHADER_READ) + G-buffer (GENERAL) images + a valid
        // Tiles SSBO @6 + valid brick descriptors @9..=14 (whether the brick gates are ON
        // or OFF, those descriptors are always bound — caller contract); `dispatch_group_count_x`
        // covers `present_extent`'s pixel count (the G-buffer images + dispatch grid + camera UBO
        // `count` are all sized to `present_extent`, the composite — NOT the swapchain `extent`;
        // caller contract); `&...descriptor_set` is a single-element local alive for the call
        // (first_set 0, count 1, zero dynamic offsets); `marcher_push.as_bytes()` is
        // `GBUFFER_MARCHER_PUSH_BYTES` (80) bytes at offset 0, exactly the declared 80-byte range,
        // and the backing `marcher_push` local outlives the call.
        let marcher_push_bytes = marcher_push.as_bytes();
        unsafe {
            (self.fns.cmd_bind_pipeline)(
                cmd,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                scene.marcher.pipeline,
            );
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                scene.marcher.layout,
                0,
                1,
                &targets.vocab_set.descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_push_constants)(
                cmd,
                scene.marcher.layout,
                VK_SHADER_STAGE_COMPUTE_BIT,
                0,
                GBUFFER_MARCHER_PUSH_BYTES,
                marcher_push_bytes.as_ptr().cast(),
            );
            (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
        }

        // (5a) PBR MVP-2: make the marcher's gAlbedo + gNormal + gMaterial STORES available
        // + visible to the resolve's LOADS. A real memory+execution dependency
        // (SHADER_WRITE→SHADER_READ, COMPUTE→COMPUTE), GENERAL→GENERAL (no layout change).
        // gNormal is now READ by the resolve (oct-normal decode + 16-bit material id), so it
        // joins gAlbedo + gMaterial in the barrier (MVP-1 omitted it — gNormal was unread).
        // Lighting L0b: the gViewT lane is marcher-STORED + resolve-READ, so it joins too.
        for tex in [
            &targets.albedo,
            &targets.normal,
            &targets.material,
            &targets.viewt,
        ] {
            let store_to_load = VkImageMemoryBarrier {
                s_type: VkStructureType::ImageMemoryBarrier,
                p_next: ptr::null(),
                src_access_mask: VK_ACCESS_SHADER_WRITE_BIT,
                dst_access_mask: VK_ACCESS_SHADER_READ_BIT,
                old_layout: VK_IMAGE_LAYOUT_GENERAL,
                new_layout: VK_IMAGE_LAYOUT_GENERAL,
                src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                image: tex.image,
                subresource_range: COLOR_SUBRESOURCE_RANGE,
            };
            // SAFETY: recording is open; one image barrier on a live G-buffer image;
            // COMPUTE_SHADER→COMPUTE_SHADER with SHADER_WRITE→SHADER_READ + GENERAL→GENERAL
            // makes the marcher's attribute store available + visible to the resolve's
            // load; `&store_to_load` outlives the iteration.
            unsafe {
                (self.fns.cmd_pipeline_barrier)(
                    cmd,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    0,
                    0,
                    ptr::null(),
                    0,
                    ptr::null(),
                    1,
                    (&store_to_load as *const VkImageMemoryBarrier).cast(),
                );
            }
        }

        // === Render P7: the SSAO compute pass. Recorded ONLY when the scene wires the SSAO
        // activation (`scene.ssao.is_some()`); otherwise skipped entirely — NO bind, NO dispatch,
        // NO barrier — so the command stream is byte-identical to the pre-P7 windowed path (the
        // 0%-gate; the `ssao` image is always allocated + transitioned by C1's batch regardless of
        // this branch). The SSAO pass gathers a horizon-based AO factor from the G-buffer (gNormal/
        // gMaterial/gViewT, READ) and STORES it into the `ssao` lane the resolve combines under
        // `ssao_mode != 0`. Its inputs are already SHADER_READ-visible: the marcher→resolve
        // store-to-load barrier above (5a) covers gNormal/gMaterial/gViewT (the SSAO reads the same
        // three the resolve reads), so NO new input barrier is needed. After the dispatch, a NEW
        // COMPUTE→COMPUTE / SHADER_WRITE→SHADER_READ / GENERAL→GENERAL barrier on `ssao` orders the
        // SSAO store before the resolve's `gSsao.Load` (the cull→resolve barrier shape, on the
        // `ssao` image). The SSAO pass reads its camera from the UBO bound at the SSAO set's binding
        // 4, so it pushes NO constant (unlike the marcher). ===
        if let Some(activation) = &scene.ssao {
            let ssao_set = targets
                .ssao_set
                .as_ref()
                .expect("invariant: scene.ssao is Some ⇒ GBufferTargets::create wrote ssao_set");
            // SAFETY: recording is open; the SSAO pipeline + its layout (declaring the SSAO set
            // layout at set 0 + the shared 80-byte COMPUTE push range) are live on this device
            // (caller contract); `ssao_set` binds the now-stored (SHADER_READ-visible, GENERAL)
            // gNormal/gMaterial/gViewT + the `ssao` out (GENERAL) images + the scene's camera UBO;
            // `dispatch_group_count_x` covers `present_extent`'s pixel count (the same grid the
            // marcher/resolve dispatch); `&ssao_set.descriptor_set` is a single-element local alive
            // for the call (first_set 0, count 1, zero dynamic offsets). The SSAO shader reads its
            // camera from the UBO @4, so no push constant is recorded.
            unsafe {
                (self.fns.cmd_bind_pipeline)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    activation.pipeline.pipeline,
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    activation.pipeline.layout,
                    0,
                    1,
                    &ssao_set.descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
            }

            // Order the SSAO pass's `ssao` WRITES (COMPUTE/SHADER_WRITE) before the resolve's
            // `gSsao.Load` READS (COMPUTE/SHADER_READ) — a COMPUTE→COMPUTE, GENERAL→GENERAL image
            // barrier on `ssao` (the cull→resolve barrier shape, on the `ssao` image).
            let ssao_store_to_load = VkImageMemoryBarrier {
                s_type: VkStructureType::ImageMemoryBarrier,
                p_next: ptr::null(),
                src_access_mask: VK_ACCESS_SHADER_WRITE_BIT,
                dst_access_mask: VK_ACCESS_SHADER_READ_BIT,
                old_layout: VK_IMAGE_LAYOUT_GENERAL,
                new_layout: VK_IMAGE_LAYOUT_GENERAL,
                src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                image: targets.ssao.image,
                subresource_range: COLOR_SUBRESOURCE_RANGE,
            };
            // SAFETY: recording is open; one image barrier on the live `ssao` image (the SSAO pass
            // just wrote it); COMPUTE_SHADER→COMPUTE_SHADER with SHADER_WRITE→SHADER_READ +
            // GENERAL→GENERAL makes the SSAO store available + visible to the resolve's
            // `gSsao.Load`; `&ssao_store_to_load` outlives the call.
            unsafe {
                (self.fns.cmd_pipeline_barrier)(
                    cmd,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    0,
                    0,
                    ptr::null(),
                    0,
                    ptr::null(),
                    1,
                    (&ssao_store_to_load as *const VkImageMemoryBarrier).cast(),
                );
            }
        }

        // === Lighting L1: the clustered froxel light-cull pass (Decision 6). Recorded ONLY
        // when the scene wires the cull pipeline + cull set; otherwise skipped entirely (the
        // resolve's `clusters_enabled` header gate then loops the flat table — the L1 OFF /
        // 0%-gate, byte-identical command stream). The cull reads the camera UBO + light table
        // (the L0-r0 copy above already ordered the table for COMPUTE reads) and writes the
        // ClusterGrid + LightIndexList; the resolve reads them, so a COMPUTE→COMPUTE buffer
        // barrier orders the cull WRITE before the resolve READ. The cull does NOT depend on
        // gViewT (it is geometric), so it can run after the marcher without further sync. ===
        if let (Some(cull_pipeline), Some(cull_set), Some(grid), Some(index), Some(alloc)) = (
            scene.cluster_cull,
            targets.cull_set.as_ref(),
            scene.cluster_grid,
            scene.light_index,
            scene.light_index_alloc,
        ) {
            // (L1-0) Reset the global slice-allocation counter to 0 (a transfer fill), then
            // order the fill before the cull's atomic reads/writes (TRANSFER→COMPUTE).
            let alloc_reset_barrier = VkBufferMemoryBarrier {
                s_type: VkStructureType::BufferMemoryBarrier,
                p_next: ptr::null(),
                src_access_mask: VK_ACCESS_TRANSFER_WRITE_BIT,
                dst_access_mask: VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT,
                src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                buffer: alloc.buffer,
                offset: 0,
                size: VK_WHOLE_SIZE,
            };
            // SAFETY: recording is open; `alloc` is a live device-local STORAGE buffer (≥ 4 B,
            // the single u32 counter); `cmd_fill_buffer` zero-fills it (Vulkan 1.0 core), and
            // the barrier orders that TRANSFER write before the cull's COMPUTE atomics on the
            // GPU timeline; `&alloc_reset_barrier` outlives the call.
            unsafe {
                (self.fns.cmd_fill_buffer)(cmd, alloc.buffer, 0, VK_WHOLE_SIZE, 0);
                (self.fns.cmd_pipeline_barrier)(
                    cmd,
                    VK_PIPELINE_STAGE_TRANSFER_BIT,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    0,
                    0,
                    ptr::null(),
                    1,
                    (&alloc_reset_barrier as *const VkBufferMemoryBarrier).cast(),
                    0,
                    ptr::null(),
                );
            }

            // (L1-1) Bind the cull pipeline + the cull set (written ONCE at sync_gbuffer),
            // push the 16-byte ClusterCullPush, dispatch over CLUSTER_COUNT froxels.
            let cull_groups = scene.cluster_count.div_ceil(LIGHT_CULL_LOCAL_SIZE_X);
            // SAFETY: recording is open; the cull pipeline + its layout (declaring `cull_layout`
            // at set 0 + the 16-byte COMPUTE push range) are live on this device (caller
            // contract); the cull set binds the camera UBO + light table + the cluster buffers;
            // `cull_groups` covers `cluster_count` froxels at the 64-wide group; the push bytes
            // are exactly `CLUSTER_CULL_PUSH_BYTES` (16) at offset 0; `&cull_set.descriptor_set`
            // is a single-element local alive for the call (first_set 0, count 1).
            unsafe {
                (self.fns.cmd_bind_pipeline)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    cull_pipeline.pipeline,
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    cull_pipeline.layout,
                    0,
                    1,
                    &cull_set.descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_push_constants)(
                    cmd,
                    cull_pipeline.layout,
                    VK_SHADER_STAGE_COMPUTE_BIT,
                    0,
                    CLUSTER_CULL_PUSH_BYTES,
                    scene.cluster_cull_push.as_ptr().cast(),
                );
                (self.fns.cmd_dispatch)(cmd, cull_groups, 1, 1);
            }

            // (L1-2) Make the cull's ClusterGrid + LightIndexList writes available + visible to
            // the resolve's reads (COMPUTE→COMPUTE, SHADER_WRITE→SHADER_READ) on both buffers.
            for buf in [grid, index] {
                let cull_to_resolve = VkBufferMemoryBarrier {
                    s_type: VkStructureType::BufferMemoryBarrier,
                    p_next: ptr::null(),
                    src_access_mask: VK_ACCESS_SHADER_WRITE_BIT,
                    dst_access_mask: VK_ACCESS_SHADER_READ_BIT,
                    src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    buffer: buf.buffer,
                    offset: 0,
                    size: VK_WHOLE_SIZE,
                };
                // SAFETY: recording is open; one buffer barrier on a live cluster SSBO;
                // COMPUTE_SHADER→COMPUTE_SHADER with SHADER_WRITE→SHADER_READ makes the cull's
                // grid/index store available + visible to the resolve's load on the GPU
                // timeline; `&cull_to_resolve` outlives the iteration.
                unsafe {
                    (self.fns.cmd_pipeline_barrier)(
                        cmd,
                        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                        0,
                        0,
                        ptr::null(),
                        1,
                        (&cull_to_resolve as *const VkBufferMemoryBarrier).cast(),
                        0,
                        ptr::null(),
                    );
                }
            }
        }

        // (5b) Deferred RESOLVE pass: bind the resolve pipeline + the resolve set (gAlbedo
        // @0, gMaterial @1, lit @2 — all STORAGE in GENERAL), dispatch at the SAME grid the
        // marcher used (1:1 the marched pixels). It composites `lit = mask ? base*vis : base`.
        // SAFETY: recording is open; the resolve pipeline + its layout (declaring
        // `resolve_layout` at set 0) are live on this device (caller contract); the resolve
        // set binds the now-stored (GENERAL) albedo/material + the lit (GENERAL) images;
        // `dispatch_group_count_x` covers `present_extent`'s pixel count (the same grid the
        // marcher dispatched); `&...descriptor_set` is a single-element local alive for the
        // call (first_set 0, count 1, zero dynamic offsets). The resolve pushes NO constants.
        unsafe {
            (self.fns.cmd_bind_pipeline)(
                cmd,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                scene.resolve_pipeline.pipeline,
            );
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                scene.resolve_pipeline.layout,
                0,
                1,
                &targets.resolve_set.descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
        }

        // (5c) LIT: GENERAL → SHADER_READ_ONLY_OPTIMAL for the present-blit sample. The
        // present now samples LIT (the resolve's output), NOT albedo (the deletion target
        // of the old step-6 albedo→SHADER_READ_ONLY barrier — albedo stays GENERAL,
        // consumed only by the resolve as a STORAGE-in-GENERAL load).
        let lit_to_sampled = VkImageMemoryBarrier {
            s_type: VkStructureType::ImageMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: VK_ACCESS_SHADER_WRITE_BIT,
            dst_access_mask: VK_ACCESS_SHADER_READ_BIT,
            old_layout: VK_IMAGE_LAYOUT_GENERAL,
            new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image: targets.lit.image,
            subresource_range: COLOR_SUBRESOURCE_RANGE,
        };
        // SAFETY: recording is open; COMPUTE_SHADER→FRAGMENT_SHADER with
        // SHADER_WRITE→SHADER_READ and GENERAL→SHADER_READ_ONLY makes the resolve's lit
        // store available + visible to the present-blit's sample; `&lit_to_sampled`
        // outlives the call.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                cmd,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
                0,
                0,
                ptr::null(),
                0,
                ptr::null(),
                1,
                (&lit_to_sampled as *const VkImageMemoryBarrier).cast(),
            );
        }

        // === Pass C: present-blit the LIT image (the resolve's output) into the swapchain. ===

        // (7) Barrier (swapchain color): UNDEFINED → COLOR_ATTACHMENT_OPTIMAL.
        let to_color = VkImageMemoryBarrier {
            s_type: VkStructureType::ImageMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: 0,
            dst_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image,
            subresource_range: COLOR_SUBRESOURCE_RANGE,
        };
        // SAFETY: recording is open; one image barrier on the live swapchain `image`;
        // TOP_OF_PIPE→COLOR_ATTACHMENT_OUTPUT with UNDEFINED→COLOR is the
        // superset-correct acquire→render transition; `&to_color` outlives the call.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                cmd,
                VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                0,
                0,
                ptr::null(),
                0,
                ptr::null(),
                1,
                (&to_color as *const VkImageMemoryBarrier).cast(),
            );
        }

        // (8) Dynamic rendering: the swapchain image (CLEAR/STORE), no depth. The
        // present pipeline's declared color format equals the swapchain format (W2-b).
        let color_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue {
                color: VkClearColorValue { float32: clear },
            },
        };
        let present_rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: VkRect2D {
                offset: VkOffset2D { x: 0, y: 0 },
                extent,
            },
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: 1,
            p_color_attachments: &color_attachment,
            p_depth_attachment: ptr::null(),
            p_stencil_attachment: ptr::null(),
        };
        // Present the composite at its NATIVE size in the swapchain image's TOP-LEFT,
        // NOT stretched to the (possibly WSI-clamped wider) swapchain extent. The
        // viewport/scissor are clamped to `min(swapchain_extent, present_extent)` at
        // origin: the fullscreen triangle writes exactly the composite's pixels 1:1, and
        // a wider swapchain image's remainder keeps the clear color. A 1:1 top-left
        // mapping makes a per-texel golden exact regardless of any WSI clamp.
        let blit_extent = VkExtent2D {
            width: extent.width.min(present_extent.width),
            height: extent.height.min(present_extent.height),
        };
        let blit_viewport = VkViewport {
            x: 0.0,
            y: 0.0,
            width: blit_extent.width as f32,
            height: blit_extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let blit_scissor = VkRect2D {
            offset: VkOffset2D { x: 0, y: 0 },
            extent: blit_extent,
        };
        // SAFETY: recording is open; `present_rendering` is fully initialized — its color
        // attachment names the live swapchain `view` (now COLOR_ATTACHMENT_OPTIMAL);
        // dynamic rendering is enabled. The present pipeline + its bind-group layout
        // belong to this device (caller contract) and its declared color format equals
        // the swapchain's (W2-b). The present set binds the LIT image (now
        // SHADER_READ_ONLY_OPTIMAL) + sampler at set 0 of the pipeline's layout;
        // `blit_viewport`/`blit_scissor` outlive the bracketed calls; `draw(3, 1, 0, 0)`
        // is the `SV_VertexID` fullscreen triangle (no vertex buffer). Begin/End bracket
        // pass C exactly.
        unsafe {
            (self.fns.cmd_begin_rendering)(cmd, &present_rendering);
            (self.fns.cmd_bind_pipeline)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                scene.present_pipeline.pipeline,
            );
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                scene.present_pipeline.layout,
                0,
                1,
                &targets.present_set.descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_set_viewport)(cmd, 0, 1, &blit_viewport);
            (self.fns.cmd_set_scissor)(cmd, 0, 1, &blit_scissor);
            (self.fns.cmd_draw)(cmd, 3, 1, 0, 0);
            (self.fns.cmd_end_rendering)(cmd);
        }

        // (9) The post-draw swapchain transition: steady → PRESENT, or the readback
        // path → TRANSFER_SRC, copy-to-buffer, → PRESENT (identical to
        // `record_present_sampled`'s branch — the swapchain still presents after the
        // copy).
        match readback {
            None => {
                let to_present = VkImageMemoryBarrier {
                    s_type: VkStructureType::ImageMemoryBarrier,
                    p_next: ptr::null(),
                    src_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                    dst_access_mask: 0,
                    old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    new_layout: VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
                    src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    image,
                    subresource_range: COLOR_SUBRESOURCE_RANGE,
                };
                // SAFETY: recording is open; COLOR_ATTACHMENT_OUTPUT→BOTTOM_OF_PIPE with
                // COLOR→PRESENT makes the blit's writes visible to the present engine;
                // `&to_present` outlives the call.
                unsafe {
                    (self.fns.cmd_pipeline_barrier)(
                        cmd,
                        VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                        VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
                        0,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        1,
                        (&to_present as *const VkImageMemoryBarrier).cast(),
                    );
                }
            }
            Some(staging) => {
                let to_transfer = VkImageMemoryBarrier {
                    s_type: VkStructureType::ImageMemoryBarrier,
                    p_next: ptr::null(),
                    src_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                    dst_access_mask: VK_ACCESS_TRANSFER_READ_BIT,
                    old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    new_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    image,
                    subresource_range: COLOR_SUBRESOURCE_RANGE,
                };
                // SAFETY: recording is open; COLOR_ATTACHMENT_OUTPUT→TRANSFER with
                // COLOR→TRANSFER_SRC makes the blit's writes available to the copy;
                // `&to_transfer` outlives the call.
                unsafe {
                    (self.fns.cmd_pipeline_barrier)(
                        cmd,
                        VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                        VK_PIPELINE_STAGE_TRANSFER_BIT,
                        0,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        1,
                        (&to_transfer as *const VkImageMemoryBarrier).cast(),
                    );
                }

                let region = VkBufferImageCopy {
                    buffer_offset: 0,
                    buffer_row_length: 0,
                    buffer_image_height: 0,
                    image_subresource: VkImageSubresourceLayers {
                        aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    },
                    image_offset: VkOffset3D { x: 0, y: 0, z: 0 },
                    image_extent: VkExtent3D {
                        width: extent.width,
                        height: extent.height,
                        depth: 1,
                    },
                };
                // SAFETY: recording is open; the swapchain image is TRANSFER_SRC_OPTIMAL
                // per the barrier above; one full-image tightly-packed color region
                // copies into the live host-visible `staging.buffer` (≥ the image's byte
                // size per this fn's contract); `&region` outlives the call. This copies
                // the SWAPCHAIN image (the on-screen golden) — NOT the depth (the depth
                // copy is the deletion target this path proves absent).
                unsafe {
                    (self.fns.cmd_copy_image_to_buffer)(
                        cmd,
                        image,
                        VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                        staging.buffer,
                        1,
                        &region,
                    );
                }

                let to_present = VkImageMemoryBarrier {
                    s_type: VkStructureType::ImageMemoryBarrier,
                    p_next: ptr::null(),
                    src_access_mask: VK_ACCESS_TRANSFER_READ_BIT,
                    dst_access_mask: 0,
                    old_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    new_layout: VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
                    src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    image,
                    subresource_range: COLOR_SUBRESOURCE_RANGE,
                };
                // SAFETY: recording is open; TRANSFER→BOTTOM_OF_PIPE with
                // TRANSFER_SRC→PRESENT releases the image to the present engine after the
                // readback copy; `&to_present` outlives the call.
                unsafe {
                    (self.fns.cmd_pipeline_barrier)(
                        cmd,
                        VK_PIPELINE_STAGE_TRANSFER_BIT,
                        VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
                        0,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        1,
                        (&to_present as *const VkImageMemoryBarrier).cast(),
                    );
                }
            }
        }

        // SAFETY: recording is open; ending it matches the `begin` above.
        let raw = unsafe { (self.fns.end_command_buffer)(cmd) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkEndCommandBuffer", result));
        }
        Ok(())
    }

    /// Waits the device idle, recreates the swapchain to `width`×`height`, and
    /// rebuilds the per-image render-finished semaphores (the image count may
    /// change). A `ZeroExtent` (minimized) is swallowed — the caller retries.
    fn recreate(
        &mut self,
        surface: &Surface<'_>,
        swapchain: &mut Swapchain<'ctx>,
        width: u32,
        height: u32,
    ) -> Result<(), SwapchainError> {
        // SAFETY: `device` is live; waiting idle guarantees no command buffer /
        // image view / semaphore is in use before we destroy + recreate them.
        unsafe { (self.fns.device_wait_idle)(self.device) };

        match swapchain.recreate(surface, width, height) {
            Ok(()) => {}
            // A minimized window has a zero extent: keep the old (now-idle)
            // swapchain and report "skipped"; the next frame retries.
            Err(SwapchainError::ZeroExtent) => return Ok(()),
            Err(e) => return Err(e),
        }

        // Rebuild render-finished semaphores to match the new image count.
        // SAFETY: device is idle; each old semaphore is destroyed once.
        unsafe {
            for &s in &self.render_finished {
                (self.fns.destroy_semaphore)(self.device, s, ptr::null());
            }
        }
        self.render_finished.clear();
        for _ in 0..swapchain.image_count() {
            self.render_finished.push(create_semaphore(self.fns, self.device)?);
        }
        self.frame_index = 0;
        Ok(())
    }
}

/// The MVP push-constant size (a `float4x4`), matching the committed rung-3/4 MVP
/// vertex shader's `VERTEX`-stage push range.
pub const SCENE_MVP_BYTES: usize = 64;

/// A device-local depth image (`D32_SFLOAT` + DEPTH-aspect view) sized to one
/// swapchain extent. Recreated when the extent changes (resize); the wrapping
/// [`VulkanTexture`] owns the image/view/memory and is torn down through the
/// originating [`VulkanContext`].
struct DepthImage {
    /// The owned `VkImage` + DEPTH view + dedicated allocation.
    texture: VulkanTexture,
    /// The extent the depth image was created at (so [`Scene::sync_depth`] can detect
    /// a resize and recreate it).
    extent: VkExtent2D,
}

/// The rung-7 on-screen scene resources: the depth-tested graphics pipeline, the
/// hardcoded mesh's vertex buffer, the MVP push constant, and the per-extent depth
/// image — everything [`Renderer::render_scene_frame`] needs beyond the swapchain.
///
/// The pipeline + vertex buffer are created by the caller through the
/// [`RhiDevice`] trait (so the proven S0 pipeline-creation
/// path is reused, not duplicated) and moved into the `Scene`; the depth image is
/// created + resized internally via the same device's `create_texture` path. The
/// `Scene` is **not** `Copy`/`Clone`: it is torn down by value through
/// [`Scene::destroy`] (the move encodes "destroyed exactly once").
///
/// The pipeline's declared color format MUST equal the swapchain's color format and
/// its declared depth format MUST equal [`Format::D32Sfloat`](boyko_rhi::Format) —
/// the W2-b format-matching contract — or the validation layer faults at draw time.
///
/// # Safety
///
/// The originating [`VulkanContext`] MUST still be alive whenever the scene is
/// rendered or destroyed: each of its owned handles is torn down through that
/// context's device fn-table. There is no compile-time `'ctx` tie this phase (plan
/// F1; mirrors the other S0 graphics resources).
pub struct Scene {
    /// The depth-tested graphics pipeline (raw `VkPipeline` + `VkPipelineLayout`),
    /// created via `RhiDevice::create_graphics_pipeline` and owned here.
    pipeline: VulkanGraphicsPipeline,
    /// The hardcoded mesh's host-visible vertex buffer (position + color), created
    /// via `RhiDevice::create_buffer` and owned here.
    vertex_buffer: BoundBuffer,
    /// The number of vertices to `draw` (the hardcoded mesh's vertex count).
    vertex_count: u32,
    /// The MVP `float4x4` pushed to the pipeline's VERTEX range each frame.
    mvp: [u8; SCENE_MVP_BYTES],
    /// The per-extent depth image, created lazily on the first frame and recreated
    /// on resize ([`Scene::sync_depth`]).
    depth: Option<DepthImage>,
}

impl Scene {
    /// Bundles a caller-created depth-tested graphics pipeline + vertex buffer + MVP
    /// into a renderable scene. The depth image is created lazily on the first frame
    /// (sized to the swapchain extent then), so no extent is needed here.
    ///
    /// `pipeline` MUST declare the swapchain's color format as its single color
    /// attachment and [`Format::D32Sfloat`](boyko_rhi::Format) as its depth format
    /// (W2-b); `vertex_buffer` holds `vertex_count` vertices in the pipeline's
    /// declared vertex layout; `mvp` is the 64-byte `float4x4` push constant.
    #[inline]
    pub fn new(
        pipeline: VulkanGraphicsPipeline,
        vertex_buffer: BoundBuffer,
        vertex_count: u32,
        mvp: [u8; SCENE_MVP_BYTES],
    ) -> Self {
        Self {
            pipeline,
            vertex_buffer,
            vertex_count,
            mvp,
            depth: None,
        }
    }

    /// Overwrites the per-frame MVP push-constant bytes (a column-major 4x4 `f32`
    /// matrix the vertex shader reads at VERTEX-stage offset 0). The next
    /// [`Renderer::render_scene_frame`] (its `record_scene` re-pushes `mvp`
    /// unconditionally each frame — swapchain.rs:1356) — and likewise the raster
    /// [`Renderer::render_gbuffer_frame`] (`record_gbuffer` re-pushes it at
    /// swapchain.rs:2542) — picks up these bytes with NO pipeline/scene rebuild.
    /// This is the live render-view seam a windowed loop uses to drive the
    /// on-screen view each frame from a per-frame `ViewUniform.view_proj`.
    #[inline]
    pub fn set_mvp(&mut self, mvp: [u8; SCENE_MVP_BYTES]) {
        self.mvp = mvp;
    }

    /// Ensures the depth image exists and matches `extent`, (re)creating it through
    /// `ctx` when it is absent (first frame) or stale (resize). The caller
    /// ([`Renderer::render_scene_frame`]) calls this only after fence-waiting the
    /// frame slot, so no in-flight frame still references an old depth image.
    fn sync_depth(&mut self, ctx: &VulkanContext, extent: VkExtent2D) -> Result<(), SwapchainError> {
        if let Some(d) = &self.depth
            && d.extent.width == extent.width
            && d.extent.height == extent.height
        {
            return Ok(());
        }

        // A (re)create is rare (first frame + resize). When REPLACING an existing
        // depth image, wait the device idle first: with multiple frames in flight a
        // sibling slot may still reference the old depth image, and the caller only
        // fence-waited THIS slot. The idle guarantees no submission references the old
        // image before it is freed (the same belt-and-braces the swapchain `recreate`
        // uses). The first-ever create (no old image) needs no idle.
        if self.depth.is_some() {
            // SAFETY: `ctx` is live; waiting idle guarantees every prior submission —
            // including any sibling-slot frame still referencing the old depth image —
            // has completed before it is destroyed below.
            unsafe { (ctx.device_fns().device_wait_idle)(ctx.device()) };
        }

        // Build the new depth image BEFORE tearing down the old one so an allocation
        // failure leaves the previous (still-valid) depth image in place.
        let desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format: Format::D32Sfloat,
            dimension: TextureDimension::D2,
            usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT,
        };
        let texture = RhiDevice::create_texture(ctx, &desc).map_err(SwapchainError::DepthImage)?;

        // Destroy the previous depth image (the device was waited idle above, so no
        // submission references it).
        if let Some(old) = self.depth.take() {
            // SAFETY: the old depth texture was created on `ctx` by a prior
            // `sync_depth`; the device was waited idle above so its last referencing
            // frame completed; the by-value move destroys it exactly once.
            unsafe { RhiDevice::destroy_texture(ctx, old.texture) };
        }

        self.depth = Some(DepthImage { texture, extent });
        Ok(())
    }

    /// Tears down the scene's owned resources (depth image, vertex buffer, graphics
    /// pipeline) through `ctx`, consuming `self`. The caller MUST have made the
    /// device idle (e.g. dropped the [`Renderer`], whose `Drop` waits idle) so no
    /// submission still references them.
    ///
    /// # Safety
    ///
    /// `ctx` is the live context the scene's resources were created on; no GPU work
    /// referencing them is in flight (caller `wait_idle`'d / dropped the renderer);
    /// each is destroyed exactly once (the by-value `self` enforces the latter).
    pub unsafe fn destroy(mut self, ctx: &VulkanContext) {
        // SAFETY: per the contract `ctx` is live and nothing references these
        // resources; each was created on `ctx` and is destroyed exactly once, in
        // reverse acquisition order (depth → vertex buffer → pipeline).
        unsafe {
            if let Some(depth) = self.depth.take() {
                RhiDevice::destroy_texture(ctx, depth.texture);
            }
            RhiDevice::destroy_buffer(ctx, self.vertex_buffer);
            RhiDevice::destroy_graphics_pipeline(ctx, self.pipeline);
        }
    }
}

/// The rung-11 on-screen hybrid-composite present inputs: the ALREADY-UPLOADED,
/// already-resident SAMPLED texture the compute composite lives in, the sampler +
/// bind group binding it, and the fullscreen-sample graphics pipeline that samples
/// it into the swapchain image.
///
/// # The texture is resident + read-only BEFORE this bundle is built
///
/// The composite is STATIC across the whole present loop (it never changes between
/// frames), so the caller uploads the compute composite into `texture` and
/// transitions it to `SHADER_READ_ONLY_OPTIMAL` EXACTLY ONCE — in its own fenced
/// submit (or folded into the composite-producing submit) — BEFORE the present loop.
/// From then on the texture stays in `SHADER_READ_ONLY_OPTIMAL` permanently and
/// [`Renderer::present_sampled`] only ever READS it (a `FRAGMENT_SHADER` sample).
/// Multiple frames-in-flight concurrently reading a read-only texture is sound — no
/// write-after-read hazard, no per-frame upload, no per-frame barrier, and no
/// cross-frame fence on the texture. This is what makes the present loop sound across
/// `FRAMES_IN_FLIGHT` (the bundle carries no source buffer / copy extent precisely
/// because the present path performs no copy).
///
/// Unlike [`Scene`] (which OWNS its resources and is destroyed by value), this is a
/// lightweight BORROW bundle: the caller creates the resources through the
/// [`RhiDevice`] trait, owns them, and tears them down (the
/// `'a` lifetime ties the bundle to those borrows for the present call). It exists
/// only to keep [`Renderer::present_sampled`]'s signature compact.
///
/// The pipeline's declared `color_formats[0]` MUST equal the swapchain's color
/// format (the W2-b format-matching contract) and its layout MUST declare
/// `bind_group`'s set-0 layout (one COMBINED_IMAGE_SAMPLER), or the validation layer
/// faults at draw time.
///
/// # Native-size, top-left present
///
/// The composite is presented at its NATIVE size ([`texture_extent`](Self::texture_extent))
/// in the top-left of the swapchain image — never stretched to the (possibly
/// WSI-clamped) swapchain extent. See [`texture_extent`](Self::texture_extent).
pub struct SampledComposite<'a> {
    /// The SAMPLED texture the compute composite has ALREADY been uploaded into and
    /// transitioned to `SHADER_READ_ONLY_OPTIMAL` (caller's pre-loop one-time
    /// submit). The present path only samples it.
    pub texture: &'a VulkanTexture,
    /// The sampler bound alongside `texture` in `bind_group`. Not read by the present
    /// path directly; the bind group already references it. Kept here as a lifetime
    /// tie so the sampler outlives the bind group's use.
    pub sampler: &'a VulkanSampler,
    /// The bind group (one COMBINED_IMAGE_SAMPLER at set 0) binding `texture` +
    /// `sampler` for the fullscreen-sample draw.
    pub bind_group: &'a VulkanBindGroup,
    /// The fullscreen-sample graphics pipeline (no vertex buffer, no depth; its
    /// `color_formats[0]` equals the swapchain format, W2-b).
    pub pipeline: &'a VulkanGraphicsPipeline,
    /// The `texture`'s OWN dimensions (the composite's native size), NOT the
    /// swapchain extent.
    ///
    /// [`Renderer::present_sampled`] presents the composite at its native size in
    /// the TOP-LEFT of the swapchain image: it sets the present pass's
    /// viewport/scissor to `min(swapchain_extent, texture_extent)`, so the
    /// fullscreen-sample triangle writes exactly the composite's pixels 1:1 and the
    /// rest of the (possibly wider, WSI-clamped) swapchain image stays the clear
    /// color. A 1:1 top-left mapping makes a per-texel golden exact regardless of
    /// any WSI `current_extent` clamp (e.g. a driver-minimum swapchain width wider
    /// than the texture).
    ///
    /// This denotes the TEXTURE, never the swapchain — passing the swapchain extent
    /// here would re-introduce the stretch this field exists to remove.
    pub texture_extent: VkExtent2D,
}

/// The on-screen UI rect sub-pass inputs (GUI P5a Rung 5 / Decision 9), recorded by
/// [`Renderer::present_sampled`] into the SAME swapchain `cmd` AFTER the composite
/// scope ends and BEFORE the COLOR→PRESENT barrier.
///
/// All fields are CONCRETE `boyko_rhi_vulkan` handles + POD — `boyko_rhi_vulkan` does
/// not (and must not) depend on `boyko_render`, so the caller (the render host, which
/// owns the `RhiContext`) RE-RESOLVES the current-frame UI pipeline + bind-group by
/// `frame_index` (`RhiContext::ui_handles`, MF-7) and passes them here by reference,
/// together with the instance count + the 16-byte ortho push block. The pass opens
/// its OWN `begin_rendering(LoadOp::Load)` at the FULL swapchain extent (preserving
/// the composited scene), so a rect at the bottom-right corner lands at the
/// bottom-right swapchain texel (the ortho denominator = the swapchain extent).
///
/// A pass with `instance_count == 0` records NOTHING (no empty draw, no UI scope).
pub struct UiPass<'a> {
    /// The UI graphics pipeline (vertexless quad, blend = premultiplied, its
    /// `color_formats[0]` equals the swapchain format — W2-b). Re-resolved by the
    /// caller from the current `frame_index`.
    pub pipeline: &'a VulkanGraphicsPipeline,
    /// The current-FIF ring's bind-group (one STORAGE buffer @ set0/binding0). The
    /// backing ring holds `instance_count` valid `UiInstance` records uploaded for
    /// THIS frame index before this draw. Re-resolved by the caller.
    pub bind_group: &'a VulkanBindGroup,
    /// The number of UI instances to draw (`draw(6, instance_count, 0, 0)`); `0`
    /// records nothing.
    pub instance_count: u32,
    /// The 16-byte pixel→NDC ortho push block (`UiOrtho` byte image), pushed to the
    /// pipeline's VERTEX range. Borrowed for the record call only.
    pub ortho_bytes: &'a [u8],
}

/// The byte size of the marcher's MVP push constant (a `float4x4`), pushed to the
/// mesh-raster pipeline's `VERTEX` range each on-screen G-buffer frame (Render P1c).
pub const GBUFFER_MVP_BYTES: usize = 64;

/// The byte size of the marcher's COMPUTE push constant — DERIVED from the
/// [`FineMarcherPush`](crate::compute::FineMarcherPush) `#[repr(C)]` struct (Render A1/A2
/// widened it 8 → 32 bytes: it now carries `lighting_flags` @8 + `light_dir` @16 alongside
/// the P4b `coarse_enabled` @0 + the B1 `omega` @4). The windowed path pushes
/// `coarse_enabled = 0` (the coarse cull pass is not run on-screen), `omega =
/// DEFAULT_MARCHER_OMEGA` (the B1 over-relaxation speedup), and lighting ON with the
/// default directional light (the demo). It is a subset of the marcher pipeline's declared
/// 80-byte (`COMPOSITE_PUSH_CONSTANT_BYTES`) range.
const GBUFFER_MARCHER_PUSH_BYTES: u32 = crate::compute::GBUFFER_MARCHER_PUSH_BYTES;

/// The Lighting-L1 cull pipeline's COMPUTE push range size (16 B
/// [`crate::compute::ClusterCullPush`]). Re-exported so [`GBufferScene::cluster_cull_push`]
/// can size its inline byte array without depending on `compute` at the field-decl site.
const CLUSTER_CULL_PUSH_BYTES: u32 = crate::compute::CLUSTER_CULL_PUSH_BYTES;

/// The Lighting-L1 cull shader's `[numthreads(64,1,1)]` group width. The cull's 1D dispatch
/// group count is `ceil(cluster_count / LIGHT_CULL_LOCAL_SIZE_X)`.
const LIGHT_CULL_LOCAL_SIZE_X: u32 = 64;

/// The runtime brick-cache activation the windowed/offscreen G-buffer present applies to the
/// marcher's [`FineMarcherPush`] (the SDF brick-atlas campaign — empty-skip + trilinear/cubic
/// surface cache + clip-map LOD). `None` on [`GBufferScene::brick`] is the OFF path: the recorder
/// builds the push exactly as before (`brick_enabled == 0` / `brick_trilinear == 0` /
/// `brick_levels == 1`), byte-identical to the pre-brick command stream. `Some(_)` turns the brick
/// path ON per-frame, so the caller can flip it at runtime (an A/B toggle) without re-recording any
/// pipeline — the gates live entirely in the per-frame push.
///
/// When `Some`, the recorder stamps the empty-skip grid uniforms (`grid_origin`/`grid_dims`/
/// `brick_world` — level 0's [`boyko_sdf_math::brick::PointerGrid`] geometry the marcher's `lvl == 0`
/// arm indexes binding 9 with) via [`FineMarcherPush::with_brick`], turns on the trilinear+cubic
/// surface path via [`FineMarcherPush::with_brick_trilinear`], and sets the clip-map level count via
/// [`FineMarcherPush::with_brick_levels`]. The per-level atlas/grid SSBOs the marcher samples MUST
/// already be bound at bindings 9..=14 (via the [`GBufferScene`]'s `pointer_grid` / `atlas` /
/// `level_grids` / `level_atlases` fields — pointed at a real [`crate::brick_atlas::BrickClipmap`]),
/// and the b5 camera UBO's `M4GridParams` tail (offset 80) MUST hold the clip-map's baked per-level
/// origins — exactly the offscreen RTX-verified binding discipline. This struct carries ONLY the
/// push-side gates; the descriptor binding + the UBO tail are the caller's (they are extent-stable,
/// written once, NOT per frame).
///
/// `#[repr(C)]`, `Copy` — a small POD the caller flips each frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrickActivation {
    /// Level 0's empty-skip pointer-grid minimum world corner (cell `(0,0,0)`'s min). The marcher's
    /// `lvl == 0` arm indexes binding 9 with `(grid_origin, grid_dims, brick_world)` — this MUST equal
    /// the [`boyko_sdf_math::brick::PointerGrid`] geometry the level-0 grid bound at binding 9 was
    /// baked at (`PointerGrid::default_near_field().origin` for the origin-centered demo clip-map).
    pub grid_origin: [f32; 3],
    /// Level 0's pointer-grid cell count per axis (`PointerGrid::dims`, e.g. `[16, 16, 16]`).
    pub grid_dims: [u32; 3],
    /// Level 0's pointer-grid cell size — the world width of one brick cell (`PointerGrid::brick_world`,
    /// e.g. `0.5`).
    pub brick_world: f32,
    /// The clip-map level count the marcher loops over (`with_brick_levels`): `BRICK_LEVELS` (3) for the
    /// full clip-map, `1` for the single-level near-field cache. `0` is treated as OFF by the shader.
    pub levels: u32,
}

/// The on-screen Render-P1c G-buffer frame's STATIC inputs: the resources the
/// [`Renderer::render_gbuffer_frame`] 3-pass needs that do NOT depend on the
/// (WSI-clamped) swapchain extent. The EXTENT-dependent targets (depth + the MRT
/// G-buffer images + the descriptor sets bound against them) are owned by
/// [`GBufferTargets`] and (re)allocated by [`GBufferTargets::sync_gbuffer`].
///
/// This is the P1c on-screen counterpart of the P1b OFFSCREEN driver
/// (`tests/sdf_gbuffer_hybrid.rs::run_gbuffer_hybrid`): it mirrors the SAME
/// vocabulary (SSBO edit-list, sampled depth, albedo/normal/material storage, camera
/// UBO) and the SAME marcher compute pipeline, but routes the marcher's ALBEDO
/// (final composite) onto the swapchain image via a present-blit (pass C) instead of a
/// `copy_image_to_buffer` readback.
///
/// # Borrow bundle (like [`SampledComposite`])
///
/// The caller creates each resource through the [`RhiDevice`] trait, OWNS it, and
/// tears it down; the `'a` lifetime ties the bundle to those borrows for the frame
/// call. The bundle keeps [`Renderer::render_gbuffer_frame`]'s signature compact and
/// keeps the recorder out of the resource-creation business (the P1b marcher +
/// G-buffer + present-blit are REUSED verbatim).
///
/// # Static inputs only — the camera UBO + SSBO are seeded ONCE
///
/// The camera/extent UBO (`camera_uniform`) and the edit-list SSBO (`edit_list`) are
/// host-seeded by the caller BEFORE the present loop and are READ-ONLY for the
/// marcher across every frame — so multiple frames-in-flight may dispatch against
/// them with no host write-after-read hazard (the SAME read-only-resident contract
/// [`SampledComposite`] relies on for its texture). The vocabulary descriptor set
/// that binds them is written ONCE per extent in [`GBufferTargets::sync_gbuffer`],
/// NEVER per frame.
///
/// # W2-b format contracts
///
/// `raster_pipeline`'s declared depth format MUST be [`Format::D32Sfloat`] (the
/// depth image the recorder rasterizes into) and its single color format the
/// throwaway-raster format; `marcher`'s layout MUST declare `vocab_layout` at `set
/// 0`; `present_pipeline`'s `color_formats[0]` MUST equal the swapchain format and
/// its layout MUST declare `present_layout` (one COMBINED_IMAGE_SAMPLER) — or the
/// validation layer faults at record/draw time.
/// The Render P7 SSAO compute pass activation: the SSAO pipeline + its DEDICATED 5-binding
/// bind-group LAYOUT, threaded into [`GBufferScene::ssao`] as `Some` to turn the SSAO pass ON.
///
/// `None` on [`GBufferScene::ssao`] is the OFF path: the recorder records NOTHING new (no SSAO
/// descriptor-set write in [`GBufferTargets::create`], no transition / dispatch / barrier in
/// [`Renderer::record_gbuffer`]), so the command stream is BYTE-IDENTICAL to the pre-P7 path
/// (the 0%-gate — proven by C1, since the `ssao` image is always allocated + transitioned
/// regardless of this field). The resolve's `ssao_mode` header gate must be set in lock-step:
/// `Some` ⇒ the scene's light table carries `ssao_mode != 0`; `None` ⇒ `ssao_mode == 0` (the
/// resolve then never reads the SSAO image, so the un-written contents are irrelevant).
///
/// # Borrow bundle (like the marcher / resolve pipelines)
///
/// The caller OWNS the SSAO pipeline + layout and tears them down; the `'a` lifetime ties this
/// activation to those borrows for the frame call. [`GBufferTargets`] writes a 5-binding
/// `ssao_set` against [`Self::layout`] ONCE per extent in [`GBufferTargets::sync_gbuffer`] —
/// binding { gNormal @0 (R), gMaterial @1 (R), gViewT @2 (R), the `ssao` out image @3 (W), the
/// camera UBO @4 } — exactly the SSAO shader's interface (`sdf_ssao.comp.hlsl`).
///
/// `#[derive(Clone, Copy)]` — a pair of borrows the caller flips between frames with no re-record.
#[derive(Clone, Copy)]
pub struct SsaoActivation<'a> {
    /// The Render P7 SSAO compute pipeline (`sdf_ssao.comp` / [`crate::compute::sdf_ssao_spirv`]):
    /// its layout declares [`Self::layout`] at `set 0` + the 80-byte COMPUTE push range (the
    /// shared `CompositePushConstants` range — the SSAO pass reads its camera from the UBO @4, so
    /// it pushes NO constant, but the layout's range must match for create-time validity). The
    /// recorder binds it + dispatches `dispatch_group_count_x` BEFORE the resolve.
    pub pipeline: &'a ComputePipeline,
    /// The DEDICATED 5-binding SSAO bind-group LAYOUT { gNormal STORAGE image @0, gMaterial
    /// STORAGE image @1, gViewT STORAGE image @2, the `ssao` out STORAGE image @3, the camera
    /// UNIFORM buffer @4 } — matching `sdf_ssao.comp`'s set 0. The renderer writes a `ssao_set`
    /// against it once per extent (pointing at the per-extent G-buffer + `ssao` images + the
    /// scene's camera UBO).
    pub layout: &'a VulkanBindGroupLayout,
}

pub struct GBufferScene<'a> {
    /// The mesh-raster graphics pipeline (pass A). Render P5-r0: a 3-MRT G-buffer
    /// PRODUCER — the fronto-parallel quad is drawn into the D32 depth image AND the three
    /// RGBA8 G-buffer color attachments (albedo@0, normal@1, material@2) in the marcher's
    /// exact encoding (mask=1). The caller MUST build it with `color_formats =
    /// [R8G8B8A8_UNORM; 3]` + 3 per-target blend states (W2-b) and the new
    /// `gbuffer_mrt.{vs,fs}` shader pair. (Pre-P5 it was a depth-only prepass with one
    /// throwaway color format.)
    pub raster_pipeline: &'a VulkanGraphicsPipeline,
    /// The mesh quad's host-visible vertex buffer (position + color).
    pub vertex_buffer: &'a BoundBuffer,
    /// The number of vertices to `draw` (the mesh quad's vertex count, e.g. 6).
    pub vertex_count: u32,
    /// The 64-byte `float4x4` MVP pushed to `raster_pipeline`'s `VERTEX` range.
    pub mvp: [u8; GBUFFER_MVP_BYTES],
    /// The P1b SDF G-buffer marcher compute pipeline (its layout declares
    /// `vocab_layout` at `set 0`). Byte-untouched from P1b (pass B).
    pub marcher: &'a ComputePipeline,
    /// The vocabulary bind-group LAYOUT { SSBO @0, sampled depth @1, storage albedo
    /// @2, storage normal @3, storage material @4, UNIFORM camera @5, STORAGE tiles
    /// @6, STORAGE material-table @7, STORAGE `gViewT` @8, STORAGE `PointerGrid` @9,
    /// COMBINED_IMAGE_SAMPLER `BrickAtlas` @10 (M2) }. 11 bindings, within the 12-binding cap. The
    /// renderer allocates + writes a SET against it once per extent (pointing at the
    /// per-extent G-buffer + `gViewT` images + the bundle's `edit_list` / `camera_uniform` /
    /// `depth_sampler` / `tiles_buffer` / `material_table` / `pointer_grid`). PBR MVP-2 added
    /// binding 7 (the material SSBO the marcher fetches `base_color` from); Lighting L0b added
    /// binding 8 (the `gViewT` lane the marcher stores the surface `t` into); M1 added binding
    /// 9 (the empty-skip `PointerGrid`).
    ///
    /// The caller MUST declare binding 6 = `DescriptorKind::StorageBuffer`
    /// (`ShaderStage::COMPUTE`): the P4b marcher shader unconditionally DECLARES `[Set
    /// 0, Binding 6, "Tiles"]`, so the layout + the bound set must carry a VALID
    /// descriptor there even when the coarse cull is gated OFF (`coarse_enabled == 0`).
    ///
    /// The caller MUST likewise declare binding 9 = `DescriptorKind::StorageBuffer`
    /// (`ShaderStage::COMPUTE`): the M1 marcher SPIR-V STATICALLY references
    /// `StructuredBuffer<uint> PointerGrid : register(t9)` inside the empty-skip branch (DXC
    /// does NOT dead-strip the reference despite the runtime `brick_enabled` gate), so the
    /// layout + the bound set must carry a VALID descriptor there even when the empty skip is
    /// gated OFF (`brick_enabled == 0`, the windowed-present path), or
    /// `vkCreateComputePipelines` / `vkCmdDispatch` fail validation
    /// (VUID-…-layout-07988 / -08114).
    ///
    /// The caller MUST likewise declare binding 10 = `DescriptorKind::CombinedImageSampler`
    /// (`ShaderStage::COMPUTE`): the M2 marcher SPIR-V STATICALLY references
    /// `Texture3D BrickAtlas : register(t10)` + `SamplerState BrickSampler : register(s10)`
    /// (collapsed to ONE combined descriptor by DXC) inside the runtime-gated `brick_trilinear`
    /// branch, so the layout + the bound set must carry a VALID combined image+sampler there even
    /// when the trilinear path is gated OFF (`brick_trilinear == 0`, the windowed-present path), or
    /// the SAME layout VUIDs fail.
    ///
    /// The caller MUST likewise declare the M4 clip-map (Slice C) LEVEL-1 + LEVEL-2 bindings:
    /// 11 = `StorageBuffer` (`PointerGrid1`@t11), 12 = `CombinedImageSampler` (`BrickAtlas1`@t12),
    /// 13 = `StorageBuffer` (`PointerGrid2`@t13), 14 = `CombinedImageSampler` (`BrickAtlas2`@t14).
    /// The M4 marcher SPIR-V STATICALLY references all four inside the runtime level branch-ladder
    /// (DXC keeps them past the `brick_levels` gate), so the layout + set must bind VALID descriptors
    /// even on the OFF/N=1 path (`brick_levels == 1` takes only the lvl==0 arm → bound-but-unread).
    /// 6 brick bindings total (9..=14) under the 16-binding cap (`MAX_BIND_GROUP_BINDINGS`).
    pub vocab_layout: &'a VulkanBindGroupLayout,
    /// The edit-list StorageBuffer (binding 0), host-seeded ONCE before the loop.
    pub edit_list: &'a BoundBuffer,
    /// The camera/extent UNIFORM buffer (binding 5), host-seeded ONCE before the loop
    /// (at the WSI-clamped extent the recorder dispatches).
    pub camera_uniform: &'a BoundBuffer,
    /// The P4b per-tile coarse-cull StorageBuffer (binding 6), sized to the full tile
    /// grid (`tile_grid_extent(w, h)` → `tw * th * TILE_BOUND_BYTES`, STORAGE usage).
    ///
    /// The windowed present path runs the marcher with the coarse cull GATED OFF
    /// (`coarse_enabled == 0`), so the marcher never reads this buffer's contents — but
    /// the marcher shader unconditionally DECLARES binding 6, so Vulkan requires a
    /// VALID StorageBuffer descriptor bound there regardless. The scene OWNS this
    /// buffer; [`GBufferTargets`] only borrows it into the vocabulary set.
    pub tiles_buffer: &'a BoundBuffer,
    /// The M1 empty-space-skip `PointerGrid` StorageBuffer (vocab binding 9): the dense
    /// `dims.0 × dims.1 × dims.2` lattice of [`boyko_sdf_math::brick::BrickClass`] codes
    /// (one `u32` each — the GPU `StructuredBuffer<uint>` element), baked from the ONE edit
    /// authority via [`boyko_sdf_math::brick::build_pointer_grid`] (principle 0 — no parallel
    /// field store) and host-seeded ONCE before the loop, exactly like `edit_list`.
    ///
    /// The windowed present path runs the marcher with the empty skip GATED OFF
    /// (`brick_enabled == 0`), so the marcher NEVER reads this buffer's contents — the
    /// on-screen output stays BYTE-IDENTICAL to the pre-M1 marcher. But the M1 marcher SPIR-V
    /// STATICALLY references `PointerGrid : register(t9)` (DXC keeps the reference past the
    /// runtime gate), so Vulkan requires a VALID StorageBuffer descriptor bound at binding 9
    /// regardless. The scene OWNS this buffer; [`GBufferTargets`] only borrows it into the
    /// vocabulary set. Activating the empty skip on-screen is a separate step
    /// (`FineMarcherPush::with_brick`) — NOT done here.
    pub pointer_grid: &'a BoundBuffer,
    /// The M2 brick-atlas 3D image (vocab binding 10): the dense `M2_ATLAS_DIM³` `R8_SNORM`
    /// (or `R16_SFLOAT` fallback) tile-grid, baked from the ONE edit authority via
    /// [`crate::compute::bake_brick_atlas`] (principle 0 — no parallel field store; the atlas is a
    /// transient GPU mirror rebuilt on the edit `gen`). Created + filled by
    /// [`crate::brick_atlas::BrickAtlas`]; pass [`BrickAtlas::texture`](crate::brick_atlas::BrickAtlas::texture).
    ///
    /// Bound as a `COMBINED_IMAGE_SAMPLER` at binding 10 (with [`Self::atlas_sampler`]): the M2
    /// marcher SPIR-V STATICALLY references `Texture3D BrickAtlas : register(t10)` +
    /// `SamplerState BrickSampler : register(s10)` (collapsed to ONE combined descriptor by DXC)
    /// inside the runtime-gated `brick_trilinear` branch, so the layout MUST declare binding 10 =
    /// `DescriptorKind::CombinedImageSampler` and bind a VALID atlas here even when the trilinear
    /// path is gated OFF (the windowed present path runs `brick_trilinear == 0` → bound-but-unread,
    /// byte-identical output), or `vkCreateComputePipelines` / `vkCmdDispatch` trip the layout VUIDs
    /// (the M1 R2 lesson at binding 9). Activating the trilinear path on-screen is a separate step
    /// (`FineMarcherPush::with_brick_trilinear`) — NOT done here.
    pub atlas: &'a VulkanTexture,
    /// The M2 brick-atlas trilinear / clamp-to-edge / no-mip sampler (vocab binding 10, alongside
    /// [`Self::atlas`] in the combined-image-sampler). Pass
    /// [`BrickAtlas::sampler`](crate::brick_atlas::BrickAtlas::sampler). The hardware trilinear
    /// fetch decodes the `R8_SNORM`/`R16_SFLOAT` codes; clamp keeps an out-of-tile fetch reading the
    /// apron, not a neighbour tile.
    pub atlas_sampler: &'a VulkanSampler,
    /// The M4 clip-map LEVEL-1 + LEVEL-2 pointer grids (vocab bindings 11 + 13): the coarser levels'
    /// `M2_GRID_DIM³` empty-skip lattices ([`crate::brick_atlas::BrickClipmap::grid_buffer`]). The M4
    /// marcher SPIR-V STATICALLY references `PointerGrid1 : register(t11)` + `PointerGrid2 :
    /// register(t13)` inside the runtime level branch-ladder (DXC keeps them past the gate), so the
    /// layout MUST declare bindings 11/13 = `StorageBuffer` and bind VALID buffers even on the OFF/N=1
    /// path (`brick_levels == 1` takes only the lvl==0 arm → bound-but-unread). With no clipmap, bind
    /// level 0's grid ([`Self::pointer_grid`]) as a benign duplicate; `[0]` = level 1, `[1]` = level 2.
    pub level_grids: [&'a BoundBuffer; 2],
    /// The M4 clip-map LEVEL-1 + LEVEL-2 brick atlases (vocab bindings 12 + 14): the coarser levels'
    /// `M2_ATLAS_DIM³` tile-grids ([`crate::brick_atlas::BrickClipmap::atlas`]'s texture). The M4
    /// marcher SPIR-V STATICALLY references `BrickAtlas1 : register(t12)` + `BrickAtlas2 :
    /// register(t14)` (each a COMBINED_IMAGE_SAMPLER with [`Self::level_atlas_samplers`]) inside the
    /// branch-ladder, so the layout MUST declare bindings 12/14 = `CombinedImageSampler` and bind VALID
    /// atlases even on the OFF/N=1 path (bound-but-unread). With no clipmap, bind level 0's atlas
    /// ([`Self::atlas`]) as a benign duplicate; `[0]` = level 1, `[1]` = level 2.
    pub level_atlases: [&'a VulkanTexture; 2],
    /// The M4 clip-map LEVEL-1 + LEVEL-2 atlas samplers (vocab bindings 12 + 14, alongside
    /// [`Self::level_atlases`]). NEAREST / clamp-to-edge / no-mip like [`Self::atlas_sampler`]. With no
    /// clipmap, bind level 0's sampler; `[0]` = level 1, `[1]` = level 2.
    pub level_atlas_samplers: [&'a VulkanSampler; 2],
    /// The sampler bound alongside the depth image at binding 1 (ignored by the
    /// marcher's unfiltered `.Load`, but the SAMPLED_IMAGE descriptor requires one).
    pub depth_sampler: &'a VulkanSampler,
    /// The fullscreen-sample present pipeline (pass C): samples the LIT image (the
    /// deferred resolve's output) into the swapchain (`color_formats[0]` == the swapchain
    /// format). The deferred split rewired this from ALBEDO → LIT (the only present change).
    pub present_pipeline: &'a VulkanGraphicsPipeline,
    /// The present-sample bind-group LAYOUT (one COMBINED_IMAGE_SAMPLER @ set 0). The
    /// renderer allocates + writes a SET against it once per extent (pointing at the
    /// per-extent LIT image + `present_sampler`).
    pub present_layout: &'a VulkanBindGroupLayout,
    /// The sampler the present-blit samples the LIT image with (nearest/clamp for
    /// a 1:1 sample).
    pub present_sampler: &'a VulkanSampler,
    /// The PBR MVP-2 material table SSBO (`MaterialGpu[]`), host-seeded ONCE before the
    /// loop. Bound at the marcher vocab set's binding 7 (the marcher fetches `base_color`)
    /// AND the resolve set's binding 4 (the resolve fetches metallic/roughness/etc.). The
    /// scene OWNS it; [`GBufferTargets`] borrows it into both sets.
    pub material_table: &'a BoundBuffer,
    /// The Lighting-L0 light table SSBO (`[LightHeaderGpu || GpuLight[]]`, word-indexed;
    /// `light_table.hlsli`). A DEVICE-LOCAL buffer minted with `TRANSFER_DST | STORAGE`
    /// usage, bound to the resolve set's binding 6. Seeded ONCE via the fence-waited
    /// `upload_initial`; re-uploaded on-change via the async recorded copy below (C3 /
    /// rung L0-r0). The scene OWNS it; [`GBufferTargets`] borrows it into the resolve set.
    pub light_table: &'a BoundBuffer,
    /// The host-coherent STAGING source for the light table (rung L0-r0). On a dirty
    /// frame the recorder copies `light_upload_bytes` from this into `light_table` +
    /// records a TRANSFER_WRITE→SHADER_READ barrier, fence-free, BEFORE the marcher
    /// dispatch. The collection system writes the new table into this buffer's mapped
    /// bytes and sets `light_dirty`.
    pub light_staging: &'a BoundBuffer,
    /// The number of bytes to copy on a dirty frame (`[header || GpuLight[]]` length).
    pub light_upload_bytes: u64,
    /// `true` on a frame where the light table changed: the recorder records the async
    /// staging→`light_table` copy + barrier; `false` records NOTHING (idle frame → zero
    /// cost, byte-identical command stream — the rung L0-r0 0%-gate).
    pub light_dirty: bool,
    /// The Lighting-L1 clustered froxel light-cull compute pipeline (`cluster_cull.comp`):
    /// its layout declares the cull bind-group LAYOUT at `set 0` + a 16-byte
    /// [`crate::compute::ClusterCullPush`] COMPUTE push range. `None` ⇒ L1 is not wired (the
    /// L0b-only build) and the cull pass + its barriers are skipped entirely (the resolve's
    /// `clusters_enabled` header gate then loops the flat table — the L1 OFF path). When
    /// `Some`, the recorder dispatches it (over [`Self::cluster_count`] froxels) BEFORE the
    /// resolve, with a COMPUTE→COMPUTE buffer barrier so the resolve reads see the cull writes.
    pub cluster_cull: Option<&'a ComputePipeline>,
    /// The cull bind-group LAYOUT { camera UBO @0, light table SSBO @1, `ClusterGrid` SSBO
    /// @2, `LightIndexList` SSBO @3, `LightIndexAlloc` SSBO @4 } — matching `cluster_cull.hlsl`'s
    /// set 0. The renderer writes a `cull_set` against it once per extent (pointing at the
    /// scene's camera UBO + light table + cluster buffers). `None` when [`Self::cluster_cull`]
    /// is `None`.
    pub cull_layout: Option<&'a VulkanBindGroupLayout>,
    /// The L1 per-froxel `ClusterCell`/`{offset,count}` grid SSBO (`DEVICE_LOCAL`, STORAGE),
    /// sized `cluster_count * 8 B`. Written by the cull pass, read by the resolve set's
    /// binding 8. The scene OWNS it; [`GBufferTargets`] borrows it into both the cull set and
    /// the resolve set. `None` when L1 is off (the resolve set then binds the light table at
    /// @8/@9 as a harmless valid placeholder — see [`GBufferTargets::create`]).
    pub cluster_grid: Option<&'a BoundBuffer>,
    /// The L1 flat light-index list SSBO (`DEVICE_LOCAL`, STORAGE), sized `index_list_cap *
    /// 4 B`. The cull atomic-appends survivor indices; the resolve reads the pixel's froxel
    /// slice from it (resolve binding 9). `None` when L1 is off.
    pub light_index: Option<&'a BoundBuffer>,
    /// The L1 global slice-allocation counter SSBO (one `u32`, `DEVICE_LOCAL`, STORAGE). The
    /// cull `InterlockedAdd`s element 0 to claim disjoint `light_index` slices. It is RESET to
    /// 0 (a `cmd_fill_buffer`) before each cull dispatch (the per-frame rebuild). `None` when
    /// L1 is off.
    pub light_index_alloc: Option<&'a BoundBuffer>,
    /// The 16-byte [`crate::compute::ClusterCullPush`] bytes (exp-Z near/far + the caps) the
    /// cull pass pushes. Ignored when [`Self::cluster_cull`] is `None`.
    pub cluster_cull_push: [u8; CLUSTER_CULL_PUSH_BYTES as usize],
    /// The L1 froxel count (`dim_x * dim_y * dim_z`, default 3456) — the cull's 1D dispatch
    /// thread count (`ceil(cluster_count / LOCAL_SIZE_X)` groups). Ignored when L1 is off.
    pub cluster_count: u32,
    /// The deferred PBR RESOLVE compute pipeline (`deferred_pbr.comp`): its layout declares
    /// `resolve_layout` at `set 0`. Reads the marcher's gAlbedo + gNormal + gMaterial
    /// (STORAGE, GENERAL) + the material SSBO + the camera UBO, runs Cook-Torrance, and
    /// stores the final LIT color into the dedicated lit image.
    pub resolve_pipeline: &'a ComputePipeline,
    /// The deferred resolve bind-group LAYOUT (8 bindings, ≤ 12): { storage gAlbedo @0,
    /// storage gNormal @1, storage gMaterial @2, storage lit @3, material SSBO @4, camera
    /// UBO @5, light table SSBO @6, storage `gViewT` @7 }. The renderer allocates + writes a
    /// SET against it once per extent (pointing at the per-extent G-buffer + lit + `gViewT`
    /// images + the scene's material SSBO + camera UBO + light table). Binding 6 (Lighting
    /// L0a) replaces the compiled-in `LIGHT_DIR`/`SKY_*` constants with the header+table
    /// read; binding 7 (Lighting L0b) is the `gViewT` lane the resolve reconstructs `P` from.
    pub resolve_layout: &'a VulkanBindGroupLayout,
    /// The marcher's 1D dispatch group count (`ceil(pixels / LOCAL_SIZE_X)` at the
    /// WSI-clamped extent the recorder dispatches). The deferred resolve dispatches at the
    /// SAME grid (1:1 the marched pixels).
    pub dispatch_group_count_x: u32,
    /// The SDF brick-cache activation applied to the marcher push THIS frame. `None` = the OFF
    /// path (`brick_enabled == 0` / `brick_trilinear == 0` / `brick_levels == 1`), byte-identical
    /// to the pre-brick command stream — the bound brick descriptors at 9..=14 stay bound-but-unread.
    /// `Some(_)` turns the empty-skip + trilinear/cubic surface cache + clip-map LOD ON (the gates
    /// live entirely in the per-frame push, so the caller may flip this every frame for an A/B
    /// toggle). When `Some`, the caller MUST have bound the real [`crate::brick_atlas::BrickClipmap`]
    /// per-level resources at 9..=14 and written its `M4GridParams` tail into the b5 UBO (see
    /// [`BrickActivation`]).
    pub brick: Option<BrickActivation>,
    /// The P4b COARSE TILE-CULL compute pipeline (`sdf_tile_cull.comp`), applied to this frame.
    /// `None` = the OFF path (the default, byte-identical to the pre-P0 command stream): NO coarse
    /// dispatch + NO `tiles_buffer` barrier are recorded, and the marcher push carries
    /// `coarse_enabled == 0`, so the marcher never reads [`Self::tiles_buffer`]'s contents.
    ///
    /// `Some(coarse)` = the ON path (a PERF optimization, not a visual one): BEFORE the marcher
    /// (pass B), the recorder binds this pipeline against the SAME vocabulary descriptor set (the
    /// coarse-cull shader declares only a subset of the vocab layout — sharing the full layout is
    /// valid), dispatches `ceil(tile_count / LOCAL_SIZE_X)` groups (one invocation per 8×8 tile,
    /// each writing a `TileBound` into vocab binding 6 — [`Self::tiles_buffer`]), records a
    /// COMPUTE-WRITE → COMPUTE-READ buffer barrier on `tiles_buffer`, and then the marcher push
    /// carries `coarse_enabled == 1`, so the fine marcher reads the per-tile bounds and skips
    /// empty / cone-rejected tiles. The cull MUST NOT change pixels (only fewer marches), so the ON
    /// output equals the OFF output within the goldens' per-channel tolerance.
    ///
    /// `coarse`'s layout MUST declare [`Self::vocab_layout`] at `set 0` (it shares the marcher's
    /// vocabulary set verbatim) and the same compute push range; the depth image it samples is
    /// already `SHADER_READ_ONLY_OPTIMAL` (the dual-use depth barrier the recorder emits before pass
    /// B) and the `tiles_buffer` it writes is bound at vocab binding 6 (always — caller contract).
    /// Flipping `coarse` between frames needs NO re-record (it gates only the recorded dispatch +
    /// the push byte), so the caller may A/B-toggle it live.
    pub coarse: Option<&'a ComputePipeline>,
    /// The marcher's coarse-cull CONSUMPTION mode ([`CoarseMode`]) stamped into the push when
    /// [`Self::coarse`] is `Some` (when `None`, the recorder forces [`CoarseMode::Off`], so this
    /// field is a don't-care on the OFF path). The cull DISPATCH is identical across modes — only
    /// the marcher's reading of the per-tile bounds differs:
    ///
    /// - [`CoarseMode::Full`] — the historical EMPTY-skip + `near_t` seed (the offscreen goldens'
    ///   mode; image-transparent under the UNLIT contract).
    /// - [`CoarseMode::EmptySkipOnly`] — the LIT-TRANSPARENT cull: EMPTY-skip only, NO `near_t`
    ///   seed (the seed shifts the grazing-silhouette AO/shadow rim; dropping it removes the rim).
    ///   This is the on-screen windowed-present mode (lit-transparent, near-identical perf).
    ///
    /// A per-frame push field (no re-record on a flip). Defaults to [`CoarseMode::Off`].
    pub coarse_mode: CoarseMode,
    /// The A1/A2 lighting flags stamped into the marcher's [`FineMarcherPush`] `lighting_flags`
    /// (offset 8) THIS frame: `LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO` for the on-screen demo's
    /// soft-shadow + AO shading, or `0` for the byte-identical Lambert path.
    ///
    /// This is a per-frame push field (NOT a descriptor), so flipping it needs no re-record and the
    /// OFF (`coarse == None`) command stream stays byte-identical for any fixed value.
    ///
    /// # Why it is a field (the P0 coarse-cull transparency contract)
    ///
    /// The coarse cull ([`Self::coarse`]) is proven IMAGE-TRANSPARENT — cull-ON equals cull-OFF
    /// within the goldens' tolerance — by the offscreen golden
    /// `sdf_gbuffer_hybrid::p4b_cull_on_conservative_within_tol_of_cull_off`, which runs that
    /// comparison on the UNLIT marcher (`lighting_flags == 0`). With shadows + AO ON, the cull's
    /// conservative per-tile `near_t` / EMPTY classification (tuned for the primary hit test) is
    /// NOT transparent to the secondary AO / shadow rays near a grazing silhouette: a tile the cull
    /// deems empty-enough for the primary ray still owes an AO darkening the un-culled march would
    /// have produced, so the lit cull-ON image drops that darkening (a visible ring). That cull ⇄
    /// lighting interaction is a separate, un-shipped invariant — the shipped cull contract is the
    /// unlit one. Exposing `lighting_flags` lets a cull-transparency test compare under the proven
    /// (`0`) condition while the on-screen present keeps shadows + AO.
    pub lighting_flags: u32,
    /// The directional-light direction (the un-normalized "direction TO the light", `L`) the
    /// marcher marches the A1 soft shadow toward — stamped into the marcher's [`FineMarcherPush`]
    /// `light_dir` (offset 16) THIS frame. It MUST equal the resolve's PRIMARY directional light
    /// direction (the first directional in the light table, the one whose `vis` reads
    /// `gMaterial.r`): the marcher bakes the cast shadow toward `light_dir`, and the resolve
    /// consumes it as the primary directional's visibility — a mismatch detaches the shadow from
    /// the light. A per-frame push field (no re-record on a change). For a head-on scene use the
    /// legacy [`DEFAULT_LIGHT_DIR`](crate::compute::DEFAULT_LIGHT_DIR) `[0, 0, 1]`; an angled /
    /// floor-and-object scene supplies the real sun direction so a real cast shadow lands.
    pub light_dir: [f32; 3],
    /// The Render P7 SSAO compute pass activation. `None` = the OFF path (the default): the
    /// recorder records NOTHING new (no SSAO set-write, no transition / dispatch / barrier), so
    /// the command stream is BYTE-IDENTICAL to the pre-P7 path (the 0%-gate — the `ssao` image is
    /// allocated + transitioned regardless, C1). `Some(_)` = the ON path: [`GBufferTargets`]
    /// writes the 5-binding `ssao_set` against the activation's layout, and BEFORE the resolve the
    /// recorder binds the SSAO pipeline + that set, dispatches [`Self::dispatch_group_count_x`],
    /// and barriers the `ssao` image (COMPUTE→COMPUTE, GENERAL) so the resolve's `gSsao.Load` sees
    /// the store. The caller MUST set the scene's light table `ssao_mode` in lock-step (`!= 0` ON,
    /// `0` OFF) — the resolve's structural gate that decides whether the combine reads the image.
    pub ssao: Option<SsaoActivation<'a>>,
}

/// The per-extent on-screen G-buffer targets for [`Renderer::render_gbuffer_frame`]:
/// the D32 depth image (rasterize into + sample), the MRT storage G-buffer (albedo /
/// normal / material), and the two descriptor sets bound against them (the marcher
/// vocabulary set + the present-sample set). (Re)allocated ONLY on an extent change
/// by [`GBufferTargets::sync_gbuffer`] — NEVER per frame.
///
/// This is the renderer-owned counterpart of the [`Scene`]'s [`DepthImage`] (the
/// per-extent depth), generalized to the full image-based G-buffer + its descriptor
/// sets. Owned by value; torn down through [`GBufferTargets::destroy`].
///
/// # The descriptor sets are written ONCE per extent (NO per-frame update)
///
/// `vocab_set` binds {SSBO, sampled depth, albedo/normal/material storage, camera
/// UBO, P4b tiles SSBO, M1 pointer-grid SSBO} and `present_set` binds {ALBEDO combined-image-sampler}; both are written at
/// `create_bind_group` time inside `sync_gbuffer` and reused unchanged across every
/// frame at that extent. The recorder records NO `vkUpdateDescriptorSets` — only the
/// per-frame barriers + bind + dispatch + draw. On an extent change `sync_gbuffer`
/// waits the device idle, destroys the old targets, and rebuilds them (the same
/// belt-and-braces [`Scene::sync_depth`] uses).
pub struct GBufferTargets {
    /// The D32_SFLOAT depth image: DEPTH_STENCIL_ATTACHMENT (rasterize into) |
    /// SAMPLED (the marcher's `.Load`). Re-`UNDEFINED`'d every frame by the recorder.
    depth: VulkanTexture,
    /// The ALBEDO storage image (R8G8B8A8): the marcher's FINAL composite sink; also
    /// sampled by the present-blit (pass C). Render P5-r0: it additionally carries
    /// `COLOR_ATTACHMENT` usage — the mesh raster pass A writes it as MRT@0.
    albedo: VulkanTexture,
    /// The NORMAL storage image (R8G8B8A8): the PBR MVP-2 marcher's `(oct.x, oct.y,
    /// matid_lo, matid_hi)` attribute — the octahedral world normal in RG + the 16-bit
    /// material id in BA. NOW READ by the deferred resolve (STORAGE, GENERAL).
    normal: VulkanTexture,
    /// The MATERIAL storage image (R8G8B8A8): the PBR MVP-2 marcher's `(shadow, ao, mask)`
    /// attribute, consumed by the deferred resolve (STORAGE, GENERAL — never sampled).
    material: VulkanTexture,
    /// The LIT storage image (R8G8B8A8): the deferred resolve's OUTPUT (STORAGE store);
    /// also SAMPLED by the present-blit (pass C). The deferred split added it — the
    /// present now samples THIS (not albedo).
    lit: VulkanTexture,
    /// The Lighting-L0b `gViewT` lane (R32_SFLOAT STORAGE): the marcher stores the surface
    /// ray param `t`, the deferred resolve reads it (under `mask == 1`) to reconstruct
    /// `P = ro + rd * t`. Bound as an OUTPUT on the vocab set (binding 8) and an INPUT on
    /// the resolve set (binding 7). Transitioned UNDEFINED→GENERAL with the other G-buffer
    /// images and joins the marcher store → resolve load barrier.
    viewt: VulkanTexture,
    /// The Render P7 SSAO term `gSsao` (R8_UNORM STORAGE): the per-pixel HBAO-lite ambient
    /// occlusion the (C2) SSAO pass writes and the deferred resolve reads under the
    /// `ssao_mode != 0` gate. Bound as an INPUT on the resolve set (binding 11). ALWAYS
    /// allocated (the resolve descriptor interface is stable regardless of `ssao_mode`);
    /// transitioned UNDEFINED→GENERAL with `lit`/`viewt` and kept in GENERAL its whole life.
    /// No SSAO pass writes it yet (C2 adds that) — with `ssao_mode == 0` the resolve never
    /// reads it, so its undefined contents are irrelevant (the 0%-gate is the byte-identical
    /// PIXELS + command stream, which the always-allocate preserves).
    ssao: VulkanTexture,
    /// The marcher vocabulary descriptor set, written ONCE against
    /// [`GBufferScene::vocab_layout`] (pointing at `depth`/`albedo`/`normal`/`material`
    /// + the scene's SSBO/UBO/sampler + the M1 `pointer_grid` SSBO @9). NO per-frame update.
    vocab_set: VulkanBindGroup,
    /// The PBR MVP-2 RESOLVE descriptor set, written ONCE against
    /// [`GBufferScene::resolve_layout`] (10 bindings: `albedo` @0, `normal` @1, `material`
    /// @2, `lit` @3 STORAGE images, the material SSBO @4, the camera UBO @5, the L0a light
    /// table SSBO @6, the L0b `gViewT` STORAGE image @7, the L1 `ClusterGrid` SSBO @8, the L1
    /// `LightIndexList` SSBO @9, the P6 R1 SDF edit-list `Buf` SSBO @10, the Render P7 SSAO
    /// term `gSsao` STORAGE image @11). When L1 is off the scene's `cluster_grid`/`light_index`
    /// are `None`, so @8/@9 bind the light table as a harmless valid placeholder (the resolve's
    /// `clusters_enabled` header gate never reads them on the OFF path). `gSsao` @11 is always
    /// bound; the resolve reads it only under `ssao_mode != 0` (0 every pre-P7 scene). NO
    /// per-frame update.
    resolve_set: VulkanBindGroup,
    /// The Lighting-L1 CULL descriptor set, written ONCE against
    /// [`GBufferScene::cull_layout`] (camera UBO @0, light table SSBO @1, `ClusterGrid` SSBO
    /// @2, `LightIndexList` SSBO @3, `LightIndexAlloc` SSBO @4) — `None` when L1 is off
    /// ([`GBufferScene::cluster_cull`] is `None`). NO per-frame update.
    cull_set: Option<VulkanBindGroup>,
    /// The Render P7 SSAO descriptor set, written ONCE against [`SsaoActivation::layout`]
    /// (5 bindings: gNormal @0, gMaterial @1, gViewT @2 STORAGE images READ, the `ssao` out
    /// STORAGE image @3 WRITE, the camera UBO @4) — `None` when SSAO is off
    /// ([`GBufferScene::ssao`] is `None`). The recorder then skips the SSAO pass entirely (the
    /// 0%-gate, byte-identical command stream). NO per-frame update.
    ssao_set: Option<VulkanBindGroup>,
    /// The present-blit descriptor set, written ONCE against
    /// [`GBufferScene::present_layout`] (one COMBINED_IMAGE_SAMPLER pointing at
    /// `lit` + the scene's present sampler). NO per-frame update.
    present_set: VulkanBindGroup,
    /// The extent the images were created at (so [`GBufferTargets::sync_gbuffer`] can
    /// detect a resize and reallocate).
    extent: VkExtent2D,
}

/// The G-buffer color format (albedo / normal / material): `R8G8B8A8_UNORM`, the
/// STORAGE-image store target the marcher writes (matches the P1b offscreen driver's
/// `GBUFFER_FORMAT`). The ALBEDO image is also `SAMPLED` (the present-blit) — never
/// stretched; presented 1:1 in the swapchain's top-left like [`SampledComposite`].
const GBUFFER_FORMAT: Format = Format::R8G8B8A8Unorm;

/// The Lighting-L0b `gViewT` lane format: `R32_SFLOAT`, a STORAGE image the marcher
/// stores the full-fp32 surface ray param `t` into and the resolve reads to reconstruct
/// the world position `P = ro + rd * t`. fp32 (not a packed 8-bit lane) avoids the
/// attenuation/cone banding a low-precision `t` would cause. W2: `STORAGE_IMAGE` support
/// on this format is fail-fast-checked at device boot.
const GVIEWT_FORMAT: Format = Format::R32Sfloat;

/// The Render P7 SSAO term `gSsao` format: `R8_UNORM`, a single 8-bit ambient-occlusion lane
/// the (C2) SSAO pass stores and the deferred resolve loads under the `ssao_mode != 0` gate.
/// 8 bits is the engine AO tolerance (the A2 march lands in `gMaterial.g`, also 8-bit). P7:
/// `R8_UNORM`/`STORAGE_IMAGE` support is fail-fast-checked at device boot
/// ([`crate::device::DeviceCaps::r8_unorm_storage_ok`]), so the SSAO image create can never
/// fault on an unsupported format.
const SSAO_FORMAT: Format = Format::R8Unorm;

impl GBufferTargets {
    /// Creates a 2D `R8G8B8A8_UNORM` storage image at `extent` with `usage`. A small
    /// helper shared by the albedo/normal/material allocations in [`Self::create`].
    fn create_gbuffer_image(
        ctx: &VulkanContext,
        extent: VkExtent2D,
        usage: ImageUsage,
    ) -> Result<VulkanTexture, SwapchainError> {
        let desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format: GBUFFER_FORMAT,
            dimension: TextureDimension::D2,
            usage,
        };
        RhiDevice::create_texture(ctx, &desc).map_err(SwapchainError::DepthImage)
    }

    /// Creates the Lighting-L0b `gViewT` lane: a 2D `R32_SFLOAT` STORAGE image at `extent`
    /// (the marcher's surface ray param `t`). A separate helper from
    /// [`Self::create_gbuffer_image`] because the lane is `R32_SFLOAT`, not the RGBA8
    /// [`GBUFFER_FORMAT`]. W2: `R32_SFLOAT`/`STORAGE_IMAGE` support is fail-fast-checked
    /// at device boot ([`crate::device::DeviceCaps::viewt_storage_format_ok`]), so this
    /// create can never fault on an unsupported format.
    fn create_viewt_image(
        ctx: &VulkanContext,
        extent: VkExtent2D,
    ) -> Result<VulkanTexture, SwapchainError> {
        let desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format: GVIEWT_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE,
        };
        RhiDevice::create_texture(ctx, &desc).map_err(SwapchainError::DepthImage)
    }

    /// Creates the Render P7 SSAO term `gSsao`: a 2D `R8_UNORM` STORAGE image at `extent`
    /// (the per-pixel HBAO-lite ambient occlusion). A separate helper from
    /// [`Self::create_gbuffer_image`] because the lane is `R8_UNORM`, not the RGBA8
    /// [`GBUFFER_FORMAT`]. P7: `R8_UNORM`/`STORAGE_IMAGE` support is fail-fast-checked at
    /// device boot ([`crate::device::DeviceCaps::r8_unorm_storage_ok`]), so this create can
    /// never fault on an unsupported format.
    fn create_ssao_image(
        ctx: &VulkanContext,
        extent: VkExtent2D,
    ) -> Result<VulkanTexture, SwapchainError> {
        let desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format: SSAO_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE,
        };
        RhiDevice::create_texture(ctx, &desc).map_err(SwapchainError::DepthImage)
    }

    /// Allocates the depth + MRT G-buffer images at `extent` and writes the marcher
    /// vocabulary set + the present-sample set against them (ONCE). The caller
    /// ([`GBufferTargets::sync_gbuffer`]) destroys any prior targets + waits idle
    /// first; this only builds the new ones.
    ///
    /// On any partial failure every object created so far in this call is torn down
    /// in reverse order before the error returns (no leak on the error path), exactly
    /// like [`Scene::sync_depth`]'s build-before-teardown discipline.
    fn create(
        ctx: &VulkanContext,
        scene: &GBufferScene<'_>,
        extent: VkExtent2D,
    ) -> Result<Self, SwapchainError> {
        // Depth: DEPTH_STENCIL_ATTACHMENT (rasterize into) | SAMPLED (marcher .Load).
        let depth = {
            let desc = TextureDesc {
                width: extent.width,
                height: extent.height,
                depth: 1,
                format: Format::D32Sfloat,
                dimension: TextureDimension::D2,
                usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED,
            };
            RhiDevice::create_texture(ctx, &desc).map_err(SwapchainError::DepthImage)?
        };

        // Render P5-r0: the throwaway depth-prepass color attachment is DELETED — pass A
        // now binds the three REAL G-buffer images (albedo/normal/material) as MRT color
        // attachments, so a separate throwaway color image is obsolete.

        // ALBEDO: STORAGE (marcher store) | SAMPLED (the present-blit, pass C) |
        // COLOR_ATTACHMENT (Render P5-r0: the mesh raster pass A writes it as MRT@0).
        let albedo = match Self::create_gbuffer_image(
            ctx,
            extent,
            ImageUsage::STORAGE | ImageUsage::SAMPLED | ImageUsage::COLOR_ATTACHMENT,
        ) {
            Ok(t) => t,
            Err(e) => {
                // SAFETY: `depth` was created on `ctx` above; referenced by no submission;
                // destroyed exactly once on this error path.
                unsafe {
                    RhiDevice::destroy_texture(ctx, depth);
                }
                return Err(e);
            }
        };
        // NORMAL / MATERIAL: STORAGE (marcher store) | COLOR_ATTACHMENT (Render P5-r0: the
        // mesh raster pass A writes them as MRT@1 / MRT@2). Read by the deferred resolve.
        let normal = match Self::create_gbuffer_image(
            ctx,
            extent,
            ImageUsage::STORAGE | ImageUsage::COLOR_ATTACHMENT,
        ) {
            Ok(t) => t,
            Err(e) => {
                // SAFETY: the two textures above were created on `ctx`; referenced by
                // no submission; each destroyed exactly once on this error path.
                unsafe {
                    RhiDevice::destroy_texture(ctx, albedo);
                    RhiDevice::destroy_texture(ctx, depth);
                }
                return Err(e);
            }
        };
        let material = match Self::create_gbuffer_image(
            ctx,
            extent,
            ImageUsage::STORAGE | ImageUsage::COLOR_ATTACHMENT,
        ) {
            Ok(t) => t,
            Err(e) => {
                // SAFETY: the three textures above were created on `ctx`; referenced by
                // no submission; each destroyed exactly once on this error path.
                unsafe {
                    RhiDevice::destroy_texture(ctx, normal);
                    RhiDevice::destroy_texture(ctx, albedo);
                    RhiDevice::destroy_texture(ctx, depth);
                }
                return Err(e);
            }
        };
        // LIT: the deferred resolve's STORAGE store output; also SAMPLED by the
        // present-blit (pass C) and TRANSFER_SRC so an offscreen golden could read it back.
        let lit = match Self::create_gbuffer_image(
            ctx,
            extent,
            ImageUsage::STORAGE | ImageUsage::SAMPLED | ImageUsage::TRANSFER_SRC,
        ) {
            Ok(t) => t,
            Err(e) => {
                // SAFETY: the four textures above were created on `ctx`; referenced by
                // no submission; each destroyed exactly once on this error path.
                unsafe {
                    RhiDevice::destroy_texture(ctx, material);
                    RhiDevice::destroy_texture(ctx, normal);
                    RhiDevice::destroy_texture(ctx, albedo);
                    RhiDevice::destroy_texture(ctx, depth);
                }
                return Err(e);
            }
        };
        // Lighting L0b: the R32_SFLOAT `gViewT` lane (the marcher's surface `t`).
        let viewt = match Self::create_viewt_image(ctx, extent) {
            Ok(t) => t,
            Err(e) => {
                // SAFETY: the five textures above were created on `ctx`; referenced by
                // no submission; each destroyed exactly once on this error path.
                unsafe {
                    RhiDevice::destroy_texture(ctx, lit);
                    RhiDevice::destroy_texture(ctx, material);
                    RhiDevice::destroy_texture(ctx, normal);
                    RhiDevice::destroy_texture(ctx, albedo);
                    RhiDevice::destroy_texture(ctx, depth);
                }
                return Err(e);
            }
        };
        // Render P7: the R8_UNORM `gSsao` term (ALWAYS allocated — the resolve descriptor
        // interface is stable regardless of `ssao_mode`; no SSAO pass writes it yet, C2 adds
        // that). Read by the resolve only under `ssao_mode != 0` (0 every pre-P7 scene).
        let ssao = match Self::create_ssao_image(ctx, extent) {
            Ok(t) => t,
            Err(e) => {
                // SAFETY: the six textures above were created on `ctx`; referenced by
                // no submission; each destroyed exactly once on this error path.
                unsafe {
                    RhiDevice::destroy_texture(ctx, viewt);
                    RhiDevice::destroy_texture(ctx, lit);
                    RhiDevice::destroy_texture(ctx, material);
                    RhiDevice::destroy_texture(ctx, normal);
                    RhiDevice::destroy_texture(ctx, albedo);
                    RhiDevice::destroy_texture(ctx, depth);
                }
                return Err(e);
            }
        };

        // The marcher vocabulary set, written ONCE here (NO per-frame update). The
        // entry order matches the layout: SSBO @0, sampled depth @1, storage albedo @2,
        // storage normal @3, storage material @4, UNIFORM camera @5, STORAGE tiles @6,
        // STORAGE material-table @7, STORAGE gViewT @8 (Lighting L0b), STORAGE PointerGrid @9
        // (M1), COMBINED_IMAGE_SAMPLER BrickAtlas @10 (M2). Bindings 6/9/10 are the P4b coarse-cull
        // tiles, the M1 empty-skip pointer grid, and the M2 brick atlas: the marcher shader DECLARES
        // all three unconditionally (DXC keeps the @9/@10 references past the runtime
        // `brick_enabled`/`brick_trilinear` gates), so VALID descriptors are bound here even though
        // the windowed path gates ALL reads OFF (`coarse_enabled == 0` / `brick_enabled == 0` /
        // `brick_trilinear == 0` — byte-identical output, bindings bound-but-unread).
        let vocab_set = {
            let entries = [
                BindGroupEntry::StorageBuffer { buffer: scene.edit_list },
                BindGroupEntry::SampledImage {
                    texture: &depth,
                    sampler: scene.depth_sampler,
                },
                BindGroupEntry::StorageImage { texture: &albedo },
                BindGroupEntry::StorageImage { texture: &normal },
                BindGroupEntry::StorageImage { texture: &material },
                BindGroupEntry::UniformBuffer { buffer: scene.camera_uniform },
                BindGroupEntry::StorageBuffer { buffer: scene.tiles_buffer },
                // PBR MVP-2: the material table SSBO @7 (the marcher fetches `base_color`).
                BindGroupEntry::StorageBuffer { buffer: scene.material_table },
                // Lighting L0b: the gViewT lane @8 (the marcher STORES the surface `t`).
                BindGroupEntry::StorageImage { texture: &viewt },
                // M1: the empty-skip PointerGrid SSBO @9. Statically referenced by the marcher
                // SPIR-V (`register(t9)`); the windowed path gates the read OFF
                // (`brick_enabled == 0`), so it is bound-but-unread (byte-identical output).
                BindGroupEntry::StorageBuffer { buffer: scene.pointer_grid },
                // M2: the brick-atlas 3D image @10 as a COMBINED_IMAGE_SAMPLER (the marcher's
                // hardware trilinear `.SampleLevel` needs the sampler). Statically referenced by the
                // marcher SPIR-V (`register(t10)` + `register(s10)`, collapsed to one combined
                // descriptor by DXC); the windowed path gates the read OFF (`brick_trilinear == 0`),
                // so it is bound-but-unread (byte-identical output, the M2 R2 contract).
                BindGroupEntry::CombinedImage {
                    texture: scene.atlas,
                    sampler: scene.atlas_sampler,
                },
                // M4 clip-map LOD: the LEVEL-1 + LEVEL-2 brick resources (bindings 11/12 + 13/14). The
                // marcher SPIR-V statically references `PointerGrid1`@t11, `BrickAtlas1`@t12,
                // `PointerGrid2`@t13, `BrickAtlas2`@t14 inside the runtime level branch-ladder (NOT
                // dead-stripped past the gate), so VALID descriptors are bound here even on the OFF/N=1
                // path (`brick_levels == 1` takes only the lvl==0 arm → bound-but-unread, byte-identical).
                // Order matches the layout: PointerGrid1 @11, BrickAtlas1 @12, PointerGrid2 @13, BrickAtlas2 @14.
                BindGroupEntry::StorageBuffer { buffer: scene.level_grids[0] },
                BindGroupEntry::CombinedImage {
                    texture: scene.level_atlases[0],
                    sampler: scene.level_atlas_samplers[0],
                },
                BindGroupEntry::StorageBuffer { buffer: scene.level_grids[1] },
                BindGroupEntry::CombinedImage {
                    texture: scene.level_atlases[1],
                    sampler: scene.level_atlas_samplers[1],
                },
            ];
            let desc = BindGroupDesc::<Vulkan> {
                layout: scene.vocab_layout,
                entries: &entries,
            };
            match RhiDevice::create_bind_group(ctx, &desc) {
                Ok(g) => g,
                Err(e) => {
                    // SAFETY: the eight textures above were created on `ctx`; referenced
                    // by no submission; each destroyed exactly once on this error path.
                    unsafe {
                        RhiDevice::destroy_texture(ctx, ssao);
                        RhiDevice::destroy_texture(ctx, viewt);
                        RhiDevice::destroy_texture(ctx, lit);
                        RhiDevice::destroy_texture(ctx, material);
                        RhiDevice::destroy_texture(ctx, normal);
                        RhiDevice::destroy_texture(ctx, albedo);
                        RhiDevice::destroy_texture(ctx, depth);
                    }
                    return Err(SwapchainError::DepthImage(e));
                }
            }
        };

        // The deferred RESOLVE set, written ONCE here (12 bindings, 0..=11): gAlbedo @0,
        // gNormal @1, gMaterial @2, lit @3 (STORAGE images), material SSBO @4, camera UBO
        // @5, light table SSBO @6 (Lighting L0a), gViewT @7 (Lighting L0b), ClusterGrid @8 +
        // LightIndexList @9 (Lighting L1) — matching `deferred_pbr.comp`'s set 0. When L1 is
        // off the scene's cluster buffers are `None`, so @8/@9 bind the light table as a
        // harmless VALID placeholder (the resolve's `clusters_enabled` header gate never reads
        // them on the OFF path — the layout requires a valid descriptor regardless).
        let cluster_grid_buf = scene.cluster_grid.unwrap_or(scene.light_table);
        let light_index_buf = scene.light_index.unwrap_or(scene.light_table);
        let resolve_set = {
            let entries = [
                BindGroupEntry::StorageImage { texture: &albedo },
                BindGroupEntry::StorageImage { texture: &normal },
                BindGroupEntry::StorageImage { texture: &material },
                BindGroupEntry::StorageImage { texture: &lit },
                BindGroupEntry::StorageBuffer { buffer: scene.material_table },
                BindGroupEntry::UniformBuffer { buffer: scene.camera_uniform },
                BindGroupEntry::StorageBuffer { buffer: scene.light_table },
                // Lighting L0b: the gViewT lane @7 (the resolve READS it under `mask == 1`).
                BindGroupEntry::StorageImage { texture: &viewt },
                // Lighting L1: the ClusterGrid @8 + LightIndexList @9 (resolve READS the
                // pixel's froxel slice when `clusters_enabled`).
                BindGroupEntry::StorageBuffer { buffer: cluster_grid_buf },
                BindGroupEntry::StorageBuffer { buffer: light_index_buf },
                // P6 R1: the SDF edit-list `Buf` @10 — the SAME buffer the marcher binds +
                // uploads + barriers. The resolve dispatch is ordered after the marcher in the
                // same submit, so the prior upload+barrier covers this second COMPUTE read (no
                // new barrier). The resolve's `sdf_soft_shadow_ranged` march reads it read-only
                // (a strict field-CONSUMER); on a `shadow_mode==0` scene the march is never
                // executed, so the binding is a harmless valid descriptor (the 0%-gate).
                BindGroupEntry::StorageBuffer { buffer: scene.edit_list },
                // Render P7: the SSAO term `gSsao` @11 — ALWAYS bound (the resolve descriptor
                // interface is stable regardless of `ssao_mode`). The resolve reads it only under
                // `ssao_mode != 0` (0 every pre-P7 scene), so the binding is a harmless valid
                // descriptor (the 0%-gate); no SSAO pass writes it yet (C2 adds that).
                BindGroupEntry::StorageImage { texture: &ssao },
            ];
            let desc = BindGroupDesc::<Vulkan> {
                layout: scene.resolve_layout,
                entries: &entries,
            };
            match RhiDevice::create_bind_group(ctx, &desc) {
                Ok(g) => g,
                Err(e) => {
                    // SAFETY: the eight textures + the vocabulary set above were created on
                    // `ctx`; referenced by no submission; each destroyed exactly once on
                    // this error path (reverse acquisition order: set → images).
                    unsafe {
                        RhiDevice::destroy_bind_group(ctx, vocab_set);
                        RhiDevice::destroy_texture(ctx, ssao);
                        RhiDevice::destroy_texture(ctx, viewt);
                        RhiDevice::destroy_texture(ctx, lit);
                        RhiDevice::destroy_texture(ctx, material);
                        RhiDevice::destroy_texture(ctx, normal);
                        RhiDevice::destroy_texture(ctx, albedo);
                        RhiDevice::destroy_texture(ctx, depth);
                    }
                    return Err(SwapchainError::DepthImage(e));
                }
            }
        };

        // The Lighting-L1 CULL set, written ONCE here when L1 is wired (camera UBO @0, light
        // table SSBO @1, ClusterGrid @2, LightIndexList @3, LightIndexAlloc @4) — matching
        // `cluster_cull.comp`'s set 0. `None` when the scene does not supply the cull layout
        // (the L0b-only build); the recorder then skips the cull pass entirely.
        let cull_set = match (scene.cull_layout, scene.cluster_grid, scene.light_index, scene.light_index_alloc) {
            (Some(cull_layout), Some(grid), Some(index), Some(alloc)) => {
                let entries = [
                    BindGroupEntry::UniformBuffer { buffer: scene.camera_uniform },
                    BindGroupEntry::StorageBuffer { buffer: scene.light_table },
                    BindGroupEntry::StorageBuffer { buffer: grid },
                    BindGroupEntry::StorageBuffer { buffer: index },
                    BindGroupEntry::StorageBuffer { buffer: alloc },
                ];
                let desc = BindGroupDesc::<Vulkan> { layout: cull_layout, entries: &entries };
                match RhiDevice::create_bind_group(ctx, &desc) {
                    Ok(g) => Some(g),
                    Err(e) => {
                        // SAFETY: the resolve + vocabulary sets + the eight textures above
                        // were created on `ctx`; referenced by no submission; each destroyed
                        // exactly once on this error path (reverse acquisition order).
                        unsafe {
                            RhiDevice::destroy_bind_group(ctx, resolve_set);
                            RhiDevice::destroy_bind_group(ctx, vocab_set);
                            RhiDevice::destroy_texture(ctx, ssao);
                            RhiDevice::destroy_texture(ctx, viewt);
                            RhiDevice::destroy_texture(ctx, lit);
                            RhiDevice::destroy_texture(ctx, material);
                            RhiDevice::destroy_texture(ctx, normal);
                            RhiDevice::destroy_texture(ctx, albedo);
                            RhiDevice::destroy_texture(ctx, depth);
                        }
                        return Err(SwapchainError::DepthImage(e));
                    }
                }
            }
            _ => None,
        };

        // Render P7: the SSAO set, written ONCE here when the SSAO pass is wired (gNormal @0,
        // gMaterial @1, gViewT @2 STORAGE images READ, the `ssao` out STORAGE image @3 WRITE, the
        // camera UBO @4) — matching `sdf_ssao.comp`'s set 0. `None` when the scene does not supply
        // the SSAO activation (the default OFF path); the recorder then skips the SSAO pass
        // entirely (the 0%-gate, byte-identical command stream). The `ssao` image is the SAME one
        // the resolve set binds at @11 — the SSAO pass WRITES it, the resolve READS it (ordered by
        // the recorder's COMPUTE→COMPUTE barrier on the SSAO ON path).
        let ssao_set = match scene.ssao {
            Some(activation) => {
                let entries = [
                    BindGroupEntry::StorageImage { texture: &normal },
                    BindGroupEntry::StorageImage { texture: &material },
                    BindGroupEntry::StorageImage { texture: &viewt },
                    BindGroupEntry::StorageImage { texture: &ssao },
                    BindGroupEntry::UniformBuffer { buffer: scene.camera_uniform },
                ];
                let desc = BindGroupDesc::<Vulkan> { layout: activation.layout, entries: &entries };
                match RhiDevice::create_bind_group(ctx, &desc) {
                    Ok(g) => Some(g),
                    Err(e) => {
                        // SAFETY: the resolve + vocabulary sets + the (optional) cull set + the
                        // eight textures above were created on `ctx`; referenced by no submission;
                        // each destroyed exactly once on this error path (reverse acquisition
                        // order). The cull set is `Option`-guarded (only when L1 wired).
                        unsafe {
                            if let Some(cs) = cull_set {
                                RhiDevice::destroy_bind_group(ctx, cs);
                            }
                            RhiDevice::destroy_bind_group(ctx, resolve_set);
                            RhiDevice::destroy_bind_group(ctx, vocab_set);
                            RhiDevice::destroy_texture(ctx, ssao);
                            RhiDevice::destroy_texture(ctx, viewt);
                            RhiDevice::destroy_texture(ctx, lit);
                            RhiDevice::destroy_texture(ctx, material);
                            RhiDevice::destroy_texture(ctx, normal);
                            RhiDevice::destroy_texture(ctx, albedo);
                            RhiDevice::destroy_texture(ctx, depth);
                        }
                        return Err(SwapchainError::DepthImage(e));
                    }
                }
            }
            None => None,
        };

        // The present-blit set, written ONCE here: one COMBINED_IMAGE_SAMPLER pointing
        // at the LIT image (the resolve's output) + the scene's present sampler.
        let present_set = {
            let entries = [BindGroupEntry::CombinedImage {
                texture: &lit,
                sampler: scene.present_sampler,
            }];
            let desc = BindGroupDesc::<Vulkan> {
                layout: scene.present_layout,
                entries: &entries,
            };
            match RhiDevice::create_bind_group(ctx, &desc) {
                Ok(g) => g,
                Err(e) => {
                    // SAFETY: the eight textures + the vocabulary, resolve, (optional) cull &
                    // (optional) SSAO sets above were created on `ctx`; referenced by no
                    // submission; each destroyed exactly once on this error path (reverse
                    // acquisition order: sets → images). The cull & SSAO sets are `Option`-guarded
                    // (present only when L1 / SSAO are wired).
                    unsafe {
                        if let Some(ss) = ssao_set {
                            RhiDevice::destroy_bind_group(ctx, ss);
                        }
                        if let Some(cs) = cull_set {
                            RhiDevice::destroy_bind_group(ctx, cs);
                        }
                        RhiDevice::destroy_bind_group(ctx, resolve_set);
                        RhiDevice::destroy_bind_group(ctx, vocab_set);
                        RhiDevice::destroy_texture(ctx, ssao);
                        RhiDevice::destroy_texture(ctx, viewt);
                        RhiDevice::destroy_texture(ctx, lit);
                        RhiDevice::destroy_texture(ctx, material);
                        RhiDevice::destroy_texture(ctx, normal);
                        RhiDevice::destroy_texture(ctx, albedo);
                        RhiDevice::destroy_texture(ctx, depth);
                    }
                    return Err(SwapchainError::DepthImage(e));
                }
            }
        };

        Ok(Self {
            depth,
            albedo,
            normal,
            material,
            lit,
            viewt,
            ssao,
            vocab_set,
            resolve_set,
            cull_set,
            ssao_set,
            present_set,
            extent,
        })
    }

    /// Ensures the G-buffer images + descriptor sets exist and match `extent`,
    /// (re)building them through `ctx` when absent (first frame) or stale (resize).
    /// The vocabulary + present descriptor sets are re-written here — and ONLY here —
    /// so the per-frame recorder records no `vkUpdateDescriptorSets`.
    ///
    /// The caller ([`Renderer::render_gbuffer_frame`]) calls this only after
    /// fence-waiting the frame slot, so no in-flight frame still references the old
    /// targets; on a REPLACE this additionally waits the device idle (a sibling
    /// frame-in-flight slot may still reference the old images — the same
    /// belt-and-braces [`Scene::sync_depth`] uses) before destroying them.
    fn sync_gbuffer(
        targets: &mut Option<Self>,
        ctx: &VulkanContext,
        scene: &GBufferScene<'_>,
        extent: VkExtent2D,
    ) -> Result<(), SwapchainError> {
        if let Some(t) = targets.as_ref()
            && t.extent.width == extent.width
            && t.extent.height == extent.height
        {
            return Ok(());
        }

        // A (re)create is rare (first frame + resize). When REPLACING, wait idle first:
        // a sibling frame-in-flight slot may still reference the old targets, and the
        // caller only fence-waited THIS slot. The first-ever create needs no idle.
        if targets.is_some() {
            // SAFETY: `ctx` is live; waiting idle guarantees every prior submission —
            // including a sibling-slot frame still referencing the old targets — has
            // completed before they are destroyed below.
            unsafe { (ctx.device_fns().device_wait_idle)(ctx.device()) };
        }

        // Build the new targets BEFORE tearing down the old ones, so an allocation
        // failure leaves the previous (still-valid) targets in place.
        let fresh = Self::create(ctx, scene, extent)?;

        if let Some(old) = targets.take() {
            // SAFETY: the new targets were built above; the device was waited idle (a
            // replace), so no submission references the old targets; `destroy` consumes
            // them exactly once on the live `ctx` they were created on.
            unsafe { old.destroy(ctx) };
        }

        *targets = Some(fresh);
        Ok(())
    }

    /// Tears down the G-buffer targets (descriptor sets first, then the images),
    /// consuming `self`. The caller MUST have made the device idle (the renderer's
    /// `Drop` waits idle, or `sync_gbuffer` waits idle on a replace) so no submission
    /// still references them.
    ///
    /// # Safety
    ///
    /// `ctx` is the live context the targets were created on; no GPU work referencing
    /// them is in flight; each is destroyed exactly once (the by-value `self`).
    unsafe fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: per the contract `ctx` is live and nothing references these
        // resources; each was created on `ctx` and is destroyed exactly once, in
        // reverse acquisition order (sets → images). The cull & SSAO sets are
        // `Option`-guarded (present only when L1 / SSAO were wired).
        unsafe {
            RhiDevice::destroy_bind_group(ctx, self.present_set);
            if let Some(ss) = self.ssao_set {
                RhiDevice::destroy_bind_group(ctx, ss);
            }
            if let Some(cs) = self.cull_set {
                RhiDevice::destroy_bind_group(ctx, cs);
            }
            RhiDevice::destroy_bind_group(ctx, self.resolve_set);
            RhiDevice::destroy_bind_group(ctx, self.vocab_set);
            RhiDevice::destroy_texture(ctx, self.ssao);
            RhiDevice::destroy_texture(ctx, self.viewt);
            RhiDevice::destroy_texture(ctx, self.lit);
            RhiDevice::destroy_texture(ctx, self.material);
            RhiDevice::destroy_texture(ctx, self.normal);
            RhiDevice::destroy_texture(ctx, self.albedo);
            RhiDevice::destroy_texture(ctx, self.depth);
        }
    }
}

/// The renderer-side state for the on-screen Render-P1c G-buffer frame: the
/// per-extent [`GBufferTargets`], created lazily on the first
/// [`Renderer::render_gbuffer_frame`] and reallocated on resize. A caller drives one
/// across the present loop (analogous to a [`Scene`], but image-based).
///
/// Held by value; torn down through [`GBufferFrame::destroy`] AFTER the renderer is
/// dropped (the renderer's `Drop` waits the device idle).
pub struct GBufferFrame {
    /// The per-extent depth + MRT G-buffer + descriptor sets, `None` until the first
    /// frame syncs them.
    targets: Option<GBufferTargets>,
}

impl Default for GBufferFrame {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl GBufferFrame {
    /// Creates the on-screen G-buffer frame state with no targets yet (the first
    /// [`Renderer::render_gbuffer_frame`] allocates them sized to the swapchain
    /// extent).
    #[inline]
    pub fn new() -> Self {
        Self { targets: None }
    }

    /// Tears down the per-extent G-buffer targets through `ctx`, consuming `self`. The
    /// caller MUST have made the device idle (dropped the [`Renderer`], whose `Drop`
    /// waits idle) so no submission still references them.
    ///
    /// # Safety
    ///
    /// `ctx` is the live context the targets were created on; no GPU work referencing
    /// them is in flight (the caller `wait_idle`'d / dropped the renderer); they are
    /// destroyed exactly once (the by-value `self`).
    pub unsafe fn destroy(self, ctx: &VulkanContext) {
        if let Some(targets) = self.targets {
            // SAFETY: per this fn's contract `ctx` is live and nothing references the
            // targets; they are destroyed exactly once (moved out of `self`).
            unsafe { targets.destroy(ctx) };
        }
    }
}

impl Drop for Renderer<'_> {
    fn drop(&mut self) {
        // SAFETY: `device` is live; waiting idle ensures no command buffer /
        // semaphore / fence is in use. Then every sync object is destroyed once in
        // reverse creation order, and the command pool (which frees its command
        // buffers) last. The render-finished + per-frame semaphores and fences all
        // belong to this device.
        unsafe {
            (self.fns.device_wait_idle)(self.device);
            for &s in &self.render_finished {
                (self.fns.destroy_semaphore)(self.device, s, ptr::null());
            }
            for slot in &self.frames {
                (self.fns.destroy_fence)(self.device, slot.in_flight, ptr::null());
                (self.fns.destroy_semaphore)(self.device, slot.acquire, ptr::null());
            }
            (self.fns.destroy_command_pool)(self.device, self.command_pool, ptr::null());
        }
    }
}

// ---------------------------------------------------------------------------
// Free helpers.
// ---------------------------------------------------------------------------

/// The single-color, single-mip, single-layer subresource range used for every
/// swapchain image view + barrier.
const COLOR_SUBRESOURCE_RANGE: VkImageSubresourceRange = VkImageSubresourceRange {
    aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

/// The single-mip, single-layer DEPTH-aspect subresource range used for the
/// rung-7 scene depth image's barrier (the depth counterpart of
/// [`COLOR_SUBRESOURCE_RANGE`]).
const DEPTH_SUBRESOURCE_RANGE: VkImageSubresourceRange = VkImageSubresourceRange {
    aspect_mask: VK_IMAGE_ASPECT_DEPTH_BIT,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

/// Picks a surface format: prefer `B8G8R8A8_UNORM`/`_SRGB` (or RGBA equivalents)
/// in SRGB_NONLINEAR space; else the first advertised format. The
/// `current_extent == u32::MAX` special case (any extent) is `0xFFFFFFFF`; a
/// single advertised `{UNDEFINED, SRGB_NONLINEAR}` means "any format".
fn pick_surface_format(
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
fn present_mode_supported(
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
fn resolve_extent(caps: &VkSurfaceCapabilitiesKhr, width: u32, height: u32) -> VkExtent2D {
    if caps.current_extent.width != u32::MAX {
        return caps.current_extent;
    }
    VkExtent2D {
        width: width.clamp(caps.min_image_extent.width, caps.max_image_extent.width),
        height: height.clamp(caps.min_image_extent.height, caps.max_image_extent.height),
    }
}

/// Creates an unsignalled binary semaphore.
fn create_semaphore(fns: &DeviceFns, device: VkDevice) -> Result<VkSemaphore, SwapchainError> {
    let ci = VkSemaphoreCreateInfo {
        s_type: VkStructureType::SemaphoreCreateInfo,
        p_next: ptr::null(),
        flags: 0,
    };
    let mut sem = VkSemaphore::NULL;
    // SAFETY: `device` is live; `ci` is a fully-initialized create-info; `&mut
    // sem` is a valid out-pointer; NULL allocator.
    let raw = unsafe { (fns.create_semaphore)(device, &ci, ptr::null(), &mut sem) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() {
        return Err(SwapchainError::VkError("vkCreateSemaphore", result));
    }
    Ok(sem)
}

/// `VkFenceCreateFlagBits::VK_FENCE_CREATE_SIGNALED_BIT`.
const VK_FENCE_CREATE_SIGNALED_BIT: VkFlags = 0x0000_0001;

/// Creates a fence in the SIGNALLED state (so the first per-frame wait returns
/// immediately rather than deadlocking on a never-submitted fence).
fn create_fence_signalled(fns: &DeviceFns, device: VkDevice) -> Result<VkFence, SwapchainError> {
    let ci = VkFenceCreateInfo {
        s_type: VkStructureType::FenceCreateInfo,
        p_next: ptr::null(),
        flags: VK_FENCE_CREATE_SIGNALED_BIT,
    };
    let mut fence = VkFence::NULL;
    // SAFETY: `device` is live; `ci` is a fully-initialized signalled create-info;
    // `&mut fence` is a valid out-pointer; NULL allocator.
    let raw = unsafe { (fns.create_fence)(device, &ci, ptr::null(), &mut fence) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() {
        return Err(SwapchainError::VkError("vkCreateFence", result));
    }
    Ok(fence)
}

/// Destroys whatever frame-sync + render-finished objects were created so far on
/// a `Renderer::new` error path (NULL handles are skipped).
///
/// # Safety
///
/// Every non-null handle in `frames` / `render_finished` must have been created
/// on `device` and not yet destroyed.
unsafe fn destroy_partial_frames(
    fns: &DeviceFns,
    device: VkDevice,
    frames: &[FrameSync],
    render_finished: &[VkSemaphore],
) {
    // SAFETY: each non-null handle was created on `device` per this fn's contract
    // and is destroyed exactly once here.
    unsafe {
        for &s in render_finished {
            if !s.is_null() {
                (fns.destroy_semaphore)(device, s, ptr::null());
            }
        }
        for slot in frames {
            if !slot.in_flight.is_null() {
                (fns.destroy_fence)(device, slot.in_flight, ptr::null());
            }
            if !slot.acquire.is_null() {
                (fns.destroy_semaphore)(device, slot.acquire, ptr::null());
            }
        }
    }
}

/// The swapchain image handle at `index`. `Swapchain` retains the `VkImage`
/// handles (in `images`) alongside their views; the per-frame barriers operate
/// on the handle returned here. The images are owned by the swapchain object
/// and are not destroyed individually (destroying the swapchain frees them).
#[inline]
fn swapchain_image_for(swapchain: &Swapchain<'_>, index: usize) -> VkImage {
    debug_assert!(index < swapchain.images.len(), "image index out of range");
    swapchain.images[index]
}
