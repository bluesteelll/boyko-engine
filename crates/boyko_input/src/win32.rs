//! Win32 message → [`RawInputEvent`] translation (plan §5.3, §10 / I6 + I6b).
//!
//! [`translate`] is a **pure** function: it takes the raw `(msg, wparam, lparam)`
//! triple a `WNDPROC` receives and returns the source-agnostic
//! [`RawInputEvent`] it encodes, or `None` for a message this layer does not map.
//! It performs **no FFI** and owns no window — it is plain bit-arithmetic over the
//! documented Win32 message ABI, so it lives in the leaf `boyko_input` crate and
//! stays exhaustively unit-testable with synthetic triples. The message-id / bit
//! VALUES it matches on (`WM_*`, `XBUTTON*`, `WHEEL_DELTA`, `KF_EXTENDED`,
//! `KF_REPEAT`) are sourced from the OFFICIAL MS-maintained `windows-sys` crate
//! (TARGET-GATED to `cfg(windows)`; a literal fallback keeps every other host
//! building), not hand-transcribed — but `translate` still makes no FFI call, so
//! the leaf pulls no linkage and the wasm / non-Windows builds are unaffected.
//!
//! # The crate seam (Decision 1 / §4)
//!
//! `boyko_input` must NOT depend on `boyko_rhi_vulkan` (the renderer/window
//! crate). The window (in `boyko_rhi_vulkan`) captures the raw `(msg, wparam,
//! lparam)` triples into a ring via `GWLP_USERDATA` and drains them through
//! `Window::drain_input`; the **edge** (the runner / demo) calls
//! [`translate`] and pushes the result into
//! [`RawInputQueue`](crate::raw::queue::RawInputQueue). The window never names a
//! `boyko_input` type, and this module never names a Win32 FFI type — the only
//! shared vocabulary is three integers.
//!
//! # Staging (W3)
//!
//! - **I6 (Stage 1):** keyboard, mouse buttons, wheel, and a `WM_MOUSEMOVE`-derived
//!   *absolute* cursor position ([`RawInputEvent::CursorMoved`]). Relative
//!   camera-look delta is derived by the application from successive cursor
//!   positions until I6b lands.
//! - **I6b (Stage 2):** the window registers the mouse for raw input
//!   (`RegisterRawInputDevices`) and forwards the parsed `WM_INPUT` delta as a
//!   `(dx, dy)` pair; [`translate_raw_mouse`] turns that into
//!   [`RawInputEvent::MouseMotion`] (un-accelerated relative delta — the correct
//!   camera source). Parsing the `RAWINPUT` blob is FFI and lives in the window
//!   crate; the `i32 → f64` mapping is pure and lives here so it is unit-testable.

use crate::raw::event::RawInputEvent;
use crate::raw::keycode::{ButtonState, MouseButton, ScrollDelta};
use crate::raw::scancode::keycode_from_scancode;

// ---------------------------------------------------------------------------
// Win32 message + bit constants, sourced from the OFFICIAL MS-maintained
// `windows-sys` bindings (explicitly approved for the OS layer).
//
// `translate` is PURE (no FFI) — these are just `u32`/`u16`/`i16` constant VALUES,
// so depending on `windows-sys` for them adds no linkage and no runtime cost; it
// only sources the documented `winuser.h` ids/bits from MS instead of
// hand-transcribing them (the prior local literals + their value-guard
// const-asserts are deleted — the official crate is the source of truth).
//
// `windows-sys` is TARGET-GATED to `cfg(windows)` (see `Cargo.toml`). On every
// non-Windows host the identical literal VALUES are provided by the
// `#[cfg(not(windows))]` fallback below so this leaf — and its cross-host unit /
// integration tests — still compile and behave byte-identically without pulling
// any platform dependency. The public TYPES (`WM_*: u32`, `XBUTTON*: u16`,
// `WHEEL_DELTA: i16`) are preserved across both arms so callers are unaffected.
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod sys {
    pub use windows_sys::Win32::UI::WindowsAndMessaging::{
        KF_EXTENDED, KF_REPEAT, WHEEL_DELTA, WM_INPUT, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN,
        WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL,
        WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
        XBUTTON1, XBUTTON2,
    };
}

// Non-Windows portability shim: the identical documented `winuser.h` VALUES, so
// the pure `translate` (and its cross-host tests) build on every target with NO
// platform dependency. Names/types match the `windows-sys` re-exports above 1:1.
#[cfg(not(windows))]
mod sys {
    pub const WM_KEYDOWN: u32 = 0x0100;
    pub const WM_KEYUP: u32 = 0x0101;
    pub const WM_SYSKEYDOWN: u32 = 0x0104;
    pub const WM_SYSKEYUP: u32 = 0x0105;
    pub const WM_MOUSEMOVE: u32 = 0x0200;
    pub const WM_LBUTTONDOWN: u32 = 0x0201;
    pub const WM_LBUTTONUP: u32 = 0x0202;
    pub const WM_RBUTTONDOWN: u32 = 0x0204;
    pub const WM_RBUTTONUP: u32 = 0x0205;
    pub const WM_MBUTTONDOWN: u32 = 0x0207;
    pub const WM_MBUTTONUP: u32 = 0x0208;
    pub const WM_XBUTTONDOWN: u32 = 0x020B;
    pub const WM_XBUTTONUP: u32 = 0x020C;
    pub const WM_MOUSEWHEEL: u32 = 0x020A;
    pub const WM_MOUSEHWHEEL: u32 = 0x020E;
    pub const WM_INPUT: u32 = 0x00FF;
    pub const XBUTTON1: u16 = 0x0001;
    pub const XBUTTON2: u16 = 0x0002;
    pub const WHEEL_DELTA: u32 = 120;
    // `KF_EXTENDED` / `KF_REPEAT` are HIWORD-relative flags (`winuser.h`): the
    // full-`lParam` bit is the flag shifted left 16 (bit 24 / bit 30).
    pub const KF_EXTENDED: u32 = 0x0100;
    pub const KF_REPEAT: u32 = 0x4000;
}

/// `WM_KEYDOWN` — a non-system key was pressed.
pub const WM_KEYDOWN: u32 = sys::WM_KEYDOWN;
/// `WM_KEYUP` — a non-system key was released.
pub const WM_KEYUP: u32 = sys::WM_KEYUP;
/// `WM_SYSKEYDOWN` — a system key (Alt held, or F10) was pressed.
pub const WM_SYSKEYDOWN: u32 = sys::WM_SYSKEYDOWN;
/// `WM_SYSKEYUP` — a system key was released.
pub const WM_SYSKEYUP: u32 = sys::WM_SYSKEYUP;

/// `WM_MOUSEMOVE` — the cursor moved over the client area.
pub const WM_MOUSEMOVE: u32 = sys::WM_MOUSEMOVE;
/// `WM_LBUTTONDOWN`.
pub const WM_LBUTTONDOWN: u32 = sys::WM_LBUTTONDOWN;
/// `WM_LBUTTONUP`.
pub const WM_LBUTTONUP: u32 = sys::WM_LBUTTONUP;
/// `WM_RBUTTONDOWN`.
pub const WM_RBUTTONDOWN: u32 = sys::WM_RBUTTONDOWN;
/// `WM_RBUTTONUP`.
pub const WM_RBUTTONUP: u32 = sys::WM_RBUTTONUP;
/// `WM_MBUTTONDOWN`.
pub const WM_MBUTTONDOWN: u32 = sys::WM_MBUTTONDOWN;
/// `WM_MBUTTONUP`.
pub const WM_MBUTTONUP: u32 = sys::WM_MBUTTONUP;
/// `WM_XBUTTONDOWN` — an extra (Back/Forward) mouse button was pressed.
pub const WM_XBUTTONDOWN: u32 = sys::WM_XBUTTONDOWN;
/// `WM_XBUTTONUP`.
pub const WM_XBUTTONUP: u32 = sys::WM_XBUTTONUP;
/// `WM_MOUSEWHEEL` — the vertical scroll wheel turned.
pub const WM_MOUSEWHEEL: u32 = sys::WM_MOUSEWHEEL;
/// `WM_MOUSEHWHEEL` — the horizontal scroll wheel turned.
pub const WM_MOUSEHWHEEL: u32 = sys::WM_MOUSEHWHEEL;
/// `WM_INPUT` — a Raw Input device reported data (I6b; parsed by the window).
pub const WM_INPUT: u32 = sys::WM_INPUT;

/// The high `WORD` of `wParam` for `WM_XBUTTON*` selecting button 1 (Back).
pub const XBUTTON1: u16 = sys::XBUTTON1;
/// The high `WORD` of `wParam` for `WM_XBUTTON*` selecting button 2 (Forward).
pub const XBUTTON2: u16 = sys::XBUTTON2;

/// One scroll notch as reported in the high `WORD` of `WM_MOUSE*WHEEL`'s
/// `wParam`. The wheel delta is a signed multiple of this (`winuser.h`). Kept as
/// `i16` (the public type callers/tests rely on) since the notch count is read as
/// a signed `i16`; `windows-sys` types the constant as `u32`, narrowed here.
pub const WHEEL_DELTA: i16 = sys::WHEEL_DELTA as i16;

// `lParam` bit layout for `WM_KEYDOWN`/`WM_KEYUP` (the "key flags", `winuser.h`).
//
// The scancode occupies bits 16..=23; there is no single named `winuser.h`
// constant for that mask/shift, so they remain derived bit-arithmetic helpers.
// The `KF_EXTENDED` / `KF_REPEAT` flags ARE named — `windows-sys` exposes them as
// their HIWORD-relative values (bit 8 / bit 14), so the full-`lParam` bit is the
// flag shifted left by `KEY_LPARAM_FLAGS_HIWORD_SHIFT` (bits 24 / 30).
/// Mask selecting the OEM scancode in `lParam` bits 16..=23.
const KEY_LPARAM_SCANCODE_MASK: isize = 0x00FF_0000;
/// Right-shift to move the scancode byte down to bits 0..=7.
const KEY_LPARAM_SCANCODE_SHIFT: u32 = 16;
/// Left-shift mapping a HIWORD-relative key flag to its full-`lParam` bit.
const KEY_LPARAM_FLAGS_HIWORD_SHIFT: u32 = 16;
/// `KF_EXTENDED` in `lParam` bit 24 — the key is an `E0`-prefixed extended key.
const KEY_LPARAM_EXTENDED_BIT: isize = (sys::KF_EXTENDED as isize) << KEY_LPARAM_FLAGS_HIWORD_SHIFT;
/// `KF_REPEAT` in `lParam` bit 30 — the previous key state was down (an OS
/// auto-repeat for a `WM_KEYDOWN`; always set for `WM_KEYUP`, where it is not an
/// edge concept and is reported only for `Pressed`).
const KEY_LPARAM_REPEAT_BIT: isize = (sys::KF_REPEAT as isize) << KEY_LPARAM_FLAGS_HIWORD_SHIFT;

/// Translates one Win32 window message into a [`RawInputEvent`].
///
/// Returns `None` for any message this layer does not map (the caller forwards
/// those to `DefWindowProcW` unchanged). This is a pure function over the raw
/// `(msg, wparam, lparam)` triple — no FFI, no allocation, no global state.
///
/// # Coverage (I6)
/// - `WM_KEYDOWN`/`WM_SYSKEYDOWN` → [`RawInputEvent::Key`] (`Pressed`), with the
///   scancode extracted from `lParam` bits 16..=23, the extended-key table
///   selected by bit 24, and `repeat` read from bit 30.
/// - `WM_KEYUP`/`WM_SYSKEYUP` → [`RawInputEvent::Key`] (`Released`).
/// - `WM_*BUTTONDOWN`/`WM_*BUTTONUP` (left/right/middle/X) →
///   [`RawInputEvent::MouseButton`]; the X-button is selected by the high `WORD`
///   of `wParam`.
/// - `WM_MOUSEMOVE` → [`RawInputEvent::CursorMoved`] (absolute client-area
///   position from the `lParam` `WORD`s — the I6 delta source until I6b).
/// - `WM_MOUSEWHEEL`/`WM_MOUSEHWHEEL` → [`RawInputEvent::Wheel`]
///   ([`ScrollDelta::Lines`], the high `WORD` of `wParam` as a signed multiple of
///   [`WHEEL_DELTA`]).
///
/// `WM_INPUT` is intentionally NOT handled here: its payload is an opaque blob
/// the window must read via `GetRawInputData` (FFI), then forward the parsed
/// `(dx, dy)` to [`translate_raw_mouse`].
#[inline]
pub fn translate(msg: u32, wparam: usize, lparam: isize) -> Option<RawInputEvent> {
    match msg {
        WM_KEYDOWN | WM_SYSKEYDOWN => Some(key_event(lparam, ButtonState::Pressed)),
        WM_KEYUP | WM_SYSKEYUP => Some(key_event(lparam, ButtonState::Released)),

        WM_LBUTTONDOWN => Some(mouse_button(MouseButton::Left, ButtonState::Pressed)),
        WM_LBUTTONUP => Some(mouse_button(MouseButton::Left, ButtonState::Released)),
        WM_RBUTTONDOWN => Some(mouse_button(MouseButton::Right, ButtonState::Pressed)),
        WM_RBUTTONUP => Some(mouse_button(MouseButton::Right, ButtonState::Released)),
        WM_MBUTTONDOWN => Some(mouse_button(MouseButton::Middle, ButtonState::Pressed)),
        WM_MBUTTONUP => Some(mouse_button(MouseButton::Middle, ButtonState::Released)),
        WM_XBUTTONDOWN => Some(mouse_button(xbutton(wparam), ButtonState::Pressed)),
        WM_XBUTTONUP => Some(mouse_button(xbutton(wparam), ButtonState::Released)),

        WM_MOUSEMOVE => Some(cursor_moved(lparam)),

        WM_MOUSEWHEEL => Some(wheel(wparam, false)),
        WM_MOUSEHWHEEL => Some(wheel(wparam, true)),

        _ => None,
    }
}

/// Translates a parsed raw-mouse delta (from a `WM_INPUT` `RAWMOUSE`, I6b) into a
/// [`RawInputEvent::MouseMotion`].
///
/// The window crate owns the FFI that reads the `RAWINPUT` blob and extracts the
/// signed `lLastX`/`lLastY` relative-motion fields; this pure mapping
/// (`i32 → f64`) lives here so the camera-delta path is unit-testable without a
/// window. Relative mouse motion is un-accelerated — the correct camera-look
/// source (it supersedes the I6 `WM_MOUSEMOVE`-derived delta).
#[inline]
pub fn translate_raw_mouse(dx: i32, dy: i32) -> RawInputEvent {
    RawInputEvent::MouseMotion {
        dx: dx as f64,
        dy: dy as f64,
    }
}

/// Builds a [`RawInputEvent::Key`] from a key message's `lParam` and the
/// press/release `state`. The scancode (bits 16..=23) selects a canonical
/// [`KeyCode`](crate::raw::keycode::KeyCode) via the extended-aware table; the
/// `repeat` flag is `lParam` bit 30.
#[inline]
fn key_event(lparam: isize, state: ButtonState) -> RawInputEvent {
    let scancode = ((lparam & KEY_LPARAM_SCANCODE_MASK) >> KEY_LPARAM_SCANCODE_SHIFT) as u8;
    let extended = (lparam & KEY_LPARAM_EXTENDED_BIT) != 0;
    // `KF_EXTENDED`/`KF_REPEAT` live in `lParam`. `KF_REPEAT` (bit 30) means "the previous key
    // state was down" — for `WM_KEYDOWN` that is an OS AUTO-REPEAT, but for `WM_KEYUP` it is
    // ALWAYS set (you are releasing a held key), where it is NOT a repeat. Reading it
    // unconditionally marked every release `repeat = true`; a consumer that drops repeats
    // (`PhysicalInput::apply`'s `if repeat { return; }`) then NEVER saw the release and the key
    // STUCK (held forever). `repeat` is a press-only concept — gate it to `Pressed`.
    let repeat = matches!(state, ButtonState::Pressed) && (lparam & KEY_LPARAM_REPEAT_BIT) != 0;
    RawInputEvent::Key {
        code: keycode_from_scancode(scancode, extended),
        state,
        repeat,
    }
}

/// Builds a [`RawInputEvent::MouseButton`].
#[inline]
fn mouse_button(button: MouseButton, state: ButtonState) -> RawInputEvent {
    RawInputEvent::MouseButton { button, state }
}

/// Selects the [`MouseButton`] for a `WM_XBUTTON*` message from the high `WORD`
/// of `wParam`. `XBUTTON1` is the canonical Back button, `XBUTTON2` the Forward
/// button; any other value (no real device reports one) is carried losslessly by
/// [`MouseButton::Other`] so no button is dropped.
#[inline]
fn xbutton(wparam: usize) -> MouseButton {
    match hiword(wparam) {
        XBUTTON1 => MouseButton::Back,
        XBUTTON2 => MouseButton::Forward,
        other => MouseButton::Other(other),
    }
}

/// Builds a [`RawInputEvent::CursorMoved`] from a mouse message's `lParam`
/// (signed 16-bit client-area `x` in the low `WORD`, `y` in the high `WORD`).
#[inline]
fn cursor_moved(lparam: isize) -> RawInputEvent {
    let x = (lparam & 0xFFFF) as u16 as i16 as f64;
    let y = ((lparam >> 16) & 0xFFFF) as u16 as i16 as f64;
    RawInputEvent::CursorMoved { x, y }
}

/// Builds a [`RawInputEvent::Wheel`] from a wheel message's `wParam`. The signed
/// notch count is the high `WORD` interpreted as `i16`, divided by
/// [`WHEEL_DELTA`] to lines; `horizontal` routes it to the `x` (else `y`) axis.
#[inline]
fn wheel(wparam: usize, horizontal: bool) -> RawInputEvent {
    let raw = hiword(wparam) as i16;
    let lines = raw as f32 / WHEEL_DELTA as f32;
    let delta = if horizontal {
        ScrollDelta::Lines { x: lines, y: 0.0 }
    } else {
        ScrollDelta::Lines { x: 0.0, y: lines }
    };
    RawInputEvent::Wheel(delta)
}

/// The high `WORD` (bits 16..=31) of a `wParam`, as the Win32 `HIWORD` macro
/// does. Used for the X-button selector and the wheel notch count.
#[inline]
fn hiword(wparam: usize) -> u16 {
    ((wparam >> 16) & 0xFFFF) as u16
}

// Guards on the DERIVED bit-arithmetic helpers (the message-id / button / wheel
// VALUES themselves now come from `windows-sys`, the source of truth, so they need
// no value-guard). These pin the `lParam` key-flags layout `translate` relies on:
// scancode in bits 16..=23, `KF_EXTENDED` at bit 24, `KF_REPEAT` at bit 30
// (`winuser.h`). A wrong shift would silently mis-decode a key, so the resulting
// full-`lParam` bit positions are asserted against their documented values.
const _: () = assert!(KEY_LPARAM_SCANCODE_MASK == 0x00FF_0000);
const _: () = assert!(KEY_LPARAM_SCANCODE_SHIFT == 16);
const _: () = assert!(KEY_LPARAM_EXTENDED_BIT == 0x0100_0000);
const _: () = assert!(KEY_LPARAM_REPEAT_BIT == 0x4000_0000);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::keycode::KeyCode;

    /// Builds a `WM_KEY*` `lParam` from the parts `translate` reads back: the
    /// OEM scancode (bits 16..=23), the extended flag (bit 24), the repeat flag
    /// (bit 30). The low `WORD` (repeat count) is irrelevant to translation.
    fn key_lparam(scancode: u8, extended: bool, repeat: bool) -> isize {
        let mut l = (scancode as isize) << 16;
        if extended {
            l |= 1 << 24;
        }
        if repeat {
            l |= 1 << 30;
        }
        l
    }

    /// Packs a value into the high `WORD` of a `wParam` (the X-button selector /
    /// the wheel notch count live there).
    fn hiword_wparam(hi: u16) -> usize {
        (hi as usize) << 16
    }

    #[test]
    fn keydown_maps_scancode_to_keycode_pressed_no_repeat() {
        // 0x11 = KeyW in the base Set-1 table.
        let ev = translate(WM_KEYDOWN, 0, key_lparam(0x11, false, false));
        assert_eq!(
            ev,
            Some(RawInputEvent::Key {
                code: KeyCode::KeyW,
                state: ButtonState::Pressed,
                repeat: false,
            })
        );
    }

    #[test]
    fn syskeydown_is_a_press_too() {
        // 0x38 = AltLeft (base). WM_SYSKEYDOWN fires while Alt is held.
        let ev = translate(WM_SYSKEYDOWN, 0, key_lparam(0x38, false, false));
        assert_eq!(
            ev,
            Some(RawInputEvent::Key {
                code: KeyCode::AltLeft,
                state: ButtonState::Pressed,
                repeat: false,
            })
        );
    }

    #[test]
    fn keyup_is_released() {
        let ev = translate(WM_KEYUP, 0, key_lparam(0x11, false, false));
        assert_eq!(
            ev,
            Some(RawInputEvent::Key {
                code: KeyCode::KeyW,
                state: ButtonState::Released,
                repeat: false,
            })
        );
    }

    #[test]
    fn syskeyup_is_released() {
        let ev = translate(WM_SYSKEYUP, 0, key_lparam(0x38, false, false));
        assert_eq!(
            ev,
            Some(RawInputEvent::Key {
                code: KeyCode::AltLeft,
                state: ButtonState::Released,
                repeat: false,
            })
        );
    }

    #[test]
    fn keyup_with_repeat_bit_is_not_a_repeat() {
        // REGRESSION: `WM_KEYUP` ALWAYS carries `KF_REPEAT` (bit 30 = "previous state was
        // down", always true for a release). It must NOT be reported as `repeat`, else a
        // consumer that drops repeats (`PhysicalInput::apply`) never observes the release and
        // the key sticks held forever (the interactive-viewer "camera flies, can't stop" bug).
        let ev = translate(WM_KEYUP, 0, key_lparam(0x11, false, true));
        assert_eq!(
            ev,
            Some(RawInputEvent::Key {
                code: KeyCode::KeyW,
                state: ButtonState::Released,
                repeat: false,
            })
        );
    }

    #[test]
    fn keydown_repeat_bit_sets_repeat_flag() {
        let ev = translate(WM_KEYDOWN, 0, key_lparam(0x11, false, true));
        assert_eq!(
            ev,
            Some(RawInputEvent::Key {
                code: KeyCode::KeyW,
                state: ButtonState::Pressed,
                repeat: true,
            })
        );
    }

    #[test]
    fn extended_bit_selects_the_extended_table() {
        // Base 0x1C = Enter; extended 0x1C = NumpadEnter. The extended bit must
        // route the SAME low byte to the extended table.
        let base = translate(WM_KEYDOWN, 0, key_lparam(0x1C, false, false));
        let ext = translate(WM_KEYDOWN, 0, key_lparam(0x1C, true, false));
        assert_eq!(
            base,
            Some(RawInputEvent::Key {
                code: KeyCode::Enter,
                state: ButtonState::Pressed,
                repeat: false,
            })
        );
        assert_eq!(
            ext,
            Some(RawInputEvent::Key {
                code: KeyCode::NumpadEnter,
                state: ButtonState::Pressed,
                repeat: false,
            })
        );
    }

    #[test]
    fn extended_arrow_key_maps_through_extended_table() {
        // Extended 0x48 = ArrowUp (base 0x48 = Numpad8).
        let up = translate(WM_KEYDOWN, 0, key_lparam(0x48, true, false));
        assert_eq!(
            up,
            Some(RawInputEvent::Key {
                code: KeyCode::ArrowUp,
                state: ButtonState::Pressed,
                repeat: false,
            })
        );
    }

    #[test]
    fn unmapped_scancode_round_trips_as_unidentified() {
        // 0x56 has no override in the base table → Unidentified(0x56).
        let ev = translate(WM_KEYDOWN, 0, key_lparam(0x56, false, false));
        assert_eq!(
            ev,
            Some(RawInputEvent::Key {
                code: KeyCode::Unidentified(0x56),
                state: ButtonState::Pressed,
                repeat: false,
            })
        );
    }

    #[test]
    fn left_right_middle_buttons_map_with_state() {
        assert_eq!(
            translate(WM_LBUTTONDOWN, 0, 0),
            Some(RawInputEvent::MouseButton {
                button: MouseButton::Left,
                state: ButtonState::Pressed,
            })
        );
        assert_eq!(
            translate(WM_LBUTTONUP, 0, 0),
            Some(RawInputEvent::MouseButton {
                button: MouseButton::Left,
                state: ButtonState::Released,
            })
        );
        assert_eq!(
            translate(WM_RBUTTONDOWN, 0, 0),
            Some(RawInputEvent::MouseButton {
                button: MouseButton::Right,
                state: ButtonState::Pressed,
            })
        );
        assert_eq!(
            translate(WM_MBUTTONDOWN, 0, 0),
            Some(RawInputEvent::MouseButton {
                button: MouseButton::Middle,
                state: ButtonState::Pressed,
            })
        );
    }

    #[test]
    fn xbutton1_is_back_xbutton2_is_forward() {
        assert_eq!(
            translate(WM_XBUTTONDOWN, hiword_wparam(XBUTTON1), 0),
            Some(RawInputEvent::MouseButton {
                button: MouseButton::Back,
                state: ButtonState::Pressed,
            })
        );
        assert_eq!(
            translate(WM_XBUTTONUP, hiword_wparam(XBUTTON2), 0),
            Some(RawInputEvent::MouseButton {
                button: MouseButton::Forward,
                state: ButtonState::Released,
            })
        );
    }

    #[test]
    fn unknown_xbutton_is_carried_as_other() {
        let ev = translate(WM_XBUTTONDOWN, hiword_wparam(7), 0);
        assert_eq!(
            ev,
            Some(RawInputEvent::MouseButton {
                button: MouseButton::Other(7),
                state: ButtonState::Pressed,
            })
        );
    }

    #[test]
    fn mousemove_decodes_signed_absolute_position() {
        // x = 100 (low WORD), y = 200 (high WORD).
        let lparam = (200isize << 16) | 100;
        assert_eq!(
            translate(WM_MOUSEMOVE, 0, lparam),
            Some(RawInputEvent::CursorMoved { x: 100.0, y: 200.0 })
        );
    }

    #[test]
    fn mousemove_negative_coords_are_signed() {
        // The client area can report negative coords (cursor dragged past the
        // top-left). x = -1 (0xFFFF), y = -2 (0xFFFE).
        let lparam = ((0xFFFEisize) << 16) | 0xFFFF;
        assert_eq!(
            translate(WM_MOUSEMOVE, 0, lparam),
            Some(RawInputEvent::CursorMoved { x: -1.0, y: -2.0 })
        );
    }

    #[test]
    fn wheel_up_is_positive_y_lines() {
        // One notch up: high WORD = +WHEEL_DELTA → +1.0 line on y.
        let ev = translate(WM_MOUSEWHEEL, hiword_wparam(WHEEL_DELTA as u16), 0);
        assert_eq!(
            ev,
            Some(RawInputEvent::Wheel(ScrollDelta::Lines { x: 0.0, y: 1.0 }))
        );
    }

    #[test]
    fn wheel_down_is_negative_y_lines() {
        // One notch down: high WORD = -WHEEL_DELTA (as u16) → -1.0 line on y.
        let ev = translate(WM_MOUSEWHEEL, hiword_wparam((-WHEEL_DELTA) as u16), 0);
        assert_eq!(
            ev,
            Some(RawInputEvent::Wheel(ScrollDelta::Lines { x: 0.0, y: -1.0 }))
        );
    }

    #[test]
    fn hwheel_routes_to_x_axis() {
        let ev = translate(WM_MOUSEHWHEEL, hiword_wparam(WHEEL_DELTA as u16), 0);
        assert_eq!(
            ev,
            Some(RawInputEvent::Wheel(ScrollDelta::Lines { x: 1.0, y: 0.0 }))
        );
    }

    #[test]
    fn unhandled_messages_return_none() {
        // WM_INPUT is not handled by `translate` (the window parses its blob).
        assert_eq!(translate(WM_INPUT, 0, 0), None);
        // WM_PAINT (0x000F), WM_SIZE (0x0005), WM_QUIT (0x0012) — never mapped.
        assert_eq!(translate(0x000F, 0, 0), None);
        assert_eq!(translate(0x0005, 0, 0), None);
        assert_eq!(translate(0x0012, 0, 0), None);
    }

    #[test]
    fn raw_mouse_delta_is_signed_f64() {
        assert_eq!(
            translate_raw_mouse(3, -5),
            RawInputEvent::MouseMotion { dx: 3.0, dy: -5.0 }
        );
        assert_eq!(
            translate_raw_mouse(0, 0),
            RawInputEvent::MouseMotion { dx: 0.0, dy: 0.0 }
        );
    }
}
