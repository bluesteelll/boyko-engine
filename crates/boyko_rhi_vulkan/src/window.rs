//! Slice-1 — a raw Win32 window (no winit / no windows-sys), the on-screen seam.
//!
//! Per `docs/RENDER-PHYSICS-GPU-PLAN.md` D8 ("окно наше" — our window), the
//! foundation's first on-screen path uses a hand-rolled Win32 window via raw FFI
//! to `user32` / `kernel32` (mirroring the `boyko_ecs::ecs::memory::vm.rs`
//! `LoadLibrary`/`extern "system"` discipline). The Vulkan surface
//! ([`crate::swapchain::Surface`]) is created from this window's `HWND` +
//! `HINSTANCE`.
//!
//! # The window-procedure / close contract
//!
//! A `WNDPROC` is a plain `extern "system" fn` and cannot capture state. Rather
//! than juggle a raw pointer through `GWLP_USERDATA`, the close signal flows
//! through the message queue itself: the OS routes `WM_CLOSE` → `DefWindowProcW`
//! → `WM_DESTROY`, the [`window_proc`] handles `WM_DESTROY` by calling
//! `PostQuitMessage(0)`, and [`Window::pump_events`] reports "should close" the
//! moment `PeekMessageW` yields a `WM_QUIT`. This needs no shared mutable state
//! in the callback (the spec-mandated source of `WM_QUIT`).
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
}

#[cfg(windows)]
impl Window {
    /// Opens a window of `width` × `height` client pixels with the given title.
    ///
    /// Registers a window class with [`window_proc`], creates an overlapped
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

        let mut window = Self {
            hwnd,
            hinstance,
            class_atom,
            class_name,
            width,
            height,
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
    /// in [`window_proc`] on `WM_DESTROY`). Call once per frame before rendering.
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
        unsafe {
            os::DestroyWindow(self.hwnd);
            os::UnregisterClassW(self.class_name.as_ptr(), self.hinstance);
        }
        let _ = self.class_atom;
    }
}

/// The window procedure. Handles `WM_DESTROY` by posting a quit message (so the
/// pump's `WM_QUIT` check reports close), `WM_CLOSE` by destroying the window,
/// and forwards everything else to `DefWindowProcW`.
///
/// # Safety
///
/// This is an FFI callback the OS invokes with a valid `hwnd` and message
/// parameters; it dereferences nothing and only calls back into `user32`.
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
            // SAFETY: a parameterless OS call that enqueues `WM_QUIT`.
            unsafe { os::PostQuitMessage(0) };
            0
        }
        // SAFETY: forwarding unhandled messages to the default proc with the
        // OS-supplied parameters is the documented default-handling contract.
        _ => unsafe { os::DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
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
