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

use boyko_render::ResolvedRenderPath;
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
    /// The ARMED render scale per axis — a member of [`SSAA_SCALES`] when
    /// [`Self::ssaa_armed`], and **1** otherwise, so `composite_extent == native_extent *
    /// ssaa_scale` is total rather than conditional. Added at VG-R0 rung R0e, when the admitted
    /// set stopped being a single constant: a `bool` can no longer say WHICH scale armed, and the
    /// density census asserts the achieved extent against the rung it requested.
    pub(crate) ssaa_scale: u32,
    /// SSAA (W2): the pre-scale window client size (`window.width()`/`height()` at
    /// boot) — ALWAYS the render extent `aa_out` uses (native, never scaled), regardless of
    /// [`Self::ssaa_armed`]. Equals [`Self::composite_extent`] when SSAA is not armed.
    pub(crate) native_extent: (u32, u32),
    /// Multi-paradigm render-path plan, rung R1: the boot-committed render-path selection
    /// (Decision 1) — resolved exactly ONCE by `run_windowed`, right after this struct boots
    /// (device caps + the World's config Resources are both live by then), and written into
    /// this field (the `ssaa_armed` precedent: a host-authoritative boot commitment, never a
    /// per-frame `World` read). Seeded to [`ResolvedRenderPath::default`] here (`Deferred +
    /// Both`, the byte-identity anchor) so the field is never observed uninitialized between
    /// [`Self::boot`] returning and the runner's boot-lock write. Threaded into
    /// `GpuSceneBundles::scene()` every frame, where it becomes the plain-POD
    /// `ResolvedRenderPathGpu` the RHI dispatches its per-path declarator on. (This doc read
    /// "DEAD-BUT-THREADED at R1 (nothing reads it yet)" for as long as that was false.)
    pub(crate) resolved_render_path: ResolvedRenderPath,
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
        // VG-R0 rung R0e: the admitted set gained 4×. The probe is now parameterised BY the
        // requested scale instead of by one constant, so `want == 2` follows exactly the arithmetic
        // it followed before (`SSAA_SCALES[0] == 2`) and every golden that arms SSAA is
        // byte-identical across the widening; `want == 4` runs the same probe against 4.
        let admitted = SSAA_SCALES.contains(&want);
        let dims_ok = admitted
            && native_extent.0.saturating_mul(want) <= caps.max_image_dimension_2d
            && native_extent.1.saturating_mul(want) <= caps.max_image_dimension_2d;
        let est = if admitted { ssaa_ring_bytes_estimate(native_extent, want) } else { 0 };
        let vram_ok = admitted && est < caps.device_local_heap_bytes / SSAA_VRAM_FRACTION_DEN;
        let ssaa_armed = admitted && dims_ok && vram_ok;
        // 1 when SSAA is off or degraded, so `composite = native * ssaa_scale` is total.
        let ssaa_scale = if ssaa_armed { want } else { 1 };
        // Cold, boot-once diagnostics (mirrors `query_device_caps`'s DDGI/shadow-denoise
        // degrade logging) — never on the frame path, never a panic. Emitted UNCONDITIONALLY
        // (not `#[cfg(debug_assertions)]`): a RELEASE-build degrade-to-Off must be observable,
        // else an owner requesting `BOYKO_AA=ssaa` on a device that fails the dims/VRAM probe
        // silently gets no supersampling with zero explanation (spec B11).
        if admitted && !ssaa_armed {
            eprintln!(
                "SSAA {want}x unavailable (dims_ok={dims_ok} vram_ok={vram_ok} est={est} heap={}) -> Off",
                caps.device_local_heap_bytes
            );
        }
        if want != 0 && want != 1 && !admitted {
            eprintln!("SSAA scale {want} unsupported (admitted: {SSAA_SCALES:?}) -> Off");
        }
        let composite_extent = (native_extent.0 * ssaa_scale, native_extent.1 * ssaa_scale);

        let gpu = GpuSceneBundles::boot(ctx, composite_extent, swap_format);

        Ok(Self {
            renderer,
            frame: GBufferFrame::new(),
            gpu,
            draw_scratch: DrawListScratch::new(),
            retire_scratch: Vec::new(),
            composite_extent,
            ssaa_armed,
            ssaa_scale,
            native_extent,
            resolved_render_path: ResolvedRenderPath::default(),
            // u64::MAX ≠ any real generation ⇒ both slots upload the ECS light
            // table on their first frames (host plan D5/R4).
            light_uploaded_gen: [u64::MAX; FRAMES_IN_FLIGHT],
            swapchain,
            surface,
            window,
        })
    }
}

/// SSAA: the admitted render scales, per axis. The boot arming probe
/// ([`WindowHost::boot`]) admits ONLY these; any other `WindowDesc::ssaa_scale` degrades to `Off`.
///
/// ⚠️ **4× is VG-R0 rung R0e's addition and it is a MEASUREMENT capability, not a quality feature.**
/// R0d measured `visible_tris` as NOT CONVERGED on either committed camera path (residuals 0.3545
/// and 0.2444 against a 0.05 margin), and `[k1_instrument].on_not_converged_refute_direction`'s own
/// disposition for that is to **extend the ladder upward**, never to adjudicate on an underestimate.
/// Extending it needs a composite beyond `2 × 1920×1080`. The arithmetic that made 4× worth adding:
/// on `orbit_mid`, `D_est` needs a further **1.35×** in `visible_tris` to cross `[k1].d_est_min`,
/// which the measured growth exponent puts at **14.4 Mpx** — reachable at `4 × 1280×720` = 14.75 Mpx
/// for an estimated 0.95 GB, well inside this box. On `approach_close` the same arithmetic says
/// **320 Mpx** and a 42 GB heap, which is why the ladder can settle one path and not the other.
const SSAA_SCALES: [u32; 2] = [2, 4];

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
