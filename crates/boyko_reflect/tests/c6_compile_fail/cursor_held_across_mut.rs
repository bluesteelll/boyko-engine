//! CORE C6 gate 5 — a `NestedCursor` held across a `&mut` op does not compile.
//!
//! This is the `&mut EcsMaster` hazard in miniature: the cursor reads bytes the mutation
//! is free to move. With the cursor's `'a` in place the borrow checker refuses it; with
//! the bare `{ptr, info}` cursor (C6's second RED) this file compiles and the refusal
//! becomes a runtime question nobody asks.

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
    type_name: "cursor_held_across_mut::Inner",
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
    type_name: "cursor_held_across_mut::Outer",
    type_id_fn: outer_type_id,
    size: size_of::<Outer>(),
    align: align_of::<Outer>(),
    fields: &OUTER_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

fn main() {
    let mut value = Outer { inner: Inner { x: 1.0 } };

    // SAFETY (for the shape of the call — this file is not expected to link):
    // `OUTER_TYPE_INFO` describes `Outer` and validates clean.
    let cursor = unsafe { NestedCursor::new(&value, &OUTER_TYPE_INFO) };

    // The mutation the cursor must forbid.
    value.inner.x = 2.0;

    let _ = cursor.descend(0);

    // Read it back after the borrow would have ended, so the fixture's blessed bytes carry
    // the borrow error and nothing else.
    println!("{}", value.inner.x);
}
