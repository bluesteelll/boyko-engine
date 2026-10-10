//! **CORE C6 gate 1, fixture half** — the descend, exercised from a *consumer* package.
//!
//! `reflect_fixture` is the campaign's primary gated subject and its stand-in for a
//! game's own crate: FFI-free, three dependencies, and the package the CI Miri row names
//! (`-p reflect-fixture --features reflect-fixture/reflect`). Descending here rather than
//! only inside `boyko_reflect` is what makes C6's claim a claim about the **API a
//! consumer has**, not about the crate's own internals.
//!
//! Two subjects, both locally declared:
//!
//! * a **depth-2 named-field nest** (`Body → Point → f32`), and
//! * a **depth-2 tuple struct** (`Slot → Handle → u32`), whose field names are the
//!   decimal indices (§3).
//!
//! and a leaf of each is read.
//!
//! # Why the statics are hand-baked here too
//!
//! `#[component(reflect)]` does not exist until **C7**; today the `Component` derive
//! hard-errors on an unknown key. `src/bin/reflect_on.rs` records the same deviation for
//! the same reason. C7 replaces these statics with generated ones and inherits this file
//! as an independently-pinned comparison target — the discipline C3 already used for
//! `offset_of!`.
//!
//! # Why the whole file is feature-gated
//!
//! `boyko-reflect` is an **optional** dependency here (GATES D3 C2/C3): feature off ⇒ the
//! crate is not in this package's resolved graph at all, which is the campaign's central
//! claim. So a plain `cargo test --workspace` compiles this file to nothing, by design,
//! and the gate runs under `--features reflect-fixture/reflect`.
#![cfg(feature = "reflect")]

use std::any::TypeId;
use std::mem::offset_of;

use boyko_reflect::cursor::{FieldValue, NestedCursor};
use boyko_reflect::prim;
use boyko_reflect::scalar::{Scalar, ScalarKind};
use boyko_reflect::type_info::{FieldInfo, TypeInfo, TypeKind, ValueKind, validate};

// ───────────────────────── the named-field nest ─────────────────────────────

/// The leaf.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    /// X.
    pub x: f32,
    /// Y.
    pub y: f32,
}

/// The middle level.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Where.
    pub at: Point,
    /// Which layer.
    pub layer: u32,
}

/// The root.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Body {
    /// Health.
    pub hp: f32,
    /// The nested placement — the edge C6 descends.
    pub placement: Placement,
}

// ────────────────────────── the tuple-struct nest ───────────────────────────

/// The tuple-struct leaf.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Handle(pub u32);

/// The tuple-struct root.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot(pub Handle);

fn point_type_id() -> TypeId {
    TypeId::of::<Point>()
}
fn placement_type_id() -> TypeId {
    TypeId::of::<Placement>()
}
fn body_type_id() -> TypeId {
    TypeId::of::<Body>()
}
fn handle_type_id() -> TypeId {
    TypeId::of::<Handle>()
}
fn slot_type_id() -> TypeId {
    TypeId::of::<Slot>()
}
fn f32_type_id() -> TypeId {
    TypeId::of::<f32>()
}
fn u32_type_id() -> TypeId {
    TypeId::of::<u32>()
}

static POINT_FIELDS: [FieldInfo; 2] = [
    FieldInfo {
        name: "x",
        offset: offset_of!(Point, x),
        type_id_fn: f32_type_id,
        kind: ValueKind::Prim(ScalarKind::F32),
        get: Some(prim::get_f32),
        set: Some(prim::set_f32),
        nested: None,
        enum_info: None,
        array: None,
    },
    FieldInfo {
        name: "y",
        offset: offset_of!(Point, y),
        type_id_fn: f32_type_id,
        kind: ValueKind::Prim(ScalarKind::F32),
        get: Some(prim::get_f32),
        set: Some(prim::set_f32),
        nested: None,
        enum_info: None,
        array: None,
    },
];

static POINT_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c6_nested_descend::Point",
    type_id_fn: point_type_id,
    size: size_of::<Point>(),
    align: align_of::<Point>(),
    fields: &POINT_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

static PLACEMENT_FIELDS: [FieldInfo; 2] = [
    FieldInfo {
        name: "at",
        offset: offset_of!(Placement, at),
        type_id_fn: point_type_id,
        kind: ValueKind::Nested,
        get: None,
        set: None,
        nested: Some(&POINT_TYPE_INFO),
        enum_info: None,
        array: None,
    },
    FieldInfo {
        name: "layer",
        offset: offset_of!(Placement, layer),
        type_id_fn: u32_type_id,
        kind: ValueKind::Prim(ScalarKind::U32),
        get: Some(prim::get_u32),
        set: Some(prim::set_u32),
        nested: None,
        enum_info: None,
        array: None,
    },
];

static PLACEMENT_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c6_nested_descend::Placement",
    type_id_fn: placement_type_id,
    size: size_of::<Placement>(),
    align: align_of::<Placement>(),
    fields: &PLACEMENT_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

static BODY_FIELDS: [FieldInfo; 2] = [
    FieldInfo {
        name: "hp",
        offset: offset_of!(Body, hp),
        type_id_fn: f32_type_id,
        kind: ValueKind::Prim(ScalarKind::F32),
        get: Some(prim::get_f32),
        set: Some(prim::set_f32),
        nested: None,
        enum_info: None,
        array: None,
    },
    FieldInfo {
        name: "placement",
        offset: offset_of!(Body, placement),
        type_id_fn: placement_type_id,
        kind: ValueKind::Nested,
        get: None,
        set: None,
        nested: Some(&PLACEMENT_TYPE_INFO),
        enum_info: None,
        array: None,
    },
];

static BODY_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c6_nested_descend::Body",
    type_id_fn: body_type_id,
    size: size_of::<Body>(),
    align: align_of::<Body>(),
    fields: &BODY_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

static HANDLE_FIELDS: [FieldInfo; 1] = [FieldInfo {
    name: "0",
    offset: 0,
    type_id_fn: u32_type_id,
    kind: ValueKind::Prim(ScalarKind::U32),
    get: Some(prim::get_u32),
    set: Some(prim::set_u32),
    nested: None,
    enum_info: None,
    array: None,
}];

static HANDLE_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c6_nested_descend::Handle",
    type_id_fn: handle_type_id,
    size: size_of::<Handle>(),
    align: align_of::<Handle>(),
    fields: &HANDLE_FIELDS,
    kind: TypeKind::TupleStruct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

static SLOT_FIELDS: [FieldInfo; 1] = [FieldInfo {
    name: "0",
    offset: 0,
    type_id_fn: handle_type_id,
    kind: ValueKind::Nested,
    get: None,
    set: None,
    nested: Some(&HANDLE_TYPE_INFO),
    enum_info: None,
    array: None,
}];

static SLOT_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c6_nested_descend::Slot",
    type_id_fn: slot_type_id,
    size: size_of::<Slot>(),
    align: align_of::<Slot>(),
    fields: &SLOT_FIELDS,
    kind: TypeKind::TupleStruct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

// ─────────────────────────────── the gate ───────────────────────────────────

/// The precondition every descend below rests on, checked rather than assumed: every
/// descriptor is coherent, every `Nested` edge is inline-contained, and the graph is
/// acyclic (CORE D21). A green here is what licenses the `unsafe` constructors.
#[test]
fn every_fixture_descriptor_is_coherent() {
    for info in [
        &BODY_TYPE_INFO,
        &PLACEMENT_TYPE_INFO,
        &POINT_TYPE_INFO,
        &SLOT_TYPE_INFO,
        &HANDLE_TYPE_INFO,
    ] {
        if let Err(problems) = validate(info) {
            let lines: Vec<String> = problems.iter().map(ToString::to_string).collect();
            panic!("`{}` is INCOHERENT:\n  {}", info.type_name, lines.join("\n  "));
        }
    }
}

/// **CORE C6 gate 1, fixture half (named-field).** Descend two levels, read a leaf.
#[test]
fn a_depth_two_named_field_nest_descends_and_reads_its_leaf() {
    let value = Body { hp: 42.0, placement: Placement { at: Point { x: 3.5, y: -7.25 }, layer: 2 } };
    // SAFETY: `BODY_TYPE_INFO` describes `Body` -- `type_id_fn`, `size`, `align` and every
    // `offset` are that type's own (pinned by `the_descriptors_describe_the_real_types`)
    // -- and it validates clean, so every `Nested` edge is inline-contained and the graph
    // is acyclic. `value` is a live, initialized `Body` this frame owns and does not
    // write while the cursor is alive.
    let root = unsafe { NestedCursor::new(&value, &BODY_TYPE_INFO) };

    assert_eq!(root.get(0), Some(Scalar::from(42.0f32)), "the root's own Prim");

    let placement = root.descend(1).expect("field #1 `placement` is Nested");
    assert_eq!(placement.type_info().type_name, "c6_nested_descend::Placement");
    assert_eq!(
        placement.fields().iter().map(|f| f.name).collect::<Vec<_>>(),
        ["at", "layer"],
        "enumeration must work at depth 1"
    );
    assert_eq!(placement.get(1), Some(Scalar::from(2u32)));

    let point = placement.descend(0).expect("field #0 `at` is Nested");
    assert_eq!(point.type_info().type_name, "c6_nested_descend::Point");
    assert_eq!(point.get(0), Some(Scalar::from(3.5f32)), "the depth-2 leaf");
    assert_eq!(point.get(1), Some(Scalar::from(-7.25f32)));
}

/// **CORE C6 gate 1, fixture half (tuple struct).** The same descend where the field
/// names are the decimal indices.
#[test]
fn a_depth_two_tuple_struct_descends_and_reads_its_leaf() {
    let value = Slot(Handle(0x0BAD_F00D));
    // SAFETY: as above, for `Slot` -- both levels are `#[repr(transparent)]`, so every
    // offset is 0 and the sizes agree, and `SLOT_TYPE_INFO` validates clean.
    let root = unsafe { NestedCursor::new(&value, &SLOT_TYPE_INFO) };

    let handle = root.descend(0).expect("field #0 is Nested");
    assert_eq!(handle.fields()[0].name, "0", "a tuple struct's field name is its index");
    assert_eq!(handle.get(0), Some(Scalar::from(0x0BAD_F00Du32)));

    match root.value(0) {
        Some(FieldValue::Nested(c)) => {
            assert_eq!(c.type_info().type_name, "c6_nested_descend::Handle");
        }
        other => panic!("field #0 must be a Nested cursor, got {other:?}"),
    }
}

/// The descriptors describe the real types. `descend`'s pointer arithmetic is only sound
/// if these numbers are the compiler's own, so they are pinned rather than trusted — the
/// same reason C3 gate 4 exists.
#[test]
fn the_descriptors_describe_the_real_types() {
    assert_eq!((BODY_TYPE_INFO.type_id_fn)(), TypeId::of::<Body>());
    assert_eq!(BODY_TYPE_INFO.size, size_of::<Body>());
    assert_eq!(BODY_TYPE_INFO.align, align_of::<Body>());
    assert_eq!(BODY_FIELDS[1].offset, offset_of!(Body, placement));

    assert_eq!((PLACEMENT_TYPE_INFO.type_id_fn)(), TypeId::of::<Placement>());
    assert_eq!(PLACEMENT_FIELDS[0].offset, offset_of!(Placement, at));
    assert_eq!(PLACEMENT_FIELDS[1].offset, offset_of!(Placement, layer));

    assert_eq!((POINT_TYPE_INFO.type_id_fn)(), TypeId::of::<Point>());
    assert_eq!(POINT_FIELDS[1].offset, offset_of!(Point, y));

    assert_eq!(SLOT_TYPE_INFO.size, size_of::<Slot>());
    assert_eq!(HANDLE_TYPE_INFO.size, size_of::<Handle>());
}
