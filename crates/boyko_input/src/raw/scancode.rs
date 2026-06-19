//! Static scancode → [`KeyCode`] translation tables (plan §5, §5.3).
//!
//! These map PC "Set 1" (AT) hardware scancodes — the values a Win32
//! `WM_KEYDOWN`/`WM_KEYUP` reports in `lParam` bits 16..=23 — onto canonical
//! physical [`KeyCode`]s. Two flat `[KeyCode; 256]` tables are provided: the
//! base set and the extended (`E0`-prefixed) set. A lookup is one array index —
//! no hashing, no branching beyond the extended/base select.
//!
//! The tables live in core (source-agnostic data) so the feature-gated Win32
//! adapter (I6) can index them directly; nothing in core *depends* on Win32.
//! Unknown scancodes resolve to [`KeyCode::Unidentified`] carrying the raw
//! value, so no key is ever dropped.

use super::keycode::KeyCode;

/// Builds a `[KeyCode; 256]` initialized to `Unidentified(i)` for every slot,
/// then applies the `(scancode, KeyCode)` overrides. `const` so the tables are
/// baked into the binary with zero startup cost.
const fn build_table(overrides: &[(u8, KeyCode)]) -> [KeyCode; 256] {
    let mut table = [KeyCode::Unidentified(0); 256];
    // Fill the default fallbacks: each slot carries its own raw scancode so an
    // unmapped key round-trips losslessly.
    let mut i = 0usize;
    while i < 256 {
        table[i] = KeyCode::Unidentified(i as u32);
        i += 1;
    }
    let mut j = 0usize;
    while j < overrides.len() {
        let (sc, code) = overrides[j];
        table[sc as usize] = code;
        j += 1;
    }
    table
}

/// Base (non-extended) Set-1 scancode → [`KeyCode`] table.
pub static SCANCODE_TO_KEY: [KeyCode; 256] = build_table(&[
    (0x01, KeyCode::Escape),
    (0x02, KeyCode::Digit1),
    (0x03, KeyCode::Digit2),
    (0x04, KeyCode::Digit3),
    (0x05, KeyCode::Digit4),
    (0x06, KeyCode::Digit5),
    (0x07, KeyCode::Digit6),
    (0x08, KeyCode::Digit7),
    (0x09, KeyCode::Digit8),
    (0x0A, KeyCode::Digit9),
    (0x0B, KeyCode::Digit0),
    (0x0C, KeyCode::Minus),
    (0x0D, KeyCode::Equal),
    (0x0E, KeyCode::Backspace),
    (0x0F, KeyCode::Tab),
    (0x10, KeyCode::KeyQ),
    (0x11, KeyCode::KeyW),
    (0x12, KeyCode::KeyE),
    (0x13, KeyCode::KeyR),
    (0x14, KeyCode::KeyT),
    (0x15, KeyCode::KeyY),
    (0x16, KeyCode::KeyU),
    (0x17, KeyCode::KeyI),
    (0x18, KeyCode::KeyO),
    (0x19, KeyCode::KeyP),
    (0x1A, KeyCode::BracketLeft),
    (0x1B, KeyCode::BracketRight),
    (0x1C, KeyCode::Enter),
    (0x1D, KeyCode::ControlLeft),
    (0x1E, KeyCode::KeyA),
    (0x1F, KeyCode::KeyS),
    (0x20, KeyCode::KeyD),
    (0x21, KeyCode::KeyF),
    (0x22, KeyCode::KeyG),
    (0x23, KeyCode::KeyH),
    (0x24, KeyCode::KeyJ),
    (0x25, KeyCode::KeyK),
    (0x26, KeyCode::KeyL),
    (0x27, KeyCode::Semicolon),
    (0x28, KeyCode::Quote),
    (0x29, KeyCode::Backquote),
    (0x2A, KeyCode::ShiftLeft),
    (0x2B, KeyCode::Backslash),
    (0x2C, KeyCode::KeyZ),
    (0x2D, KeyCode::KeyX),
    (0x2E, KeyCode::KeyC),
    (0x2F, KeyCode::KeyV),
    (0x30, KeyCode::KeyB),
    (0x31, KeyCode::KeyN),
    (0x32, KeyCode::KeyM),
    (0x33, KeyCode::Comma),
    (0x34, KeyCode::Period),
    (0x35, KeyCode::Slash),
    (0x36, KeyCode::ShiftRight),
    (0x37, KeyCode::NumpadMultiply),
    (0x38, KeyCode::AltLeft),
    (0x39, KeyCode::Space),
    (0x3A, KeyCode::CapsLock),
    (0x3B, KeyCode::F1),
    (0x3C, KeyCode::F2),
    (0x3D, KeyCode::F3),
    (0x3E, KeyCode::F4),
    (0x3F, KeyCode::F5),
    (0x40, KeyCode::F6),
    (0x41, KeyCode::F7),
    (0x42, KeyCode::F8),
    (0x43, KeyCode::F9),
    (0x44, KeyCode::F10),
    (0x45, KeyCode::NumLock),
    (0x46, KeyCode::ScrollLock),
    (0x47, KeyCode::Numpad7),
    (0x48, KeyCode::Numpad8),
    (0x49, KeyCode::Numpad9),
    (0x4A, KeyCode::NumpadSubtract),
    (0x4B, KeyCode::Numpad4),
    (0x4C, KeyCode::Numpad5),
    (0x4D, KeyCode::Numpad6),
    (0x4E, KeyCode::NumpadAdd),
    (0x4F, KeyCode::Numpad1),
    (0x50, KeyCode::Numpad2),
    (0x51, KeyCode::Numpad3),
    (0x52, KeyCode::Numpad0),
    (0x53, KeyCode::NumpadDecimal),
    (0x57, KeyCode::F11),
    (0x58, KeyCode::F12),
]);

/// Extended (`E0`-prefixed) Set-1 scancode → [`KeyCode`] table. The same low
/// byte that means one thing in the base set means another when preceded by the
/// `E0` prefix (e.g. base `0x1C` = main Enter, extended `0x1C` = NumpadEnter).
pub static SCANCODE_TO_KEY_EXTENDED: [KeyCode; 256] = build_table(&[
    (0x1C, KeyCode::NumpadEnter),
    (0x1D, KeyCode::ControlRight),
    (0x35, KeyCode::NumpadDivide),
    (0x37, KeyCode::PrintScreen),
    (0x38, KeyCode::AltRight),
    (0x47, KeyCode::Home),
    (0x48, KeyCode::ArrowUp),
    (0x49, KeyCode::PageUp),
    (0x4B, KeyCode::ArrowLeft),
    (0x4D, KeyCode::ArrowRight),
    (0x4F, KeyCode::End),
    (0x50, KeyCode::ArrowDown),
    (0x51, KeyCode::PageDown),
    (0x52, KeyCode::Insert),
    (0x53, KeyCode::Delete),
    (0x5B, KeyCode::SuperLeft),
    (0x5C, KeyCode::SuperRight),
    (0x5D, KeyCode::ContextMenu),
]);

/// Translates a Set-1 hardware scancode to a canonical [`KeyCode`].
///
/// `extended` selects the `E0`-prefixed table (the OS sets this flag for the
/// duplicated navigation / numpad-control keys). The index is a `u8` widened to
/// `usize`, so it is always in `0..256` — the array access cannot go out of
/// bounds and needs no `unsafe`.
#[inline]
pub fn keycode_from_scancode(scancode: u8, extended: bool) -> KeyCode {
    let table = if extended {
        &SCANCODE_TO_KEY_EXTENDED
    } else {
        &SCANCODE_TO_KEY
    };
    table[scancode as usize]
}
