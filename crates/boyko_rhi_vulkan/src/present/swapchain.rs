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
    /// **The mode this swapchain actually got** — profiling rung 8, D12. Equal to the request when
    /// the surface advertised it, `Fifo` when it did not.
    pub(crate) present_mode: PresentModeConfig,
}

/// **Profiling rung 8, D12** — the present mode a caller REQUESTS.
///
/// Requesting is not getting: only `Fifo` is guaranteed by the spec, so the request is PROBED
/// against the surface and an unsupported one falls back to `Fifo` with a loud notice. The mode a
/// swapchain actually got is [`Swapchain::present_mode`], and the artifact records THAT, never the
/// request — a file that recorded what was asked for would attribute a FIFO-bounded frame time to a
/// tearing present.
///
/// # Why this exists at all
///
/// While FIFO was hard-coded, **no wall-clock gate could fail for GPU-side work**: every frame is
/// bounded below by the refresh interval, so a regression that made the GPU twice as slow reported
/// the same 16.67 ms. This project treats a gate that cannot fail as a defect, and the precedent is
/// measured: `-ValidationOn` reported *"clean, 0 messages"* for all 22 pins while an illegal
/// `mip_levels: 12` drew zero.
///
/// Default `Fifo`, so **no golden pin moves** — [`Swapchain::new`] keeps its signature and its
/// behaviour, and only a caller that names a different mode gets one.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PresentModeConfig {
    /// Block on the refresh interval. Spec-guaranteed, and the default.
    #[default]
    Fifo,
    /// Present as soon as submitted; tearing allowed. Optional — probed.
    Immediate,
    /// One queued image, replaced rather than blocked. Optional — probed.
    ///
    /// **Declared, and it takes the SAME code path as the other two.** The corpus says it *"returns
    /// `Unsupported` until a harness needs it — one code path, not three"*; what makes that true
    /// here is that nothing special-cases it: it probes, and it falls back like anything else. A
    /// separate "not implemented" arm would be a second path pretending to be an absence.
    Mailbox,
}

impl PresentModeConfig {
    /// The Vulkan enum this requests.
    #[must_use]
    pub const fn as_vk(self) -> i32 {
        match self {
            PresentModeConfig::Fifo => VK_PRESENT_MODE_FIFO_KHR,
            PresentModeConfig::Immediate => VK_PRESENT_MODE_IMMEDIATE_KHR,
            PresentModeConfig::Mailbox => VK_PRESENT_MODE_MAILBOX_KHR,
        }
    }

    /// The wire word, for the artifact.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            PresentModeConfig::Fifo => "fifo",
            PresentModeConfig::Immediate => "immediate",
            PresentModeConfig::Mailbox => "mailbox",
        }
    }

    /// Whether a frame time under this mode is bounded below by the display's refresh interval.
    ///
    /// `true` for `Fifo` only. This is what the `Frame` channel's wall clock must carry beside it
    /// (`bound=FIFO(refresh)` or `bound=none`): **even under `Immediate` the wall clock stays
    /// secondary** — the primary CPU number is the `__frame` span and the primary GPU number is the
    /// device-tick delta — but a wall clock with no stated bound is a number a reader will compare
    /// across modes without knowing they are not comparable.
    #[must_use]
    pub const fn is_refresh_bounded(self) -> bool {
        matches!(self, PresentModeConfig::Fifo)
    }
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
        Self::new_with_present_mode(ctx, surface, width, height, PresentModeConfig::Fifo)
    }

    /// As [`Self::new`], with an explicit present-mode REQUEST — profiling rung 8, D12.
    ///
    /// A separate constructor rather than a widened `new`, and the reason is not tidiness: nine
    /// call sites across five crates build a swapchain, every one of them wants FIFO, and every
    /// golden pin in the tree was blessed under it. Threading a parameter through all nine to have
    /// them all pass the same value would put a knob in front of every caller that must never touch
    /// it. `new` IS the default, structurally.
    ///
    /// The request is probed; see [`PresentModeConfig`] for what a refusal does.
    pub fn new_with_present_mode(
        ctx: &'ctx VulkanContext,
        surface: &Surface<'_>,
        width: u32,
        height: u32,
        requested: PresentModeConfig,
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
            requested,
        )
    }

    /// The mode this swapchain ACTUALLY got, which is what an artifact records.
    #[must_use]
    pub fn present_mode(&self) -> PresentModeConfig {
        self.present_mode
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
        // The RESOLVED mode is re-requested, not the original request: a recreate that re-probed
        // a refused `Immediate` would print the fallback notice on every resize, and a recreate
        // that dropped to `Fifo` unconditionally would silently change what the frames after a
        // resize measure. Re-requesting what was granted is the only option that changes nothing.
        let rebuilt = Self::build(
            self.device,
            self.fns,
            self.swap_fns,
            surface,
            width,
            height,
            self.swapchain,
            self.present_mode,
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
        self.present_mode = rebuilt.present_mode;
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
        requested: PresentModeConfig,
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

        // Profiling rung 8, D12: RESOLVE the request. `Fifo` skips the query entirely -- it was
        // just confirmed above, and a second call would be a driver round trip on every recreate of
        // every shipped path to learn what the line above already established.
        //
        // REFUSE OR ANNOUNCE, NEVER SILENTLY DEGRADE -- the `BootError::ValidationUnavailable`
        // precedent. A silent fallback would leave the artifact recording `fifo` while the operator
        // believed they were measuring an unbounded frame, and every number would be off by a
        // refresh interval they had already decided to eliminate.
        let present_mode = if requested == PresentModeConfig::Fifo
            || present_mode_supported(
                surface_fns,
                surface.physical_device,
                surface.surface,
                requested.as_vk(),
            )? {
            requested
        } else {
            eprintln!(
                "present mode: `{}` is NOT advertised by this surface -- falling back to `fifo`.                  Frame wall clock stays bounded below by the refresh interval, so a wall-clock                  comparison across this boot and one that got `{}` is not a comparison.",
                requested.as_str(),
                requested.as_str()
            );
            PresentModeConfig::Fifo
        };

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
            present_mode: present_mode.as_vk(),
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
            present_mode,
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
