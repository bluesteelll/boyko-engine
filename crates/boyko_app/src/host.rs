//! The host-owned window + present chain (host plan R2 subset).
//!
//! [`WindowHost`] owns everything the OS/present side needs OUTSIDE the World:
//! the Win32 window, the surface, the swapchain and the renderer — all
//! instantiated at `'static` against the device singleton pinned by
//! [`VulkanContext::boot_singleton`]. G-buffer targets and the GPU scene
//! bundles arrive in R3.
//!
//! Windows-only: the OS window is Windows-first (`boyko_rhi_vulkan::window`
//! D8); the runner's non-Windows arm exits gracefully before this module is
//! ever needed.

use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::swapchain::{Renderer, Surface, Swapchain, SwapchainError};
use boyko_rhi_vulkan::window::{Window, WindowError};

use crate::runner::WindowDesc;

/// A typed boot-chain failure — which link of the window → surface → swapchain
/// → renderer chain failed, with the underlying error (no `eprintln`-SKIP flow;
/// that is test-harness style, the library reports and the runner decides).
#[derive(Debug)]
pub(crate) enum HostBootError {
    /// `Window::open` failed (no module handle / class registration / window
    /// creation).
    Window(WindowError),
    /// `Surface::new` failed (no WSI extensions on the instance, no
    /// present-capable queue, or surface creation itself).
    Surface(SwapchainError),
    /// `Swapchain::new` failed (surface caps query / swapchain / view creation).
    Swapchain(SwapchainError),
    /// `Renderer::new` failed (command pool / buffers / sync-object creation).
    Renderer(SwapchainError),
}

impl core::fmt::Display for HostBootError {
    // Cold error path: the formatter never runs on the frame path.
    #[cold]
    #[inline(never)]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            HostBootError::Window(e) => write!(f, "window creation failed: {e:?}"),
            HostBootError::Surface(e) => write!(f, "surface creation failed: {e:?}"),
            HostBootError::Swapchain(e) => write!(f, "swapchain creation failed: {e:?}"),
            HostBootError::Renderer(e) => write!(f, "renderer creation failed: {e:?}"),
        }
    }
}

/// The host-owned window + present chain (host plan R2 subset — no G-buffer
/// targets yet; those arrive in R3).
///
/// # Field order IS drop order (plan D2 steps 1–2)
///
/// Rust drops struct fields in declaration order, and this struct's teardown
/// correctness depends on it — do NOT reorder:
///
/// 1. `renderer` — its `Drop` performs `vkDeviceWaitIdle` FIRST, then destroys
///    every sync object + the command pool, so nothing later drops while the
///    GPU still references it;
/// 2. `swapchain` — destroys the image views + swapchain (device idle per 1);
/// 3. `surface` — destroys the `VkSurfaceKHR` BEFORE the window it borrows;
/// 4. `window` — last: the surface's HWND/HINSTANCE stay live until 3 ran.
pub(crate) struct WindowHost {
    /// The frame driver (sync + command buffers). Dropped FIRST — waits idle.
    pub(crate) renderer: Renderer<'static>,
    /// The swapchain + per-image views. Dropped second (device idle by then).
    pub(crate) swapchain: Swapchain<'static>,
    /// The `VkSurfaceKHR` over the window. Dropped third, before the window.
    pub(crate) surface: Surface<'static>,
    /// The Win32 window + input ring. Dropped LAST.
    pub(crate) window: Window,
}

impl WindowHost {
    /// Boots the window → surface → swapchain → renderer chain at `'static`
    /// against the pinned device singleton (the boot-chain shape of the
    /// `window_present_gbuffer` harness, as a clean library fn).
    ///
    /// On a mid-chain failure the already-created links drop in reverse
    /// creation order (locals), so no partial resource leaks.
    ///
    /// # Errors
    ///
    /// [`HostBootError`] naming the failed link — a windowless / WSI-less
    /// machine surfaces here and the runner exits gracefully.
    pub(crate) fn boot(
        ctx: &'static VulkanContext,
        desc: &WindowDesc,
    ) -> Result<Self, HostBootError> {
        let window =
            Window::open(desc.title, desc.width, desc.height).map_err(HostBootError::Window)?;

        // SAFETY: `window` outlives the surface — both move into the returned
        // `WindowHost`, whose declared field order drops `surface` BEFORE
        // `window` (and on this fn's error paths the surface local drops
        // before the window local), so the HWND/HINSTANCE are live for the
        // surface's whole lifetime.
        let surface = unsafe { Surface::new(ctx, window.hinstance(), window.hwnd()) }
            .map_err(HostBootError::Surface)?;

        let swapchain = Swapchain::new(ctx, &surface, window.width(), window.height())
            .map_err(HostBootError::Swapchain)?;

        let renderer =
            Renderer::new(ctx, &surface, &swapchain).map_err(HostBootError::Renderer)?;

        Ok(Self {
            renderer,
            swapchain,
            surface,
            window,
        })
    }
}
