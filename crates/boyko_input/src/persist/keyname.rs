//! Canonical name ↔ [`KeyCode`] / [`MouseButton`] tables for the `.keys` format
//! (plan §9).
//!
//! These names are the human-editable tokens in a `.keys` file: `W`, `Space`,
//! `LCtrl`, `Mouse1`, etc. They are intentionally **stable** — a name is a
//! persisted token a user (or a mod) may have typed by hand, so renames are
//! breaking. Append-only, exactly like the [`KeyCode`] discriminant ABI (V6).
//!
//! Every canonical [`KeyCode`] variant has exactly one name; the round-trip
//! `name_of(code).and_then(keycode_from_name) == Some(code)` holds for every
//! fieldless variant. Exotic keys never appear here — they serialize as
//! `raw(0xNN)` ([`KeyCode::Unidentified`]) and parse back losslessly (plan §9.3),
//! so the table need not name them.

use crate::raw::keycode::{KeyCode, MouseButton};

/// The canonical `(name, KeyCode)` table. The first entry whose name matches a
/// token wins on parse; serialization uses [`name_of`], which returns the *same*
/// canonical name for each variant, so `parse ∘ serialize` is byte-identical.
///
/// Names use short, conventional spellings (`W`, `LCtrl`, `Space`) — the forms a
/// player expects to see and type in a config file. There are no aliases here:
/// one variant, one name, both directions, so the round-trip is total.
static KEY_NAMES: &[(&str, KeyCode)] = &[
    // Letters.
    ("A", KeyCode::KeyA),
    ("B", KeyCode::KeyB),
    ("C", KeyCode::KeyC),
    ("D", KeyCode::KeyD),
    ("E", KeyCode::KeyE),
    ("F", KeyCode::KeyF),
    ("G", KeyCode::KeyG),
    ("H", KeyCode::KeyH),
    ("I", KeyCode::KeyI),
    ("J", KeyCode::KeyJ),
    ("K", KeyCode::KeyK),
    ("L", KeyCode::KeyL),
    ("M", KeyCode::KeyM),
    ("N", KeyCode::KeyN),
    ("O", KeyCode::KeyO),
    ("P", KeyCode::KeyP),
    ("Q", KeyCode::KeyQ),
    ("R", KeyCode::KeyR),
    ("S", KeyCode::KeyS),
    ("T", KeyCode::KeyT),
    ("U", KeyCode::KeyU),
    ("V", KeyCode::KeyV),
    ("W", KeyCode::KeyW),
    ("X", KeyCode::KeyX),
    ("Y", KeyCode::KeyY),
    ("Z", KeyCode::KeyZ),
    // Top-row digits.
    ("Digit0", KeyCode::Digit0),
    ("Digit1", KeyCode::Digit1),
    ("Digit2", KeyCode::Digit2),
    ("Digit3", KeyCode::Digit3),
    ("Digit4", KeyCode::Digit4),
    ("Digit5", KeyCode::Digit5),
    ("Digit6", KeyCode::Digit6),
    ("Digit7", KeyCode::Digit7),
    ("Digit8", KeyCode::Digit8),
    ("Digit9", KeyCode::Digit9),
    // Function keys.
    ("F1", KeyCode::F1),
    ("F2", KeyCode::F2),
    ("F3", KeyCode::F3),
    ("F4", KeyCode::F4),
    ("F5", KeyCode::F5),
    ("F6", KeyCode::F6),
    ("F7", KeyCode::F7),
    ("F8", KeyCode::F8),
    ("F9", KeyCode::F9),
    ("F10", KeyCode::F10),
    ("F11", KeyCode::F11),
    ("F12", KeyCode::F12),
    ("F13", KeyCode::F13),
    ("F14", KeyCode::F14),
    ("F15", KeyCode::F15),
    ("F16", KeyCode::F16),
    ("F17", KeyCode::F17),
    ("F18", KeyCode::F18),
    ("F19", KeyCode::F19),
    ("F20", KeyCode::F20),
    ("F21", KeyCode::F21),
    ("F22", KeyCode::F22),
    ("F23", KeyCode::F23),
    ("F24", KeyCode::F24),
    // Editing / whitespace.
    ("Space", KeyCode::Space),
    ("Enter", KeyCode::Enter),
    ("Escape", KeyCode::Escape),
    ("Tab", KeyCode::Tab),
    ("Backspace", KeyCode::Backspace),
    ("Insert", KeyCode::Insert),
    ("Delete", KeyCode::Delete),
    ("Home", KeyCode::Home),
    ("End", KeyCode::End),
    ("PageUp", KeyCode::PageUp),
    ("PageDown", KeyCode::PageDown),
    ("CapsLock", KeyCode::CapsLock),
    // Arrows.
    ("Up", KeyCode::ArrowUp),
    ("Down", KeyCode::ArrowDown),
    ("Left", KeyCode::ArrowLeft),
    ("Right", KeyCode::ArrowRight),
    // Punctuation.
    ("Minus", KeyCode::Minus),
    ("Equal", KeyCode::Equal),
    ("BracketLeft", KeyCode::BracketLeft),
    ("BracketRight", KeyCode::BracketRight),
    ("Backslash", KeyCode::Backslash),
    ("Semicolon", KeyCode::Semicolon),
    ("Quote", KeyCode::Quote),
    ("Backquote", KeyCode::Backquote),
    ("Comma", KeyCode::Comma),
    ("Period", KeyCode::Period),
    ("Slash", KeyCode::Slash),
    // Numpad.
    ("Numpad0", KeyCode::Numpad0),
    ("Numpad1", KeyCode::Numpad1),
    ("Numpad2", KeyCode::Numpad2),
    ("Numpad3", KeyCode::Numpad3),
    ("Numpad4", KeyCode::Numpad4),
    ("Numpad5", KeyCode::Numpad5),
    ("Numpad6", KeyCode::Numpad6),
    ("Numpad7", KeyCode::Numpad7),
    ("Numpad8", KeyCode::Numpad8),
    ("Numpad9", KeyCode::Numpad9),
    ("NumpadAdd", KeyCode::NumpadAdd),
    ("NumpadSubtract", KeyCode::NumpadSubtract),
    ("NumpadMultiply", KeyCode::NumpadMultiply),
    ("NumpadDivide", KeyCode::NumpadDivide),
    ("NumpadDecimal", KeyCode::NumpadDecimal),
    ("NumpadEnter", KeyCode::NumpadEnter),
    ("NumLock", KeyCode::NumLock),
    // Modifiers (the short `LCtrl` family the example uses, plan §9.2).
    ("LShift", KeyCode::ShiftLeft),
    ("RShift", KeyCode::ShiftRight),
    ("LCtrl", KeyCode::ControlLeft),
    ("RCtrl", KeyCode::ControlRight),
    ("LAlt", KeyCode::AltLeft),
    ("RAlt", KeyCode::AltRight),
    ("LSuper", KeyCode::SuperLeft),
    ("RSuper", KeyCode::SuperRight),
    // Misc system.
    ("PrintScreen", KeyCode::PrintScreen),
    ("ScrollLock", KeyCode::ScrollLock),
    ("Pause", KeyCode::Pause),
    ("ContextMenu", KeyCode::ContextMenu),
];

/// Resolves a name token to its canonical [`KeyCode`], if any.
///
/// Returns `None` for an unknown token; the parser then records a per-line error
/// (plan §9.3 recoverable errors). The lookup is a linear scan over the static
/// table — cold load-time path only, never per-frame.
pub fn keycode_from_name(name: &str) -> Option<KeyCode> {
    KEY_NAMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, code)| *code)
}

/// The canonical name for a fieldless [`KeyCode`], or `None` for
/// [`KeyCode::Unidentified`] (which serializes as `raw(0xNN)` instead).
///
/// Compares by discriminant via [`KeyCode::dense_index`] so it is independent of
/// the table's textual order.
pub fn name_of(code: KeyCode) -> Option<&'static str> {
    let dense = code.dense_index()?;
    KEY_NAMES
        .iter()
        .find(|(_, c)| c.dense_index() == Some(dense))
        .map(|(n, _)| *n)
}

/// Parses a mouse-button spec token (`Mouse1`, `MouseBack`, `MouseFwd`,
/// `MouseOther(N)`), returning the [`MouseButton`] it names.
///
/// `Mouse1..=Mouse3` map to `Left`/`Right`/`Middle` (1-based, the convention the
/// example uses); `Mouse4`/`Mouse5` map to `Back`/`Forward`. `MouseBack` /
/// `MouseFwd` are explicit aliases for `Back` / `Forward`; `name_of_mouse`
/// serializes `Back`/`Forward` as the explicit forms so the round-trip is total.
/// Returns `None` for an unrecognized token.
pub fn mousebutton_from_token(token: &str) -> Option<MouseButton> {
    match token {
        "MouseBack" => Some(MouseButton::Back),
        "MouseFwd" => Some(MouseButton::Forward),
        _ => {
            if let Some(rest) = token.strip_prefix("MouseOther(") {
                let inner = rest.strip_suffix(')')?;
                let n: u16 = parse_int_u32(inner.trim())?.try_into().ok()?;
                return Some(MouseButton::Other(n));
            }
            let n = token.strip_prefix("Mouse")?;
            match n {
                "1" => Some(MouseButton::Left),
                "2" => Some(MouseButton::Right),
                "3" => Some(MouseButton::Middle),
                "4" => Some(MouseButton::Back),
                "5" => Some(MouseButton::Forward),
                _ => None,
            }
        }
    }
}

/// The canonical serialized token for a [`MouseButton`].
///
/// `Left`/`Right`/`Middle` serialize as `Mouse1`/`Mouse2`/`Mouse3`; `Back` and
/// `Forward` serialize as the explicit `MouseBack` / `MouseFwd` forms (chosen as
/// canonical over `Mouse4`/`Mouse5` so the form is self-documenting); `Other(n)`
/// as `MouseOther(n)`. Writes into `out` to avoid a per-call allocation in the
/// writer's buffer-building path.
pub fn write_mouse_token(button: MouseButton, out: &mut String) {
    match button {
        MouseButton::Left => out.push_str("Mouse1"),
        MouseButton::Right => out.push_str("Mouse2"),
        MouseButton::Middle => out.push_str("Mouse3"),
        MouseButton::Back => out.push_str("MouseBack"),
        MouseButton::Forward => out.push_str("MouseFwd"),
        MouseButton::Other(n) => {
            out.push_str("MouseOther(");
            push_u32(n as u32, out);
            out.push(')');
        }
    }
}

/// Parses an integer literal, accepting either decimal or a `0x`/`0X` hex prefix
/// (plan §9.1: `raw(0xNN)` and decimal `MouseOther(N)`). Returns `None` on an
/// empty string, an invalid digit, or overflow.
pub fn parse_int_u32(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        if hex.is_empty() {
            return None;
        }
        u32::from_str_radix(hex, 16).ok()
    } else {
        if s.is_empty() {
            return None;
        }
        s.parse::<u32>().ok()
    }
}

/// Appends a `u32` to `out` without allocating a temporary `String`.
pub fn push_u32(mut n: u32, out: &mut String) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    // SAFETY: `buf[i..]` holds only ASCII digit bytes written above, so the
    // slice is valid UTF-8.
    out.push_str(unsafe { core::str::from_utf8_unchecked(&buf[i..]) });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::keycode::CANONICAL_KEY_COUNT;

    /// Guards the `write_key` totality invariant: every canonical `KeyCode` slot
    /// (`0..CANONICAL_KEY_COUNT`) has exactly one `KEY_NAMES` row. Without this, a
    /// future canonical variant added without a name would silently serialize to
    /// `raw(0x<dense_index>)` and re-parse to a *different* value
    /// (`Unidentified`), corrupting that key on every save — a bug no round-trip
    /// test over the *existing* names would catch. This makes the `write_key`
    /// table-gap fallback (and its `debug_assert!`) provably unreachable.
    #[test]
    fn every_canonical_key_is_named() {
        let mut seen = [false; CANONICAL_KEY_COUNT];
        for (name, code) in KEY_NAMES {
            let dense = code
                .dense_index()
                .expect("invariant: KEY_NAMES never holds KeyCode::Unidentified");
            assert!(
                dense < CANONICAL_KEY_COUNT,
                "dense index {dense} for `{name}` is out of range"
            );
            assert!(
                !seen[dense],
                "duplicate KEY_NAMES entry for dense index {dense} (`{name}`)"
            );
            seen[dense] = true;
        }
        assert_eq!(
            KEY_NAMES.len(),
            CANONICAL_KEY_COUNT,
            "KEY_NAMES has {} entries but there are {CANONICAL_KEY_COUNT} canonical keys",
            KEY_NAMES.len()
        );
        for (dense, named) in seen.iter().enumerate() {
            assert!(
                *named,
                "canonical KeyCode dense index {dense} has no KEY_NAMES entry"
            );
        }
    }
}
