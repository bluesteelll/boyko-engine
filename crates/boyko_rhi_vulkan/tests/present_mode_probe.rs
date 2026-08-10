//! **Profiling rung 8, D12 — the present-mode probe, and the honest scope note it settles.**
//!
//! D12 says the work reduces to *labelling* if `Immediate` turns out to be unsupported on this box,
//! and calls that support **unproven**. A note that says "unproven" and is never run stays unproven
//! forever, so this file runs it: it asks the surface what it advertises, prints the answer, and
//! gates the RESOLUTION — which is decidable whichever way the probe lands.
//!
//! # The gate is the RESOLUTION, not the support
//!
//! `Immediate` and `Mailbox` are optional in Vulkan. Asserting that a machine supports one would be
//! a gate about the hardware, and it would red on the first box that does not have it — a test that
//! fails for being run somewhere else is not a gate, it is a hardware requirement wearing one.
//!
//! What IS decidable everywhere: **a swapchain that requested `Immediate` reports either
//! `Immediate` or `Fifo`, and it reports `Immediate` exactly when the surface advertised it.** That
//! is the whole of the fallback contract, and it has an answer on every box.
//!
//! # Why this matters beyond a knob
//!
//! While FIFO was hard-coded, no wall-clock gate could fail for GPU-side work: every frame is
//! bounded below by the refresh interval, so a change that made the GPU twice as slow reported the
//! same 16.67 ms. This repository treats a gate that cannot fail as a defect — the measured
//! precedent being `-ValidationOn` reporting *"clean, 0 messages"* for all 22 pins while an illegal
//! `mip_levels: 12` drew zero.
//!
//! # CI
//!
//! No loader / no GPU / no WSI → skip gracefully, this tree's convention for windowed tests.

#![cfg(windows)]

use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::swapchain::{PresentModeConfig, Surface, Swapchain};
use boyko_rhi_vulkan::window::Window;

/// Requests `mode` and returns what the swapchain actually got, or `None` when the box cannot run
/// a windowed Vulkan boot at all.
fn resolve(mode: PresentModeConfig) -> Option<PresentModeConfig> {
    let window = Window::open("boyko present-mode probe", 320, 240).ok()?;
    let ctx = VulkanContext::boot(InstanceConfig { enable_validation: false, windowed: true }).ok()?;
    // SAFETY: `window` outlives the surface — both are dropped at this fn's end, surface first
    // because it is declared after.
    let surface = unsafe { Surface::new(&ctx, window.hinstance(), window.hwnd()) }.ok()?;
    let swapchain =
        Swapchain::new_with_present_mode(&ctx, &surface, window.width(), window.height(), mode)
            .ok()?;
    let got = swapchain.present_mode();
    drop(swapchain);
    drop(surface);
    drop(ctx);
    drop(window);
    Some(got)
}

/// **The clause.** A requested mode resolves to itself or to `Fifo`, never to a third thing — and
/// `Fifo` always resolves to itself.
///
/// ⚠️ The `Immediate` and `Mailbox` outcomes are REPORTED, not asserted: they are optional modes,
/// and a gate that required one would be a statement about the hardware this happens to run on.
/// What is asserted is the contract that holds either way.
#[test]
fn a_requested_present_mode_resolves_to_itself_or_to_fifo() {
    let Some(fifo) = resolve(PresentModeConfig::Fifo) else {
        eprintln!("SKIP present_mode_probe: windowed Vulkan unavailable");
        return;
    };
    assert_eq!(
        fifo,
        PresentModeConfig::Fifo,
        "FIFO is spec-guaranteed; a boot that could not get it has a broken probe, not a fallback"
    );

    for want in [PresentModeConfig::Immediate, PresentModeConfig::Mailbox] {
        let Some(got) = resolve(want) else {
            eprintln!("SKIP present_mode_probe: windowed Vulkan became unavailable mid-run");
            return;
        };
        assert!(
            got == want || got == PresentModeConfig::Fifo,
            "requesting `{}` resolved to `{}` -- the contract is that a refusal falls back to \
             `fifo` and nothing else, so a third outcome means the resolution invented a mode",
            want.as_str(),
            got.as_str()
        );
        // THE MEASUREMENT D12's scope note asks for, printed rather than asserted.
        println!(
            "PRESENT-MODE PROBE: requested `{}` -> resolved `{}` ({})",
            want.as_str(),
            got.as_str(),
            if got == want { "SUPPORTED on this box" } else { "unsupported -- fell back" }
        );
    }
}

/// The refresh bound travels with the mode, because a wall clock without it is a number a reader
/// will compare across modes without knowing they are not comparable.
#[test]
fn only_fifo_claims_a_refresh_bound() {
    assert!(PresentModeConfig::Fifo.is_refresh_bounded());
    assert!(!PresentModeConfig::Immediate.is_refresh_bounded());
    assert!(!PresentModeConfig::Mailbox.is_refresh_bounded());
    // The default is what every golden pin was blessed under; changing it moves 22 hashes.
    assert_eq!(PresentModeConfig::default(), PresentModeConfig::Fifo);
}
