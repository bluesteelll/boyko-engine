//! The windowed clear-color runner (host plan D6, R2 subset).
//!
//! Installed by [`EnginePlugins`](crate::plugins::EnginePlugins) via
//! `App::set_runner`. Owns the whole app lifecycle: the device-singleton boot,
//! the window host boot, the World's GPU residents, `finish()`, the frame
//! loop, and the D2 teardown — in that order, by construction.

use boyko_ecs::{App, AppExit};

#[cfg(windows)]
use std::time::Instant;

#[cfg(windows)]
use boyko_input::{ButtonState, KeyCode, RawInputEvent, translate_win32};
#[cfg(windows)]
use boyko_render::RhiContext;
#[cfg(windows)]
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
#[cfg(windows)]
use boyko_rhi_vulkan::window::CapturedMsg;

#[cfg(windows)]
use crate::device::GpuDevice;
#[cfg(windows)]
use crate::host::WindowHost;

/// Window description handed from [`EnginePlugins`](crate::plugins::EnginePlugins)
/// to the runner (R2 subset: title + requested client size; `present_mode` etc.
/// arrive with later rungs).
#[derive(Clone, Copy)]
pub(crate) struct WindowDesc {
    /// The window caption.
    pub(crate) title: &'static str,
    /// Requested client-area width in pixels.
    pub(crate) width: u32,
    /// Requested client-area height in pixels.
    pub(crate) height: u32,
}

/// The R2 clear color presented every frame — a dark neutral so a live window
/// is visually distinct from an undrawn one.
#[cfg(windows)]
const CLEAR_COLOR: [f32; 4] = [0.05, 0.07, 0.10, 1.0];

/// The windowed runner body (host plan D6): boot → World residents →
/// insert-if-absent `AppExit(false)` → `finish()` → frame loop → D2 teardown.
///
/// Boot failure (no loader / GPU / window) is NOT a panic: the runner logs one
/// line at the binary boundary and returns `AppExit(true)` — the app must exit
/// gracefully on a GPU-less machine.
///
/// The shipped runner does NOT request the validation layer (review P1-3): per
/// [`InstanceConfig`]'s contract an ABSENT `VK_LAYER_KHRONOS_validation` fails
/// boot with `ValidationUnavailable` (no silent fallback), which would kill
/// the app on every machine without the Vulkan SDK. The messenger oracle stays
/// a test-harness convention; a debug validation knob arrives with a later
/// rung.
#[cfg(windows)]
pub(crate) fn run_windowed(app: &mut App, desc: WindowDesc) -> AppExit {
    // ── Boot: the device singleton (plan D2 step 1). ─────────────────────────
    let config = InstanceConfig {
        enable_validation: false,
        windowed: true,
    };
    let ctx = match VulkanContext::boot_singleton(config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("boyko_app: Vulkan device boot failed - exiting ({e:?})");
            return AppExit(true);
        }
    };

    // ── Boot: the window host chain (window → surface → swapchain → renderer).
    let mut host = match WindowHost::boot(ctx, &desc) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("boyko_app: window host boot failed - exiting ({e})");
            // SAFETY: the `boot_singleton` above succeeded and its singleton
            // is still live (the null-swap tripwire would catch a violation);
            // the device is idle (no GPU work was ever submitted); and no
            // `&'static VulkanContext` reference remains in any live structure
            // — the failed boot chain dropped its partial links before
            // returning, nothing was inserted into the World yet, and `ctx` is
            // not used past this statement.
            unsafe { VulkanContext::destroy_singleton() };
            return AppExit(true);
        }
    };

    // ── World residents BEFORE `finish()` so the startup one-shots drain WITH
    // the device present (plan D2 step 4 / D6). Both are NonSend: all GPU
    // access stays runner-thread-only.
    app.world_mut()
        .insert_non_send_resource(RhiContext::from_shared(ctx));
    app.world_mut().insert_non_send_resource(GpuDevice(ctx));

    // ── Windowed `AppExit` semantics: insert-IF-ABSENT (plan D6; the legacy
    // headless path keeps its unconditional insert).
    if !app.world().contains_resource::<AppExit>() {
        app.world_mut().insert_resource(AppExit(false));
    }

    app.finish();

    // A startup-requested exit is honored (plan D6): skip the loop, tear down.
    if !app.world().resource::<AppExit>().0 {
        frame_loop(app, &mut host);
    }

    teardown(app, host);
    AppExit(true)
}

/// The non-Windows runner arm: windowing is Windows-first (`boyko_rhi_vulkan`
/// D8 — the XCB/Wayland arm lands when Linux on-screen is first targeted), so
/// the windowed runner exits gracefully, mirroring the boot-failure path.
#[cfg(not(windows))]
pub(crate) fn run_windowed(_app: &mut App, _desc: WindowDesc) -> AppExit {
    eprintln!("boyko_app: windowing is not supported on this platform - exiting");
    AppExit(true)
}

/// The per-frame loop (plan D6 / the runner-frame table, R2 subset): pump →
/// input drain (Escape only) → `update_with_delta` → `AppExit` check → clear
/// present → size refresh. Returns when the window closes, Escape is pressed,
/// a system requests exit, or the renderer fails terminally.
#[cfg(windows)]
fn frame_loop(app: &mut App, host: &mut WindowHost) {
    let mut last = Instant::now();
    loop {
        // 1. Pump the OS queue; `false` = WM_QUIT (the window closed).
        if !host.window.pump_events() {
            return;
        }

        // 2. Drain the captured input. R2 discards everything except the
        //    Escape key-down exit; the InputPlugin ingest arrives in R6.
        let mut escape = false;
        host.window.drain_input(|captured| {
            if let CapturedMsg::Raw { msg, wparam, lparam } = captured
                && let Some(RawInputEvent::Key {
                    code: KeyCode::Escape,
                    state: ButtonState::Pressed,
                    ..
                }) = translate_win32(msg, wparam, lparam)
            {
                escape = true;
            }
        });
        if escape {
            return;
        }

        // 3. The ECS frame with the real wall delta (Time clamps/scales it).
        let now = Instant::now();
        let dt = now - last;
        last = now;
        app.update_with_delta(dt);

        // 4. `AppExit` check — after the frame completes, before the present.
        if app.world().resource::<AppExit>().0 {
            return;
        }

        // 5. Present ONE cleared frame. R2 mints NO `FrameWriteToken`:
        //    `render_frame` is the tokenless clear path (it waits this slot's
        //    in-flight fence internally), and no per-slot mapped host write
        //    exists yet to demand the fence proof — the token flow arrives
        //    with the first per-slot write in R3.
        match host.renderer.render_frame(
            &host.surface,
            &mut host.swapchain,
            host.window.width(),
            host.window.height(),
            CLEAR_COLOR,
        ) {
            // Presented normally.
            Ok(true) => {}
            // Swapchain (re)created this call, or zero extent (minimized):
            // the frame was skipped; keep pumping — the size refresh below
            // feeds the next attempt.
            Ok(false) => {}
            // Terminal: the renderer must not be reused after a post-acquire
            // failure (`frame_driver` contract) — exit and tear down.
            Err(e) => {
                eprintln!("boyko_app: terminal render error - exiting ({e:?})");
                return;
            }
        }

        // 6. Re-observe the client size so a resize feeds the next frame's
        //    render / recreate (plan runner-frame step 8; `WindowInfo` lands
        //    in R3).
        host.window.refresh_size();
    }
}

/// The D2 teardown — a named, ordered sequence; every step is load-bearing.
///
/// Takes no device handle: `destroy_singleton` is paramless by soundness
/// necessity (review P0 — a reference parameter would be protected for the
/// call, making the in-call deallocation UB) and reclaims the allocation
/// through the owning pointer retained inside `boyko_rhi_vulkan`.
#[cfg(windows)]
fn teardown(app: &mut App, host: WindowHost) {
    // Steps 1+2 — drop the host. `Renderer` exposes no public wait-idle API;
    // its `Drop` performs the `vkDeviceWaitIdle` FIRST (frame_driver.rs), then
    // `WindowHost`'s declared field order drops renderer → swapchain →
    // surface → window (the surface before the window it borrows).
    drop(host);

    // Step 3 — EVICT every device-referencing World resident (the runner
    // borrows the App and cannot drop it — explicit eviction is the
    // replacement; plan critic delta A1):
    //   - `RhiContext` (shared mode): its `Drop` runs `destroy_all` (frees
    //     every column/UI resource; the registry teardown wait-idles the
    //     already-idle device — a benign no-op) and NEVER touches the device
    //     lifecycle;
    //   - `GpuDevice`: the last world-resident `&'static` handle — no dangling
    //     `&'static` may remain in a live structure past this point.
    drop(app.world_mut().remove_non_send_resource::<RhiContext>());
    // `GpuDevice` is a plain reference newtype (no `Drop` glue) — removal alone
    // ends its residency; the returned `Option` is discarded.
    let _ = app.world_mut().remove_non_send_resource::<GpuDevice>();

    // Step 4 — end the singleton's lifecycle. The LAST statement.
    // SAFETY: `run_windowed`'s `boot_singleton` succeeded and its singleton is
    // still live — this is the runner's only destroy on this path (the
    // null-swap tripwire would catch a violation); the device is idle
    // (`Renderer::Drop` waited idle in steps 1-2 and nothing submitted since);
    // and NO `&'static VulkanContext` reference remains in any live structure
    // — the host chain (renderer / swapchain / surface / window) dropped in
    // steps 1-2, the World's GPU residents (`RhiContext`, `GpuDevice`) were
    // evicted and dropped in step 3, and `run_windowed` does not use its `ctx`
    // local past this call — so the documented `'static` fiction ends with no
    // surviving reference.
    unsafe { VulkanContext::destroy_singleton() };
}
