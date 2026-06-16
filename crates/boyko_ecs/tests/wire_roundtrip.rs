//! Phase S1.5 — value-level round-trips for the in-house `Wire` codec.
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` §3.1 / §3.7. Each `Wire` impl
//! (ints / floats / bool / char / String / Vec / Option / array / Entity) must
//! write → read back to the original value, and a malformed / truncated stream must
//! return `Err(DecodeError)` rather than panic or produce an invalid value (the C3
//! validate-never-transmute obligation).

use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::serialize::wire::Wire;
use boyko_ecs::ecs::core::serialize::{DecodeError, LoadCursor, SaveCursor};
use boyko_ecs::ecs::identifiers::primitives::EntityId;

/// Encodes a value into a fresh buffer, then decodes it back, asserting equality.
fn roundtrip<T: Wire + PartialEq + std::fmt::Debug>(value: T) {
    let mut buf = Vec::new();
    {
        let mut sink = SaveCursor::new(&mut buf);
        value.wire_write(&mut sink);
    }
    let mut src = LoadCursor::new(&buf);
    let decoded = T::wire_read(&mut src).expect("decode must succeed on well-formed bytes");
    assert_eq!(decoded, value, "Wire round-trip must preserve the value");
    assert_eq!(src.remaining(), 0, "the decode must consume exactly the bytes written");
}

// ── Scalars ──────────────────────────────────────────────────────────────────

#[test]
fn integers_roundtrip() {
    roundtrip(0u8);
    roundtrip(255u8);
    roundtrip(0xABCDu16);
    roundtrip(0xDEAD_BEEFu32);
    roundtrip(0x0123_4567_89AB_CDEFu64);
    roundtrip(u128::MAX);
    roundtrip(usize::MAX);
    roundtrip(-1i8);
    roundtrip(i16::MIN);
    roundtrip(i32::MIN);
    roundtrip(i64::MAX);
    roundtrip(i128::MIN);
    roundtrip(isize::MIN);
}

#[test]
fn floats_roundtrip_including_special_values() {
    roundtrip(0.0f32);
    roundtrip(-0.0f32);
    roundtrip(12.5f32);
    roundtrip(f32::INFINITY);
    roundtrip(f32::NEG_INFINITY);
    roundtrip(-987.625f64);
    roundtrip(f64::MIN);
    roundtrip(f64::MAX);

    // NaN does not compare equal to itself, so verify the bit pattern round-trips.
    let mut buf = Vec::new();
    {
        let mut sink = SaveCursor::new(&mut buf);
        f64::NAN.wire_write(&mut sink);
    }
    let mut src = LoadCursor::new(&buf);
    let decoded = f64::wire_read(&mut src).expect("NaN decodes");
    assert!(decoded.is_nan(), "a NaN bit pattern must round-trip as a NaN");
}

#[test]
fn bool_roundtrip() {
    roundtrip(true);
    roundtrip(false);
}

#[test]
fn bool_rejects_invalid_byte() {
    let buf = [2u8];
    let mut src = LoadCursor::new(&buf);
    assert_eq!(
        bool::wire_read(&mut src),
        Err(DecodeError::InvalidBitPattern),
        "a bool byte other than 0|1 must be rejected"
    );
}

#[test]
fn char_roundtrip() {
    roundtrip('a');
    roundtrip('Z');
    roundtrip('0');
    roundtrip('\u{1F600}'); // emoji (4-byte scalar)
    roundtrip('\u{0}');
    roundtrip(char::MAX);
}

#[test]
fn char_rejects_surrogate_and_out_of_range() {
    // A lone surrogate (0xD800) is not a valid `char`.
    let mut buf = Vec::new();
    {
        let mut sink = SaveCursor::new(&mut buf);
        0xD800u32.wire_write(&mut sink);
    }
    let mut src = LoadCursor::new(&buf);
    assert_eq!(char::wire_read(&mut src), Err(DecodeError::InvalidBitPattern));

    // A value above 0x10FFFF is not a valid `char`.
    let mut buf2 = Vec::new();
    {
        let mut sink = SaveCursor::new(&mut buf2);
        0x11_0000u32.wire_write(&mut sink);
    }
    let mut src2 = LoadCursor::new(&buf2);
    assert_eq!(char::wire_read(&mut src2), Err(DecodeError::InvalidBitPattern));
}

// ── String ─────────────────────────────────────────────────────────────────

#[test]
fn string_roundtrip() {
    roundtrip(String::new());
    roundtrip("hello".to_string());
    roundtrip("a longer string with spaces and symbols !@#".to_string());
    roundtrip("unicode: café — naïve — 日本語 — 🎉".to_string());
}

#[test]
fn string_rejects_invalid_utf8() {
    // A u32 length of 2 followed by an invalid UTF-8 sequence (0xFF, 0xFE).
    let mut buf = Vec::new();
    {
        let mut sink = SaveCursor::new(&mut buf);
        2u32.wire_write(&mut sink);
    }
    buf.push(0xFF);
    buf.push(0xFE);
    let mut src = LoadCursor::new(&buf);
    assert_eq!(
        String::wire_read(&mut src),
        Err(DecodeError::InvalidBitPattern),
        "invalid UTF-8 bytes must be rejected, never transmuted into a String"
    );
}

#[test]
fn string_rejects_truncated_payload() {
    // Length prefix claims 10 bytes but only 3 follow.
    let mut buf = Vec::new();
    {
        let mut sink = SaveCursor::new(&mut buf);
        10u32.wire_write(&mut sink);
    }
    buf.extend_from_slice(b"abc");
    let mut src = LoadCursor::new(&buf);
    assert_eq!(String::wire_read(&mut src), Err(DecodeError::UnexpectedEof));
}

// ── Vec<T> ─────────────────────────────────────────────────────────────────

#[test]
fn vec_roundtrip() {
    roundtrip::<Vec<u8>>(Vec::new());
    roundtrip(vec![1u8, 2, 3, 4, 5]);
    roundtrip(vec![-1i32, 0, 1, i32::MIN, i32::MAX]);
    roundtrip(vec![
        "a".to_string(),
        "bb".to_string(),
        "ccc".to_string(),
    ]);
    roundtrip(vec![vec![1u16, 2], vec![3, 4, 5], Vec::new()]);
}

#[test]
fn vec_rejects_hostile_count() {
    // A forged element count of u32::MAX with no following bytes must be rejected
    // before a huge allocation (the remaining-input bound).
    let mut buf = Vec::new();
    {
        let mut sink = SaveCursor::new(&mut buf);
        u32::MAX.wire_write(&mut sink);
    }
    let mut src = LoadCursor::new(&buf);
    assert_eq!(
        Vec::<u8>::wire_read(&mut src),
        Err(DecodeError::BadLengthPrefix),
        "a count larger than the remaining bytes must be rejected"
    );
}

#[test]
fn vec_rejects_truncated_elements() {
    // Claims 3 u32 elements (12 bytes) but only 4 bytes follow.
    let mut buf = Vec::new();
    {
        let mut sink = SaveCursor::new(&mut buf);
        3u32.wire_write(&mut sink);
        7u32.wire_write(&mut sink);
    }
    let mut src = LoadCursor::new(&buf);
    assert_eq!(Vec::<u32>::wire_read(&mut src), Err(DecodeError::UnexpectedEof));
}

// ── Option<T> ──────────────────────────────────────────────────────────────

#[test]
fn option_roundtrip() {
    roundtrip::<Option<u32>>(None);
    roundtrip(Some(42u32));
    roundtrip::<Option<String>>(None);
    roundtrip(Some("payload".to_string()));
    roundtrip(Some(vec![1u8, 2, 3]));
}

#[test]
fn option_rejects_invalid_tag() {
    let buf = [2u8]; // a tag byte other than 0|1
    let mut src = LoadCursor::new(&buf);
    assert_eq!(
        Option::<u32>::wire_read(&mut src),
        Err(DecodeError::InvalidBitPattern)
    );
}

// ── [T; N] ─────────────────────────────────────────────────────────────────

#[test]
fn array_roundtrip() {
    roundtrip([1u8, 2, 3, 4]);
    roundtrip([1.0f32, 2.0, 3.0]);
    roundtrip([true, false, true]);
    roundtrip(["a".to_string(), "bb".to_string()]);
    let empty: [u32; 0] = [];
    roundtrip(empty);
}

#[test]
fn array_rejects_truncated() {
    // A [u32; 3] needs 12 bytes; supply only 8.
    let mut buf = Vec::new();
    {
        let mut sink = SaveCursor::new(&mut buf);
        1u32.wire_write(&mut sink);
        2u32.wire_write(&mut sink);
    }
    let mut src = LoadCursor::new(&buf);
    assert_eq!(<[u32; 3]>::wire_read(&mut src), Err(DecodeError::UnexpectedEof));
}

// ── Entity (raw saved id; remap is S2) ───────────────────────────────────────

#[test]
fn entity_roundtrip_writes_raw_id() {
    let e = Entity::new(EntityId(12345), 7);
    let mut buf = Vec::new();
    {
        let mut sink = SaveCursor::new(&mut buf);
        e.wire_write(&mut sink);
    }
    // The id is written verbatim (u64) + generation (u32) = 12 bytes; no remap here.
    assert_eq!(buf.len(), 12, "Entity encodes a u64 id + u32 generation");

    let mut src = LoadCursor::new(&buf);
    let decoded = Entity::wire_read(&mut src).expect("Entity decodes");
    assert_eq!(decoded.id(), EntityId(12345), "the raw saved id round-trips");
    assert_eq!(decoded.generation(), 7);
}

#[test]
fn entity_rejects_truncated() {
    let buf = [0u8; 4]; // only 4 bytes; an Entity needs 12
    let mut src = LoadCursor::new(&buf);
    assert_eq!(Entity::wire_read(&mut src), Err(DecodeError::UnexpectedEof));
}
