//! BOOT-1 window tests that need a desktop but no GPU: the dark class brush (F1), and the boot
//! pump's thread-local seam and deferred close (F4).
//!
//! Every test opens a real Win32 window on the test thread, so the binary runs one test at a time:
//! `cargo test -p boyko_rhi_vulkan --test window_boot_pump -- --ignored --test-threads=1`.

#![cfg(windows)]

use core::ffi::c_void;

use boyko_rhi_vulkan::window::{Window, boot_pump_is_armed, pump_boot_messages};

// SAFETY: signatures match the Win64 user32 / gdi32 ABI (HWND / HGDIOBJ are opaque pointers;
// `GetClassLongPtrW` returns a `ULONG_PTR`).
#[link(name = "user32")]
unsafe extern "system" {
    fn GetClassLongPtrW(hwnd: *mut c_void, index: i32) -> usize;
    fn IsWindow(hwnd: *mut c_void) -> i32;
    fn IsWindowVisible(hwnd: *mut c_void) -> i32;
    fn ShowWindow(hwnd: *mut c_void, cmd: i32) -> i32;
    fn PostMessageW(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> i32;
    fn PeekMessageW(msg: *mut Msg, hwnd: *mut c_void, min: u32, max: u32, remove: u32) -> i32;
    fn TranslateMessage(msg: *const Msg) -> i32;
    fn DispatchMessageW(msg: *const Msg) -> isize;
    fn PostQuitMessage(code: i32);
}

/// `MSG`, as `PeekMessageW` writes it (Win64: 48 bytes).
#[repr(C)]
#[derive(Default)]
struct Msg {
    hwnd: usize,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: [i32; 2],
    private: u32,
}
const _: () = assert!(core::mem::size_of::<Msg>() == 48);

const WM_CLOSE: u32 = 0x0010;
const WM_QUIT: u32 = 0x0012;
const WM_USER: u32 = 0x0400;
const PM_REMOVE: u32 = 0x0001;
const SW_SHOW: i32 = 5;
/// How many messages [`drain_until_quit`] dispatches before it gives up: far above anything a
/// test window's queue holds, and a bound against a message that regenerates (an unvalidated
/// `WM_PAINT`).
const DRAIN_BOUND: usize = 10_000;
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

/// F4: during boot, `WM_CLOSE` hides the window at once and is reported, while the `HWND` stays
/// valid (the surface is live mid-boot); after `end_boot` a close destroys the window as before.
#[test]
#[ignore = "solo: opens a real Win32 window on the test thread and posts WM_CLOSE to it; run with --test-threads=1"]
fn close_during_boot_is_deferred_and_hides() {
    let window = Window::open("boyko boot1 deferred close", 64, 64).expect("a desktop session can open a window");
    let hwnd = window.hwnd();
    // SAFETY: `hwnd` is the live window just opened (shown even under BOYKO_WIN_HIDDEN, so the hide
    // is observable).
    unsafe { ShowWindow(hwnd, SW_SHOW) };
    window.begin_boot();
    // SAFETY: a posted message with no pointer payload to the live window.
    unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
    pump_boot_messages();
    assert!(window.close_requested(), "the close during boot is reported");
    // SAFETY: message-free queries on a window handle.
    let (alive, visible) = unsafe { (IsWindow(hwnd) != 0, IsWindowVisible(hwnd) != 0) };
    assert!(alive, "the HWND survives a close during boot (the surface still needs it)");
    assert!(!visible, "the window is hidden at once");
    let report = window.end_boot();
    assert!(report.close_requested, "end_boot reports the deferred close");
    assert_eq!(report.stage_pumps, 1, "one pump through the public seam");
    // SAFETY: as above.
    unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
    assert!(!window.pump_events(), "after end_boot a close destroys the window as before");
}

/// F4: unarmed, the seam drains nothing — a posted message stays queued for the frame loop.
#[test]
#[ignore = "solo: opens a real Win32 window on the test thread; run with --test-threads=1"]
fn boot_pump_is_inert_when_unarmed() {
    let window = Window::open("boyko boot1 inert pump", 64, 64).expect("a desktop session can open a window");
    let hwnd = window.hwnd();
    // SAFETY: as above.
    unsafe { PostMessageW(hwnd, WM_USER + 7, 0, 0) };
    pump_boot_messages();
    let mut msg = Msg::default();
    // SAFETY: a valid `MSG` out-pointer; the filter names exactly the posted message.
    let queued = unsafe { PeekMessageW(&mut msg, hwnd, WM_USER + 7, WM_USER + 7, PM_REMOVE) } != 0;
    assert!(queued, "an unarmed boot pump must not drain the queue");
    drop(window);
}

/// F4: a `WM_QUIT` met while draining is re-posted, so the frame loop's `pump_events` still sees it.
///
/// The quit is read back the way the frame loop reads it: drain (`PM_REMOVE`, translate, dispatch),
/// bounded, until `WM_QUIT` or an empty queue. A single filtered `PeekMessageW(.., WM_QUIT, WM_QUIT,
/// PM_NOREMOVE)` is not that: the system turns the quit flag into a message only once the queue
/// holds no posted message, whatever the filter (Raymond Chen, "Why is there a special
/// PostQuitMessage function?"), so any message posted after the pump returned hid the quit and
/// failed the check with no defect in the pump (triage r1 F3). The late post below makes that case
/// deterministic: the old check reads it as a swallowed quit.
#[test]
#[ignore = "solo: opens a real Win32 window on the test thread; run with --test-threads=1"]
fn boot_pump_reposts_wm_quit() {
    let window = Window::open("boyko boot1 quit repost", 64, 64).expect("a desktop session can open a window");
    let hwnd = window.hwnd();
    window.begin_boot();
    // SAFETY: a parameterless OS call on this thread.
    unsafe { PostQuitMessage(7) };
    pump_boot_messages();
    // SAFETY: a posted message with no pointer payload to the live window, landing after the pump.
    unsafe { PostMessageW(hwnd, WM_USER + 9, 0, 0) };
    let quit = drain_until_quit();
    assert_eq!(quit, Some(7), "the boot pump swallowed WM_QUIT (or changed its code)");
    let _ = window.end_boot();
    drop(window);
}

/// F4: a window dropped while its boot pump is armed (a boot panic, an early unwind, the runner's
/// bindless-failure teardown) leaves the thread's pump disarmed, so no later tick drains on behalf
/// of a window that no longer exists (triage r1 F4; the tester's r1 probe).
#[test]
#[ignore = "solo: opens a real Win32 window on the test thread; run with --test-threads=1"]
fn dropping_an_armed_window_disarms_the_boot_pump() {
    {
        let window = Window::open("boyko boot1 drop disarm", 64, 64).expect("a desktop session can open a window");
        window.begin_boot();
        assert!(boot_pump_is_armed(), "begin_boot arms the pump");
    }
    assert!(!boot_pump_is_armed(), "the pump stayed armed after its window was dropped");
}

/// Drains this thread's queue as the frame loop does (`PM_REMOVE`, translate, dispatch), at most
/// [`DRAIN_BOUND`] messages, until `WM_QUIT`; returns its exit code, or `None` when the queue
/// emptied (or the bound ran out) without one.
fn drain_until_quit() -> Option<usize> {
    let mut msg = Msg::default();
    for _ in 0..DRAIN_BOUND {
        // SAFETY: a valid `MSG` out-pointer; a null `hWnd` filter reads this thread's whole queue.
        if unsafe { PeekMessageW(&mut msg, core::ptr::null_mut(), 0, 0, PM_REMOVE) } == 0 {
            return None;
        }
        if msg.message == WM_QUIT {
            return Some(msg.wparam);
        }
        // SAFETY: `&msg` is the message just retrieved for this thread; translate + dispatch route
        // it to its window procedure and only read it.
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    None
}
