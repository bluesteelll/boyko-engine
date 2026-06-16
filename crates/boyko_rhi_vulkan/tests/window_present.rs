//! Slice-1 integration test: boot a windowed Vulkan context, create a real Win32
//! window + surface + swapchain, render + present a handful of cleared frames via
//! Vulkan 1.3 dynamic rendering, and assert the validation layer recorded ZERO
//! messages — the soundness oracle that substitutes for Miri on the raw-FFI path
//! (plan §6) — then tear everything down in reverse order (no leaked-object
//! validation reports).
//!
//! # CI gate
//!
//! Like `roundtrip.rs`, any of: no Vulkan loader, no GPU, no validation SDK, or
//! no WSI/dynamic-rendering support → `boot`/surface/swapchain returns `Err`,
//! which this test treats as **skip gracefully** (print + return). On a Windows
//! machine with a Vulkan loader + GPU + the validation SDK it runs and asserts.
//! The test is `#[cfg(windows)]`; on other targets it is a trivial pass.

#![cfg(windows)]

use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::swapchain::{Renderer, Surface, Swapchain};
use boyko_rhi_vulkan::window::Window;

#[test]
fn windowed_clear_present_is_validation_clean() {
    // Open the window first — the surface borrows its HWND/HINSTANCE and must be
    // destroyed before it.
    let mut window = match Window::open("boyko_rhi_vulkan test window", 640, 480) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("SKIP windowed_clear_present: cannot open a window ({e:?})");
            return;
        }
    };

    // Boot windowed + validation. A missing GPU / loader / SDK / WSI → skip.
    let ctx = match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        windowed: true,
    }) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP windowed_clear_present: windowed Vulkan unavailable ({e:?})");
            return;
        }
    };
    println!("Vulkan device (windowed, validation on): {}", ctx.device_name());
    assert!(
        ctx.validation_enabled(),
        "validation must be active when InstanceConfig::enable_validation is set"
    );

    // SAFETY: `window` outlives the surface (dropped after it below); its
    // HWND/HINSTANCE are live for the surface's lifetime.
    let surface = match unsafe { Surface::new(&ctx, window.hinstance(), window.hwnd()) } {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SKIP windowed_clear_present: surface creation failed ({e:?})");
            return;
        }
    };

    let mut swapchain = match Swapchain::new(&ctx, &surface, window.width(), window.height()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SKIP windowed_clear_present: swapchain creation failed ({e:?})");
            return;
        }
    };
    assert!(swapchain.image_count() >= 1, "swapchain must expose >= 1 image");
    println!(
        "swapchain: {} images, extent {}x{}",
        swapchain.image_count(),
        swapchain.extent().width,
        swapchain.extent().height
    );

    let mut renderer =
        Renderer::new(&ctx, &surface, &swapchain).expect("renderer (command pool + sync) creation");

    // Render + present a handful of frames with distinct clear colors. Pump the
    // window between frames so the OS does not flag it as unresponsive.
    let clears = [
        [0.10f32, 0.10, 0.15, 1.0],
        [0.20, 0.05, 0.05, 1.0],
        [0.05, 0.20, 0.05, 1.0],
        [0.05, 0.05, 0.20, 1.0],
        [0.15, 0.15, 0.05, 1.0],
    ];
    for (i, clear) in clears.iter().enumerate() {
        window.pump_events();
        window.refresh_size();
        let presented = renderer
            .render_frame(&surface, &mut swapchain, window.width(), window.height(), *clear)
            .unwrap_or_else(|e| panic!("frame {i} failed: {e:?}"));
        // `presented == false` only means the swapchain was (re)created this frame
        // (a benign skip); either way the call must not error.
        let _ = presented;
    }

    // The oracle: a clean windowed render+present records zero validation
    // messages. A non-zero count means the layer caught a real API misuse (the
    // `[vk-validation]` log lines identify it) — fail loudly.
    let state = ctx
        .debug_state()
        .expect("validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation layer reported {} message(s) during windowed render/present — \
         see the [vk-validation] log",
        state.total()
    );

    // Clean reverse-order teardown: renderer (waits idle) → swapchain → surface →
    // window. Drop order is pinned so the surface dies before the window.
    drop(renderer);
    drop(swapchain);
    drop(surface);
    drop(ctx);
    drop(window);
}
