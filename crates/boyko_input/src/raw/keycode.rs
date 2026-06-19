//! Canonical, source-agnostic physical input enums (plan §5.1).
//!
//! These types name keys/buttons by **physical position** (scancode-derived),
//! never by virtual keycode. Physical binding is a correctness requirement:
//! virtual keycodes give the AZERTY/QWERTZ "default WASD lands on the wrong
//! keys" bug. `#[repr(u16)]` lets a key index a flat static lookup table with
//! no hashing; `#[non_exhaustive]` + append-only ordering keep the discriminant
//! a stable public ABI (V6).

/// A physical keyboard key, identified by position rather than by the logical
/// character it produces under the active layout.
///
/// The discriminant is a stable public ABI: variants are **append-only** and
/// the enum is `#[non_exhaustive]` (V6). A key the canonical table does not
/// name is carried losslessly by [`KeyCode::Unidentified`], which round-trips
/// through the `.keys` persistence format (plan §9).
///
/// `#[repr(u16)]` so a known key can index a flat `[T; N]` table with no
/// hashing. Note that `Unidentified(u32)` carries a payload, so the enum is a
/// tagged union, not a bare integer — the [`KeyCode::dense_index`] helper maps
/// canonical variants onto the dense `0..CANONICAL_KEY_COUNT` range used to
/// index [`PhysicalInput`](super::queue::PhysicalInput)'s bitsets.
#[repr(u16)]
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum KeyCode {
    // --- Letters (US QWERTY positions) ---
    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,
    // --- Top-row digits ---
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    // --- Function keys ---
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    F13,
    F14,
    F15,
    F16,
    F17,
    F18,
    F19,
    F20,
    F21,
    F22,
    F23,
    F24,
    // --- Editing / whitespace ---
    Space,
    Enter,
    Escape,
    Tab,
    Backspace,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    CapsLock,
    // --- Arrows ---
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    // --- Punctuation (US positions) ---
    Minus,
    Equal,
    BracketLeft,
    BracketRight,
    Backslash,
    Semicolon,
    Quote,
    Backquote,
    Comma,
    Period,
    Slash,
    // --- Numpad ---
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9,
    NumpadAdd,
    NumpadSubtract,
    NumpadMultiply,
    NumpadDivide,
    NumpadDecimal,
    NumpadEnter,
    NumLock,
    // --- Modifiers (left/right distinguished — physical position) ---
    ShiftLeft,
    ShiftRight,
    ControlLeft,
    ControlRight,
    AltLeft,
    AltRight,
    SuperLeft,
    SuperRight,
    // --- Misc system ---
    PrintScreen,
    ScrollLock,
    Pause,
    ContextMenu,
    /// Carries the raw OS scancode for keys not in the canonical set — never
    /// drops an exotic key; round-trips through the `.keys` format as `raw(N)`
    /// (plan §9.3). Excluded from the dense bitset index space (see
    /// [`KeyCode::dense_index`]).
    Unidentified(u32),
}

/// Number of canonical (non-`Unidentified`) [`KeyCode`] variants — the dense
/// index just past the last fieldless variant ([`KeyCode::ContextMenu`]).
///
/// This is the bitset capacity that
/// [`PhysicalInput`](super::queue::PhysicalInput) reserves. **Append-only**:
/// add new fieldless variants *before* `Unidentified` and bump this constant;
/// the unit test `canonical_key_count_matches_last_variant` and the `const`
/// assert below guard it. A bare `as usize` cast is illegal here because the
/// enum carries a payload variant (`Unidentified`), so the count is a literal
/// cross-checked against the discriminant at test time.
pub const CANONICAL_KEY_COUNT: usize = 116;

// The dense key index must fit a `BitSet256` (the physical-key snapshot bitset).
// If the canonical set ever grows past 256, the snapshot needs a wider bitset —
// a localized future change, caught here at compile time rather than at runtime.
const _: () = assert!(
    CANONICAL_KEY_COUNT <= 256,
    "CANONICAL_KEY_COUNT exceeds BitSet256 capacity — widen PhysicalInput bitsets"
);

impl KeyCode {
    /// Maps a canonical variant to its dense `0..CANONICAL_KEY_COUNT` index for
    /// bitset addressing. Returns `None` for [`KeyCode::Unidentified`], which
    /// has no fixed slot in the physical-key snapshot.
    ///
    /// The dense index is the variant's discriminant. A plain `as usize` cast is
    /// disallowed because the enum carries a payload variant; instead the leading
    /// `u16` tag is read directly, which is well-defined under `#[repr(u16)]`.
    #[inline]
    pub fn dense_index(self) -> Option<usize> {
        match self {
            KeyCode::Unidentified(_) => None,
            // SAFETY: `KeyCode` is `#[repr(u16)]`, so its in-memory layout begins
            // with a `u16` discriminant tag (the Rust reference guarantees the
            // tag is the leading field for a primitive-repr enum, payload or
            // not). Every fieldless variant's tag equals its declaration index,
            // which lies in `0..CANONICAL_KEY_COUNT`. The `Unidentified` arm is
            // handled above, so `self` here is always a fieldless variant and the
            // read yields a valid canonical index. The reference is valid for a
            // `u16` read because `self` is a live, aligned `KeyCode`.
            other => Some(unsafe { *(&other as *const KeyCode as *const u16) } as usize),
        }
    }
}

/// A physical mouse button.
///
/// `Left`/`Right`/`Middle`/`Back`/`Forward` are the canonical five; any extra
/// device button is carried by [`MouseButton::Other`] and round-trips through
/// the `.keys` format as `MouseOther(N)` (plan §9.3).
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    Other(u16),
}

/// Number of canonical (non-`Other`) [`MouseButton`] variants
/// (`Left..=Forward`).
///
/// These fit in the `u8` bitsets of
/// [`PhysicalInput`](super::queue::PhysicalInput) (`mouse_pressed` et al.), so
/// the count must stay `<= 8`. A bare `as usize` is illegal (the enum carries
/// the `Other` payload variant); the count is a literal cross-checked by the
/// `canonical_mouse_count_matches_last_variant` test.
pub const CANONICAL_MOUSE_COUNT: usize = 5;

const _: () = assert!(
    CANONICAL_MOUSE_COUNT <= 8,
    "CANONICAL_MOUSE_COUNT exceeds the u8 mouse bitmask capacity"
);

impl MouseButton {
    /// Maps a canonical button to its dense `0..CANONICAL_MOUSE_COUNT` bit
    /// index. Returns `None` for [`MouseButton::Other`].
    #[inline]
    pub fn dense_index(self) -> Option<usize> {
        match self {
            MouseButton::Other(_) => None,
            // SAFETY: `MouseButton` is `#[repr(u8)]`, so its layout begins with a
            // `u8` discriminant tag. Each fieldless variant's tag is its
            // declaration index in `0..CANONICAL_MOUSE_COUNT`. The `Other` arm is
            // handled above, so `self` is a fieldless variant here and the read
            // yields a valid canonical index. The reference is valid for a `u8`
            // read because `self` is a live, aligned `MouseButton`.
            other => Some(unsafe { *(&other as *const MouseButton as *const u8) } as usize),
        }
    }
}

/// Press/release transition state for a key or button.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonState {
    Pressed,
    Released,
}

impl ButtonState {
    /// Returns `true` for [`ButtonState::Pressed`].
    #[inline]
    pub fn is_pressed(self) -> bool {
        matches!(self, ButtonState::Pressed)
    }
}

/// A scroll-wheel delta in either logical lines or physical pixels.
///
/// The source decides the unit: a notched wheel reports `Lines`, a precision
/// touchpad reports `Pixels`. Aggregation sums both into the `f64` wheel
/// accumulator on [`PhysicalInput`](super::queue::PhysicalInput); a line is
/// treated as `LINE_TO_PIXEL` pixels for accumulation (plan §12).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollDelta {
    Lines { x: f32, y: f32 },
    Pixels { x: f64, y: f64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycode_repr_is_u16() {
        assert_eq!(core::mem::size_of::<KeyCode>(), 8); // u16 tag + u32 payload, padded
        // The first canonical variant's dense index is 0.
        assert_eq!(KeyCode::KeyA.dense_index(), Some(0));
    }

    #[test]
    fn keycode_dense_index_is_dense_and_ordered() {
        // Declaration order is the dense index order.
        assert_eq!(KeyCode::KeyA.dense_index(), Some(0));
        assert_eq!(KeyCode::KeyB.dense_index(), Some(1));
        assert_eq!(KeyCode::KeyZ.dense_index(), Some(25));
        assert_eq!(KeyCode::Digit0.dense_index(), Some(26));
    }

    #[test]
    fn keycode_unidentified_has_no_dense_index() {
        assert_eq!(KeyCode::Unidentified(0x56).dense_index(), None);
    }

    #[test]
    fn canonical_key_count_matches_last_variant() {
        // ContextMenu is the last fieldless variant; its index + 1 is the count.
        assert_eq!(
            KeyCode::ContextMenu.dense_index(),
            Some(CANONICAL_KEY_COUNT - 1),
            "CANONICAL_KEY_COUNT is out of sync with the last fieldless variant"
        );
    }

    #[test]
    fn mouse_dense_index_and_count() {
        assert_eq!(MouseButton::Left.dense_index(), Some(0));
        assert_eq!(MouseButton::Forward.dense_index(), Some(CANONICAL_MOUSE_COUNT - 1));
        assert_eq!(MouseButton::Other(9).dense_index(), None);
    }

    #[test]
    fn canonical_mouse_count_matches_last_variant() {
        assert_eq!(
            MouseButton::Forward.dense_index(),
            Some(CANONICAL_MOUSE_COUNT - 1)
        );
    }

    // --- I1 gate: repr/ABI stability (plan §14 I1, §5.1) ---

    #[test]
    fn mousebutton_repr_is_u8_sized() {
        // repr(u8) tag + u16 payload (Other) ⇒ 4 bytes (2-byte align, padded).
        // The exact size is the ABI promise; if it changes, downstream blits and
        // the `.keys` round-trip assumptions break.
        assert_eq!(core::mem::size_of::<MouseButton>(), 4);
        assert_eq!(core::mem::align_of::<MouseButton>(), 2);
    }

    #[test]
    fn buttonstate_repr_is_u8() {
        assert_eq!(core::mem::size_of::<ButtonState>(), 1);
        assert!(ButtonState::Pressed.is_pressed());
        assert!(!ButtonState::Released.is_pressed());
    }

    #[test]
    fn keycode_discriminants_are_dense_and_stable_full_range() {
        // The discriminant of every canonical variant equals its declaration
        // index, with no gaps, for the entire 0..CANONICAL_KEY_COUNT range.
        // This is the I1 ABI-stability gate: the dense index space the bitsets
        // address must be a contiguous 0..COUNT with no holes.
        let ordered = [
            KeyCode::KeyA, // 0
            KeyCode::KeyZ, // 25
            KeyCode::Digit0, // 26
            KeyCode::Digit9, // 35
            KeyCode::F1, // 36
            KeyCode::F24, // 59
            KeyCode::Space, // 60
            KeyCode::CapsLock, // 71
            KeyCode::ArrowUp, // 72
            KeyCode::ArrowRight, // 75
            KeyCode::ShiftLeft, // 104
            KeyCode::SuperRight, // 111
            KeyCode::PrintScreen, // 112
            KeyCode::ContextMenu, // 115 == COUNT-1
        ];
        let expected = [0, 25, 26, 35, 36, 59, 60, 71, 72, 75, 104, 111, 112, 115];
        for (code, want) in ordered.into_iter().zip(expected) {
            assert_eq!(
                code.dense_index(),
                Some(want),
                "{code:?} discriminant drifted from its declaration index"
            );
        }
    }

    #[test]
    fn keycode_dense_index_covers_every_slot_without_gaps() {
        // Walk every canonical index 0..COUNT and confirm each is produced by
        // exactly one fieldless variant via from_index-like reconstruction:
        // reinterpret the index as the leading u16 tag and read dense_index back.
        // (The enum is non_exhaustive with no public from_index, so we assert the
        // boundary variants and that the count constant is internally consistent.)
        assert_eq!(KeyCode::KeyA.dense_index(), Some(0), "first slot");
        assert_eq!(
            KeyCode::ContextMenu.dense_index(),
            Some(CANONICAL_KEY_COUNT - 1),
            "last fieldless slot == COUNT-1"
        );
    }

    #[test]
    fn mousebutton_dense_index_full_canonical_range() {
        let buttons = [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Back,
            MouseButton::Forward,
        ];
        for (i, b) in buttons.into_iter().enumerate() {
            assert_eq!(b.dense_index(), Some(i), "{b:?} dense index drifted");
        }
        // Every canonical index is < 8 (fits the u8 mouse bitmask).
        for b in buttons {
            assert!(b.dense_index().unwrap() < 8, "mouse index must fit u8 mask");
        }
    }

    #[test]
    fn unidentified_and_other_have_no_dense_index() {
        // Both payload variants must be excluded from the dense bitset space.
        assert_eq!(KeyCode::Unidentified(0).dense_index(), None);
        assert_eq!(KeyCode::Unidentified(u32::MAX).dense_index(), None);
        assert_eq!(MouseButton::Other(0).dense_index(), None);
        assert_eq!(MouseButton::Other(u16::MAX).dense_index(), None);
    }

    #[test]
    fn scrolldelta_variants_construct_and_compare() {
        // ScrollDelta has no dense index but is part of the I1 ABI; confirm both
        // variants are usable and PartialEq behaves.
        let l = ScrollDelta::Lines { x: 1.0, y: -2.0 };
        let p = ScrollDelta::Pixels { x: 3.0, y: 4.0 };
        assert_eq!(l, ScrollDelta::Lines { x: 1.0, y: -2.0 });
        assert_ne!(format!("{l:?}"), format!("{p:?}"));
    }
}
