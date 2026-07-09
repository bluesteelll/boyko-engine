//! Slice-1 — a raw Win32 window (no winit), the on-screen seam.
//!
//! Per `docs/RENDER-PHYSICS-GPU-PLAN.md` D8 ("окно наше" — our window), the
//! foundation's first on-screen path uses a hand-rolled Win32 window via raw FFI
//! to `user32` / `kernel32` (mirroring the `boyko_ecs::ecs::memory::vm.rs`
//! `LoadLibrary`/`extern "system"` discipline). The Vulkan surface
//! ([`crate::swapchain::Surface`]) is created from this window's `HWND` +
//! `HINSTANCE`. The INPUT FFI it uses (`SetWindowLongPtrW` / `GetRawInputData` /
//! the `RAWINPUT*` structs and `WM_*` / `GWLP_USERDATA` constants) comes from the
//! official MS-maintained `windows-sys` bindings, re-exported through
//! [`crate::ffi::os`]; the window-creation / message-pump FFI stays hand-rolled
//! (its `HWND`/`HINSTANCE` are `*mut c_void`, identical to the windows-sys aliases,
//! so no cast is needed at the input call sites).
//!
//! # The window-procedure / close contract
//!
//! A `WNDPROC` is a plain `extern "system" fn` and cannot capture state. The
//! close signal flows through the message queue itself: the OS routes `WM_CLOSE`
//! → `DefWindowProcW` → `WM_DESTROY`, the `window_proc` handles `WM_DESTROY` by
//! calling `PostQuitMessage(0)`, and [`Window::pump_events`] reports "should
//! close" the moment `PeekMessageW` yields a `WM_QUIT`.
//!
//! # Input capture (I6 / I6b)
//!
//! Keyboard / mouse messages DO need shared state — a ring the stateless
//! `window_proc` writes into and [`Window::drain_input`] reads. That ring is
//! reached via the one per-window pointer slot `SetWindowLongPtrW(GWLP_USERDATA)`
//! provides: [`Window::open`] boxes an [`InputRing`], stores its raw pointer in
//! the slot, and `window_proc` retrieves it. The pointer is cleared and the box
//! reclaimed on `WM_DESTROY` so it never outlives the window. The proc captures
//! raw `(msg, wparam, lparam)` triples ([`CapturedMsg`]) — it does NOT translate
//! them, keeping `boyko_input` (which owns `translate`) a leaf with no edge from
//! this crate. The one exception is `WM_INPUT` (I6b): its `lParam` is a transient
//! `HRAWINPUT` handle only valid inside the message, so the proc parses the
//! `RAWINPUT` blob immediately (FFI) and stores the resulting relative delta as a
//! [`CapturedMsg::RawMouse`] variant the edge maps via
//! `boyko_input::win32::translate_raw_mouse`.
//!
//! # Lifetime / teardown
//!
//! [`Window`] owns the `HWND` and the registered class atom; its `Drop` destroys
//! the window then unregisters the class, in reverse registration order. The
//! Vulkan surface borrows the `HWND`/`HINSTANCE` and MUST be destroyed before the
//! window (the caller orders this — see the integration test / example).
//!
//! # Non-Windows
//!
//! On non-Windows targets [`Window::open`] returns
//! [`WindowError::UnsupportedPlatform`] so the crate still compiles cross-target
//! (the XCB/Wayland arm is added when Linux on-screen is first targeted).

#[cfg(windows)]
use core::ffi::c_void;

/// Capacity of the per-window captured-input ring (I6/I6b). A power of two for a
/// branchless wrap; 1024 comfortably absorbs a single frame's input burst (it
/// matches `boyko_input::constants::RAW_QUEUE_CAP`). Overflow is drop-oldest.
#[cfg(windows)]
const INPUT_RING_CAP: usize = 1024;

/// Errors from window creation / the OS windowing layer.
#[derive(Debug)]
pub enum WindowError {
    /// `GetModuleHandleW` returned a null HINSTANCE.
    NoModuleHandle,
    /// `RegisterClassExW` failed (returned a zero atom).
    ClassRegistrationFailed,
    /// `CreateWindowExW` returned a null HWND.
    WindowCreationFailed,
    /// Windowing is not implemented for this OS (non-Windows; the XCB arm is
    /// added when Linux on-screen is first targeted).
    UnsupportedPlatform,
}

/// One captured input message, as drained by [`Window::drain_input`] (I6/I6b).
///
/// The variants carry exactly the data the source-agnostic translation layer
/// needs, with NO `boyko_input` dependency (the renderer crate stays free of the
/// input crate — the edge owns the translation):
///
/// - [`CapturedMsg::Raw`] — a verbatim Win32 `(msg, wparam, lparam)` triple
///   (keyboard / mouse-button / move / wheel). The edge feeds it to
///   `boyko_input::win32::translate(msg, wparam, lparam)`.
/// - [`CapturedMsg::RawMouse`] — a pre-parsed relative-mouse delta from a
///   `WM_INPUT` message (I6b), whose transient `HRAWINPUT` handle cannot be
///   deferred to drain time. The edge feeds it to
///   `boyko_input::win32::translate_raw_mouse(dx, dy)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapturedMsg {
    /// A verbatim Win32 message triple awaiting `win32::translate`.
    Raw {
        /// The `uMsg` (e.g. `WM_KEYDOWN`).
        msg: u32,
        /// The `wParam`.
        wparam: usize,
        /// The `lParam`.
        lparam: isize,
    },
    /// A parsed relative-mouse delta from `WM_INPUT` (I6b) awaiting
    /// `win32::translate_raw_mouse`.
    RawMouse {
        /// Signed relative X motion (`RAWMOUSE::lLastX`).
        dx: i32,
        /// Signed relative Y motion (`RAWMOUSE::lLastY`).
        dy: i32,
    },
}

/// A fixed-capacity drop-oldest ring of [`CapturedMsg`], owned by a [`Window`]
/// and written by the stateless `window_proc` through a `GWLP_USERDATA` pointer.
///
/// Drop-oldest (mirroring `boyko_input::RawInputQueue`'s policy): on a slow
/// frame the newest input — the player's latest intent — survives; the oldest
/// stale events are evicted. Consecutive raw-mouse deltas are COALESCED at push
/// (they are additive and the only high-rate source — 1–8 kHz mice, plus the
/// backlog flush on the first pump after a slow boot), so in practice the ring
/// holds human-rate events and overflow is load-shedding for pathological
/// bursts, never a fault. The ring is drained fully by [`Window::drain_input`]
/// each frame. `head`/`tail` use a power-of-two mask for a branchless wrap.
#[cfg(windows)]
struct InputRing {
    buf: Box<[CapturedMsg]>,
    /// Index of the oldest element (read cursor).
    head: usize,
    /// Index one past the newest element (write cursor).
    tail: usize,
    /// `tail - head`, kept explicit so a full ring is distinguishable from empty
    /// without sacrificing a slot.
    len: usize,
    /// Count of drop-oldest evictions since the last drain (debug observability).
    dropped: usize,
}

#[cfg(windows)]
impl InputRing {
    /// A placeholder used to fill the freshly-allocated ring; never observed by a
    /// reader (only `head..head+len` slots are live).
    const FILL: CapturedMsg = CapturedMsg::RawMouse { dx: 0, dy: 0 };

    /// Allocates a ring of `cap` slots (rounded up to a power of two, min 1) once.
    fn with_capacity(cap: usize) -> Self {
        let cap = cap.max(1).next_power_of_two();
        Self {
            buf: vec![Self::FILL; cap].into_boxed_slice(),
            head: 0,
            tail: 0,
            len: 0,
            dropped: 0,
        }
    }

    /// Power-of-two index mask for the branchless wrap.
    #[inline]
    fn mask(&self) -> usize {
        self.buf.len() - 1
    }

    /// Pushes one captured message, coalescing consecutive raw-mouse deltas and
    /// evicting the oldest entry if the ring is genuinely full.
    ///
    /// Relative mouse deltas are additive, and every consumer sums them per
    /// frame (`PhysicalInput::mouse_delta` accumulates across the drain), so
    /// merging a `RawMouse` into a `RawMouse` NEWEST entry is
    /// semantics-preserving. Only CONSECUTIVE mouse events merge — a key event
    /// between two deltas splits the run — so ordering relative to key events
    /// is preserved exactly.
    fn push(&mut self, ev: CapturedMsg) {
        let mask = self.mask();
        if let CapturedMsg::RawMouse { dx, dy } = ev
            && self.len > 0
        {
            let newest = self.tail.wrapping_sub(1) & mask;
            if let CapturedMsg::RawMouse { dx: ndx, dy: ndy } = &mut self.buf[newest] {
                // Saturating: an unbounded backlog (hours of motion before the
                // first pump) must clamp, not wrap.
                *ndx = ndx.saturating_add(dx);
                *ndy = ndy.saturating_add(dy);
                return;
            }
        }
        if self.len == self.buf.len() {
            // Full: drop the oldest by advancing head before overwriting at tail.
            self.head = (self.head + 1) & mask;
            self.dropped += 1;
        } else {
            self.len += 1;
        }
        self.buf[self.tail] = ev;
        self.tail = (self.tail + 1) & mask;
    }

    /// Pops the oldest captured message, or `None` if empty.
    #[inline]
    fn pop(&mut self) -> Option<CapturedMsg> {
        if self.len == 0 {
            return None;
        }
        let ev = self.buf[self.head];
        self.head = (self.head + 1) & self.mask();
        self.len -= 1;
        Some(ev)
    }
}

/// A raw Win32 window owning its `HWND` + registered class atom.
///
/// The `HWND` / `HINSTANCE` getters feed [`crate::swapchain::Surface::new`].
/// [`pump_events`](Self::pump_events) drains the message queue and returns
/// `false` once the window has been asked to close.
#[cfg(windows)]
pub struct Window {
    /// The `HWND` (opaque). Destroyed in `Drop`.
    hwnd: *mut c_void,
    /// The `HINSTANCE` of the process image (from `GetModuleHandleW(null)`).
    hinstance: *mut c_void,
    /// The registered class atom — unregistered in `Drop` after the window dies.
    class_atom: u16,
    /// The NUL-terminated UTF-16 class name kept alive for `UnregisterClassW`
    /// (which takes the class name, not the atom, on this path).
    class_name: Vec<u16>,
    /// Cached client-area dimensions (updated by [`Self::refresh_size`]).
    width: u32,
    height: u32,
    /// Owning raw pointer to the captured-input ring (I6/I6b). The ring is
    /// heap-allocated in [`Self::open`] (so its address is stable) and the SAME
    /// pointer is installed into `GWLP_USERDATA` for the stateless `window_proc`
    /// to reach. [`Window`] is the SOLE owner: `Drop` calls `DestroyWindow`
    /// (which synchronously dispatches `WM_DESTROY`, where the proc clears the
    /// `GWLP_USERDATA` slot so no later message can dereference it) and then
    /// reclaims the box via `Box::from_raw`. The pointer is never null after a
    /// successful `open`.
    input_ring: *mut InputRing,
}

#[cfg(windows)]
impl Window {
    /// Opens a window of `width` × `height` client pixels with the given title.
    ///
    /// Registers a window class with `window_proc`, creates an overlapped
    /// (titled, resizable) window sized so its *client area* matches the request,
    /// and shows it. Returns a [`WindowError`] (never panics) on any OS failure.
    pub fn open(title: &str, width: u32, height: u32) -> Result<Self, WindowError> {
        use crate::ffi::os;

        // SAFETY: `GetModuleHandleW(null)` returns the HINSTANCE of the calling
        // process image per the Win32 contract; null-checked before use.
        let hinstance = unsafe { os::GetModuleHandleW(core::ptr::null()) };
        if hinstance.is_null() {
            return Err(WindowError::NoModuleHandle);
        }

        // A process-unique class name (UTF-16, NUL-terminated). The pointer is
        // taken below and must stay valid for the `RegisterClassExW` call and for
        // `UnregisterClassW` in `Drop`, so the `Vec` is owned by `self`.
        let class_name = to_wide("boyko_rhi_vulkan_window_class");
        let title_wide = to_wide(title);

        // SAFETY: `LoadCursorW(null, IDC_ARROW)` loads the shared arrow cursor; a
        // null instance with a predefined `MAKEINTRESOURCE` id is the documented
        // call. A null return is tolerated (the OS falls back to a default).
        let cursor = unsafe { os::LoadCursorW(core::ptr::null_mut(), os::IDC_ARROW as *const u16) };

        let class = os::WNDCLASSEXW {
            cb_size: core::mem::size_of::<os::WNDCLASSEXW>() as u32,
            style: os::CS_HREDRAW | os::CS_VREDRAW,
            lpfn_wnd_proc: Some(window_proc),
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance: hinstance,
            h_icon: core::ptr::null_mut(),
            h_cursor: cursor,
            hbr_background: core::ptr::null_mut(),
            lpsz_menu_name: core::ptr::null(),
            lpsz_class_name: class_name.as_ptr(),
            h_icon_sm: core::ptr::null_mut(),
        };
        // SAFETY: `class` is a fully-initialized `#[repr(C)]` `WNDCLASSEXW` whose
        // `cb_size` is its own size and whose pointer members (`class_name`,
        // `hinstance`, `cursor`) outlive the call; `RegisterClassExW` returns a
        // non-zero atom on success, 0 on failure.
        let class_atom = unsafe { os::RegisterClassExW(&class) };
        if class_atom == 0 {
            return Err(WindowError::ClassRegistrationFailed);
        }

        // Size the window so the *client area* is exactly width × height: compute
        // the outer rect by inflating the client rect with the frame.
        let mut rect = os::RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 };
        // SAFETY: `&mut rect` is a valid out-pointer for the `#[repr(C)]` `RECT`
        // the OS inflates in place; the style matches the window we create below.
        unsafe { os::AdjustWindowRectEx(&mut rect, os::WS_OVERLAPPEDWINDOW, 0, 0) };
        let outer_w = rect.right - rect.left;
        let outer_h = rect.bottom - rect.top;

        // SAFETY: the class atom was just registered against `hinstance`; the
        // class name + title are NUL-terminated UTF-16 strings alive for the
        // call; the parent/menu/param handles are null (a top-level window);
        // `CreateWindowExW` returns the HWND or null (checked below).
        let hwnd = unsafe {
            os::CreateWindowExW(
                0,
                class_name.as_ptr(),
                title_wide.as_ptr(),
                os::WS_OVERLAPPEDWINDOW,
                os::CW_USEDEFAULT,
                os::CW_USEDEFAULT,
                outer_w,
                outer_h,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                hinstance,
                core::ptr::null_mut(),
            )
        };
        if hwnd.is_null() {
            // SAFETY: the class was registered above and no window references it
            // (creation failed); unregister it once on this error path so the
            // process-global class table is not leaked.
            unsafe { os::UnregisterClassW(class_name.as_ptr(), hinstance) };
            return Err(WindowError::WindowCreationFailed);
        }

        // SAFETY: `hwnd` is the live window just created; `ShowWindow` +
        // `UpdateWindow` are idempotent display calls.
        unsafe {
            os::ShowWindow(hwnd, os::SW_SHOW);
            os::UpdateWindow(hwnd);
        }

        // I6: allocate the captured-input ring on the heap (stable address) and
        // install its pointer into the window's `GWLP_USERDATA` slot so the
        // stateless `window_proc` can reach it. `Box::into_raw` transfers
        // ownership to the raw pointer; `Window` reclaims it in `Drop`.
        let input_ring = Box::into_raw(Box::new(InputRing::with_capacity(INPUT_RING_CAP)));
        // SAFETY: `hwnd` is the live window just created; `SetWindowLongPtrW` with
        // `GWLP_USERDATA` writes the application's per-window `LONG_PTR` slot. The
        // stored value is the ring's heap address (reinterpreted as `isize`),
        // which stays valid until `Drop` clears the slot and frees the box. The
        // previous slot value is 0 (a fresh window) and is discarded.
        unsafe {
            os::SetWindowLongPtrW(hwnd, os::GWLP_USERDATA, input_ring as isize);
        }

        // I6b: register the system mouse for raw input routed to this window, so
        // `WM_INPUT` delivers un-accelerated relative deltas. A failure here is
        // non-fatal: the I6 `WM_MOUSEMOVE`-derived path still functions, so the
        // window opens regardless (the camera just falls back to accelerated
        // motion). `dwFlags = 0` means "receive while the window has focus".
        let rid = os::RAWINPUTDEVICE {
            usUsagePage: os::HID_USAGE_PAGE_GENERIC,
            usUsage: os::HID_USAGE_GENERIC_MOUSE,
            dwFlags: 0,
            hwndTarget: hwnd,
        };
        // SAFETY: `&rid` points to one fully-initialized `#[repr(C)]`
        // `RAWINPUTDEVICE` (the MS-maintained windows-sys layout); the count is 1
        // and the size is its own `size_of`, matching the Win64 ABI. The return is
        // ignored on purpose (see the non-fatal rationale above).
        unsafe {
            os::RegisterRawInputDevices(
                &rid,
                1,
                core::mem::size_of::<os::RAWINPUTDEVICE>() as u32,
            );
        }

        let mut window = Self {
            hwnd,
            hinstance,
            class_atom,
            class_name,
            width,
            height,
            input_ring,
        };
        window.refresh_size();
        Ok(window)
    }

    /// The window's `HWND` (opaque pointer) for `vkCreateWin32SurfaceKHR`.
    #[inline]
    pub fn hwnd(&self) -> *mut c_void {
        self.hwnd
    }

    /// The process `HINSTANCE` for `vkCreateWin32SurfaceKHR`.
    #[inline]
    pub fn hinstance(&self) -> *mut c_void {
        self.hinstance
    }

    /// The last-observed client-area width in pixels.
    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The last-observed client-area height in pixels.
    #[inline]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Re-queries the current client-area size from the OS, updating
    /// [`width`](Self::width) / [`height`](Self::height). Called after a resize so
    /// the swapchain can be recreated to the new extent.
    pub fn refresh_size(&mut self) {
        use crate::ffi::os;
        let mut rect = os::RECT::default();
        // SAFETY: `hwnd` is the live window; `&mut rect` is a valid out-pointer
        // for the `#[repr(C)]` `RECT` the OS fills with the client area.
        let ok = unsafe { os::GetClientRect(self.hwnd, &mut rect) };
        if ok != 0 {
            self.width = (rect.right - rect.left).max(0) as u32;
            self.height = (rect.bottom - rect.top).max(0) as u32;
        }
    }

    /// Drains all pending window messages.
    ///
    /// Returns `true` while the window is still open, and `false` once it has
    /// been asked to close (a `WM_QUIT` surfaced — produced by `PostQuitMessage`
    /// in `window_proc` on `WM_DESTROY`). Call once per frame before rendering.
    pub fn pump_events(&self) -> bool {
        use crate::ffi::os;
        let mut msg = os::MSG {
            hwnd: core::ptr::null_mut(),
            message: 0,
            w_param: 0,
            l_param: 0,
            time: 0,
            pt: os::POINT::default(),
            l_private: 0,
        };
        loop {
            // SAFETY: `&mut msg` is a valid out-pointer for the `#[repr(C)]` `MSG`
            // the OS fills; a null `hWnd` filter drains messages for every window
            // on this thread; `PM_REMOVE` pops the message. Returns non-zero while
            // a message was retrieved.
            let got = unsafe {
                os::PeekMessageW(&mut msg, core::ptr::null_mut(), 0, 0, os::PM_REMOVE)
            };
            if got == 0 {
                // Queue drained; the window is still alive.
                return true;
            }
            if msg.message == os::WM_QUIT {
                return false;
            }
            // SAFETY: `&msg` points to the just-retrieved message; translate +
            // dispatch route it to `window_proc` (read-only borrows of `msg`).
            unsafe {
                os::TranslateMessage(&msg);
                os::DispatchMessageW(&msg);
            }
        }
    }

    /// Drains every input message captured since the last call, invoking `sink`
    /// for each in FIFO order (I6/I6b).
    ///
    /// Call once per frame AFTER [`pump_events`](Self::pump_events) (the pump
    /// dispatches the OS messages that `window_proc` captures into the ring). The
    /// edge maps each [`CapturedMsg`] into a `boyko_input::RawInputEvent` via
    /// `boyko_input::win32::translate` / `translate_raw_mouse` and pushes it onto
    /// the `RawInputQueue` — that translation lives at the edge so this crate
    /// stays free of any `boyko_input` dependency.
    ///
    /// Returns the number of drop-oldest evictions the ring suffered since the
    /// last drain — diagnostic only. Eviction is the ring's load-shedding
    /// CONTRACT (the newest input survives; raw-mouse coalescing bounds the one
    /// high-rate source), never a fault: a burst of `> INPUT_RING_CAP` distinct
    /// non-mouse events within one frame sheds the stalest ones and the frame
    /// goes on.
    pub fn drain_input(&mut self, mut sink: impl FnMut(CapturedMsg)) -> usize {
        // SAFETY: `self.input_ring` is the box installed in `open` and not yet
        // freed (only `Drop` frees it). `&mut self` guarantees no other reference
        // to the ring is live on this thread, and `window_proc` runs only inside
        // a `DispatchMessageW` call (within `pump_events`), never concurrently
        // with `drain_input`. The pointer is non-null after a successful `open`.
        let ring = unsafe { &mut *self.input_ring };
        let dropped = ring.dropped;
        while let Some(ev) = ring.pop() {
            sink(ev);
        }
        ring.dropped = 0;
        dropped
    }
}

#[cfg(windows)]
impl Drop for Window {
    fn drop(&mut self) {
        use crate::ffi::os;
        // SAFETY: `hwnd` is the live window created in `open`, destroyed exactly
        // once here; then the class (registered against `hinstance` with the
        // owned `class_name`) is unregistered exactly once, in reverse order. The
        // Vulkan surface that borrowed this `hwnd` is destroyed by the caller
        // BEFORE the window is dropped (teardown order is the caller's contract).
        // `DestroyWindow` synchronously dispatches `WM_DESTROY` to `window_proc`,
        // which clears the `GWLP_USERDATA` slot, so no later message can
        // dereference the ring pointer once `DestroyWindow` returns.
        unsafe {
            os::DestroyWindow(self.hwnd);
            os::UnregisterClassW(self.class_name.as_ptr(), self.hinstance);
        }
        // SAFETY: `self.input_ring` was produced by `Box::into_raw` in `open` and
        // is owned solely by this `Window`; it has not been freed (only this
        // `Drop` frees it). After `DestroyWindow` above, the OS has finished
        // dispatching messages to `window_proc`, so no callback holds an aliasing
        // `&mut` to the ring. Reclaiming the box here drops it exactly once.
        unsafe {
            drop(Box::from_raw(self.input_ring));
        }
        let _ = self.class_atom;
    }
}

/// The window procedure. Handles `WM_CLOSE`/`WM_DESTROY` for the close contract,
/// captures keyboard / mouse / wheel / raw-input messages into the per-window
/// ring (I6/I6b), and forwards everything to `DefWindowProcW` for default
/// handling.
///
/// # Safety
///
/// This is an FFI callback the OS invokes with a valid `hwnd` and message
/// parameters. It reads the per-window ring pointer from `GWLP_USERDATA` (set in
/// [`Window::open`], cleared on `WM_DESTROY`), so a dereference happens only
/// while that pointer is a live, exclusively-owned [`InputRing`]; see the inline
/// SAFETY comments.
#[cfg(windows)]
unsafe extern "system" fn window_proc(
    hwnd: *mut c_void,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    use crate::ffi::os;
    match msg {
        os::WM_CLOSE => {
            // SAFETY: `hwnd` is the OS-supplied live window; destroying it on
            // close drives the `WM_DESTROY` path below.
            unsafe { os::DestroyWindow(hwnd) };
            0
        }
        os::WM_DESTROY => {
            // Clear the ring pointer BEFORE the window dies so no late-dispatched
            // message can dereference it (the box is freed by `Window::drop`,
            // which runs after `DestroyWindow` returns).
            // SAFETY: `hwnd` is the OS-supplied live window; writing 0 to its
            // `GWLP_USERDATA` slot is the documented way to invalidate the
            // application pointer. `PostQuitMessage` is a parameterless OS call.
            unsafe {
                os::SetWindowLongPtrW(hwnd, os::GWLP_USERDATA, 0);
                os::PostQuitMessage(0);
            }
            0
        }
        os::WM_KEYDOWN | os::WM_KEYUP | os::WM_SYSKEYDOWN | os::WM_SYSKEYUP
        | os::WM_MOUSEMOVE | os::WM_LBUTTONDOWN | os::WM_LBUTTONUP | os::WM_RBUTTONDOWN
        | os::WM_RBUTTONUP | os::WM_MBUTTONDOWN | os::WM_MBUTTONUP | os::WM_XBUTTONDOWN
        | os::WM_XBUTTONUP | os::WM_MOUSEWHEEL | os::WM_MOUSEHWHEEL => {
            capture(hwnd, CapturedMsg::Raw { msg, wparam, lparam });
            // SAFETY: also forward to the default proc so the OS performs its
            // default handling (focus, system-key beeps, etc.) with the
            // OS-supplied parameters.
            unsafe { os::DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        os::WM_INPUT => {
            // I6b: the `lParam` is a transient `HRAWINPUT` valid only inside this
            // message, so parse it now and capture the relative delta.
            // SAFETY: `lparam` is the OS-supplied `HRAWINPUT` for this `WM_INPUT`;
            // `read_raw_mouse_delta` reads it through `GetRawInputData` with a
            // correctly-sized stack buffer (see its own SAFETY comments).
            if let Some((dx, dy)) = unsafe { read_raw_mouse_delta(lparam as *mut c_void) } {
                capture(hwnd, CapturedMsg::RawMouse { dx, dy });
            }
            // SAFETY: `WM_INPUT` must still be passed to the default proc for raw
            // input cleanup, with the OS-supplied parameters.
            unsafe { os::DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        // SAFETY: forwarding unhandled messages to the default proc with the
        // OS-supplied parameters is the documented default-handling contract.
        _ => unsafe { os::DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Pushes one captured message into the window's ring, looked up via the
/// `GWLP_USERDATA` pointer. A no-op if the slot is null (the ring was never
/// installed or was already cleared on `WM_DESTROY`).
#[cfg(windows)]
fn capture(hwnd: *mut c_void, ev: CapturedMsg) {
    use crate::ffi::os;
    // SAFETY: `hwnd` is the OS-supplied live window; `GetWindowLongPtrW` reads its
    // application pointer slot. The value is either 0 (no ring, handled below) or
    // the exact `*mut InputRing` `open` installed, which stays valid until
    // `WM_DESTROY` zeroes the slot — strictly before `Window::drop` frees the box.
    let ptr = unsafe { os::GetWindowLongPtrW(hwnd, os::GWLP_USERDATA) } as *mut InputRing;
    if ptr.is_null() {
        return;
    }
    // SAFETY: `ptr` is non-null (checked) and is the live, heap-owned ring. The
    // `window_proc` runs only on the message-pump thread inside a
    // `DispatchMessageW` call; `Window::drain_input` (the only other mutator) runs
    // on the same thread and never overlaps a dispatch, so this `&mut` does not
    // alias. The borrow ends before this function returns.
    let ring = unsafe { &mut *ptr };
    ring.push(ev);
}

/// Reads the relative mouse delta from a `WM_INPUT` `HRAWINPUT` (I6b), or `None`
/// if the event is not a relative-mouse motion.
///
/// # Safety
///
/// `hrawinput` must be the `lParam` of a live `WM_INPUT` message (a valid
/// `HRAWINPUT` that `GetRawInputData` accepts). The OS writes at most
/// `size_of::<RAWINPUT>()` bytes into the stack buffer, which is exactly its
/// size, so there is no overflow (the ABI-guarded `RAWINPUT` size matches the
/// driver's mouse-case write).
#[cfg(windows)]
unsafe fn read_raw_mouse_delta(hrawinput: *mut c_void) -> Option<(i32, i32)> {
    use crate::ffi::os;
    let mut raw = core::mem::MaybeUninit::<os::RAWINPUT>::uninit();
    let mut size = core::mem::size_of::<os::RAWINPUT>() as u32;
    // SAFETY: `hrawinput` is a live `HRAWINPUT` (caller invariant). `RID_INPUT`
    // requests the data; `raw.as_mut_ptr()` is a valid out-buffer of `size` bytes
    // (`size_of::<RAWINPUT>()`); `&mut size` is the in/out size pointer; the last
    // arg is the header size. The call returns the bytes written, or `u32::MAX`
    // on error.
    let n = unsafe {
        os::GetRawInputData(
            hrawinput,
            os::RID_INPUT,
            raw.as_mut_ptr() as *mut c_void,
            &mut size,
            core::mem::size_of::<os::RAWINPUTHEADER>() as u32,
        )
    };
    if n == u32::MAX || n == 0 {
        return None;
    }
    // SAFETY: `GetRawInputData` returned a non-error byte count, so it fully
    // initialized the `RAWINPUTHEADER` + the device-specific arm. We read the
    // header (always present) and, for a mouse, its relative-motion fields.
    let raw = unsafe { raw.assume_init() };
    if raw.header.dwType != os::RIM_TYPEMOUSE {
        return None;
    }
    // SAFETY: `RAWINPUT::data` is a C union (windows-sys `RAWINPUT_0`); the mouse
    // arm is the active member because `dwType == RIM_TYPEMOUSE` was just checked.
    // `GetRawInputData` initialized that arm, so reading `lLastX`/`lLastY` is a
    // read of valid, initialized bytes of the correct union variant.
    let mouse = unsafe { raw.data.mouse };
    Some((mouse.lLastX, mouse.lLastY))
}

/// Encodes a `&str` as a NUL-terminated UTF-16 vector for the wide Win32 APIs.
#[cfg(windows)]
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

// --- Non-Windows stub so the crate compiles cross-target. ---

/// A windowing handle that is unavailable on non-Windows targets. Its
/// constructor returns [`WindowError::UnsupportedPlatform`]; the XCB/Wayland arm
/// lands when Linux on-screen is first targeted.
#[cfg(not(windows))]
pub struct Window {
    _private: (),
}

#[cfg(not(windows))]
impl Window {
    /// Always fails on non-Windows: windowing is Windows-first (D8).
    pub fn open(_title: &str, _width: u32, _height: u32) -> Result<Self, WindowError> {
        Err(WindowError::UnsupportedPlatform)
    }
}

#[cfg(all(test, windows))]
mod input_ring_tests {
    use super::{CapturedMsg, InputRing};

    /// A distinguishable non-mouse event (the exact msg values are irrelevant to
    /// the ring; it stores `CapturedMsg` opaquely).
    fn key(n: usize) -> CapturedMsg {
        CapturedMsg::Raw { msg: 0x100, wparam: n, lparam: 0 }
    }

    fn drain(ring: &mut InputRing) -> Vec<CapturedMsg> {
        let mut out = Vec::new();
        while let Some(ev) = ring.pop() {
            out.push(ev);
        }
        out
    }

    #[test]
    fn consecutive_raw_mouse_coalesces_into_one_entry() {
        let mut ring = InputRing::with_capacity(8);
        for _ in 0..10_000 {
            ring.push(CapturedMsg::RawMouse { dx: 1, dy: -2 });
        }
        assert_eq!(ring.len, 1, "a mouse-only burst occupies exactly one slot");
        assert_eq!(ring.dropped, 0, "coalescing means no eviction");
        let out = drain(&mut ring);
        assert_eq!(out, vec![CapturedMsg::RawMouse { dx: 10_000, dy: -20_000 }]);
    }

    #[test]
    fn key_event_splits_mouse_runs_preserving_order() {
        let mut ring = InputRing::with_capacity(8);
        ring.push(CapturedMsg::RawMouse { dx: 1, dy: 0 });
        ring.push(CapturedMsg::RawMouse { dx: 2, dy: 0 });
        ring.push(key(1));
        ring.push(CapturedMsg::RawMouse { dx: 4, dy: 0 });
        ring.push(CapturedMsg::RawMouse { dx: 8, dy: 0 });
        let out = drain(&mut ring);
        assert_eq!(
            out,
            vec![
                CapturedMsg::RawMouse { dx: 3, dy: 0 },
                key(1),
                CapturedMsg::RawMouse { dx: 12, dy: 0 },
            ],
            "runs merge; the key event splits them and keeps its position"
        );
    }

    #[test]
    fn overflow_drops_oldest_and_counts() {
        let mut ring = InputRing::with_capacity(4);
        for n in 0..6 {
            ring.push(key(n));
        }
        assert_eq!(ring.dropped, 2, "two evictions past the 4-slot capacity");
        let out = drain(&mut ring);
        assert_eq!(out, vec![key(2), key(3), key(4), key(5)], "newest survive");
    }

    #[test]
    fn coalesced_deltas_saturate_instead_of_wrapping() {
        let mut ring = InputRing::with_capacity(4);
        ring.push(CapturedMsg::RawMouse { dx: i32::MAX, dy: i32::MIN });
        ring.push(CapturedMsg::RawMouse { dx: i32::MAX, dy: i32::MIN });
        let out = drain(&mut ring);
        assert_eq!(out, vec![CapturedMsg::RawMouse { dx: i32::MAX, dy: i32::MIN }]);
    }
}
