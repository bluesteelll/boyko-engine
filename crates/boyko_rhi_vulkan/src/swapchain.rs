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

use boyko_rhi::{Format, ImageUsage, RhiDevice, TextureDesc, TextureDimension};

use crate::device::{DeviceFns, SurfaceInstanceFns, SwapchainDeviceFns, VulkanContext};
use crate::ffi::*;
use crate::memory::BoundBuffer;
use crate::rhi_impl::VulkanGraphicsPipeline;
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
    /// (recreated to match the swapchain extent on resize, see [`Scene::sync_depth`]).
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
    /// current extent (the call does this via [`Scene::sync_depth`] when needed). A
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
/// [`RhiDevice`](boyko_rhi::RhiDevice) trait (so the proven S0 pipeline-creation
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
