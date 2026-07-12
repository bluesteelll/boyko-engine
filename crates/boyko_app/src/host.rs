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
use boyko_scene::FreeEntry;

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
    /// Asset-streaming plan F6: the host-parked scratch buffer
    /// `retire_deferred_frees` drains ready [`FreeEntry`] rows into every frame —
    /// reused across frames (capacity retained), zero steady-state allocation.
    pub(crate) retire_scratch: Vec<FreeEntry>,
    /// The boot-fixed composite extent (plan D7): the G-buffer / marcher /
    /// camera-push extent, frozen at the boot client size. A window resize
    /// recreates the swapchain only; the present blit clamps to
    /// `min(window, composite)`. Under armed SSAA this is `2 * native_extent`
    /// (see [`Self::ssaa_armed`]); otherwise it equals [`Self::native_extent`].
    pub(crate) composite_extent: (u32, u32),
    /// SSAA (AA campaign Stage 3, W2): whether the boot device probe armed the 2×
    /// supersample render scale — decided ONCE here, never per-frame. `true` ⇒
    /// `composite_extent == 2 * native_extent` and the per-frame read site
    /// (`runner::frame_loop`) LOCKS `AaMode::Ssaa`. `false` ⇒ `composite_extent ==
    /// native_extent` and any `AaMode::Ssaa` resource-request degrades to `Off` — the
    /// device-capability degrade seam `AaConfig`'s doc reserves, resolved host-side
    /// because this layer is the only one that sees the boot resolution + device caps.
    pub(crate) ssaa_armed: bool,
    /// SSAA (W2): the pre-scale window client size (`window.width()`/`height()` at
    /// boot) — ALWAYS the render extent `aa_out` uses (native, never 2×), regardless of
    /// [`Self::ssaa_armed`]. Equals [`Self::composite_extent`] when SSAA is not armed.
    pub(crate) native_extent: (u32, u32),
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
        let native_extent = (window.width(), window.height());

        // SSAA (AA campaign Stage 3, C1/C2/W2): the ONE place the 2x render scale is
        // decided — a boot-time device-capability probe, never a per-frame choice.
        // `desc.ssaa_scale` is the owner's request (0/1 == off; v1 honors ONLY `2`).
        // Arms iff BOTH the device's `maxImageDimension2D` fits `native * 2` on every
        // axis AND the estimated 2x ring VRAM cost stays under half the largest
        // DEVICE_LOCAL heap; any failure degrades to `Off` — NEVER a panic, boot
        // proceeds exactly as an unscaled boot would.
        let caps = ctx.device_caps();
        let want = desc.ssaa_scale;
        let dims_ok = native_extent.0.saturating_mul(SSAA_SCALE) <= caps.max_image_dimension_2d
            && native_extent.1.saturating_mul(SSAA_SCALE) <= caps.max_image_dimension_2d;
        let est = ssaa_ring_bytes_estimate(native_extent, SSAA_SCALE);
        let vram_ok = est < caps.device_local_heap_bytes / SSAA_VRAM_FRACTION_DEN;
        let ssaa_armed = want == SSAA_SCALE && dims_ok && vram_ok;
        // Cold, boot-once diagnostics (mirrors `query_device_caps`'s DDGI/shadow-denoise
        // degrade logging) — never on the frame path, never a panic. Emitted UNCONDITIONALLY
        // (not `#[cfg(debug_assertions)]`): a RELEASE-build degrade-to-Off must be observable,
        // else an owner requesting `BOYKO_AA=ssaa` on a device that fails the dims/VRAM probe
        // silently gets no supersampling with zero explanation (spec B11).
        if want == SSAA_SCALE && !ssaa_armed {
            eprintln!("SSAA 2x unavailable (dims_ok={dims_ok} vram_ok={vram_ok}) -> Off");
        }
        if want != 0 && want != 1 && want != SSAA_SCALE {
            eprintln!("SSAA scale {want} unsupported (v1: 2x only) -> Off");
        }
        let composite_extent = if ssaa_armed {
            (native_extent.0 * SSAA_SCALE, native_extent.1 * SSAA_SCALE)
        } else {
            native_extent
        };

        let gpu = GpuSceneBundles::boot(ctx, composite_extent, swap_format);

        Ok(Self {
            renderer,
            frame: GBufferFrame::new(),
            gpu,
            draw_scratch: DrawListScratch::new(),
            retire_scratch: Vec::new(),
            composite_extent,
            ssaa_armed,
            native_extent,
            // u64::MAX ≠ any real generation ⇒ both slots upload the ECS light
            // table on their first frames (host plan D5/R4).
            light_uploaded_gen: [u64::MAX; FRAMES_IN_FLIGHT],
            swapchain,
            surface,
            window,
        })
    }
}

/// SSAA (W2): the v1 render scale — 2× per axis (4× pixels). The boot arming probe
/// (see [`WindowHost::boot`]) admits ONLY this value; any other `WindowDesc::ssaa_scale`
/// degrades to `Off`.
const SSAA_SCALE: u32 = 2;

/// SSAA (W2): the VRAM-budget divisor — arm only if the estimated 2× ring cost stays
/// under `1 / SSAA_VRAM_FRACTION_DEN` of the largest `DEVICE_LOCAL` heap.
const SSAA_VRAM_FRACTION_DEN: u64 = 2;

/// SSAA (W2): a conservative VRAM estimate (bytes) for the `scale`× composite-extent
/// CORE rings, used ONLY to decide whether to arm SSAA at boot — the real allocations
/// flow through [`GpuSceneBundles::boot`] and are never sized from this number.
/// `native` is the pre-scale `(width, height)`; the core rings cost ≈ 33 B/px ×
/// `FRAMES_IN_FLIGHT`(2) = 66 B/px at NATIVE resolution, scaled by `scale²` (area) for
/// the composite extent; `feature = "hwrt"` adds the RT ring cost (≈ 28 B/px × FIF(2)
/// at native, same `scale²` scaling).
const fn ssaa_ring_bytes_estimate(native: (u32, u32), scale: u32) -> u64 {
    const CORE_BYTES_PER_NATIVE_PX: u64 = 66;
    #[cfg(feature = "hwrt")]
    const PER_NATIVE_PX: u64 = CORE_BYTES_PER_NATIVE_PX + 28;
    #[cfg(not(feature = "hwrt"))]
    const PER_NATIVE_PX: u64 = CORE_BYTES_PER_NATIVE_PX;

    let native_px = native.0 as u64 * native.1 as u64;
    let scale_sq = (scale as u64) * (scale as u64);
    native_px * scale_sq * PER_NATIVE_PX
}
