//! CORE C6 gate 5 — a `NestedCursor` cannot outlive the value it was rooted at.
//!
//! The second half of what the `'a` buys. Held across a `&mut` (the sibling fixture) is
//! aliasing; escaping the frame is lifetime. A cursor with no lifetime parameter admits
//! both, which is why analysis M2/O3 refuses the bare `{ptr, info}` form outright rather
//! than documenting a rule.

use std::any::TypeId;

use boyko_reflect::cursor::NestedCursor;
use boyko_reflect::prim;
use boyko_reflect::scalar::ScalarKind;
use boyko_reflect::type_info::{FieldInfo, TypeInfo, TypeKind, ValueKind};

#[repr(C)]
struct Inner {
    x: f32,
}

#[repr(C)]
struct Outer {
    inner: Inner,
}

fn inner_type_id() -> TypeId {
    TypeId::of::<Inner>()
}
fn outer_type_id() -> TypeId {
    TypeId::of::<Outer>()
}
fn f32_type_id() -> TypeId {
    TypeId::of::<f32>()
}

static INNER_FIELDS: [FieldInfo; 1] = [FieldInfo {
    name: "x",
    offset: 0,
    type_id_fn: f32_type_id,
    kind: ValueKind::Prim(ScalarKind::F32),
    get: Some(prim::get_f32),
    set: Some(prim::set_f32),
    nested: None,
    enum_info: None,
    array: None,
}];

static INNER_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "cursor_outlives_value::Inner",
    type_id_fn: inner_type_id,
    size: size_of::<Inner>(),
    align: align_of::<Inner>(),
    fields: &INNER_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

static OUTER_FIELDS: [FieldInfo; 1] = [FieldInfo {
    name: "inner",
    offset: 0,
    type_id_fn: inner_type_id,
    kind: ValueKind::Nested,
    get: None,
    set: None,
    nested: Some(&INNER_TYPE_INFO),
    enum_info: None,
    array: None,
}];

static OUTER_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "cursor_outlives_value::Outer",
    type_id_fn: outer_type_id,
    size: size_of::<Outer>(),
    align: align_of::<Outer>(),
    fields: &OUTER_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

/// Hands a cursor back over a value this frame owns.
fn escape() -> NestedCursor<'static> {
    let value = Outer { inner: Inner { x: 1.0 } };
    // SAFETY (for the shape of the call — this file is not expected to link):
    // `OUTER_TYPE_INFO` describes `Outer` and validates clean.
    unsafe { NestedCursor::new(&value, &OUTER_TYPE_INFO) }
}

fn main() {
    let _ = escape();
}
