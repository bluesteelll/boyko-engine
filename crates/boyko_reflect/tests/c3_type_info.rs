//! CORE C3 — `TypeInfo` / `FieldInfo`, **hand-baked**.
//!
//! *"The model must be shown expressible BEFORE a macro is asked to emit it."* Every
//! `TYPE_INFO` below is written by hand, covering **every** [`ValueKind`] arm, and the
//! four gates walk those statics:
//!
//! 1. kind/accessor coherence, asserted by [`validate`] over an exhaustive per-kind
//!    match (a new arm fails to compile until it is classified);
//! 2. [`TypeInfo::get_field`] returns `None` for every non-`Prim` kind — no silent
//!    garbage `Scalar` (CORE D10 / analysis FIX Mi2);
//! 3. `(info.type_id_fn)() == TypeId::of::<T>()` for each fixture type;
//! 4. every `offset` agrees with `core::mem::offset_of!`.
//!
//! **Gate 4 is trivially green here and that is deliberate** — the statics *are*
//! `offset_of!`. It exists so C7 inherits a comparison target that is already
//! independently pinned, and so C3's second RED mutation (a `Prim` field's `offset`
//! hand-edited to `0`) has something to red against.
//!
//! # Why the accessors below are hand-written and stay that way
//!
//! `boyko_reflect::prim::` — the monomorphic accessor library — is **CORE C4**, the
//! next rung. At C3 it does not exist, so the `Prim` fields' `get`/`set` slots are
//! filled by the four local `unsafe fn`s in [`hand`]. That is not a workaround: it is
//! what "hand-baked" means at this rung, and it is left in place after C4 lands as the
//! standing witness that the model is expressible *without* the library.

use std::any::TypeId;
use std::mem::offset_of;

use boyko_reflect::scalar::ScalarKind;
use boyko_reflect::type_info::{
    ArrayInfo, EnumInfo, EnumRepr, FieldInfo, TypeInfo, TypeKind, ValueKind, VariantInfo, Violation,
    validate,
};

// ───────────────────────────── the fixture types ────────────────────────────

/// The `Nested` arm's inner type — itself fully reflectable, which is the whole
/// point: descent is a pointer to *this* type's own static (§3.1), never a
/// flattened path table.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
struct Inner {
    x: f32,
    y: f32,
}

/// The `Enum` arm's type: fieldless, `#[repr(u8)]`, discriminants pinned — the
/// shape `Visibility` already has in `boyko_scene`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Facing {
    North = 0,
    East = 1,
    South = 2,
    West = 3,
}

/// The `Opaque` arm's type: something the v1 model cannot describe, reachable only
/// behind `#[reflect(skip)]` (CORE D14/D15).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OpaqueBlob {
    _handle: *const u8,
}

/// The subject: one `#[repr(C)]` struct carrying **every** `ValueKind` arm, in
/// declaration order — `Prim`, `Prim`, `Array`, `Nested`, `Enum`, `Str`, `Opaque`.
#[repr(C)]
struct Everything {
    hp: f32,
    level: u32,
    corners: [f32; 4],
    inner: Inner,
    facing: Facing,
    label: String,
    blob: OpaqueBlob,
}

// ──────────────────────── the hand-written accessors ────────────────────────

/// The four accessors C3 bakes by hand (see the module header: `prim::` is C4).
mod hand {
    use boyko_reflect::scalar::Scalar;

    /// Reads an `f32` field.
    ///
    /// # Safety
    ///
    /// `p` must be `base.add(offset)` for a live, initialized, `align_of::<f32>()`-
    /// aligned `f32` field of the struct this accessor was baked for.
    pub unsafe fn get_f32(p: *const u8) -> Scalar {
        // SAFETY: the caller guarantees `p` addresses a live, initialized, aligned
        // `f32` with provenance inherited from the containing value's base; the
        // shared reborrow is read-only and does not outlive this statement (the
        // `Bindable` trampoline's precedented pattern, CORE F11).
        let this: &f32 = unsafe { &*(p as *const f32) };
        Scalar::from(*this)
    }

    /// Writes an `f32` field, refusing a kind mismatch **before** touching memory.
    ///
    /// # Safety
    ///
    /// As [`get_f32`], with write permission and no live reference outstanding.
    pub unsafe fn set_f32(p: *mut u8, v: Scalar) -> bool {
        let Some(x) = v.as_f32() else {
            return false;
        };
        // SAFETY: the kind check above proved `v` carries an `f32`; the caller
        // guarantees `p` is a writable, aligned, in-bounds `f32` field. The write is
        // raw — no intermediate `&mut f32` is ever created (analysis B.7's asymmetry).
        unsafe { std::ptr::write(p as *mut f32, x) };
        true
    }

    /// Reads a `u32` field.
    ///
    /// # Safety
    ///
    /// As [`get_f32`], for `u32`.
    pub unsafe fn get_u32(p: *const u8) -> Scalar {
        // SAFETY: as `get_f32`, for `u32`.
        let this: &u32 = unsafe { &*(p as *const u32) };
        Scalar::from(*this)
    }

    /// Writes a `u32` field, refusing a kind mismatch **before** touching memory.
    ///
    /// # Safety
    ///
    /// As [`set_f32`], for `u32`.
    pub unsafe fn set_u32(p: *mut u8, v: Scalar) -> bool {
        let Some(x) = v.as_u32() else {
            return false;
        };
        // SAFETY: as `set_f32`, for `u32`.
        unsafe { std::ptr::write(p as *mut u32, x) };
        true
    }
}

// ─────────────────────────── the hand-baked statics ─────────────────────────

fn inner_type_id() -> TypeId {
    TypeId::of::<Inner>()
}
fn everything_type_id() -> TypeId {
    TypeId::of::<Everything>()
}
fn f32_type_id() -> TypeId {
    TypeId::of::<f32>()
}
fn u32_type_id() -> TypeId {
    TypeId::of::<u32>()
}
fn corners_type_id() -> TypeId {
    TypeId::of::<[f32; 4]>()
}
fn facing_type_id() -> TypeId {
    TypeId::of::<Facing>()
}
fn string_type_id() -> TypeId {
    TypeId::of::<String>()
}
fn blob_type_id() -> TypeId {
    TypeId::of::<OpaqueBlob>()
}

/// Runs `Everything`'s drop glue — the `String` field owes one.
///
/// # Safety
///
/// `p` must hold a live, initialized `Everything` the caller owns and will not read
/// again.
unsafe fn drop_everything(p: *mut u8) {
    // SAFETY: the caller guarantees `p` holds a live, owned, initialized
    // `Everything`; `drop_in_place` consumes it exactly once.
    unsafe { std::ptr::drop_in_place(p as *mut Everything) };
}

static INNER_FIELDS: [FieldInfo; 2] = [
    FieldInfo {
        name: "x",
        offset: offset_of!(Inner, x),
        type_id_fn: f32_type_id,
        kind: ValueKind::Prim(ScalarKind::F32),
        get: Some(hand::get_f32),
        set: Some(hand::set_f32),
        nested: None,
        enum_info: None,
        array: None,
    },
    FieldInfo {
        name: "y",
        offset: offset_of!(Inner, y),
        type_id_fn: f32_type_id,
        kind: ValueKind::Prim(ScalarKind::F32),
        get: Some(hand::get_f32),
        set: Some(hand::set_f32),
        nested: None,
        enum_info: None,
        array: None,
    },
];

static INNER_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c3_type_info::Inner",
    type_id_fn: inner_type_id,
    size: size_of::<Inner>(),
    align: align_of::<Inner>(),
    fields: &INNER_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

static FACING_VARIANTS: [VariantInfo; 4] = [
    VariantInfo { name: "North", discr_bits: 0 },
    VariantInfo { name: "East", discr_bits: 1 },
    VariantInfo { name: "South", discr_bits: 2 },
    VariantInfo { name: "West", discr_bits: 3 },
];

static FACING_ENUM_INFO: EnumInfo =
    EnumInfo { repr: EnumRepr::U8, variants: &FACING_VARIANTS };

static EVERYTHING_FIELDS: [FieldInfo; 7] = [
    // Prim — the `get`/`set` slots are the live ones.
    FieldInfo {
        name: "hp",
        offset: offset_of!(Everything, hp),
        type_id_fn: f32_type_id,
        kind: ValueKind::Prim(ScalarKind::F32),
        get: Some(hand::get_f32),
        set: Some(hand::set_f32),
        nested: None,
        enum_info: None,
        array: None,
    },
    FieldInfo {
        name: "level",
        offset: offset_of!(Everything, level),
        type_id_fn: u32_type_id,
        kind: ValueKind::Prim(ScalarKind::U32),
        get: Some(hand::get_u32),
        set: Some(hand::set_u32),
        nested: None,
        enum_info: None,
        array: None,
    },
    // Array — `array` is the live slot; elements are reached by index at C5, never
    // through `get`.
    FieldInfo {
        name: "corners",
        offset: offset_of!(Everything, corners),
        type_id_fn: corners_type_id,
        kind: ValueKind::Array,
        get: None,
        set: None,
        nested: None,
        enum_info: None,
        array: Some(ArrayInfo { elem: ScalarKind::F32, stride: size_of::<f32>(), len: 4 }),
    },
    // Nested — a POINTER to the inner type's own static (§3.1: derive-time recursion
    // is depth 1, so there is no proc-macro recursion and no expansion blow-up).
    FieldInfo {
        name: "inner",
        offset: offset_of!(Everything, inner),
        type_id_fn: inner_type_id,
        kind: ValueKind::Nested,
        get: None,
        set: None,
        nested: Some(&INNER_TYPE_INFO),
        enum_info: None,
        array: None,
    },
    // Enum — the variant table is the live slot (CORE C10 builds the accessors).
    FieldInfo {
        name: "facing",
        offset: offset_of!(Everything, facing),
        type_id_fn: facing_type_id,
        kind: ValueKind::Enum,
        get: None,
        set: None,
        nested: None,
        enum_info: Some(&FACING_ENUM_INFO),
        array: None,
    },
    // Str — built LAST (CORE D13); until C11 lands its accessor pair a `Str` field is
    // structurally accessorless, and `validate` says so.
    FieldInfo {
        name: "label",
        offset: offset_of!(Everything, label),
        type_id_fn: string_type_id,
        kind: ValueKind::Str,
        get: None,
        set: None,
        nested: None,
        enum_info: None,
        array: None,
    },
    // Opaque — present in the list, not omitted (CORE D14: a shorter list would make
    // by-index access depend on which fields were skipped), and with no accessor at
    // all, which is what makes "no `Opaque` path exists in v1" true rather than
    // asserted.
    FieldInfo {
        name: "blob",
        offset: offset_of!(Everything, blob),
        type_id_fn: blob_type_id,
        kind: ValueKind::Opaque,
        get: None,
        set: None,
        nested: None,
        enum_info: None,
        array: None,
    },
];

static EVERYTHING_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c3_type_info::Everything",
    type_id_fn: everything_type_id,
    size: size_of::<Everything>(),
    align: align_of::<Everything>(),
    fields: &EVERYTHING_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: Some(drop_everything),
};

/// The top-level-enum shape (§3, CORE C10): `fields` is empty and the whole value is
/// reached through `TypeKind::Enum` + `enum_info`. Present at C3 because
/// `TypeInfo.enum_info` is `Some` **iff** `kind == Enum`, and a one-sided fixture
/// cannot exercise an "iff".
static FACING_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c3_type_info::Facing",
    type_id_fn: facing_type_id,
    size: size_of::<Facing>(),
    align: align_of::<Facing>(),
    fields: &[],
    kind: TypeKind::Enum,
    enum_info: Some(&FACING_ENUM_INFO),
    default_in_place: None,
    drop_in_place: None,
};

fn sample() -> Everything {
    Everything {
        hp: 12.5,
        level: 7,
        corners: [1.0, 2.0, 3.0, 4.0],
        inner: Inner { x: -1.5, y: 2.25 },
        facing: Facing::South,
        label: String::from("c3"),
        blob: OpaqueBlob { _handle: std::ptr::null() },
    }
}

// ───────────────────────────────── gate 1 ───────────────────────────────────

/// CORE C3 gate 1 — kind/accessor coherence over every hand-baked descriptor.
#[test]
fn every_fixture_descriptor_is_coherent() {
    for info in [&EVERYTHING_TYPE_INFO, &INNER_TYPE_INFO, &FACING_TYPE_INFO] {
        if let Err(problems) = validate(info) {
            let lines: Vec<String> = problems.iter().map(ToString::to_string).collect();
            panic!("`{}` is INCOHERENT:\n  {}", info.type_name, lines.join("\n  "));
        }
    }
}

/// Non-vacuity for gate 1: the fixture covers **every** `ValueKind` arm, so a green
/// `validate` is a statement about all six rules and not about the two arms a smaller
/// fixture happened to contain. The match is exhaustive, so a new arm reds here too.
#[test]
fn the_fixture_covers_every_value_kind_arm() {
    let mut prim = false;
    let mut array = false;
    let mut nested = false;
    let mut enum_ = false;
    let mut str_ = false;
    let mut opaque = false;
    for field in EVERYTHING_TYPE_INFO.fields {
        match field.kind {
            ValueKind::Prim(_) => prim = true,
            ValueKind::Array => array = true,
            ValueKind::Nested => nested = true,
            ValueKind::Enum => enum_ = true,
            ValueKind::Str => str_ = true,
            ValueKind::Opaque => opaque = true,
        }
    }
    assert!(
        prim && array && nested && enum_ && str_ && opaque,
        "the C3 fixture must carry EVERY ValueKind arm -- covered: \
         Prim={prim} Array={array} Nested={nested} Enum={enum_} Str={str_} Opaque={opaque}"
    );
}

/// Gate 1's own red, shown here rather than only in the rung's RED MUTATION: an
/// incoherent descriptor produces a NAMED `Problem`, and `validate` reports **every**
/// violation rather than the first.
#[test]
fn validate_names_the_rule_a_broken_descriptor_violates() {
    static BROKEN_FIELDS: [FieldInfo; 2] = [
        // A `Nested` field carrying a scalar getter -- C3's first RED mutation, in
        // permanent form so the rule keeps a reader even when no mutation is running.
        FieldInfo {
            name: "inner",
            offset: 0,
            type_id_fn: inner_type_id,
            kind: ValueKind::Nested,
            get: Some(hand::get_f32),
            set: None,
            nested: Some(&INNER_TYPE_INFO),
            enum_info: None,
            array: None,
        },
        // A `Prim` with no accessors at all: two violations from one field.
        FieldInfo {
            name: "hp",
            offset: 8,
            type_id_fn: f32_type_id,
            kind: ValueKind::Prim(ScalarKind::F32),
            get: None,
            set: None,
            nested: None,
            enum_info: None,
            array: None,
        },
    ];
    static BROKEN: TypeInfo = TypeInfo {
        type_name: "c3_type_info::Broken",
        type_id_fn: everything_type_id,
        size: 0,
        align: 1,
        fields: &BROKEN_FIELDS,
        kind: TypeKind::Struct,
        enum_info: None,
        default_in_place: None,
        drop_in_place: None,
    };

    let problems = validate(&BROKEN).expect_err("a Nested field with a getter is not coherent");
    assert!(
        problems.iter().any(|p| p.violation == Violation::NestedWithScalarAccessor
            && p.name == "inner"),
        "expected a named NestedWithScalarAccessor problem, got: {problems:?}"
    );
    assert!(
        problems.iter().any(|p| p.violation == Violation::PrimWithoutGet),
        "expected PrimWithoutGet, got: {problems:?}"
    );
    assert!(
        problems.iter().any(|p| p.violation == Violation::PrimWithoutSet),
        "validate must report EVERY violation, not the first: {problems:?}"
    );
}

/// The type-level "`Some` **iff** `kind == Enum`" rule, both directions.
#[test]
fn type_level_enum_info_is_checked_in_both_directions() {
    static ENUM_WITHOUT_TABLE: TypeInfo = TypeInfo {
        type_name: "c3_type_info::EnumWithoutTable",
        type_id_fn: facing_type_id,
        size: 1,
        align: 1,
        fields: &[],
        kind: TypeKind::Enum,
        enum_info: None,
        default_in_place: None,
        drop_in_place: None,
    };
    static STRUCT_WITH_TABLE: TypeInfo = TypeInfo {
        type_name: "c3_type_info::StructWithTable",
        type_id_fn: everything_type_id,
        size: 1,
        align: 1,
        fields: &[],
        kind: TypeKind::Struct,
        enum_info: Some(&FACING_ENUM_INFO),
        default_in_place: None,
        drop_in_place: None,
    };

    let a = validate(&ENUM_WITHOUT_TABLE).expect_err("an enum with no variant table");
    assert!(a.iter().any(|p| p.violation == Violation::TypeIsEnumWithoutEnumInfo), "{a:?}");
    let b = validate(&STRUCT_WITH_TABLE).expect_err("a struct carrying a variant table");
    assert!(b.iter().any(|p| p.violation == Violation::TypeEnumInfoOnNonEnum), "{b:?}");
}

// ───────────────────────────────── gate 2 ───────────────────────────────────

/// CORE C3 gate 2 — the scalar read API answers `None` for **every** non-`Prim` kind.
/// No silent garbage `Scalar` (CORE D10 / analysis FIX Mi2): the alternative would
/// reinterpret a nested struct's, an array's or a `String`'s first bytes as whatever
/// scalar the caller asked for.
#[test]
fn get_field_returns_none_for_every_non_prim_kind() {
    let value = sample();
    let base = (&raw const value).cast::<u8>();

    for (index, field) in EVERYTHING_TYPE_INFO.fields.iter().enumerate() {
        // SAFETY: `base` is a live, initialized, aligned `Everything` owned by this
        // frame and not concurrently written; `EVERYTHING_TYPE_INFO` describes exactly
        // that type, so every baked offset is in bounds with inherited provenance.
        let got = unsafe { EVERYTHING_TYPE_INFO.get_field(base, index) };
        match field.kind {
            ValueKind::Prim(_) => assert!(
                got.is_some(),
                "field #{index} `{}` is Prim and must read as a Scalar",
                field.name
            ),
            _ => assert!(
                got.is_none(),
                "field #{index} `{}` is {:?} and MUST NOT produce a Scalar -- a value \
                 here is the silent-garbage defect (FIX Mi2)",
                field.name,
                field.kind
            ),
        }
    }
}

/// Gate 2's independence from gate 1, stated as a test: `get_field` checks the KIND,
/// not merely the accessor slot. A malformed descriptor — a `Nested` field carrying a
/// getter, which is exactly what `validate` rejects — must still refuse to read, or
/// the release-editor build reinterprets `Inner`'s first four bytes as an `f32`.
#[test]
fn get_field_refuses_a_malformed_nested_descriptor_even_though_it_has_a_getter() {
    static MALFORMED_FIELDS: [FieldInfo; 1] = [FieldInfo {
        name: "inner",
        offset: offset_of!(Everything, inner),
        type_id_fn: inner_type_id,
        kind: ValueKind::Nested,
        get: Some(hand::get_f32),
        set: Some(hand::set_f32),
        nested: Some(&INNER_TYPE_INFO),
        enum_info: None,
        array: None,
    }];
    static MALFORMED: TypeInfo = TypeInfo {
        type_name: "c3_type_info::Malformed",
        type_id_fn: everything_type_id,
        size: size_of::<Everything>(),
        align: align_of::<Everything>(),
        fields: &MALFORMED_FIELDS,
        kind: TypeKind::Struct,
        enum_info: None,
        default_in_place: None,
        drop_in_place: None,
    };

    let value = sample();
    let base = (&raw const value).cast::<u8>();
    // SAFETY: as `get_field_returns_none_for_every_non_prim_kind` -- `MALFORMED`
    // describes the same `Everything` layout, and the one field's offset is that
    // type's own `offset_of!`.
    let got = unsafe { MALFORMED.get_field(base, 0) };
    assert!(
        got.is_none(),
        "get_field must key on the KIND, not on whether a getter happens to be \
         installed -- otherwise a malformed descriptor reads Inner.x as the whole field"
    );
}

/// An out-of-range index is `None`, never a panic and never an out-of-bounds read —
/// the same refusal discipline the registry's release bounds guard uses (CORE D11's
/// reasoning: a stale `(ComponentId, field)` triple from a hot-reloaded editor).
#[test]
fn get_field_out_of_range_index_is_none() {
    let value = sample();
    let base = (&raw const value).cast::<u8>();
    // SAFETY: as above; the index is out of range, which `get_field` checks first.
    assert!(unsafe { EVERYTHING_TYPE_INFO.get_field(base, EVERYTHING_TYPE_INFO.fields.len()) }
        .is_none());
    // SAFETY: as above.
    assert!(unsafe { EVERYTHING_TYPE_INFO.get_field(base, usize::MAX) }.is_none());
}

/// The positive half: a `Prim` field really does read back its value, so gate 2's
/// `None`s are a refusal rather than a broken read path.
#[test]
fn get_field_reads_prim_fields_back() {
    let value = sample();
    let base = (&raw const value).cast::<u8>();
    // SAFETY: as `get_field_returns_none_for_every_non_prim_kind`.
    let hp = unsafe { EVERYTHING_TYPE_INFO.get_field(base, 0) }.expect("hp is Prim(F32)");
    // SAFETY: as above.
    let level = unsafe { EVERYTHING_TYPE_INFO.get_field(base, 1) }.expect("level is Prim(U32)");
    assert_eq!(hp.as_f32(), Some(12.5_f32));
    assert_eq!(level.as_u32(), Some(7_u32));
    assert_eq!(hp.as_u32(), None, "a kind mismatch is None, never a reinterpretation");
}

// ───────────────────────────────── gate 3 ───────────────────────────────────

/// CORE C3 gate 3 — `(type_id_fn)()` is the fixture type's real `TypeId`, on the type
/// and on every field. `TypeId::of` is not `const`, which is why the slot is an fn
/// pointer at all; this gate is what keeps that indirection honest.
#[test]
fn every_baked_type_id_fn_returns_the_real_type_id() {
    assert_eq!((EVERYTHING_TYPE_INFO.type_id_fn)(), TypeId::of::<Everything>());
    assert_eq!((INNER_TYPE_INFO.type_id_fn)(), TypeId::of::<Inner>());
    assert_eq!((FACING_TYPE_INFO.type_id_fn)(), TypeId::of::<Facing>());

    let expected: [TypeId; 7] = [
        TypeId::of::<f32>(),
        TypeId::of::<u32>(),
        TypeId::of::<[f32; 4]>(),
        TypeId::of::<Inner>(),
        TypeId::of::<Facing>(),
        TypeId::of::<String>(),
        TypeId::of::<OpaqueBlob>(),
    ];
    for (index, field) in EVERYTHING_TYPE_INFO.fields.iter().enumerate() {
        assert_eq!(
            (field.type_id_fn)(),
            expected[index],
            "field #{index} `{}` reports the wrong TypeId",
            field.name
        );
    }
    assert_eq!((INNER_TYPE_INFO.fields[0].type_id_fn)(), TypeId::of::<f32>());
    assert_eq!((INNER_TYPE_INFO.fields[1].type_id_fn)(), TypeId::of::<f32>());
}

// ───────────────────────────────── gate 4 ───────────────────────────────────

/// CORE C3 gate 4 — every baked `offset` equals `core::mem::offset_of!`.
///
/// Trivially green here *by construction*, and that is the point: it is the
/// independently-pinned comparison target C7's generated offsets are measured
/// against, and it is what C3's second RED mutation (`offset` hand-edited to `0`)
/// reds on.
#[test]
fn every_baked_offset_equals_offset_of() {
    let expected: [(&str, usize); 7] = [
        ("hp", offset_of!(Everything, hp)),
        ("level", offset_of!(Everything, level)),
        ("corners", offset_of!(Everything, corners)),
        ("inner", offset_of!(Everything, inner)),
        ("facing", offset_of!(Everything, facing)),
        ("label", offset_of!(Everything, label)),
        ("blob", offset_of!(Everything, blob)),
    ];
    for (index, field) in EVERYTHING_TYPE_INFO.fields.iter().enumerate() {
        let (name, offset) = expected[index];
        assert_eq!(field.name, name, "field #{index} is out of declaration order");
        assert_eq!(
            field.offset, offset,
            "field #{index} `{name}`: baked offset {} != offset_of! {offset}",
            field.offset
        );
    }
    assert_eq!(INNER_TYPE_INFO.fields[0].offset, offset_of!(Inner, x));
    assert_eq!(INNER_TYPE_INFO.fields[1].offset, offset_of!(Inner, y));
}

/// The layout half of gate 4: `size`/`align` are the real ones. A descriptor whose
/// `size` disagrees with the type is what turns a bounds argument into a guess.
#[test]
fn baked_size_and_align_match_the_types() {
    assert_eq!(EVERYTHING_TYPE_INFO.size, size_of::<Everything>());
    assert_eq!(EVERYTHING_TYPE_INFO.align, align_of::<Everything>());
    assert_eq!(INNER_TYPE_INFO.size, size_of::<Inner>());
    assert_eq!(INNER_TYPE_INFO.align, align_of::<Inner>());
    assert_eq!(FACING_TYPE_INFO.size, size_of::<Facing>());
    assert_eq!(FACING_TYPE_INFO.align, align_of::<Facing>());
}

/// The `EnumInfo` descriptor's discriminants are the type's REAL ones, **already
/// narrowed to the repr width** (analysis FIX C2/O1 — never a lossy `i128 as u64` at
/// the call site). C10 builds the accessors; this pins the table they will read.
#[test]
fn the_variant_table_matches_the_real_discriminants() {
    let real: [(&str, Facing); 4] = [
        ("North", Facing::North),
        ("East", Facing::East),
        ("South", Facing::South),
        ("West", Facing::West),
    ];
    assert_eq!(FACING_ENUM_INFO.repr, EnumRepr::U8);
    assert_eq!(FACING_ENUM_INFO.variants.len(), real.len());
    for (index, (name, variant)) in real.into_iter().enumerate() {
        let baked = FACING_ENUM_INFO.variants[index];
        assert_eq!(baked.name, name, "variant #{index} is out of declaration order");
        assert_eq!(
            baked.discr_bits,
            u64::from(variant as u8),
            "variant `{name}`'s baked discriminant is not the type's own"
        );
    }
}

/// The `Array` descriptor's stride is `size_of::<T>()` — pinned here so C5 inherits a
/// target rather than re-deriving one (C5 gate 4 owns the padded-element case).
#[test]
fn the_array_descriptor_matches_the_real_element_layout() {
    let array = EVERYTHING_TYPE_INFO.fields[2].array.expect("the corners field is an Array");
    assert_eq!(array.elem, ScalarKind::F32);
    assert_eq!(array.stride, size_of::<f32>());
    assert_eq!(array.len, 4);
    assert_eq!(array.stride * array.len, size_of::<[f32; 4]>());
}
