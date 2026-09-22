//! CORE C6 — the `Nested` recursion contract, read side.
//!
//! Two things are gated here, and they are not the same thing:
//!
//! 1. **Gate 3, the two structural refusals** (CORE D21, plan §3.1). `validate` refuses a
//!    hand-baked `Nested` edge two ways, on separate fixtures:
//!    * **3(i)** `offset + child.size > parent.size` ⇒ [`Violation::NestedNotInline`] —
//!      the mis-described-container case, caught at **depth 1**, by arithmetic over two
//!      `usize`s, with **no list of refused types anywhere**;
//!    * **3(ii)** a cyclic `TYPE_INFO` graph ⇒ [`Violation::NestedCycle`] — on a fixture
//!      built so Check A is *satisfied* at every edge, which is what makes the two checks
//!      demonstrably non-redundant rather than argued to be.
//! 2. **The cursor** — [`NestedCursor`]'s descend, enumeration at depth ≥ 1, and its
//!    refusals. This is also the Miri subject for C6's new pointer arithmetic.
//!
//! # Why these fixtures are hand-written statics
//!
//! Because that is the case the code admits. A `Sized` Rust value cannot contain itself,
//! so a **derive-generated** graph is acyclic by the type system — but the derive does not
//! exist until C7, `TYPE_INFO` statics are written by hand at C3 and here, and a
//! `size(A) == size(B)` chain closes a cycle that inline containment cannot see. A proof
//! that holds for the case the author imagined and not for the case the code admits is
//! this campaign's recurring shape.

use std::any::TypeId;
use std::mem::offset_of;

use boyko_reflect::cursor::{FieldValue, NestedCursor};
use boyko_reflect::prim;
use boyko_reflect::scalar::{Scalar, ScalarKind};
use boyko_reflect::type_info::{
    ArrayInfo, FieldInfo, MAX_NESTED_DEPTH, MAX_NESTED_TYPES, TypeInfo, TypeKind, ValueKind,
    Violation, validate,
};

// ═════════════════════ the coherent depth-2 subject (the cursor) ════════════

/// The leaf of the depth-2 nest.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
struct Leaf {
    x: f32,
    y: f32,
}

/// The middle level — reached only by descending, which is the point: `Mid`'s fields are
/// enumerable at depth 1 through the cursor, not through the root's `TypeInfo`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
struct Mid {
    leaf: Leaf,
    tag: u32,
}

/// The root.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
struct Outer {
    hp: f32,
    mid: Mid,
}

/// The tuple-struct case (field names are the decimal indices, §3): a genuine depth-2
/// descend whose intermediate level is `#[repr(transparent)]`.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq)]
struct Handle(u32);

/// The tuple-struct root.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq)]
struct Slot(Handle);

fn leaf_type_id() -> TypeId {
    TypeId::of::<Leaf>()
}
fn mid_type_id() -> TypeId {
    TypeId::of::<Mid>()
}
fn outer_type_id() -> TypeId {
    TypeId::of::<Outer>()
}
fn f32_type_id() -> TypeId {
    TypeId::of::<f32>()
}
fn u32_type_id() -> TypeId {
    TypeId::of::<u32>()
}
fn handle_type_id() -> TypeId {
    TypeId::of::<Handle>()
}
fn slot_type_id() -> TypeId {
    TypeId::of::<Slot>()
}

static LEAF_FIELDS: [FieldInfo; 2] = [
    FieldInfo {
        name: "x",
        offset: offset_of!(Leaf, x),
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
        offset: offset_of!(Leaf, y),
        type_id_fn: f32_type_id,
        kind: ValueKind::Prim(ScalarKind::F32),
        get: Some(prim::get_f32),
        set: Some(prim::set_f32),
        nested: None,
        enum_info: None,
        array: None,
    },
];

static LEAF_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c6_nested::Leaf",
    type_id_fn: leaf_type_id,
    size: size_of::<Leaf>(),
    align: align_of::<Leaf>(),
    fields: &LEAF_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

static MID_FIELDS: [FieldInfo; 2] = [
    FieldInfo {
        name: "leaf",
        offset: offset_of!(Mid, leaf),
        type_id_fn: leaf_type_id,
        kind: ValueKind::Nested,
        get: None,
        set: None,
        nested: Some(&LEAF_TYPE_INFO),
        enum_info: None,
        array: None,
    },
    FieldInfo {
        name: "tag",
        offset: offset_of!(Mid, tag),
        type_id_fn: u32_type_id,
        kind: ValueKind::Prim(ScalarKind::U32),
        get: Some(prim::get_u32),
        set: Some(prim::set_u32),
        nested: None,
        enum_info: None,
        array: None,
    },
];

static MID_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c6_nested::Mid",
    type_id_fn: mid_type_id,
    size: size_of::<Mid>(),
    align: align_of::<Mid>(),
    fields: &MID_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

static OUTER_FIELDS: [FieldInfo; 2] = [
    FieldInfo {
        name: "hp",
        offset: offset_of!(Outer, hp),
        type_id_fn: f32_type_id,
        kind: ValueKind::Prim(ScalarKind::F32),
        get: Some(prim::get_f32),
        set: Some(prim::set_f32),
        nested: None,
        enum_info: None,
        array: None,
    },
    FieldInfo {
        name: "mid",
        offset: offset_of!(Outer, mid),
        type_id_fn: mid_type_id,
        kind: ValueKind::Nested,
        get: None,
        set: None,
        nested: Some(&MID_TYPE_INFO),
        enum_info: None,
        array: None,
    },
];

static OUTER_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c6_nested::Outer",
    type_id_fn: outer_type_id,
    size: size_of::<Outer>(),
    align: align_of::<Outer>(),
    fields: &OUTER_FIELDS,
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
    type_name: "c6_nested::Handle",
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
    type_name: "c6_nested::Slot",
    type_id_fn: slot_type_id,
    size: size_of::<Slot>(),
    align: align_of::<Slot>(),
    fields: &SLOT_FIELDS,
    kind: TypeKind::TupleStruct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

fn sample() -> Outer {
    Outer { hp: 12.5, mid: Mid { leaf: Leaf { x: -1.5, y: 2.25 }, tag: 7 } }
}

// ════════════════ gate 3(i) — the mis-described container ═══════════════════

/// The child descriptor: a real 40-byte type. Nothing about it is malformed — the
/// defect is in the *edge* that claims a 24-byte field contains it.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Wide {
    words: [f64; 5],
}

/// The parent: a real `Vec<f32>` field, hand-described as `Nested`.
///
/// This is the shape a refusal list is *supposed* to prevent and cannot in general — no
/// syntactic blacklist names a user-defined indirection, and no runtime list can
/// enumerate `Vec<T>`'s unboundedly many `TypeId`s.
#[repr(C)]
struct Leaky {
    data: Vec<f32>,
}

fn wide_type_id() -> TypeId {
    TypeId::of::<Wide>()
}
fn words_type_id() -> TypeId {
    TypeId::of::<[f64; 5]>()
}
fn leaky_type_id() -> TypeId {
    TypeId::of::<Leaky>()
}

static WIDE_FIELDS: [FieldInfo; 1] = [FieldInfo {
    name: "words",
    offset: offset_of!(Wide, words),
    type_id_fn: words_type_id,
    kind: ValueKind::Array,
    get: None,
    set: None,
    nested: None,
    enum_info: None,
    array: Some(ArrayInfo { elem: ScalarKind::F64, stride: size_of::<f64>(), len: 5 }),
}];

static WIDE_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c6_nested::Wide",
    type_id_fn: wide_type_id,
    size: size_of::<Wide>(),
    align: align_of::<Wide>(),
    fields: &WIDE_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

static LEAKY_FIELDS: [FieldInfo; 1] = [FieldInfo {
    name: "data",
    offset: offset_of!(Leaky, data),
    type_id_fn: wide_type_id,
    kind: ValueKind::Nested,
    get: None,
    set: None,
    nested: Some(&WIDE_TYPE_INFO),
    enum_info: None,
    array: None,
}];

static LEAKY_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c6_nested::Leaky",
    type_id_fn: leaky_type_id,
    size: size_of::<Leaky>(),
    align: align_of::<Leaky>(),
    fields: &LEAKY_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

/// **CORE C6 gate 3(i).** A `Nested` child larger than the field's extent is refused,
/// and the refusal names the rule.
///
/// The number is printed rather than asserted-in-the-abstract: this fixture would descend
/// **past the end of the value**, and by how much is what makes the defect legible.
#[test]
fn a_nested_child_too_large_for_its_field_is_refused() {
    let problems = validate(&LEAKY_TYPE_INFO)
        .expect_err("a 40-byte child inside a 24-byte value is not inline-contained");

    let field = &LEAKY_FIELDS[0];
    let overrun = field.offset + WIDE_TYPE_INFO.size - LEAKY_TYPE_INFO.size;
    println!(
        "gate 3(i): field `{}` at offset {} claims a {}-byte child inside a {}-byte value \
         -- the first descend would read {overrun} bytes past the end",
        field.name, field.offset, WIDE_TYPE_INFO.size, LEAKY_TYPE_INFO.size
    );

    assert_eq!(
        problems.len(),
        1,
        "the fixture is built to violate ONE rule, so C6's RED can green it by deleting \
         ONE clause; got: {problems:?}"
    );
    assert_eq!(problems[0].violation, Violation::NestedNotInline);
    assert_eq!(problems[0].field_index, Some(0));
    assert_eq!(problems[0].name, "data");
}

/// Gate 3(i)'s non-vacuity: the rule is not "refuse every `Nested` edge". The coherent
/// depth-2 subject — a real inline nest — validates clean, at both levels.
#[test]
fn a_real_inline_nest_is_accepted_at_every_level() {
    for info in [&OUTER_TYPE_INFO, &MID_TYPE_INFO, &LEAF_TYPE_INFO, &SLOT_TYPE_INFO] {
        if let Err(problems) = validate(info) {
            let lines: Vec<String> = problems.iter().map(ToString::to_string).collect();
            panic!("`{}` must be coherent:\n  {}", info.type_name, lines.join("\n  "));
        }
    }
}

/// Check A's other two clauses, each isolated on its own fixture.
///
/// They exist so C6's RED is precise: deleting the **size** clause must green the
/// oversized fixture and leave these two red, which is how "the size clause is the
/// load-bearing one" becomes a measurement rather than an assertion.
#[test]
fn a_misaligned_or_over_aligned_nested_child_is_refused() {
    static MISALIGNED_FIELDS: [FieldInfo; 1] = [FieldInfo {
        name: "at_two",
        offset: 2,
        type_id_fn: leaf_type_id,
        kind: ValueKind::Nested,
        get: None,
        set: None,
        nested: Some(&LEAF_TYPE_INFO),
        enum_info: None,
        array: None,
    }];
    static MISALIGNED: TypeInfo = TypeInfo {
        type_name: "c6_nested::Misaligned",
        type_id_fn: outer_type_id,
        size: 32,
        align: 8,
        fields: &MISALIGNED_FIELDS,
        kind: TypeKind::Struct,
        enum_info: None,
        default_in_place: None,
        drop_in_place: None,
    };
    static OVER_ALIGNED_FIELDS: [FieldInfo; 1] = [FieldInfo {
        name: "needs_eight",
        offset: 0,
        type_id_fn: wide_type_id,
        kind: ValueKind::Nested,
        get: None,
        set: None,
        nested: Some(&WIDE_TYPE_INFO),
        enum_info: None,
        array: None,
    }];
    static OVER_ALIGNED: TypeInfo = TypeInfo {
        type_name: "c6_nested::OverAligned",
        type_id_fn: outer_type_id,
        size: 64,
        align: 4,
        fields: &OVER_ALIGNED_FIELDS,
        kind: TypeKind::Struct,
        enum_info: None,
        default_in_place: None,
        drop_in_place: None,
    };

    let a = validate(&MISALIGNED).expect_err("offset 2 is not a multiple of align_of::<Leaf>()");
    assert_eq!(a.len(), 1, "{a:?}");
    assert_eq!(a[0].violation, Violation::NestedNotInline);

    let b = validate(&OVER_ALIGNED).expect_err("an 8-aligned child in a 4-aligned parent");
    assert_eq!(b.len(), 1, "{b:?}");
    assert_eq!(b[0].violation, Violation::NestedNotInline);
}

// ════════════════════ gate 3(ii) — the cyclic graph ═════════════════════════

/// One half of the ring. Both halves are 8 bytes, which is the whole point: inline
/// containment is **satisfied** at every edge of a graph that cannot terminate.
#[repr(transparent)]
#[derive(Debug, Clone, Copy)]
struct RingA(u64);

/// The other half.
#[repr(transparent)]
#[derive(Debug, Clone, Copy)]
struct RingB(u64);

fn ring_a_type_id() -> TypeId {
    TypeId::of::<RingA>()
}
fn ring_b_type_id() -> TypeId {
    TypeId::of::<RingB>()
}

static RING_A_FIELDS: [FieldInfo; 1] = [FieldInfo {
    name: "to_b",
    offset: 0,
    type_id_fn: ring_b_type_id,
    kind: ValueKind::Nested,
    get: None,
    set: None,
    nested: Some(&RING_B_TYPE_INFO),
    enum_info: None,
    array: None,
}];

static RING_A_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c6_nested::RingA",
    type_id_fn: ring_a_type_id,
    size: size_of::<RingA>(),
    align: align_of::<RingA>(),
    fields: &RING_A_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

static RING_B_FIELDS: [FieldInfo; 1] = [FieldInfo {
    name: "back_to_a",
    offset: 0,
    type_id_fn: ring_a_type_id,
    kind: ValueKind::Nested,
    get: None,
    set: None,
    nested: Some(&RING_A_TYPE_INFO),
    enum_info: None,
    array: None,
}];

static RING_B_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c6_nested::RingB",
    type_id_fn: ring_b_type_id,
    size: size_of::<RingB>(),
    align: align_of::<RingB>(),
    fields: &RING_B_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

/// A descriptor that names itself — the shortest cycle there is.
static SELF_FIELDS: [FieldInfo; 1] = [FieldInfo {
    name: "myself",
    offset: 0,
    type_id_fn: ring_a_type_id,
    kind: ValueKind::Nested,
    get: None,
    set: None,
    nested: Some(&SELF_TYPE_INFO),
    enum_info: None,
    array: None,
}];

static SELF_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c6_nested::SelfRef",
    type_id_fn: ring_a_type_id,
    size: 8,
    align: 8,
    fields: &SELF_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

/// **CORE C6 gate 3(ii).** A cyclic `TYPE_INFO` graph is refused, and the refusal names
/// the edge that closes the cycle.
///
/// Terminating at all is half the claim: without Check B this call does not return.
#[test]
fn a_cyclic_type_info_graph_is_refused() {
    let problems = validate(&RING_A_TYPE_INFO).expect_err("A -> B -> A is a cycle");
    assert!(
        problems.iter().any(|p| p.violation == Violation::NestedCycle),
        "expected a named NestedCycle, got: {problems:?}"
    );
    let cycle = problems.iter().find(|p| p.violation == Violation::NestedCycle).unwrap();
    assert_eq!(
        cycle.name, "back_to_a",
        "the problem must name the edge that CLOSES the cycle -- that is the one a fix \
         deletes"
    );

    let direct = validate(&SELF_TYPE_INFO).expect_err("a descriptor that names itself");
    assert!(
        direct.iter().any(|p| p.violation == Violation::NestedCycle),
        "the shortest cycle must be caught too: {direct:?}"
    );
}

/// **The measurement that makes the two checks non-redundant.**
///
/// The architect's ruling says Check A implies acyclicity for derive-generated statics
/// *and does not for hand-written ones*. This asserts the second half rather than
/// conceding it: the cyclic fixture produces **no** `NestedNotInline` at any node, so
/// deleting Check B would leave the cycle entirely unrefused.
#[test]
fn check_a_is_satisfied_at_every_edge_of_the_cycle() {
    for info in [&RING_A_TYPE_INFO, &RING_B_TYPE_INFO, &SELF_TYPE_INFO] {
        let problems = validate(info).expect_err("every one of these is cyclic");
        assert!(
            !problems.iter().any(|p| p.violation == Violation::NestedNotInline),
            "`{}` is a CYCLE that inline containment cannot see -- if this ever reports \
             NestedNotInline the fixture has stopped testing what it was built for: {problems:?}",
            info.type_name
        );
    }
}

// ══════════════ Check B's capacity refusal — both branches ══════════════════

/// Builds a chain of `depth` `Nested` levels above a fieldless leaf.
///
/// `Box::leak` rather than statics: a 33-deep chain of hand-written statics is 66
/// declarations of boilerplate, and the descriptors under test are pure data with no drop
/// glue. The leak is what puts this test's callers behind a `miri` ignore.
fn leak_chain(depth: usize) -> &'static TypeInfo {
    let mut current: &'static TypeInfo = leak_leaf();
    for _ in 0..depth {
        current = leak_parent_of(&[current]);
    }
    current
}

/// A fieldless 8-byte leaf.
fn leak_leaf() -> &'static TypeInfo {
    Box::leak(Box::new(TypeInfo {
        type_name: "c6_nested::ChainLeaf",
        type_id_fn: ring_a_type_id,
        size: 8,
        align: 8,
        fields: &[],
        kind: TypeKind::Struct,
        enum_info: None,
        default_in_place: None,
        drop_in_place: None,
    }))
}

/// A node with one `Nested` field per child, every one inline-contained at offset 0.
fn leak_parent_of(children: &[&'static TypeInfo]) -> &'static TypeInfo {
    let fields: Vec<FieldInfo> = children
        .iter()
        .map(|child| FieldInfo {
            name: "child",
            offset: 0,
            type_id_fn: ring_a_type_id,
            kind: ValueKind::Nested,
            get: None,
            set: None,
            nested: Some(*child),
            enum_info: None,
            array: None,
        })
        .collect();
    Box::leak(Box::new(TypeInfo {
        type_name: "c6_nested::ChainLink",
        type_id_fn: ring_a_type_id,
        size: 8,
        align: 8,
        fields: Box::leak(fields.into_boxed_slice()),
        kind: TypeKind::Struct,
        enum_info: None,
        default_in_place: None,
        drop_in_place: None,
    }))
}

/// Check B's **depth** capacity: a chain deeper than [`MAX_NESTED_DEPTH`] is a refusal,
/// never a deeper stack and never a silent completion.
///
/// A partial acyclicity proof is not one, which is why the walk reports rather than
/// truncates. The chain one level *under* the limit is the non-vacuity half: the bound is
/// where it says it is.
#[test]
#[cfg_attr(
    miri,
    ignore = "builds its fixture with `Box::leak` (a 33-deep chain of hand-written \
              statics is 66 declarations); Miri's leak checker reports the deliberate \
              leak. No `unsafe` is involved -- the Miri-relevant paths here are the \
              cursor tests and the static cyclic fixtures, which do run."
)]
fn a_chain_deeper_than_the_walk_is_refused_rather_than_truncated() {
    let ok = leak_chain(MAX_NESTED_DEPTH - 1);
    assert!(
        validate(ok).is_ok(),
        "a chain of {} levels is inside the bound and must validate clean",
        MAX_NESTED_DEPTH - 1
    );

    let too_deep = leak_chain(MAX_NESTED_DEPTH + 4);
    let problems = validate(too_deep).expect_err("deeper than the walk's fixed path array");
    assert!(
        problems.iter().any(|p| p.violation == Violation::NestedGraphTooLarge),
        "expected the capacity refusal, got: {problems:?}"
    );
    assert_eq!(
        problems.iter().filter(|p| p.violation == Violation::NestedGraphTooLarge).count(),
        1,
        "the refusal is reported ONCE and stops the walk: {problems:?}"
    );
}

/// Check B's **node-count** capacity: more distinct types than the finished set can hold
/// is the same refusal, reached through the other branch.
#[test]
#[cfg_attr(
    miri,
    ignore = "builds 257 leaked descriptors (see the depth test's reason); Miri's leak \
              checker reports them. No `unsafe` is involved."
)]
fn a_graph_wider_than_the_finished_set_is_refused() {
    let leaves: Vec<&'static TypeInfo> = (0..=MAX_NESTED_TYPES).map(|_| leak_leaf()).collect();
    let root = leak_parent_of(&leaves);

    let problems = validate(root).expect_err("more distinct types than the walk remembers");
    assert!(
        problems.iter().any(|p| p.violation == Violation::NestedGraphTooLarge),
        "expected the capacity refusal, got: {problems:?}"
    );
}

// ═══════════════════════════ the cursor itself ══════════════════════════════

/// Descending two levels and reading a leaf — the claim §3.1 makes, exercised through
/// the API a consumer has.
#[test]
fn a_cursor_descends_two_levels_and_reads_a_leaf() {
    let value = sample();
    // SAFETY: `OUTER_TYPE_INFO` describes `Outer` (its `type_id_fn`, `size` and `align`
    // are that type's own, pinned below by `the_descriptors_describe_the_real_types`),
    // and it validates clean, so every `Nested` edge is inline-contained and the graph is
    // acyclic. `value` is a live, initialized `Outer` this frame owns.
    let root = unsafe { NestedCursor::new(&value, &OUTER_TYPE_INFO) };

    let mid = root.descend(1).expect("field #1 `mid` is Nested");
    assert_eq!(mid.type_info().type_name, "c6_nested::Mid");
    assert_eq!(mid.get(1), Some(Scalar::from(7u32)), "the middle level's own Prim");

    let leaf = mid.descend(0).expect("field #0 `leaf` is Nested");
    assert_eq!(leaf.type_info().type_name, "c6_nested::Leaf");
    assert_eq!(leaf.get(0), Some(Scalar::from(-1.5f32)));
    assert_eq!(leaf.get(1), Some(Scalar::from(2.25f32)));
}

/// Enumeration works at depth ≥ 1 — the gap a root-only API leaves, and the reason the
/// cursor carries `fields()` at all.
#[test]
fn fields_are_enumerable_at_depth_one_and_two() {
    let value = sample();
    // SAFETY: as `a_cursor_descends_two_levels_and_reads_a_leaf`.
    let root = unsafe { NestedCursor::new(&value, &OUTER_TYPE_INFO) };

    let names: Vec<&str> = root.fields().iter().map(|f| f.name).collect();
    assert_eq!(names, ["hp", "mid"]);

    let mid = root.descend(1).expect("mid");
    let names: Vec<&str> = mid.fields().iter().map(|f| f.name).collect();
    assert_eq!(names, ["leaf", "tag"], "depth 1 must enumerate the INNER type's fields");

    let leaf = mid.descend(0).expect("leaf");
    let names: Vec<&str> = leaf.fields().iter().map(|f| f.name).collect();
    assert_eq!(names, ["x", "y"]);
}

/// The tuple-struct descend, whose field names are the decimal indices.
#[test]
fn a_tuple_struct_descends_the_same_way() {
    let value = Slot(Handle(0xABCD));
    // SAFETY: `SLOT_TYPE_INFO` describes `Slot`, validates clean, and `value` is a live
    // `Slot` owned by this frame.
    let root = unsafe { NestedCursor::new(&value, &SLOT_TYPE_INFO) };
    let handle = root.descend(0).expect("field #0 is Nested");
    assert_eq!(handle.fields()[0].name, "0");
    assert_eq!(handle.get(0), Some(Scalar::from(0xABCDu32)));
}

/// The cursor's refusals: an out-of-range index, a non-`Nested` kind, and the scalar API
/// on a `Nested` field. None of them reinterprets bytes (CORE D10 / FIX Mi2).
#[test]
fn the_cursor_refuses_every_index_that_is_not_a_nested_field() {
    let value = sample();
    // SAFETY: as above.
    let root = unsafe { NestedCursor::new(&value, &OUTER_TYPE_INFO) };

    assert!(root.descend(0).is_none(), "field #0 `hp` is a Prim, not a descent");
    assert!(root.descend(2).is_none(), "out of range");
    assert!(root.descend(usize::MAX).is_none(), "out of range, extreme");
    assert!(root.get(1).is_none(), "field #1 `mid` is Nested -- no Scalar exists for it");
    assert!(root.get(2).is_none(), "out of range");
}

/// `value()` answers with the arm the kind calls for, and `None` for the kinds whose
/// representation belongs to a later rung.
#[test]
fn field_value_carries_a_scalar_for_prim_and_a_cursor_for_nested() {
    let value = sample();
    // SAFETY: as above.
    let root = unsafe { NestedCursor::new(&value, &OUTER_TYPE_INFO) };

    match root.value(0) {
        Some(FieldValue::Prim(s)) => assert_eq!(s, Scalar::from(12.5f32)),
        other => panic!("field #0 `hp` must be a Prim value, got {other:?}"),
    }
    match root.value(1) {
        Some(FieldValue::Nested(c)) => assert_eq!(c.type_info().type_name, "c6_nested::Mid"),
        other => panic!("field #1 `mid` must be a Nested cursor, got {other:?}"),
    }
    assert!(root.value(2).is_none(), "out of range");

    // The `Array` kind has no `FieldValue` arm at C6 -- elements are reached by index off
    // `ArrayInfo` (C5). `WIDE_TYPE_INFO`'s only field is an `Array`.
    let wide = Wide { words: [0.0; 5] };
    // SAFETY: `WIDE_TYPE_INFO` describes `Wide` and validates clean; `wide` is live and
    // owned by this frame.
    let cursor = unsafe { NestedCursor::new(&wide, &WIDE_TYPE_INFO) };
    assert!(cursor.value(0).is_none(), "the Array arm lands with its reader, not before");
}

/// A cursor is `Copy` and re-rootable: descending forks rather than advancing, so two
/// walks of the same value cannot interfere.
#[test]
fn a_cursor_is_copy_and_descending_does_not_consume_it() {
    let value = sample();
    // SAFETY: as above.
    let root = unsafe { NestedCursor::new(&value, &OUTER_TYPE_INFO) };
    let a = root.descend(1).expect("mid");
    let b = root.descend(1).expect("mid, again -- `root` was not consumed");
    assert_eq!(a.get(1), b.get(1));
    let copy = root;
    assert_eq!(copy.get(0), root.get(0));
}

/// The descriptors really do describe the types they name — `descend`'s soundness rests
/// on `size`/`align`/`offset` being the real ones, so they are pinned here rather than
/// assumed by the SAFETY comments above.
#[test]
fn the_descriptors_describe_the_real_types() {
    assert_eq!((OUTER_TYPE_INFO.type_id_fn)(), TypeId::of::<Outer>());
    assert_eq!(OUTER_TYPE_INFO.size, size_of::<Outer>());
    assert_eq!(OUTER_TYPE_INFO.align, align_of::<Outer>());
    assert_eq!(OUTER_FIELDS[1].offset, offset_of!(Outer, mid));

    assert_eq!((MID_TYPE_INFO.type_id_fn)(), TypeId::of::<Mid>());
    assert_eq!(MID_TYPE_INFO.size, size_of::<Mid>());
    assert_eq!(MID_FIELDS[0].offset, offset_of!(Mid, leaf));
    assert_eq!(MID_FIELDS[1].offset, offset_of!(Mid, tag));

    assert_eq!((LEAF_TYPE_INFO.type_id_fn)(), TypeId::of::<Leaf>());
    assert_eq!(LEAF_TYPE_INFO.size, size_of::<Leaf>());
    assert_eq!(LEAF_FIELDS[1].offset, offset_of!(Leaf, y));

    assert_eq!(SLOT_TYPE_INFO.size, size_of::<Slot>());
    assert_eq!(HANDLE_TYPE_INFO.size, size_of::<Handle>());

    // The gate-3(i) fixture's numbers, which are what make its overrun real rather than
    // arithmetic on invented constants.
    assert_eq!(LEAKY_TYPE_INFO.size, size_of::<Vec<f32>>());
    assert_eq!(WIDE_TYPE_INFO.size, 40);
    assert!(WIDE_TYPE_INFO.size > LEAKY_TYPE_INFO.size);
}
