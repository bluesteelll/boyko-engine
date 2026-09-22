//! **CORE C6 gate 1, dogfood half** — the descend over **real engine types**.
//!
//! `reflect_fixture`'s half proves the API works on types written to be reflected. This
//! half proves it works on types written for the engine, by people who were not thinking
//! about reflection: `Transform → Vec3 → f32` and `Name → NameId → u32`, both genuine
//! depth-2 descends, and the second a tuple-struct chain in the bargain.
//!
//! # Why this lives in its own package and not beside the fixture half
//!
//! `Transform` / `Name` live in `boyko_scene`, which `reflect_fixture` **cannot reach**:
//! that package is restricted to `boyko-ecs` / `boyko-macros` / `boyko-reflect` because it
//! is the one the CI Miri row names, and Miri cannot execute FFI (GATES D4/D15). This
//! package carries the engine edges (`boyko-scene`, `boyko-render`) and is therefore
//! deliberately **off** the Miri allowlist. Both halves are recoverable; neither is
//! recoverable in one package.
//!
//! # Hand-baked, until C7
//!
//! `#[component(reflect)]` lands at **C7**. Until then the statics below are written by
//! hand against the real types' own `offset_of!` / `size_of` / `align_of`, which is
//! exactly the comparison target C7's generated output will be measured against.
//!
//! Run with `cargo test -p reflect-dogfood --features reflect-dogfood/reflect` — the leaf
//! umbrella (GATES D15), which is also what forwards `boyko-scene/reflect`.
#![cfg(feature = "reflect")]

use std::any::TypeId;
use std::mem::offset_of;

use boyko_math::{Quat, Vec3};
use boyko_reflect::cursor::NestedCursor;
use boyko_reflect::prim;
use boyko_reflect::scalar::{Scalar, ScalarKind};
use boyko_reflect::type_info::{FieldInfo, TypeInfo, TypeKind, ValueKind, validate};
use boyko_scene::identity::{Name, NameId};
use boyko_scene::transform::Transform;

fn f32_type_id() -> TypeId {
    TypeId::of::<f32>()
}
fn u32_type_id() -> TypeId {
    TypeId::of::<u32>()
}
fn vec3_type_id() -> TypeId {
    TypeId::of::<Vec3>()
}
fn quat_type_id() -> TypeId {
    TypeId::of::<Quat>()
}
fn transform_type_id() -> TypeId {
    TypeId::of::<Transform>()
}
fn name_type_id() -> TypeId {
    TypeId::of::<Name>()
}
fn name_id_type_id() -> TypeId {
    TypeId::of::<NameId>()
}

/// One `f32` component of a math vector — every field of `Vec3`/`Quat` has this shape.
const fn f32_field(name: &'static str, offset: usize) -> FieldInfo {
    FieldInfo {
        name,
        offset,
        type_id_fn: f32_type_id,
        kind: ValueKind::Prim(ScalarKind::F32),
        get: Some(prim::get_f32),
        set: Some(prim::set_f32),
        nested: None,
        enum_info: None,
        array: None,
    }
}

static VEC3_FIELDS: [FieldInfo; 3] = [
    f32_field("x", offset_of!(Vec3, x)),
    f32_field("y", offset_of!(Vec3, y)),
    f32_field("z", offset_of!(Vec3, z)),
];

static VEC3_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "boyko_math::vec::Vec3",
    type_id_fn: vec3_type_id,
    size: size_of::<Vec3>(),
    align: align_of::<Vec3>(),
    fields: &VEC3_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

static QUAT_FIELDS: [FieldInfo; 4] = [
    f32_field("x", offset_of!(Quat, x)),
    f32_field("y", offset_of!(Quat, y)),
    f32_field("z", offset_of!(Quat, z)),
    f32_field("w", offset_of!(Quat, w)),
];

static QUAT_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "boyko_math::quat::Quat",
    type_id_fn: quat_type_id,
    size: size_of::<Quat>(),
    align: align_of::<Quat>(),
    fields: &QUAT_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

/// One `Nested` field pointing at an inner type's own static — §3.1's *"a POINTER, never
/// a flattened path table"*.
const fn nested_field(
    name: &'static str,
    offset: usize,
    type_id_fn: fn() -> TypeId,
    inner: &'static TypeInfo,
) -> FieldInfo {
    FieldInfo {
        name,
        offset,
        type_id_fn,
        kind: ValueKind::Nested,
        get: None,
        set: None,
        nested: Some(inner),
        enum_info: None,
        array: None,
    }
}

static TRANSFORM_FIELDS: [FieldInfo; 3] = [
    nested_field(
        "translation",
        offset_of!(Transform, translation),
        vec3_type_id,
        &VEC3_TYPE_INFO,
    ),
    nested_field("rotation", offset_of!(Transform, rotation), quat_type_id, &QUAT_TYPE_INFO),
    nested_field("scale", offset_of!(Transform, scale), vec3_type_id, &VEC3_TYPE_INFO),
];

static TRANSFORM_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "boyko_scene::transform::Transform",
    type_id_fn: transform_type_id,
    size: size_of::<Transform>(),
    align: align_of::<Transform>(),
    fields: &TRANSFORM_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

static NAME_ID_FIELDS: [FieldInfo; 1] = [FieldInfo {
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

static NAME_ID_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "boyko_scene::identity::NameId",
    type_id_fn: name_id_type_id,
    size: size_of::<NameId>(),
    align: align_of::<NameId>(),
    fields: &NAME_ID_FIELDS,
    kind: TypeKind::TupleStruct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

static NAME_FIELDS: [FieldInfo; 1] =
    [nested_field("0", 0, name_id_type_id, &NAME_ID_TYPE_INFO)];

static NAME_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "boyko_scene::identity::Name",
    type_id_fn: name_type_id,
    size: size_of::<Name>(),
    align: align_of::<Name>(),
    fields: &NAME_FIELDS,
    kind: TypeKind::TupleStruct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

// ─────────────────────────────── the gate ───────────────────────────────────

/// The precondition, on real engine types: every descriptor is coherent, every `Nested`
/// edge is inline-contained, and the graph is acyclic (CORE D21).
///
/// `Transform` is also the first fixture in this campaign where the **finished set**
/// earns its keep: `translation` and `scale` both point at `VEC3_TYPE_INFO`, so the walk
/// meets the same node twice and must answer "already proved", not "cycle".
#[test]
fn every_engine_descriptor_is_coherent() {
    for info in [
        &TRANSFORM_TYPE_INFO,
        &VEC3_TYPE_INFO,
        &QUAT_TYPE_INFO,
        &NAME_TYPE_INFO,
        &NAME_ID_TYPE_INFO,
    ] {
        if let Err(problems) = validate(info) {
            let lines: Vec<String> = problems.iter().map(ToString::to_string).collect();
            panic!("`{}` is INCOHERENT:\n  {}", info.type_name, lines.join("\n  "));
        }
    }
}

/// **CORE C6 gate 1, dogfood half — `Transform → Vec3 → f32`.**
#[test]
fn transform_descends_to_a_vec3_component() {
    let value = Transform {
        translation: Vec3 { x: 1.5, y: -2.5, z: 3.25 },
        rotation: Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 },
        scale: Vec3 { x: 2.0, y: 2.0, z: 2.0 },
    };
    // SAFETY: `TRANSFORM_TYPE_INFO` describes `Transform` -- `type_id_fn`, `size`, `align`
    // and every `offset` are that type's own (pinned by
    // `the_descriptors_describe_the_real_engine_types`) -- and it validates clean, so each
    // `Nested` edge is inline-contained and the graph is acyclic. `value` is a live,
    // initialized `Transform` this frame owns and does not write while the cursor lives.
    let root = unsafe { NestedCursor::new(&value, &TRANSFORM_TYPE_INFO) };

    assert_eq!(
        root.fields().iter().map(|f| f.name).collect::<Vec<_>>(),
        ["translation", "rotation", "scale"]
    );

    let translation = root.descend(0).expect("field #0 `translation` is Nested");
    assert_eq!(translation.get(0), Some(Scalar::from(1.5f32)));
    assert_eq!(translation.get(1), Some(Scalar::from(-2.5f32)));
    assert_eq!(translation.get(2), Some(Scalar::from(3.25f32)));

    let rotation = root.descend(1).expect("field #1 `rotation` is Nested");
    assert_eq!(rotation.get(3), Some(Scalar::from(1.0f32)), "the identity quaternion's w");

    // The second edge into the SAME descriptor: `scale` is a `Vec3` too, and it must read
    // its own bytes rather than `translation`'s.
    let scale = root.descend(2).expect("field #2 `scale` is Nested");
    assert_eq!(scale.get(0), Some(Scalar::from(2.0f32)));
    assert_eq!(
        scale.type_info().type_name,
        translation.type_info().type_name,
        "both edges point at one shared descriptor -- that sharing is the point"
    );
}

/// **CORE C6 gate 1, dogfood half — `Name → NameId → u32`**, two
/// `#[repr(transparent)]` levels.
#[test]
fn name_descends_to_its_interned_u32() {
    let value = Name(NameId(0x1234_5678));
    // SAFETY: as above, for `Name` -- both levels are `#[repr(transparent)]`, so every
    // offset is 0 and the sizes agree; `NAME_TYPE_INFO` validates clean.
    let root = unsafe { NestedCursor::new(&value, &NAME_TYPE_INFO) };

    let id = root.descend(0).expect("field #0 is Nested");
    assert_eq!(id.type_info().type_name, "boyko_scene::identity::NameId");
    assert_eq!(id.get(0), Some(Scalar::from(0x1234_5678u32)));
}

/// The descriptors describe the real engine types. These are the numbers `descend`'s
/// arithmetic rests on, and they belong to `boyko_scene` / `boyko_math` — a layout change
/// there must red here rather than silently move a read.
#[test]
fn the_descriptors_describe_the_real_engine_types() {
    assert_eq!((TRANSFORM_TYPE_INFO.type_id_fn)(), TypeId::of::<Transform>());
    assert_eq!(TRANSFORM_TYPE_INFO.size, size_of::<Transform>());
    assert_eq!(TRANSFORM_TYPE_INFO.align, align_of::<Transform>());
    assert_eq!(TRANSFORM_FIELDS[0].offset, offset_of!(Transform, translation));
    assert_eq!(TRANSFORM_FIELDS[1].offset, offset_of!(Transform, rotation));
    assert_eq!(TRANSFORM_FIELDS[2].offset, offset_of!(Transform, scale));

    assert_eq!((VEC3_TYPE_INFO.type_id_fn)(), TypeId::of::<Vec3>());
    assert_eq!(VEC3_TYPE_INFO.size, size_of::<Vec3>());
    assert_eq!(VEC3_FIELDS[2].offset, offset_of!(Vec3, z));

    assert_eq!((QUAT_TYPE_INFO.type_id_fn)(), TypeId::of::<Quat>());
    assert_eq!(QUAT_FIELDS[3].offset, offset_of!(Quat, w));

    assert_eq!(NAME_TYPE_INFO.size, size_of::<Name>());
    assert_eq!(NAME_ID_TYPE_INFO.size, size_of::<NameId>());
}
