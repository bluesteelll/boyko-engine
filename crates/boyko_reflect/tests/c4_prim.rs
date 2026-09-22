//! CORE C4 gates 1–3 — the `prim::` accessor library and the **release** kind check.
//!
//! 1. per-kind get/set round-trip through a `#[repr(C)]` fixture, read back **through
//!    the typed field**;
//! 2. a mismatched-kind `set` returns `false` **and leaves the bytes UNCHANGED**,
//!    asserted by byte comparison of the **whole struct** — not by reading the one
//!    field, because the defect this gate exists for writes the right refusal and the
//!    wrong bytes;
//! 3. gate 2 runs in a **release-profile** test, because that is the build where
//!    `debug_assert!` has vanished and the stale-editor-triple case lives (CORE D11).
//!
//! Run BOTH profiles — they are different gates, not a repetition:
//!
//! ```text
//! cargo test -p boyko-reflect --test c4_prim
//! cargo test -p boyko-reflect --release --test c4_prim
//! ```
//!
//! # Why the fixture carries an explicit `_pad`
//!
//! Gate 2 compares the struct's **bytes**, and reading a padding byte is reading
//! uninitialized memory — UB, and Miri (gate 4) says so. So `AllPrims` is ordered
//! by descending alignment and closed with an explicit `[u8; 5]` filler, and
//! [`the_fixture_has_no_padding_bytes`] proves the absence rather than assuming it:
//! under `#[repr(C)]`, `size_of == sum of field sizes` leaves no room for padding
//! anywhere, interior or trailing.

use boyko_ecs::ecs::identifiers::primitives::EntityId;

use boyko_reflect::prim;
use boyko_reflect::scalar::{Scalar, ScalarKind};

// ───────────────────────────────── the fixture ──────────────────────────────

/// One field per [`ScalarKind`], ordered by descending alignment and closed with an
/// explicit filler so the struct has **no padding bytes at all**.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
struct AllPrims {
    e: EntityId,
    u64_: u64,
    i64_: i64,
    f64_: f64,
    u32_: u32,
    i32_: i32,
    f32_: f32,
    u16_: u16,
    i16_: i16,
    u8_: u8,
    i8_: i8,
    b: bool,
    /// Explicit trailing filler — see the module header: an implicit tail pad would
    /// make gate 2's byte comparison an uninitialized read.
    _pad: [u8; 5],
}

const SIZE: usize = size_of::<AllPrims>();

fn sample_struct() -> AllPrims {
    AllPrims {
        e: EntityId(4242),
        u64_: 0x0123_4567_89AB_CDEF,
        i64_: -5_000_000_000,
        f64_: -2.5,
        u32_: 0xDEAD_BEEF,
        i32_: -70_000,
        f32_: 12.5,
        u16_: 0xBEEF,
        i16_: -300,
        u8_: 0xAB,
        i8_: -7,
        b: true,
        _pad: [0xCC; 5],
    }
}

/// The whole struct's bytes.
///
/// # Safety contract, discharged at the call site
///
/// Sound only because `AllPrims` has **no padding** — proven by
/// [`the_fixture_has_no_padding_bytes`], which runs in the same binary.
fn bytes(v: &AllPrims) -> [u8; SIZE] {
    // SAFETY: `AllPrims` has no padding (asserted by
    // `the_fixture_has_no_padding_bytes`: under `#[repr(C)]`, `size_of` equal to the
    // sum of the field sizes leaves no padding anywhere), so all `SIZE` bytes of a
    // live `AllPrims` are initialized; `[u8; SIZE]` has align 1 and the same size, so
    // the read is in bounds, aligned and fully initialized.
    unsafe { *(std::ptr::from_ref(v).cast::<[u8; SIZE]>()) }
}

// ─────────────────────────── the per-kind case table ────────────────────────

/// One row of the 12×12 matrix: the field this kind lives in, its accessor pair, and
/// a valid [`Scalar`] of that kind.
struct Case {
    kind: ScalarKind,
    field: &'static str,
    offset: usize,
    get: unsafe fn(*const u8) -> Scalar,
    set: unsafe fn(*mut u8, Scalar) -> bool,
    sample: Scalar,
}

fn cases() -> [Case; 12] {
    use std::mem::offset_of;
    [
        Case {
            kind: ScalarKind::Bool,
            field: "b",
            offset: offset_of!(AllPrims, b),
            get: prim::get_bool,
            set: prim::set_bool,
            sample: Scalar::from(true),
        },
        Case {
            kind: ScalarKind::U8,
            field: "u8_",
            offset: offset_of!(AllPrims, u8_),
            get: prim::get_u8,
            set: prim::set_u8,
            sample: Scalar::from(0x5Au8),
        },
        Case {
            kind: ScalarKind::U16,
            field: "u16_",
            offset: offset_of!(AllPrims, u16_),
            get: prim::get_u16,
            set: prim::set_u16,
            sample: Scalar::from(0xFACEu16),
        },
        Case {
            kind: ScalarKind::U32,
            field: "u32_",
            offset: offset_of!(AllPrims, u32_),
            get: prim::get_u32,
            set: prim::set_u32,
            sample: Scalar::from(0x1234_5678u32),
        },
        Case {
            kind: ScalarKind::U64,
            field: "u64_",
            offset: offset_of!(AllPrims, u64_),
            get: prim::get_u64,
            set: prim::set_u64,
            sample: Scalar::from(u64::MAX),
        },
        Case {
            kind: ScalarKind::I8,
            field: "i8_",
            offset: offset_of!(AllPrims, i8_),
            get: prim::get_i8,
            set: prim::set_i8,
            sample: Scalar::from(i8::MIN),
        },
        Case {
            kind: ScalarKind::I16,
            field: "i16_",
            offset: offset_of!(AllPrims, i16_),
            get: prim::get_i16,
            set: prim::set_i16,
            sample: Scalar::from(-31_000i16),
        },
        Case {
            kind: ScalarKind::I32,
            field: "i32_",
            offset: offset_of!(AllPrims, i32_),
            get: prim::get_i32,
            set: prim::set_i32,
            sample: Scalar::from(i32::MIN),
        },
        Case {
            kind: ScalarKind::I64,
            field: "i64_",
            offset: offset_of!(AllPrims, i64_),
            get: prim::get_i64,
            set: prim::set_i64,
            sample: Scalar::from(-1i64),
        },
        Case {
            kind: ScalarKind::F32,
            field: "f32_",
            offset: offset_of!(AllPrims, f32_),
            get: prim::get_f32,
            set: prim::set_f32,
            sample: Scalar::from(-0.0f32),
        },
        Case {
            kind: ScalarKind::F64,
            field: "f64_",
            offset: offset_of!(AllPrims, f64_),
            get: prim::get_f64,
            set: prim::set_f64,
            sample: Scalar::from(f64::MIN_POSITIVE / 2.0),
        },
        Case {
            kind: ScalarKind::EntityId,
            field: "e",
            offset: offset_of!(AllPrims, e),
            get: prim::get_entity_id,
            set: prim::set_entity_id,
            sample: Scalar::from(EntityId(9_999)),
        },
    ]
}

// ─────────────────────── the fixture's own precondition ─────────────────────

/// The instrument's precondition, asserted before anything relies on it: `AllPrims`
/// has **no padding**, so [`bytes`] reads only initialized memory.
#[test]
fn the_fixture_has_no_padding_bytes() {
    let sum = size_of::<EntityId>()
        + size_of::<u64>()
        + size_of::<i64>()
        + size_of::<f64>()
        + size_of::<u32>()
        + size_of::<i32>()
        + size_of::<f32>()
        + size_of::<u16>()
        + size_of::<i16>()
        + size_of::<u8>()
        + size_of::<i8>()
        + size_of::<bool>()
        + size_of::<[u8; 5]>();
    println!("size_of::<AllPrims>() = {SIZE}, sum of field sizes = {sum}");
    assert_eq!(
        SIZE, sum,
        "the C4 fixture has {} padding byte(s) -- gate 2's whole-struct byte comparison \
         would be reading UNINITIALIZED memory, which is UB and which Miri (gate 4) reds \
         on. Re-order by descending alignment and widen `_pad`.",
        SIZE - sum
    );
}

/// The case table covers every [`ScalarKind`], with no kind named twice — the
/// non-vacuity clause for both gates below (a 12×12 matrix over 10 kinds is a 10×10
/// matrix that says "12").
#[test]
fn the_case_table_covers_every_scalar_kind_exactly_once() {
    let cases = cases();
    for case in &cases {
        // Exhaustive: a new `ScalarKind` fails to compile here until it is given a row.
        let covered = match case.kind {
            ScalarKind::Bool
            | ScalarKind::U8
            | ScalarKind::U16
            | ScalarKind::U32
            | ScalarKind::U64
            | ScalarKind::I8
            | ScalarKind::I16
            | ScalarKind::I32
            | ScalarKind::I64
            | ScalarKind::F32
            | ScalarKind::F64
            | ScalarKind::EntityId => true,
        };
        assert!(covered);
    }
    for (i, a) in cases.iter().enumerate() {
        for b in cases.iter().skip(i + 1) {
            assert_ne!(a.kind, b.kind, "kind {:?} appears twice in the case table", a.kind);
        }
    }
    assert_eq!(cases.len(), 12, "the taxonomy has 12 ScalarKinds (§3)");
}

// ───────────────────────────────── gate 1 ───────────────────────────────────

/// CORE C4 gate 1 — per-kind get/set round-trip through the fixture.
#[test]
fn every_kind_round_trips_through_its_accessor_pair() {
    for case in cases() {
        let mut value = sample_struct();
        let base = (&raw mut value).cast::<u8>();
        // SAFETY: `base` is a live, initialized, correctly aligned `AllPrims` this
        // frame owns exclusively; `case.offset` is that type's own `offset_of!`, so
        // `base.add(offset)` is an in-bounds, field-aligned pointer to the field
        // `case.set`/`case.get` were baked for, with inherited provenance.
        let wrote = unsafe { (case.set)(base.add(case.offset), case.sample) };
        assert!(wrote, "{}: a matching-kind set must succeed", case.field);
        // SAFETY: as above, read side.
        let read = unsafe { (case.get)(base.add(case.offset)) };
        assert_eq!(
            read, case.sample,
            "{}: the accessor pair did not round-trip {:?}",
            case.field, case.kind
        );
    }
}

/// Gate 1's *"read back through the typed field"* half, spelled out per field: the
/// round-trip above compares `Scalar`s, and a pair that consistently mis-addressed
/// the same wrong bytes would still agree with itself.
#[test]
fn every_kind_lands_in_its_own_typed_field() {
    let mut value = sample_struct();
    let base = (&raw mut value).cast::<u8>();
    let offsets = cases().map(|c| (c.offset, c.set));

    // SAFETY: every call below writes a matching-kind Scalar through the accessor
    // baked for that field, at that field's own `offset_of!`, into a live `AllPrims`
    // this frame owns exclusively.
    unsafe {
        assert!((offsets[0].1)(base.add(offsets[0].0), Scalar::from(false)));
        assert!((offsets[1].1)(base.add(offsets[1].0), Scalar::from(1u8)));
        assert!((offsets[2].1)(base.add(offsets[2].0), Scalar::from(2u16)));
        assert!((offsets[3].1)(base.add(offsets[3].0), Scalar::from(3u32)));
        assert!((offsets[4].1)(base.add(offsets[4].0), Scalar::from(4u64)));
        assert!((offsets[5].1)(base.add(offsets[5].0), Scalar::from(-5i8)));
        assert!((offsets[6].1)(base.add(offsets[6].0), Scalar::from(-6i16)));
        assert!((offsets[7].1)(base.add(offsets[7].0), Scalar::from(-7i32)));
        assert!((offsets[8].1)(base.add(offsets[8].0), Scalar::from(-8i64)));
        assert!((offsets[9].1)(base.add(offsets[9].0), Scalar::from(9.5f32)));
        assert!((offsets[10].1)(base.add(offsets[10].0), Scalar::from(10.5f64)));
        assert!((offsets[11].1)(base.add(offsets[11].0), Scalar::from(EntityId(11))));
    }

    assert_eq!(
        value,
        AllPrims {
            e: EntityId(11),
            u64_: 4,
            i64_: -8,
            f64_: 10.5,
            u32_: 3,
            i32_: -7,
            f32_: 9.5,
            u16_: 2,
            i16_: -6,
            u8_: 1,
            i8_: -5,
            b: false,
            _pad: [0xCC; 5],
        },
        "a write landed in the wrong field (or spilled into a neighbour)"
    );
}

// ───────────────────────────────── gate 2 ───────────────────────────────────

/// CORE C4 gate 2 — the 12×12 mismatch matrix. Every off-diagonal `set` returns
/// `false` **and leaves every byte of the struct untouched**.
///
/// The byte comparison is the load-bearing half: a setter that checks the kind *after*
/// storing still returns `false`, so a return-value-only gate reads green over a
/// corrupted field. That is C4's second RED mutation, and this is the assertion that
/// sees it.
#[test]
fn a_mismatched_kind_set_refuses_and_writes_nothing() {
    let cases = cases();
    for target in &cases {
        for source in &cases {
            if target.kind == source.kind {
                continue;
            }
            let mut value = sample_struct();
            let before = bytes(&value);
            let base = (&raw mut value).cast::<u8>();
            // SAFETY: as `every_kind_round_trips_through_its_accessor_pair` -- the
            // pointer is this field's own in-bounds, aligned address in a live
            // `AllPrims` this frame owns; the accessor is free to refuse the scalar,
            // which is exactly what is under test.
            let wrote = unsafe { (target.set)(base.add(target.offset), source.sample) };
            assert!(
                !wrote,
                "set_{:?} accepted a {:?} scalar -- the kind check did not refuse",
                target.kind, source.kind
            );
            let after = bytes(&value);
            assert_eq!(
                before, after,
                "set_{:?} REFUSED a {:?} scalar and still changed the bytes -- the check \
                 runs after the store (field `{}`, offset {})",
                target.kind, source.kind, target.field, target.offset
            );
        }
    }
}

/// The same refusal for a **non-canonical payload of the right kind** — a hand-built
/// `Scalar` whose `bits` cannot be the value it claims. `Scalar`'s checked extractor
/// is the kind check, so this costs no second branch, and it is the case a
/// `kind == kind` comparison alone would let through into memory.
#[test]
fn a_non_canonical_payload_is_refused_and_writes_nothing() {
    let non_canonical = [
        (ScalarKind::Bool, Scalar { kind: ScalarKind::Bool, bits: 2 }),
        (ScalarKind::U8, Scalar { kind: ScalarKind::U8, bits: 0x1FF }),
        (ScalarKind::U16, Scalar { kind: ScalarKind::U16, bits: 0x1_0000 }),
        (ScalarKind::U32, Scalar { kind: ScalarKind::U32, bits: 0x1_0000_0000 }),
        (ScalarKind::I8, Scalar { kind: ScalarKind::I8, bits: 200 }),
        (ScalarKind::F32, Scalar { kind: ScalarKind::F32, bits: 0x1_0000_0000 }),
    ];
    for (kind, scalar) in non_canonical {
        let case = cases().into_iter().find(|c| c.kind == kind).expect("kind is in the table");
        let mut value = sample_struct();
        let before = bytes(&value);
        let base = (&raw mut value).cast::<u8>();
        // SAFETY: as above.
        let wrote = unsafe { (case.set)(base.add(case.offset), scalar) };
        assert!(!wrote, "set_{kind:?} accepted a non-canonical payload {scalar:?}");
        assert_eq!(bytes(&value), before, "set_{kind:?} refused and still wrote");
    }
}

// ───────────────────────────────── gate 3 ───────────────────────────────────

/// CORE C4 gate 3's non-vacuity clause: proof that the release leg of this file
/// actually ran. Compiled only when `debug_assert!` is gone, so `running N` differs
/// between the two profiles and a filtered-out release leg cannot pass as one.
///
/// **This is the leg the rung exists for.** The RED mutation (`debug_assert_eq!` in
/// place of the `-> bool` check) panics in debug — a *different* failure, and the kind
/// a maintainer "fixes" with `#[should_panic]` — while in release the assertion
/// vanishes, the setter stores a `u32`'s bits into an `f32` field, and only gate 2's
/// byte comparison sees it.
#[test]
#[cfg(not(debug_assertions))]
fn release_leg_is_live_and_debug_assert_is_gone() {
    let mut fired = false;
    debug_assert!(raise(&mut fired));
    println!("C4 gate 3: release leg live; debug_assert! executed = {fired}");
    assert!(
        !fired,
        "a `debug_assert!` RAN in a release-profile test -- D11's whole premise (the \
         editor build compiles the assertion out, so only a `-> bool` refusal survives) \
         is false on this toolchain, and gate 3 is measuring the wrong build"
    );
}

/// The debug twin: the same probe, asserting the opposite. Together they turn *"the
/// release leg ran"* into a **runtime observation** rather than a `cfg!` restated as an
/// assertion, and each profile's `running N` names a different test.
#[test]
#[cfg(debug_assertions)]
fn debug_leg_is_live_and_debug_assert_still_fires() {
    let mut fired = false;
    debug_assert!(raise(&mut fired));
    println!("C4: debug leg live; debug_assert! executed = {fired}");
    assert!(fired, "`debug_assert!` did not run in a debug-profile test");
}

/// Sets the flag and reports success — the probe body for the two leg tests. A plain
/// `fn` rather than a block so the side effect is visible to the reader and opaque to
/// constant folding.
fn raise(flag: &mut bool) -> bool {
    *flag = true;
    true
}
