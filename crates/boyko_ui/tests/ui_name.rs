//! P2 Test #7 — `UiName` inline small-string unit tests.

use boyko_ui::components::UiName;

#[test]
fn new_roundtrips_ascii() {
    let n = UiName::new("header");
    assert_eq!(n.as_str(), "header", "ascii name round-trips through the inline buffer");
    assert_eq!(n.len(), 6, "len is the byte count");
    assert!(!n.is_empty(), "non-empty name");
}

#[test]
fn new_roundtrips_empty() {
    let n = UiName::new("");
    assert_eq!(n.as_str(), "", "empty name round-trips");
    assert_eq!(n.len(), 0, "empty len is 0");
    assert!(n.is_empty(), "empty name reports is_empty");
}

#[test]
fn new_roundtrips_multibyte_utf8() {
    // "héllo✓" — 'é' = 2 bytes, '✓' = 3 bytes -> 4 ascii + 2 + 3 = 9 bytes.
    let s = "héllo✓";
    let n = UiName::new(s);
    assert_eq!(n.as_str(), s, "multibyte UTF-8 round-trips verbatim");
    assert_eq!(n.len(), s.len(), "len is the UTF-8 byte length, not char count");
}

#[test]
fn new_at_capacity_roundtrips() {
    // Exactly CAP (60) bytes of ASCII.
    let s: String = "a".repeat(UiName::CAP);
    let n = UiName::new(&s);
    assert_eq!(n.len(), UiName::CAP, "a CAP-length name fills the buffer");
    assert_eq!(n.as_str(), s.as_str(), "a CAP-length name round-trips");
}

#[test]
fn partial_eq_matches_on_value() {
    assert_eq!(UiName::new("panel"), UiName::new("panel"), "same name compares equal");
    assert_ne!(UiName::new("panel"), UiName::new("button"), "different names differ");
    // Trailing-zero invariant: equal names with the same bytes are equal; a
    // shorter name is never equal to a longer one sharing a prefix.
    assert_ne!(UiName::new("a"), UiName::new("ab"), "prefix is not equal to the longer name");
}

#[test]
fn size_is_one_cache_line() {
    assert_eq!(core::mem::size_of::<UiName>(), 64, "UiName is exactly 64 B (one cache line)");
    assert_eq!(core::mem::align_of::<UiName>(), 64, "UiName is 64 B aligned");
}

#[test]
fn new_is_const_fn() {
    // Usable in const context (the `ui!` literal path relies on this).
    const N: UiName = UiName::new("const_name");
    assert_eq!(N.as_str(), "const_name", "const-constructed name is valid");
}

#[test]
fn copy_semantics() {
    let a = UiName::new("copyme");
    let b = a; // Copy, not move.
    assert_eq!(a.as_str(), "copyme", "original still usable after copy");
    assert_eq!(b.as_str(), "copyme", "copy carries the value");
}

/// An over-length name is a caller bug that the `debug_assert!` in `UiName::new` catches
/// in debug builds (the `ui!` macro enforces the bound at compile time; the P3 text path
/// bounds its own input). This pins that guard.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "invariant: ui name exceeds CAP")]
fn over_length_name_panics_in_debug() {
    let over = "a".repeat(UiName::CAP + 5);
    let _ = UiName::new(&over);
}

/// RELEASE-path safety net (the `debug_assert!` is compiled out): an over-length name
/// whose CAP-byte cut lands INSIDE a multi-byte char MUST truncate at the preceding char
/// boundary — never store a partial multi-byte sequence, which `as_str`'s
/// `from_utf8_unchecked` would read as invalid UTF-8 → UB from safe code.
///
/// Construction: `lead` ASCII bytes + a 3-byte '✓' straddling byte 60 (CAP). The naive
/// byte-CAP clamp would copy the FIRST bytes of '✓' — an invalid-UTF-8 prefix. The
/// char-boundary back-off must drop the split '✓', yielding the ASCII-run prefix. Every
/// stored slice, across all straddle offsets, must be valid UTF-8 on a char boundary.
#[cfg(not(debug_assertions))]
#[test]
fn over_length_name_truncates_on_a_char_boundary_in_release() {
    // '✓' (U+2713) is 3 bytes; leading byte at 58 / 59 / 60 exercises every split offset.
    for lead in [58usize, 59, 60] {
        let mut over = "a".repeat(lead);
        over.push('✓');
        over.push_str(&"b".repeat(20)); // tail well past CAP so the cut is forced inside
        assert!(over.len() > UiName::CAP, "test input exceeds CAP");

        let n = UiName::new(&over);
        let got = n.as_str();

        assert!(n.len() <= UiName::CAP, "lead {lead}: len clamped to CAP");
        // The stored bytes are valid UTF-8 (no split multi-byte char).
        assert_eq!(
            core::str::from_utf8(got.as_bytes()),
            Ok(got),
            "lead {lead}: the stored bytes are valid UTF-8"
        );
        // A genuine char-boundary prefix of the source (never a byte-split of it).
        assert!(over.is_char_boundary(got.len()), "lead {lead}: truncation is on a char boundary");
        assert_eq!(&over[..got.len()], got, "lead {lead}: the stored prefix matches the source");
    }
}

/// A name ending with a multi-byte char exactly AT the CAP boundary is stored whole (the
/// back-off only fires when the cut SPLITS a char, not when it lands on a boundary). This
/// is within CAP so it holds in both debug and release.
#[test]
fn multibyte_char_ending_exactly_at_cap_is_kept_whole() {
    // 57 ASCII + '✓' (3 bytes) = exactly 60 bytes = CAP, on a char boundary.
    let mut s = "a".repeat(UiName::CAP - 3);
    s.push('✓');
    assert_eq!(s.len(), UiName::CAP, "input is exactly CAP bytes");
    let n = UiName::new(&s);
    assert_eq!(n.len(), UiName::CAP, "an exactly-CAP name fills the buffer");
    assert_eq!(n.as_str(), s.as_str(), "the trailing multi-byte char at CAP is kept whole");
}
