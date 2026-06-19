//! I6 / I6b integration gates for the pure Win32 translation layer
//! (`boyko_input::win32`).
//!
//! These exercise the crate-public seam (`translate` / `translate_raw_mouse`)
//! the way the edge glue does — turning synthetic Win32 message triples into
//! `RawInputEvent`s — and the rebind state machine (I6) against those translated
//! events. They are pure (no window, no FFI), so they run on every host.
//!
//! The renderer-side FFI (`Window::drain_input` + the `RAWINPUT` parse) is NOT
//! covered here: it requires a live Win32 window and is Miri-N/A FFI. The
//! native physical-keypress → action path is a MANUAL integration check — there
//! is no portable way to inject a real OS keypress — and is documented as such
//! at the bottom of this file.

use boyko_input::win32::{
    self, WHEEL_DELTA, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_MOUSEWHEEL,
};
use boyko_input::{
    BindSpec, ButtonState, InputMap, KeyCode, MouseButton, RawInputEvent, RebindOutcome,
    RebindSession, ScrollDelta,
};

/// Builds a `WM_KEY*` `lParam`: OEM scancode in bits 16..=23, KF_EXTENDED in bit
/// 24, KF_REPEAT in bit 30.
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

/// A two-action test enum so the rebind conflict scan has something to collide
/// with.
#[derive(boyko_input::Actionlike, Clone, Copy, PartialEq, Eq, Debug)]
enum TestAction {
    Jump,
    Fire,
}

// ---------------------------------------------------------------------------
// I6: translate → rebind wiring (binds / conflicts / cancels on live events).
// ---------------------------------------------------------------------------

#[test]
fn rebind_binds_a_translated_keydown() {
    let mut map = InputMap::<TestAction>::builder()
        .bind(TestAction::Jump, BindSpec::Key(KeyCode::Space))
        .build();

    let mut session = RebindSession::begin(TestAction::Fire, 0);

    // A KeyDown for KeyW (scancode 0x11), translated from a synthetic Win32 msg.
    let ev = win32::translate(WM_KEYDOWN, 0, key_lparam(0x11, false, false))
        .expect("WM_KEYDOWN must translate");
    assert_eq!(
        ev,
        RawInputEvent::Key {
            code: KeyCode::KeyW,
            state: ButtonState::Pressed,
            repeat: false,
        }
    );

    let outcome = session.feed(&ev, &mut map);
    assert_eq!(outcome, Some(RebindOutcome::Bound));
    assert!(session.is_done());
}

#[test]
fn rebind_detects_a_conflict_against_an_existing_translated_binding() {
    // Jump is bound to KeyW; rebinding Fire to the SAME translated key conflicts.
    let mut map = InputMap::<TestAction>::builder()
        .bind(TestAction::Jump, BindSpec::Key(KeyCode::KeyW))
        .build();

    let mut session = RebindSession::begin(TestAction::Fire, 0);
    let ev = win32::translate(WM_KEYDOWN, 0, key_lparam(0x11, false, false)).unwrap();

    let outcome = session.feed(&ev, &mut map);
    assert_eq!(outcome, Some(RebindOutcome::Conflict { existing: "Jump" }));
}

#[test]
fn rebind_ignores_keyups_and_repeats_then_cancels() {
    let mut map = InputMap::<TestAction>::builder()
        .bind(TestAction::Jump, BindSpec::Key(KeyCode::Space))
        .build();

    let mut session = RebindSession::begin(TestAction::Fire, 0);

    // A KeyUp is not a deliberate bind input → keep listening.
    let up = win32::translate(WM_KEYUP, 0, key_lparam(0x11, false, false)).unwrap();
    assert_eq!(session.feed(&up, &mut map), None);

    // An OS auto-repeat KeyDown (bit 30 set) is also ignored.
    let repeat = win32::translate(WM_KEYDOWN, 0, key_lparam(0x11, false, true)).unwrap();
    assert_eq!(session.feed(&repeat, &mut map), None);

    // The user cancels.
    assert_eq!(session.cancel(), RebindOutcome::Cancelled);
    assert!(session.is_done());
}

#[test]
fn rebind_binds_a_translated_mouse_button() {
    let mut map = InputMap::<TestAction>::builder()
        .bind(TestAction::Jump, BindSpec::Key(KeyCode::Space))
        .build();

    let mut session = RebindSession::begin(TestAction::Fire, 0);
    let ev = win32::translate(WM_LBUTTONDOWN, 0, 0).unwrap();
    assert_eq!(
        ev,
        RawInputEvent::MouseButton {
            button: MouseButton::Left,
            state: ButtonState::Pressed,
        }
    );
    assert_eq!(session.feed(&ev, &mut map), Some(RebindOutcome::Bound));
}

// ---------------------------------------------------------------------------
// I6b: relative-delta correctness vs the cursor-derived baseline.
// ---------------------------------------------------------------------------

/// The raw-input (`translate_raw_mouse`) delta for a motion must equal the delta
/// a consumer would compute from two successive `WM_MOUSEMOVE` absolute cursor
/// positions (the I6 baseline). This proves the I6b path is a drop-in replacement
/// for the I6 cursor-derived delta — only un-accelerated and lower-latency.
#[test]
fn raw_mouse_delta_matches_cursor_derived_baseline() {
    // The cursor moved from (100, 200) to (130, 175): a (+30, -25) delta.
    let p0 = win32::translate(WM_MOUSEMOVE, 0, (200isize << 16) | 100).unwrap();
    let p1 = win32::translate(WM_MOUSEMOVE, 0, ((175isize) << 16) | 130).unwrap();

    let (RawInputEvent::CursorMoved { x: x0, y: y0 }, RawInputEvent::CursorMoved { x: x1, y: y1 }) =
        (p0, p1)
    else {
        panic!("WM_MOUSEMOVE must translate to CursorMoved");
    };
    let baseline_dx = x1 - x0;
    let baseline_dy = y1 - y0;
    assert_eq!((baseline_dx, baseline_dy), (30.0, -25.0));

    // The I6b raw path reports the same delta directly.
    let raw = win32::translate_raw_mouse(30, -25);
    assert_eq!(
        raw,
        RawInputEvent::MouseMotion {
            dx: baseline_dx,
            dy: baseline_dy,
        }
    );
}

#[test]
fn raw_mouse_delta_is_signed_and_lossless() {
    assert_eq!(
        win32::translate_raw_mouse(-7, 13),
        RawInputEvent::MouseMotion { dx: -7.0, dy: 13.0 }
    );
    assert_eq!(
        win32::translate_raw_mouse(i32::MIN, i32::MAX),
        RawInputEvent::MouseMotion {
            dx: i32::MIN as f64,
            dy: i32::MAX as f64,
        }
    );
}

#[test]
fn wheel_sign_is_preserved_through_translate() {
    // One notch up: +WHEEL_DELTA in the high WORD of wParam → +1.0 line on y.
    let wparam = (WHEEL_DELTA as u16 as usize) << 16;
    let up = win32::translate(WM_MOUSEWHEEL, wparam, 0).unwrap();
    assert_eq!(up, RawInputEvent::Wheel(ScrollDelta::Lines { x: 0.0, y: 1.0 }));

    // One notch down: -WHEEL_DELTA (as u16) in the high WORD → -1.0 line on y.
    let wparam_down = ((-WHEEL_DELTA) as u16 as usize) << 16;
    let down = win32::translate(WM_MOUSEWHEEL, wparam_down, 0).unwrap();
    assert_eq!(
        down,
        RawInputEvent::Wheel(ScrollDelta::Lines { x: 0.0, y: -1.0 })
    );
}

// ---------------------------------------------------------------------------
// MANUAL TEST NOTE (I6 gate — physical keypress → action).
//
// There is NO portable, automated way to inject a real OS physical keypress into
// the live Win32 window's `window_proc` (synthesizing `SendInput` would test the
// OS, not our path, and is not available cross-host / under CI). The end-to-end
// "press W on the keyboard → `ActionState::pressed(Move-forward)` is true" path
// is therefore a MANUAL integration check, performed by running a demo build and
// pressing keys. The automated coverage above proves every link of that chain in
// isolation:
//   - scancode → KeyCode           (raw::scancode unit tests)
//   - Win32 msg → RawInputEvent    (win32::translate unit tests + this file)
//   - RawInputEvent → ActionState  (i3_process / i4_ecs_integration tests)
//   - Window ring + GWLP_USERDATA  (boyko_rhi_vulkan, exercised by a live window)
// Only the OS→window_proc hop is manual; it is intentionally NOT faked here.
// ---------------------------------------------------------------------------
