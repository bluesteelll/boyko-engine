//! The host-owned window + present chain + G-buffer scene (host plan R2 + R3).
//!
//! [`WindowHost`] owns everything the OS/present side needs OUTSIDE the World:
//! the Win32 window, the surface, the swapchain, the renderer, the boot-fixed
//! composite extent (plan D7), the extent-dependent [`GBufferFrame`] targets,
//! and the static [`GpuSceneBundles`] — all instantiated at `'static` against
//! the device singleton pinned by [`VulkanContext::boot_singleton`].
//!
//! Windows-only: the OS window is Windows-first (`boyko_rhi_vulkan::window`
//! D8); the runner's non-Windows arm exits gracefully before this module is
//! ever needed.

use boyko_rhi::Format;
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::ffi::{VK_FORMAT_B8G8R8A8_UNORM, VK_FORMAT_R8G8B8A8_UNORM};
use boyko_rhi_vulkan::swapchain::{
    FRAMES_IN_FLIGHT, GBufferFrame, Renderer, Surface, Swapchain, SwapchainError,
};
use boyko_rhi_vulkan::window::{Window, WindowError};

use crate::gpu_scene::{DrawListScratch, GpuSceneBundles};
use crate::runner::WindowDesc;

/// A typed boot-chain failure — which link of the window → surface → swapchain
/// → renderer → scene chain failed, with the underlying error (no
/// `eprintln`-SKIP flow; that is test-harness style, the library reports and
/// the runner decides).
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
    /// The swapchain's color format has no basic-slice [`Format`] mapping —
    /// the present-blit pipeline cannot declare it (W2-b). Carries the raw
    /// `VkFormat` for the log line.
    SwapchainFormat(i32),
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
            HostBootError::SwapchainFormat(raw) => {
                write!(f, "unsupported swapchain color format (VkFormat {raw})")
            }
        }
    }
}

/// The host-owned window + present chain + G-buffer scene (host plan D2/D7).
///
/// # Field order IS drop order (plan D2 steps 1–2)
///
/// Rust drops struct fields in declaration order, and this struct's teardown
/// correctness depends on it — do NOT reorder:
///
/// 1. `renderer` — its `Drop` performs `vkDeviceWaitIdle` FIRST, then destroys
///    every sync object + the command pool, so nothing later drops while the
///    GPU still references it;
/// 2. `frame` / `gpu` — NO `Drop` glue (RHI resources are destroyed through
///    the context): the runner's teardown destroys them EXPLICITLY between the
///    renderer drop (device idle) and the swapchain drop;
/// 3. `swapchain` — destroys the image views + swapchain (device idle per 1);
/// 4. `surface` — destroys the `VkSurfaceKHR` BEFORE the window it borrows;
/// 5. `window` — last: the surface's HWND/HINSTANCE stay live until 4 ran.
pub(crate) struct WindowHost {
    /// The frame driver (sync + command buffers). Dropped FIRST — waits idle.
    pub(crate) renderer: Renderer<'static>,
    /// The extent-dependent G-buffer targets (created lazily by
    /// `render_gbuffer_frame`'s sync at the composite extent). Destroyed
    /// explicitly in teardown (no `Drop` glue).
    pub(crate) frame: GBufferFrame,
    /// The static G-buffer scene bundles (pipelines / layouts / seeded
    /// buffers / samplers / CSM+atlas trios). Destroyed explicitly in teardown.
    pub(crate) gpu: GpuSceneBundles,
    /// The reusable per-frame draw-list allocation (0 alloc/frame after warmup).
    pub(crate) draw_scratch: DrawListScratch,
    /// The boot-fixed composite extent (plan D7): the G-buffer / marcher /
    /// camera-push extent, frozen at the boot client size. A window resize
    /// recreates the swapchain only; the present blit clamps to
    /// `min(window, composite)`.
    pub(crate) composite_extent: (u32, u32),
    /// Per-in-flight-slot record of the `LightTableGeneration` whose staged
    /// bytes were last written into that slot's light staging (host plan D5/R4).
    /// Seeded `u64::MAX` (≠ any real generation) so BOTH slots upload the real
    /// ECS table on their first frames; thereafter slot `s` is rewritten iff
    /// `light_uploaded_gen[s] != generation` (the deterministic writer-side
    /// gate — see `crate::light_gate::light_upload_due`).
    pub(crate) light_uploaded_gen: [u64; FRAMES_IN_FLIGHT],
    /// The swapchain + per-image views. Dropped after the explicit
    /// frame/gpu teardown (device idle by then).
    pub(crate) swapchain: Swapchain<'static>,
    /// The `VkSurfaceKHR` over the window. Dropped before the window.
    pub(crate) surface: Surface<'static>,
    /// The Win32 window + input ring. Dropped LAST.
    pub(crate) window: Window,
}

/// Maps the swapchain's raw `VkFormat` to the basic-slice [`Format`] the
/// present-blit pipeline declares, or `None` for an unsupported format (the
/// SRGB variants have no basic-slice mapping — the boot fails typed).
fn swap_color_format(vk_format: i32) -> Option<Format> {
    match vk_format {
        f if f == VK_FORMAT_B8G8R8A8_UNORM => Some(Format::B8G8R8A8Unorm),
        f if f == VK_FORMAT_R8G8B8A8_UNORM => Some(Format::R8G8B8A8Unorm),
        _ => None,
    }
}

impl WindowHost {
    /// Boots the window → surface → swapchain → renderer → scene chain at
    /// `'static` against the pinned device singleton (the boot-chain shape of
    /// the `window_present_gbuffer` harness, as a clean library fn).
    ///
    /// The composite extent is FIXED here from the actual boot client size
    /// (plan D7) — not the requested `desc` size, which the OS may adjust.
    ///
    /// On a mid-chain failure the already-created links drop in reverse
    /// creation order (locals), so no partial resource leaks.
    ///
    /// # Errors
    ///
    /// [`HostBootError`] naming the failed link — a windowless / WSI-less
    /// machine surfaces here and the runner exits gracefully.
    ///
    /// # Panics
    ///
    /// A scene-resource create failure inside [`GpuSceneBundles::boot`] panics
    /// (`expect("invariant: ...")`): device OOM at scene-boot time is a setup
    /// failure by design, not a recoverable boot outcome.
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

        let swap_format = swap_color_format(swapchain.format())
            .ok_or(HostBootError::SwapchainFormat(swapchain.format()))?;

        let renderer =
            Renderer::new(ctx, &surface, &swapchain).map_err(HostBootError::Renderer)?;

        // Plan D7: the composite extent is boot-fixed from the ACTUAL client
        // size the window came up at.
        let composite_extent = (window.width(), window.height());
        let gpu = GpuSceneBundles::boot(ctx, composite_extent, swap_format);

        Ok(Self {
            renderer,
            frame: GBufferFrame::new(),
            gpu,
            draw_scratch: DrawListScratch::new(),
            composite_extent,
            // u64::MAX ≠ any real generation ⇒ both slots upload the ECS light
            // table on their first frames (host plan D5/R4).
            light_uploaded_gen: [u64::MAX; FRAMES_IN_FLIGHT],
            swapchain,
            surface,
            window,
        })
    }
}
