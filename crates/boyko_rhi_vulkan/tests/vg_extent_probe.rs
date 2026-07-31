//! VG-R0 extent probe — measures the largest grantable window CLIENT extent on this box.
//!
//! This is the PRODUCER of the per-rung extent-route input that
//! `docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md` section 9.1 records: a census ladder rung `R` is
//! reachable iff `R ∈ {C, 2C}` for some OS-granted client extent `C` (the `2×` arm is the armed
//! SSAA composite; the host admits only `SSAA_SCALE = 2`). Whether a candidate `C` is granted is a
//! property of THIS box's window manager, not of the engine, so it is measured here — one hidden
//! `Window::open` per candidate plus the constructor's own `GetClientRect` refresh — and the census
//! (`[census].assert_achieved_extent`) re-asserts the achieved extent per rung at run time.
//!
//! The probe MEASURES and REPORTS; it does not gate. A clamped candidate is a fact about the box
//! (recorded in the plan's section 9.1 table), not a test failure. The only assertions are that
//! window creation succeeds and returns a non-zero client area — an OS failure would leave the
//! measurement with no reading at all.
//!
//! Candidates: every ladder rung's native extent and every rung's SSAA-halved client
//! (`rung / 2`, the route through the 2× composite). Run with `--nocapture` to see the table:
//!
//! ```text
//! cargo test -p boyko_rhi_vulkan --test vg_extent_probe -- --nocapture
//! ```
#![cfg(windows)]

use boyko_rhi_vulkan::window::Window;

/// Ladder-rung natives and their SSAA-halved clients, deduplicated
/// (`1920×1080` is both rung 1's native and rung 3's halved client).
const CANDIDATES: [(u32, u32); 6] = [
    (512, 512),
    (960, 540),
    (1280, 720),
    (1920, 1080),
    (2560, 1440),
    (3840, 2160),
];

#[test]
fn the_largest_grantable_client_extent_is_measured_and_reported() {
    // SAFETY: this binary contains exactly one test, so no other thread is
    // reading the environment when the knob is set (the same discipline as the
    // sibling `window_present_gbuffer.rs` knob writes). Hidden windows keep
    // their non-zero client extent (see `Window::open`), so the measurement is
    // unchanged; only desktop visibility differs.
    unsafe { std::env::set_var("BOYKO_WIN_HIDDEN", "1") };

    println!("requested (client) -> granted (client)");
    for (w, h) in CANDIDATES {
        let win = Window::open("vg extent probe", w, h)
            .expect("invariant: window creation must succeed for the probe to read anything");
        let (gw, gh) = (win.width(), win.height());
        assert!(gw > 0 && gh > 0, "granted client area is zero at {w}x{h}");
        let verdict = if (gw, gh) == (w, h) { "GRANTED" } else { "CLAMPED" };
        println!("{w:>4}x{h:<4} -> {gw:>4}x{gh:<4}  {verdict}");
        drop(win);
    }
}
