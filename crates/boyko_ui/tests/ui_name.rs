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
