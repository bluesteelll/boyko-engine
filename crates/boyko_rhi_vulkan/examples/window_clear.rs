//! Slice-1 demo — open a raw Win32 window, boot a windowed Vulkan 1.3 context,
//! and present an animated clear color until the window is closed.
//!
//! Run with:
//! ```text
//! cargo run --example window_clear -p boyko_rhi_vulkan
//! ```
//!
//! Validation is ENABLED, so any API misuse is logged to stderr as
//! `[vk-validation] ...` lines (the soundness oracle, plan §6). The clear color
//! cycles through hue over time so the window visibly animates.
//!
//! On non-Windows targets this prints a notice and exits 0 (windowing is
//! Windows-first per D8; the XCB/Wayland arm lands when Linux on-screen is first
//! targeted).

#[cfg(windows)]
fn main() {
    use std::time::Instant;

    use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
    use boyko_rhi_vulkan::swapchain::{Renderer, Surface, Swapchain};
    use boyko_rhi_vulkan::window::Window;

    // Open the window first (the surface borrows its HWND/HINSTANCE).
    let mut window = match Window::open("boyko-engine — window_clear (Slice 1)", 1280, 720) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("failed to open window: {e:?}");
            return;
        }
    };

    // Boot a windowed, validation-enabled context.
    let ctx = match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        windowed: true,
    }) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("failed to boot windowed Vulkan context: {e:?}");
            eprintln!("(needs a Vulkan loader + GPU + the validation SDK installed)");
            return;
        }
    };
    println!("Vulkan device: {}", ctx.device_name());

    // SAFETY: `window` outlives `surface` (it is dropped after, below); its
    // HWND/HINSTANCE are live for the surface's lifetime.
    let surface = match unsafe { Surface::new(&ctx, window.hinstance(), window.hwnd()) } {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to create surface: {e:?}");
            return;
        }
    };

    let mut swapchain = match Swapchain::new(&ctx, &surface, window.width(), window.height()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to create swapchain: {e:?}");
            return;
        }
    };

    let mut renderer = match Renderer::new(&ctx, &surface, &swapchain) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("failed to create renderer: {e:?}");
            return;
        }
    };

    println!("rendering — close the window to exit.");
    let start = Instant::now();
    loop {
        if !window.pump_events() {
            break;
        }
        // A resize may have changed the client area; the renderer recreates the
        // swapchain on out-of-date, but feed it the latest size.
        window.refresh_size();

        let t = start.elapsed().as_secs_f32();
        let clear = hue_to_rgb(t * 0.15);

        match renderer.render_frame(
            &surface,
            &mut swapchain,
            window.width(),
            window.height(),
            clear,
        ) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("render error: {e:?}");
                break;
            }
        }
    }

    // Clean reverse-order teardown: renderer (waits idle) → swapchain → surface →
    // window. Each `Drop` destroys its own objects; the explicit `drop`s pin the
    // order so the surface dies before the window it borrows.
    drop(renderer);
    drop(swapchain);
    drop(surface);
    drop(ctx);
    drop(window);

    println!("clean exit.");
}

/// A simple hue→RGB sweep (saturation = value = 1) so the clear visibly cycles.
/// `h` is in turns (wraps mod 1).
#[cfg(windows)]
fn hue_to_rgb(h: f32) -> [f32; 4] {
    let h = (h.fract() + 1.0).fract() * 6.0;
    let c = 1.0f32;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r, g, b, 1.0]
}

#[cfg(not(windows))]
fn main() {
    eprintln!(
        "window_clear is Windows-only for now (D8: our window, Windows-first). \
         The XCB/Wayland arm lands when Linux on-screen is first targeted."
    );
}
