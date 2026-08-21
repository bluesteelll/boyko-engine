//! CORE C1 gates over `Scalar` / `ScalarKind`.
//!
//! * **Gate 1** — the layout pin: `size_of` / `align_of` MEASURED here (printed under
//!   `--nocapture`) and pinned in `src/scalar.rs` at the measured values (16 / 8).
//! * **Gate 2** — per-kind round-trip `Scalar::from(x).as_<kind>() == Some(x)`:
//!   full-range proptests plus deterministic edge tests carrying `i*::MIN`, `u*::MAX`,
//!   `f32`/`f64` NaN, `±0.0` and subnormals. Float trips are asserted on **bits**, not
//!   values — value equality would reject NaN wrongly and accept a `-0.0 → 0.0` swap
//!   wrongly. Signed test names say their ranges include negatives, because C1's second
//!   RED (a zero-extending extractor) is red **only** there.
//! * **Gate 3** — the 12×12 cross-kind matrix: every off-diagonal extraction is `None`.
//!
//! The full-range proptests are `#[cfg_attr(miri, ignore)]`: the per-case interpreter
//! overhead is prohibitive at a meaningful budget, and the deterministic edge tests in
//! this file carry every named edge under Miri.

use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_reflect::{Scalar, ScalarKind};
use proptest::prelude::*;

// ─── Gate 1: layout, measured then pinned ────────────────────────────────────────────

/// Prints the measured layout (C1 gate 1 records these in the commit message) and
/// asserts it equals the `const _` pin in `src/scalar.rs`.
#[test]
fn scalar_layout_measured_and_pinned() {
    println!("size_of::<Scalar>()  = {}", size_of::<Scalar>());
    println!("align_of::<Scalar>() = {}", align_of::<Scalar>());
    assert_eq!(size_of::<Scalar>(), 16, "Scalar size moved off the measured pin");
    assert_eq!(align_of::<Scalar>(), 8, "Scalar align moved off the measured pin");
}

// ─── Gate 2, deterministic edges (these run under Miri) ──────────────────────────────

/// `bool`: both inhabitants round-trip; a non-canonical payload is refused.
#[test]
fn bool_roundtrip_both_values_and_noncanonical_bits_refused() {
    assert_eq!(Scalar::from(false).as_bool(), Some(false));
    assert_eq!(Scalar::from(true).as_bool(), Some(true));
    let junk = Scalar { kind: ScalarKind::Bool, bits: 2 };
    assert_eq!(junk.as_bool(), None, "a hand-built non-canonical Bool payload must be None");
}

/// Unsigned kinds: `0` and `MAX` per width.
#[test]
fn unsigned_edges_roundtrip_including_max() {
    assert_eq!(Scalar::from(0u8).as_u8(), Some(0));
    assert_eq!(Scalar::from(u8::MAX).as_u8(), Some(u8::MAX));
    assert_eq!(Scalar::from(0u16).as_u16(), Some(0));
    assert_eq!(Scalar::from(u16::MAX).as_u16(), Some(u16::MAX));
    assert_eq!(Scalar::from(0u32).as_u32(), Some(0));
    assert_eq!(Scalar::from(u32::MAX).as_u32(), Some(u32::MAX));
    assert_eq!(Scalar::from(0u64).as_u64(), Some(0));
    assert_eq!(Scalar::from(u64::MAX).as_u64(), Some(u64::MAX));
}

/// Signed kinds: `MIN`, `-1`, `0`, `MAX` per width. The name says negatives are
/// covered because that is where a zero-extending extractor (C1's second RED) is the
/// only observable difference.
#[test]
fn signed_edges_roundtrip_including_min_and_negatives_sign_extension_rule() {
    for v in [i8::MIN, -1, 0, i8::MAX] {
        assert_eq!(Scalar::from(v).as_i8(), Some(v), "i8 {v}");
    }
    for v in [i16::MIN, -1, 0, i16::MAX] {
        assert_eq!(Scalar::from(v).as_i16(), Some(v), "i16 {v}");
    }
    for v in [i32::MIN, -1, 0, i32::MAX] {
        assert_eq!(Scalar::from(v).as_i32(), Some(v), "i32 {v}");
    }
    for v in [i64::MIN, -1, 0, i64::MAX] {
        assert_eq!(Scalar::from(v).as_i64(), Some(v), "i64 {v}");
    }
}

/// `f32` edges, asserted on BITS: NaN, `±0.0`, a subnormal, `MIN_POSITIVE`, both
/// infinities and both finite extremes all survive bit-exactly.
#[test]
fn f32_edges_roundtrip_bitwise_nan_negzero_subnormal() {
    let edges = [
        f32::NAN,
        0.0,
        -0.0,
        f32::from_bits(1), // smallest subnormal
        f32::MIN_POSITIVE,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::MIN,
        f32::MAX,
    ];
    for v in edges {
        let got = Scalar::from(v).as_f32().expect("kind matches");
        assert_eq!(got.to_bits(), v.to_bits(), "f32 bit pattern {:#010x}", v.to_bits());
    }
}

/// `f64` edges, asserted on BITS — same set as the `f32` leg.
#[test]
fn f64_edges_roundtrip_bitwise_nan_negzero_subnormal() {
    let edges = [
        f64::NAN,
        0.0,
        -0.0,
        f64::from_bits(1), // smallest subnormal
        f64::MIN_POSITIVE,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MIN,
        f64::MAX,
    ];
    for v in edges {
        let got = Scalar::from(v).as_f64().expect("kind matches");
        assert_eq!(got.to_bits(), v.to_bits(), "f64 bit pattern {:#018x}", v.to_bits());
    }
}

/// `EntityId`: zero and the platform maximum round-trip.
#[test]
fn entity_id_roundtrip_including_max() {
    assert_eq!(Scalar::from(EntityId(0)).as_entity_id(), Some(EntityId(0)));
    assert_eq!(Scalar::from(EntityId(usize::MAX)).as_entity_id(), Some(EntityId(usize::MAX)));
}

// ─── Gate 2, full-range proptests ────────────────────────────────────────────────────

proptest! {
    #[cfg_attr(miri, ignore = "full-range proptest budget is prohibitive under Miri; the deterministic edge tests in this file carry MIN/MAX/NaN/±0/subnormals under Miri")]
    #[test]
    fn u8_roundtrip_full_range(v in any::<u8>()) {
        prop_assert_eq!(Scalar::from(v).as_u8(), Some(v));
    }

    #[cfg_attr(miri, ignore = "full-range proptest budget is prohibitive under Miri; the deterministic edge tests in this file carry MIN/MAX/NaN/±0/subnormals under Miri")]
    #[test]
    fn u16_roundtrip_full_range(v in any::<u16>()) {
        prop_assert_eq!(Scalar::from(v).as_u16(), Some(v));
    }

    #[cfg_attr(miri, ignore = "full-range proptest budget is prohibitive under Miri; the deterministic edge tests in this file carry MIN/MAX/NaN/±0/subnormals under Miri")]
    #[test]
    fn u32_roundtrip_full_range(v in any::<u32>()) {
        prop_assert_eq!(Scalar::from(v).as_u32(), Some(v));
    }

    #[cfg_attr(miri, ignore = "full-range proptest budget is prohibitive under Miri; the deterministic edge tests in this file carry MIN/MAX/NaN/±0/subnormals under Miri")]
    #[test]
    fn u64_roundtrip_full_range(v in any::<u64>()) {
        prop_assert_eq!(Scalar::from(v).as_u64(), Some(v));
    }

    // The signed ranges INCLUDE negatives (`any::<iN>()` is the full two's-complement
    // range) — the test names say so because the sign-extension rule's failure mode
    // (C1's second RED) is observable only there.

    #[cfg_attr(miri, ignore = "full-range proptest budget is prohibitive under Miri; the deterministic edge tests in this file carry MIN/MAX/NaN/±0/subnormals under Miri")]
    #[test]
    fn i8_roundtrip_full_range_negatives_included(v in any::<i8>()) {
        prop_assert_eq!(Scalar::from(v).as_i8(), Some(v));
    }

    #[cfg_attr(miri, ignore = "full-range proptest budget is prohibitive under Miri; the deterministic edge tests in this file carry MIN/MAX/NaN/±0/subnormals under Miri")]
    #[test]
    fn i16_roundtrip_full_range_negatives_included(v in any::<i16>()) {
        prop_assert_eq!(Scalar::from(v).as_i16(), Some(v));
    }

    #[cfg_attr(miri, ignore = "full-range proptest budget is prohibitive under Miri; the deterministic edge tests in this file carry MIN/MAX/NaN/±0/subnormals under Miri")]
    #[test]
    fn i32_roundtrip_full_range_negatives_included(v in any::<i32>()) {
        prop_assert_eq!(Scalar::from(v).as_i32(), Some(v));
    }

    #[cfg_attr(miri, ignore = "full-range proptest budget is prohibitive under Miri; the deterministic edge tests in this file carry MIN/MAX/NaN/±0/subnormals under Miri")]
    #[test]
    fn i64_roundtrip_full_range_negatives_included(v in any::<i64>()) {
        prop_assert_eq!(Scalar::from(v).as_i64(), Some(v));
    }

    // Floats are driven by RAW BIT PATTERNS (`any::<u32>/<u64>` through `from_bits`),
    // which reaches every NaN payload, both zeros, all subnormals and both infinities —
    // strictly more of the domain than `any::<f32>` — and asserted on bits.

    #[cfg_attr(miri, ignore = "full-range proptest budget is prohibitive under Miri; the deterministic edge tests in this file carry MIN/MAX/NaN/±0/subnormals under Miri")]
    #[test]
    fn f32_roundtrip_all_bit_patterns_bitwise(bits in any::<u32>()) {
        let v = f32::from_bits(bits);
        let got = Scalar::from(v).as_f32().expect("kind matches");
        prop_assert_eq!(got.to_bits(), bits);
    }

    #[cfg_attr(miri, ignore = "full-range proptest budget is prohibitive under Miri; the deterministic edge tests in this file carry MIN/MAX/NaN/±0/subnormals under Miri")]
    #[test]
    fn f64_roundtrip_all_bit_patterns_bitwise(bits in any::<u64>()) {
        let v = f64::from_bits(bits);
        let got = Scalar::from(v).as_f64().expect("kind matches");
        prop_assert_eq!(got.to_bits(), bits);
    }

    #[cfg_attr(miri, ignore = "full-range proptest budget is prohibitive under Miri; the deterministic edge tests in this file carry MIN/MAX/NaN/±0/subnormals under Miri")]
    #[test]
    fn entity_id_roundtrip_full_range(raw in any::<usize>()) {
        prop_assert_eq!(Scalar::from(EntityId(raw)).as_entity_id(), Some(EntityId(raw)));
    }
}

// ─── Gate 3: the 12×12 cross-kind matrix ─────────────────────────────────────────────

/// Declaration order of `ScalarKind`, mirrored by [`extraction_mask`]'s column order.
const ALL: [ScalarKind; 12] = [
    ScalarKind::Bool,
    ScalarKind::U8,
    ScalarKind::U16,
    ScalarKind::U32,
    ScalarKind::U64,
    ScalarKind::I8,
    ScalarKind::I16,
    ScalarKind::I32,
    ScalarKind::I64,
    ScalarKind::F32,
    ScalarKind::F64,
    ScalarKind::EntityId,
];

/// One representative `Scalar` per kind. The match is EXHAUSTIVE on purpose: a new
/// `ScalarKind` fails to compile here until it is classified into the matrix (the same
/// discipline C3 gate 1 applies to `ValueKind`).
fn sample(kind: ScalarKind) -> Scalar {
    match kind {
        ScalarKind::Bool => Scalar::from(true),
        ScalarKind::U8 => Scalar::from(7u8),
        ScalarKind::U16 => Scalar::from(7u16),
        ScalarKind::U32 => Scalar::from(7u32),
        ScalarKind::U64 => Scalar::from(7u64),
        ScalarKind::I8 => Scalar::from(-7i8),
        ScalarKind::I16 => Scalar::from(-7i16),
        ScalarKind::I32 => Scalar::from(-7i32),
        ScalarKind::I64 => Scalar::from(-7i64),
        ScalarKind::F32 => Scalar::from(7.0f32),
        ScalarKind::F64 => Scalar::from(7.0f64),
        ScalarKind::EntityId => Scalar::from(EntityId(7)),
    }
}

/// `is_some()` of all twelve checked extractors, in [`ALL`]'s column order.
fn extraction_mask(s: Scalar) -> [bool; 12] {
    [
        s.as_bool().is_some(),
        s.as_u8().is_some(),
        s.as_u16().is_some(),
        s.as_u32().is_some(),
        s.as_u64().is_some(),
        s.as_i8().is_some(),
        s.as_i16().is_some(),
        s.as_i32().is_some(),
        s.as_i64().is_some(),
        s.as_f32().is_some(),
        s.as_f64().is_some(),
        s.as_entity_id().is_some(),
    ]
}

/// C1 gate 3: for each of the 12 kinds, its own extractor is `Some` and every one of
/// the 11 wrong-kind extractors is `None` — the full off-diagonal-`None` matrix.
#[test]
fn cross_kind_extraction_matrix_12x12_off_diagonal_none() {
    for (i, &kind) in ALL.iter().enumerate() {
        let mask = extraction_mask(sample(kind));
        for (j, &extracted) in mask.iter().enumerate() {
            if i == j {
                assert!(extracted, "diagonal must extract: kind {kind:?}, extractor {:?}", ALL[j]);
            } else {
                assert!(
                    !extracted,
                    "OFF-DIAGONAL EXTRACTION: a {kind:?}-kind Scalar answered the {:?} \
                     extractor with Some — the kind guard is not doing its job",
                    ALL[j]
                );
            }
        }
    }
}
