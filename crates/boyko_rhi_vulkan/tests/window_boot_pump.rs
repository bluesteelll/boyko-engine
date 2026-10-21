//! BOOT-1 window tests that need a desktop but no GPU: the dark class brush (F1).
//!
//! Every test opens a real Win32 window on the test thread, so the binary runs one test at a time:
//! `cargo test -p boyko_rhi_vulkan --test window_boot_pump -- --ignored --test-threads=1`.

#![cfg(windows)]

use core::ffi::c_void;

use boyko_rhi_vulkan::window::Window;

// SAFETY: signatures match the Win64 user32 / gdi32 ABI (HWND / HGDIOBJ are opaque pointers;
// `GetClassLongPtrW` returns a `ULONG_PTR`).
#[link(name = "user32")]
unsafe extern "system" {
    fn GetClassLongPtrW(hwnd: *mut c_void, index: i32) -> usize;
}
// SAFETY: as above.
#[link(name = "gdi32")]
unsafe extern "system" {
    fn GetStockObject(i: i32) -> *mut c_void;
}

const GCLP_HBRBACKGROUND: i32 = -10;
const BLACK_BRUSH: i32 = 4;

/// F1: the window class erases with the stock black brush, so the client area is dark from the
/// first `WM_ERASEBKGND` — never white before the first present.
#[test]
#[ignore = "solo: opens a real (hidden-capable) Win32 window on the test thread; run with --test-threads=1"]
fn class_background_is_the_stock_black_brush() {
    let window = Window::open("boyko boot1 class brush", 64, 64).expect("a desktop session can open a window");
    // SAFETY: `hwnd` is the live window just opened; the class slot read sends no message.
    let brush = unsafe { GetClassLongPtrW(window.hwnd(), GCLP_HBRBACKGROUND) };
    // SAFETY: a predefined stock-object index.
    let black = unsafe { GetStockObject(BLACK_BRUSH) } as usize;
    assert_ne!(black, 0, "GetStockObject(BLACK_BRUSH) returned null");
    assert_eq!(
        brush, black,
        "the window class background is {brush:#x}, not the stock black brush {black:#x}: \
         a null brush leaves the client area to DWM's white until the first present"
    );
}
