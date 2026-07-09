//! I1 gate — static scancode → [`KeyCode`] table coverage and the lossless
//! `Unidentified(raw)` fallback (plan §14 I1, §5.3).
//!
//! The two `[KeyCode; 256]` tables are indexed by a `u8` widened to `usize`, so
//! every possible scancode (`0..=255`) is in bounds and the lookup needs no
//! `unsafe`. These tests assert: (a) every mapped scancode resolves to the
//! intended canonical key; (b) every *unmapped* slot round-trips losslessly as
//! `Unidentified(raw)`; (c) the full `0..=255` index range is bounds-safe on
//! both tables and both `extended` selectors.

use boyko_input::raw::keycode::KeyCode;
use boyko_input::raw::scancode::{keycode_from_scancode, SCANCODE_TO_KEY, SCANCODE_TO_KEY_EXTENDED};

/// A representative sample of the base Set-1 mapping (the WASD cluster, the
/// modifiers, digits, and a function key) — the keys a default binding relies on.
#[test]
fn base_table_maps_known_scancodes() {
    // The WASD movement cluster — the most binding-critical keys.
    assert_eq!(keycode_from_scancode(0x11, false), KeyCode::KeyW, "0x11 = W");
    assert_eq!(keycode_from_scancode(0x1E, false), KeyCode::KeyA, "0x1E = A");
    assert_eq!(keycode_from_scancode(0x1F, false), KeyCode::KeyS, "0x1F = S");
    assert_eq!(keycode_from_scancode(0x20, false), KeyCode::KeyD, "0x20 = D");
    // Modifiers, whitespace, digits, a function key.
    assert_eq!(keycode_from_scancode(0x1D, false), KeyCode::ControlLeft);
    assert_eq!(keycode_from_scancode(0x2A, false), KeyCode::ShiftLeft);
    assert_eq!(keycode_from_scancode(0x39, false), KeyCode::Space);
    assert_eq!(keycode_from_scancode(0x01, false), KeyCode::Escape);
    assert_eq!(keycode_from_scancode(0x0B, false), KeyCode::Digit0);
    assert_eq!(keycode_from_scancode(0x3B, false), KeyCode::F1);
    assert_eq!(keycode_from_scancode(0x58, false), KeyCode::F12);
}

/// The extended (`E0`-prefixed) table must override the same low byte with the
/// navigation/numpad-control meaning — the canonical disambiguation case.
#[test]
fn extended_table_overrides_low_byte() {
    // Base 0x1C is main Enter; extended 0x1C is NumpadEnter.
    assert_eq!(keycode_from_scancode(0x1C, false), KeyCode::Enter, "base Enter");
    assert_eq!(
        keycode_from_scancode(0x1C, true),
        KeyCode::NumpadEnter,
        "extended NumpadEnter"
    );
    // Base 0x1D is left Ctrl; extended 0x1D is right Ctrl.
    assert_eq!(keycode_from_scancode(0x1D, false), KeyCode::ControlLeft);
    assert_eq!(keycode_from_scancode(0x1D, true), KeyCode::ControlRight);
    // Arrows live only in the extended table.
    assert_eq!(keycode_from_scancode(0x48, true), KeyCode::ArrowUp);
    assert_eq!(keycode_from_scancode(0x50, true), KeyCode::ArrowDown);
    assert_eq!(keycode_from_scancode(0x4B, true), KeyCode::ArrowLeft);
    assert_eq!(keycode_from_scancode(0x4D, true), KeyCode::ArrowRight);
}

/// An unmapped base scancode must round-trip losslessly as `Unidentified(raw)`,
/// never collapse to a wrong canonical key or be dropped.
#[test]
fn base_table_unmapped_falls_back_to_unidentified() {
    // 0x00 and 0x54..0x56 are not in the base override set.
    assert_eq!(
        keycode_from_scancode(0x00, false),
        KeyCode::Unidentified(0x00),
        "slot 0 carries its own raw value"
    );
    assert_eq!(keycode_from_scancode(0x54, false), KeyCode::Unidentified(0x54));
    assert_eq!(keycode_from_scancode(0x56, false), KeyCode::Unidentified(0x56));
    // High-range scancodes (no override anywhere) carry their own value.
    assert_eq!(keycode_from_scancode(0xFF, false), KeyCode::Unidentified(0xFF));
}

/// An unmapped *extended* slot must also fall back losslessly.
#[test]
fn extended_table_unmapped_falls_back_to_unidentified() {
    // 0x02 has a base mapping (Digit1) but no extended override.
    assert_eq!(
        keycode_from_scancode(0x02, true),
        KeyCode::Unidentified(0x02),
        "extended slot without an override carries its raw value"
    );
    assert_eq!(keycode_from_scancode(0xFE, true), KeyCode::Unidentified(0xFE));
}

/// Every scancode in `0..=255` is bounds-safe on both tables and both
/// selectors, and never panics. The fallback invariant: an `Unidentified`
/// result always carries exactly the index it was looked up with.
#[test]
fn full_index_range_is_bounds_safe_and_lossless() {
    for sc in 0u8..=255u8 {
        for extended in [false, true] {
            let code = keycode_from_scancode(sc, extended);
            // If it fell through to Unidentified, the payload must equal the
            // index (the lossless round-trip invariant from build_table).
            if let KeyCode::Unidentified(raw) = code {
                assert_eq!(
                    raw, sc as u32,
                    "Unidentified slot {sc:#04x} (extended={extended}) lost its raw value"
                );
            }
        }
    }
}

/// `keycode_from_scancode` must index exactly the same slot as a direct table
/// read — a cross-check that the public fn and the static tables agree.
#[test]
fn fn_agrees_with_static_tables() {
    for sc in 0u8..=255u8 {
        assert_eq!(
            keycode_from_scancode(sc, false),
            SCANCODE_TO_KEY[sc as usize],
            "base fn vs table mismatch at {sc:#04x}"
        );
        assert_eq!(
            keycode_from_scancode(sc, true),
            SCANCODE_TO_KEY_EXTENDED[sc as usize],
            "extended fn vs table mismatch at {sc:#04x}"
        );
    }
}

/// Every *canonical* `KeyCode` reachable through the base table must have a
/// distinct dense index (a table that mapped two scancodes to the same key is
/// fine, but a key with no dense index would be a bug — only `Unidentified`
/// lacks one). This guards the scancode→KeyCode→dense_index chain end-to-end.
#[test]
fn mapped_keys_have_dense_indices() {
    for sc in 0u8..=255u8 {
        let code = SCANCODE_TO_KEY[sc as usize];
        match code {
            KeyCode::Unidentified(_) => {} // by design, no dense index
            other => assert!(
                other.dense_index().is_some(),
                "canonical key {other:?} (base scancode {sc:#04x}) lacks a dense index"
            ),
        }
    }
}
