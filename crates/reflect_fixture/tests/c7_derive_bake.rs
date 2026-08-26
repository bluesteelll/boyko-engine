//! **CORE C7's gate table** — `#[component(reflect)]`'s field walk and `offset_of!`
//! baking, exercised from the *consumer* package (D23).
//!
//! # Why this file is in `reflect_fixture` and not in `boyko_reflect`
//!
//! C7's headline gate originally named C3's hand-written statics as its oracle. That
//! oracle is unreachable for three independent, measured reasons (D23): the C3 types are
//! **private items of an integration-test binary** (`c3_type_info.rs:43,52,63,70`);
//! `boyko_reflect` has **no `boyko-macros` edge** and cannot usefully get one, because
//! `boyko-macros` is a *dev*-dependency of `boyko_ecs` and does not propagate; and
//! `crates/boyko_reflect/Cargo.toml:12-16` forbids a `[features]` table *"now or ever"*,
//! so the derive's emitted `#[cfg(feature = "reflect")]` is permanently false there.
//! `reflect_fixture` is the one package that has `boyko-macros`, the `reflect` feature,
//! and locally-declared depth-2 named **and** tuple nests.
//!
//! # The oracle, and why it is copied rather than imported
//!
//! `tests/c6_nested_descend.rs` nominates itself for the role in its own header
//! (*"C7 replaces these statics with generated ones and inherits this file as an
//! independently-pinned comparison target"*), but an integration-test binary exports
//! nothing and `reflect_fixture` has no `src/lib.rs`. So the shapes and their hand-baked
//! descriptors are **copied here verbatim**, and the copy carries its own `offset_of!` /
//! `size_of` / `align_of` pin ([`the_hand_baked_oracle_describes_the_real_types`]) —
//! the same discipline C3 gate 4 uses, and the reason the copy cannot rot into agreement
//! with a wrong derive.
//!
//! # The invocation is part of the gate (D23)
//!
//! ```text
//! cargo test -p reflect-fixture --features reflect-fixture/reflect --test c7_derive_bake
//! ```
//!
//! The derive's emission is `#[cfg(feature = "reflect")]` evaluated in the **expanding**
//! crate (D2) and no package's `default` enables `reflect`, so a plain
//! `cargo test -p reflect-fixture` compiles this file to nothing and exits 0 — a vacuous
//! pass on the green side *and* on every red side. The output must read
//! `running [1-9]`.
#![cfg(feature = "reflect")]

use std::any::TypeId;
use std::cell::Cell;
use std::marker::PhantomData;
use std::mem::{MaybeUninit, offset_of};

use boyko_macros::Component;
use boyko_reflect::prim;
use boyko_reflect::scalar::{Scalar, ScalarKind};
use boyko_reflect::type_info::{FieldInfo, TypeInfo, TypeKind, ValueKind, validate};
use boyko_reflect::{Reflect, ReflectDefault};

// ───────────────────── the derived subjects: the depth-2 nests ──────────────
//
// The same shapes as `tests/c6_nested_descend.rs:43-83`, now ANNOTATED. `#[repr(C)]`
// keeps the offsets the ones C7's first RED needs to be non-zero: `Body.placement = 4`
// and `Placement.layer = 8`.

/// The leaf of the named-field nest.
#[derive(Component, Default, Debug, Clone, Copy, PartialEq)]
#[component(reflect)]
#[repr(C)]
pub struct Point {
    /// X.
    pub x: f32,
    /// Y.
    pub y: f32,
}

/// The middle level of the named-field nest.
#[derive(Component, Default, Debug, Clone, Copy, PartialEq)]
#[component(reflect)]
#[repr(C)]
pub struct Placement {
    /// Where — the depth-2 edge.
    pub at: Point,
    /// Which layer — the field whose offset (8) the first RED zeroes.
    pub layer: u32,
}

/// The root of the named-field nest.
#[derive(Component, Default, Debug, Clone, Copy, PartialEq)]
#[component(reflect)]
#[repr(C)]
pub struct Body {
    /// Health.
    pub hp: f32,
    /// The nested placement — the field whose offset (4) the first RED zeroes.
    pub placement: Placement,
}

/// The tuple-struct leaf (gate 6).
#[derive(Component, Default, Debug, Clone, Copy, PartialEq)]
#[component(reflect)]
#[repr(transparent)]
pub struct Handle(pub u32);

/// The tuple-struct root (gate 6).
#[derive(Component, Default, Debug, Clone, Copy, PartialEq)]
#[component(reflect)]
#[repr(transparent)]
pub struct Slot(pub Handle);

// ──────────────────── gate 3's swapped-declaration-order pair ───────────────
//
// Both `#[repr(Rust)]`, both resident: a standing gate cannot be a source edit.

/// Declaration order A/B.
#[derive(Component, Default)]
#[component(reflect)]
pub struct SwapAb {
    /// Declared first here, second in [`SwapBa`].
    pub first: u8,
    /// Declared second here, first in [`SwapBa`].
    pub second: u32,
}

/// Declaration order B/A — the same two fields, swapped.
#[derive(Component, Default)]
#[component(reflect)]
pub struct SwapBa {
    /// Declared first here, second in [`SwapAb`].
    pub second: u32,
    /// Declared second here, first in [`SwapAb`].
    pub first: u8,
}

// ───────────────────────── gate 4's drop-count subjects ─────────────────────

thread_local! {
    /// Drops of [`Owned`] observed on THIS thread. Thread-local rather than global for
    /// the reason `c4_prim_zero_alloc.rs` measured: libtest runs each test body on its
    /// own thread, and a process-global counter would see the other tests' subjects.
    /// `const`-init and `Drop`-free, so reading it allocates nothing.
    static DROPS: Cell<u32> = const { Cell::new(0) };
}

/// This thread's observed [`Owned`] drop count.
fn drops() -> u32 {
    DROPS.with(Cell::get)
}

/// A POD subject with no drop glue.
#[derive(Component, Default)]
#[component(reflect)]
pub struct Pod {
    /// One `Prim` field.
    pub value: f32,
}

/// [`Pod`] nested one level down — still drop-free.
#[derive(Component, Default)]
#[component(reflect)]
pub struct NestedPod {
    /// The nested POD.
    pub inner: Pod,
}

/// A struct wrapping `[f32; 4]`. The derive applies to an ITEM, so `[f32; 4]` cannot be
/// a subject on its own — the wrapper is what `c4_prim_zero_alloc.rs:466-468` already
/// does for the same reason.
#[derive(Component, Default)]
#[component(reflect)]
pub struct ArrPack {
    /// The array field.
    pub data: [f32; 4],
}

/// **The instrument gate 4 was missing (D24).** `{ Pod, NestedPod, ArrPack }` is
/// drop-free by construction, so `needs_drop` is false for all three and
/// `drop_in_place` is `None` for all three *whether the derive is right or sabotaged* —
/// the count is identically zero and "no leak, no double-free" is unfalsifiable over
/// that set. `Owned` is a type that drops, and its `Drop` deliberately reads **nothing**
/// from `self`, so the third RED (drop-before-write) observes a counter bump rather than
/// an uninitialized read.
#[derive(Component, Default)]
#[component(reflect)]
pub struct Owned {
    /// Payload, never read by the `Drop` impl.
    pub tag: u32,
}

impl Drop for Owned {
    fn drop(&mut self) {
        DROPS.with(|c| c.set(c.get() + 1));
    }
}

/// [`Owned`] nested one level down — drop glue reaches through the nest.
#[derive(Component, Default)]
#[component(reflect)]
pub struct NestedOwned {
    /// The owning inner value.
    pub inner: Owned,
}

/// The walk's empty case: a fieldless struct bakes `fields: &[]` and a **working**
/// `default_in_place` (gate 4's retargeted third clause).
#[derive(Component, Default)]
#[component(reflect)]
pub struct Empty;

// ──────────────────────── gate 8's index-faithfulness subject ───────────────

/// A struct with one field the v1 kind table cannot classify. `PhantomData<u8>` bakes
/// `Opaque` with no accessors; the field is **not** omitted, because a shorter list
/// would make by-index access depend on which fields were skipped (D14).
///
/// # The `#[reflect(skip)]` is CORE C9's migration, and it changes nothing this gate reads
///
/// Until C9 an un-skipped `Opaque` field was accepted silently; C9 makes it the spanned
/// refusal D15 requires, so this subject would otherwise stop compiling and take all
/// sixteen tests in this file with it. D14's semantics are what keep the migration
/// invisible to [`the_walk_is_index_faithful_over_an_unclassifiable_field`]: a skipped
/// field keeps its index, its name and its `offset_of!`, and bakes exactly the same
/// `Opaque` descriptor with all four accessor slots `None`. The refusal's own fixture is
/// `reflect_compile_fail/vec_field_rejected.rs`; the accepting twin is
/// `reflect_pass/vec_field_skip_accepted.rs`.
#[derive(Component, Default)]
#[component(reflect)]
pub struct Padded {
    /// Index 0.
    pub a: u32,
    /// Index 1 — unclassifiable, and the reason this gate exists.
    #[reflect(skip)]
    pub _pd: PhantomData<u8>,
    /// Index 2 — the field whose NAME an index-shifting walk moves.
    pub b: u32,
}

// ─────── the derive's NON-STRUCT arm subject — MOVED OUT BY CORE C9 ─────────
//
// `NonStruct` — a `#[repr(u8)]` enum with two payload variants — lived here from the C7
// follow-up until C9. Its gate pinned what the `Data::Enum(_) | Data::Union(_)` arm
// ACTUALLY did: bake `TypeKind::Opaque` with `fields: &[]`, a coherent descriptor
// asserting that a type with two payload variants has no fields, which `validate` accepts
// because "has no fields" is structurally well-formed.
//
// D38 turned that acceptance into a refusal, so the subject can no longer be declared in a
// file that must compile. It moved to `reflect_compile_fail/data_carrying_enum_rejected.rs`
// and its claim moved with it — the assertion it used to make is now a blessed `.stderr`.
// Its own doc said *"C10 replaces this test rather than deleting it"*; C9 replaced it four
// rungs early, and the replacement is a refusal rather than a deletion.

// ─────────────────── the `#[reflect(no_default)]` opt-out subject ───────────

/// A type with **no `Default`**, opted out of `default_in_place` (D20). It is its own
/// proof: without the opt-out the derive's `ReflectDefault` witness would refuse to
/// compile, so a green here says the opt-out suppressed BOTH the slot and the witness.
#[derive(Component)]
#[component(reflect)]
#[reflect(no_default)]
pub struct NoDefaultAtAll {
    /// One `Prim` field.
    pub v: u32,
}

// ─────────────── the hand-baked oracle (copied from c6_nested_descend.rs) ───

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

static HAND_POINT_FIELDS: [FieldInfo; 2] = [
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

static HAND_POINT_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c7_derive_bake::Point",
    type_id_fn: point_type_id,
    size: size_of::<Point>(),
    align: align_of::<Point>(),
    fields: &HAND_POINT_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

static HAND_PLACEMENT_FIELDS: [FieldInfo; 2] = [
    FieldInfo {
        name: "at",
        offset: offset_of!(Placement, at),
        type_id_fn: point_type_id,
        kind: ValueKind::Nested,
        get: None,
        set: None,
        nested: Some(&HAND_POINT_TYPE_INFO),
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

static HAND_PLACEMENT_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c7_derive_bake::Placement",
    type_id_fn: placement_type_id,
    size: size_of::<Placement>(),
    align: align_of::<Placement>(),
    fields: &HAND_PLACEMENT_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

static HAND_BODY_FIELDS: [FieldInfo; 2] = [
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
        nested: Some(&HAND_PLACEMENT_TYPE_INFO),
        enum_info: None,
        array: None,
    },
];

static HAND_BODY_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c7_derive_bake::Body",
    type_id_fn: body_type_id,
    size: size_of::<Body>(),
    align: align_of::<Body>(),
    fields: &HAND_BODY_FIELDS,
    kind: TypeKind::Struct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

static HAND_HANDLE_FIELDS: [FieldInfo; 1] = [FieldInfo {
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

static HAND_HANDLE_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c7_derive_bake::Handle",
    type_id_fn: handle_type_id,
    size: size_of::<Handle>(),
    align: align_of::<Handle>(),
    fields: &HAND_HANDLE_FIELDS,
    kind: TypeKind::TupleStruct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

static HAND_SLOT_FIELDS: [FieldInfo; 1] = [FieldInfo {
    name: "0",
    offset: 0,
    type_id_fn: handle_type_id,
    kind: ValueKind::Nested,
    get: None,
    set: None,
    nested: Some(&HAND_HANDLE_TYPE_INFO),
    enum_info: None,
    array: None,
}];

static HAND_SLOT_TYPE_INFO: TypeInfo = TypeInfo {
    type_name: "c7_derive_bake::Slot",
    type_id_fn: slot_type_id,
    size: size_of::<Slot>(),
    align: align_of::<Slot>(),
    fields: &HAND_SLOT_FIELDS,
    kind: TypeKind::TupleStruct,
    enum_info: None,
    default_in_place: None,
    drop_in_place: None,
};

// ─────────────────────────────── helpers ────────────────────────────────────

/// The five (derived, hand-baked) pairs gate 1 compares.
fn oracle_pairs() -> [(&'static TypeInfo, &'static TypeInfo); 5] {
    [
        (<Body as Reflect>::TYPE_INFO, &HAND_BODY_TYPE_INFO),
        (<Placement as Reflect>::TYPE_INFO, &HAND_PLACEMENT_TYPE_INFO),
        (<Point as Reflect>::TYPE_INFO, &HAND_POINT_TYPE_INFO),
        (<Slot as Reflect>::TYPE_INFO, &HAND_SLOT_TYPE_INFO),
        (<Handle as Reflect>::TYPE_INFO, &HAND_HANDLE_TYPE_INFO),
    ]
}

/// Every descriptor this file's derive produced — gate 7's subject set.
///
/// It was 16 until CORE C9: `NonStruct`, the derive's non-struct arm, was added at the C7
/// follow-up because until then that arm was outside every sweep in the file, and it left
/// with D38's refusal. The arm itself is still swept — a fieldless `#[repr(Int)]` enum is
/// still accepted through it — but no such subject is declared here, so what this array
/// covers is the struct half. The enum half's coverage is the corpus's.
fn every_derived_descriptor() -> [&'static TypeInfo; 15] {
    [
        <Body as Reflect>::TYPE_INFO,
        <Placement as Reflect>::TYPE_INFO,
        <Point as Reflect>::TYPE_INFO,
        <Slot as Reflect>::TYPE_INFO,
        <Handle as Reflect>::TYPE_INFO,
        <SwapAb as Reflect>::TYPE_INFO,
        <SwapBa as Reflect>::TYPE_INFO,
        <Pod as Reflect>::TYPE_INFO,
        <NestedPod as Reflect>::TYPE_INFO,
        <ArrPack as Reflect>::TYPE_INFO,
        <Owned as Reflect>::TYPE_INFO,
        <NestedOwned as Reflect>::TYPE_INFO,
        <Empty as Reflect>::TYPE_INFO,
        <Padded as Reflect>::TYPE_INFO,
        <NoDefaultAtAll as Reflect>::TYPE_INFO,
    ]
}

/// Offsets in **declaration order** — the vector gate 3's non-vacuity clause compares.
fn declaration_order_offsets(info: &'static TypeInfo) -> Vec<usize> {
    info.fields.iter().map(|f| f.offset).collect()
}

// ────────────────────────────────── gate 1 ──────────────────────────────────

/// **Gate 1 — derived == hand-baked**, field for field: names, offsets, kinds,
/// `fields.len()` and accessor presence.
///
/// This is the gate C7's first RED dies on, and it is only a gate at all because its
/// oracle is reachable: `Body.placement = 4` and `Placement.layer = 8` are the non-zero
/// offsets the rung's original subject set (every type one field wide) did not have.
#[test]
fn derived_matches_the_hand_baked_oracle_field_for_field() {
    for (derived, hand) in oracle_pairs() {
        assert_eq!(
            derived.fields.len(),
            hand.fields.len(),
            "`{}`: the derive baked {} field(s), the oracle has {}",
            hand.type_name,
            derived.fields.len(),
            hand.fields.len()
        );
        for (i, (d, h)) in derived.fields.iter().zip(hand.fields.iter()).enumerate() {
            assert_eq!(d.name, h.name, "`{}` field #{i}: name", hand.type_name);
            assert_eq!(
                d.offset, h.offset,
                "`{}` field #{i} `{}`: derived offset {} != oracle offset {}",
                hand.type_name, h.name, d.offset, h.offset
            );
            assert_eq!(d.kind, h.kind, "`{}` field #{i} `{}`: kind", hand.type_name, h.name);
            assert_eq!(
                (d.get.is_some(), d.set.is_some(), d.nested.is_some(), d.array.is_some()),
                (h.get.is_some(), h.set.is_some(), h.nested.is_some(), h.array.is_some()),
                "`{}` field #{i} `{}`: accessor presence (get, set, nested, array)",
                hand.type_name,
                h.name
            );
            assert_eq!(
                (d.type_id_fn)(),
                (h.type_id_fn)(),
                "`{}` field #{i} `{}`: field type identity",
                hand.type_name,
                h.name
            );
        }
    }
}

/// **Gate 1's added clause — accessor IDENTITY, which nothing pinned on either side.**
///
/// A descriptor carrying `get: Some(prim::get_u32)` on an `f32` field is *coherent* and
/// green across `validate` and across the presence comparison above. The only check that
/// says "the derive picked the accessor for THIS field's type" is a round trip: write a
/// known [`Scalar`] through the derived `set`, read it back through the derived `get`,
/// and compare both against a direct read of the field itself.
#[test]
fn every_derived_prim_accessor_round_trips_against_a_direct_read() {
    let mut body =
        Body { hp: 1.0, placement: Placement { at: Point { x: 2.0, y: 3.0 }, layer: 4 } };

    let hp = &<Body as Reflect>::TYPE_INFO.fields[0];
    let set_hp = hp.set.expect("`hp` is a Prim");
    let get_hp = hp.get.expect("`hp` is a Prim");
    let base = (&raw mut body).cast::<u8>();
    // SAFETY: `base` is a live, initialized, correctly aligned `Body` this frame owns
    // and does not otherwise touch while the raw writes run; `hp.offset` is `Body`'s own
    // `offset_of!`, so `base.add(offset)` is in bounds and field-aligned with the
    // frame's provenance. No reference into `body` is live across these calls.
    unsafe {
        assert!(set_hp(base.add(hp.offset), Scalar::from(9.5f32)), "set must accept an F32");
        assert_eq!(get_hp(base.add(hp.offset)), Scalar::from(9.5f32));
    }
    assert_eq!(body.hp, 9.5, "the direct read must see what the derived setter wrote");

    let layer = &<Placement as Reflect>::TYPE_INFO.fields[1];
    let set_layer = layer.set.expect("`layer` is a Prim");
    let get_layer = layer.get.expect("`layer` is a Prim");
    let pbase = (&raw mut body.placement).cast::<u8>();
    // SAFETY: as above, for the live `Placement` at `body.placement` and `Placement`'s
    // own `offset_of!(Placement, layer)`.
    unsafe {
        assert!(
            set_layer(pbase.add(layer.offset), Scalar::from(77u32)),
            "set must accept a U32"
        );
        assert_eq!(get_layer(pbase.add(layer.offset)), Scalar::from(77u32));
    }
    assert_eq!(body.placement.layer, 77);

    // The kind guard is the other half of "the right accessor": a U32 written into an
    // F32 field must be REFUSED before memory is touched (D11).
    // SAFETY: same pointer contract as above.
    unsafe {
        assert!(
            !set_hp(base.add(hp.offset), Scalar::from(1u32)),
            "`hp` is an F32 field; a U32 scalar must be refused, not stored"
        );
    }
    assert_eq!(body.hp, 9.5, "a refused set must not have touched memory");
}

/// The copied oracle describes the real types. A comparison target that has rotted into
/// agreement with a wrong derive is not a target, so its numbers are the compiler's own.
#[test]
fn the_hand_baked_oracle_describes_the_real_types() {
    assert_eq!((HAND_BODY_TYPE_INFO.type_id_fn)(), TypeId::of::<Body>());
    assert_eq!(HAND_BODY_TYPE_INFO.size, size_of::<Body>());
    assert_eq!(HAND_BODY_TYPE_INFO.align, align_of::<Body>());
    assert_eq!(HAND_BODY_FIELDS[0].offset, offset_of!(Body, hp));
    assert_eq!(HAND_BODY_FIELDS[1].offset, offset_of!(Body, placement));

    assert_eq!((HAND_PLACEMENT_TYPE_INFO.type_id_fn)(), TypeId::of::<Placement>());
    assert_eq!(HAND_PLACEMENT_FIELDS[0].offset, offset_of!(Placement, at));
    assert_eq!(HAND_PLACEMENT_FIELDS[1].offset, offset_of!(Placement, layer));

    assert_eq!((HAND_POINT_TYPE_INFO.type_id_fn)(), TypeId::of::<Point>());
    assert_eq!(HAND_POINT_FIELDS[0].offset, offset_of!(Point, x));
    assert_eq!(HAND_POINT_FIELDS[1].offset, offset_of!(Point, y));

    assert_eq!(HAND_SLOT_TYPE_INFO.size, size_of::<Slot>());
    assert_eq!(HAND_HANDLE_TYPE_INFO.size, size_of::<Handle>());

    // The RED-1 subjects are non-zero on this toolchain, or the first RED is measuring
    // nothing. This is the clause that keeps "make every offset 0" a falsifiable
    // mutation rather than a no-op over a set of one-field types.
    assert_eq!(offset_of!(Body, placement), 4, "RED 1's first non-zero subject");
    assert_eq!(offset_of!(Placement, layer), 8, "RED 1's second non-zero subject");
}

// ────────────────────────────────── gate 2 ──────────────────────────────────

/// **Gate 2 — type identity, size, align and `TypeKind`.**
#[test]
fn derived_type_identity_size_align_and_type_kind() {
    macro_rules! check {
        ($t:ty, $kind:expr) => {{
            let ti = <$t as Reflect>::TYPE_INFO;
            assert_eq!((ti.type_id_fn)(), TypeId::of::<$t>(), "type identity");
            assert_eq!(ti.size, size_of::<$t>(), "size");
            assert_eq!(ti.align, align_of::<$t>(), "align");
            assert_eq!(ti.kind, $kind, "TypeKind");
            assert!(ti.enum_info.is_none(), "a struct carries no EnumInfo");
        }};
    }
    check!(Body, TypeKind::Struct);
    check!(Placement, TypeKind::Struct);
    check!(Point, TypeKind::Struct);
    check!(Slot, TypeKind::TupleStruct);
    check!(Handle, TypeKind::TupleStruct);
    check!(Empty, TypeKind::Struct);

    // `std::any::type_name` is NOT const on this toolchain (measured: *"`std::any::
    // type_name` is not yet stable as a const fn"*, rustc 1.97.1), so the derive bakes
    // `concat!(module_path!(), "::", stringify!(T))` — the shape every hand-baked static
    // in this campaign already uses. Diagnostics only, never a save key (D8).
    assert_eq!(<Body as Reflect>::TYPE_INFO.type_name, "c7_derive_bake::Body");
    assert_eq!(<Handle as Reflect>::TYPE_INFO.type_name, "c7_derive_bake::Handle");
}

// ────────────────────────────────── gate 3 ──────────────────────────────────

/// **Gate 3 — swapped declaration order.** Two structurally identical `#[repr(Rust)]`
/// types, both resident: a standing gate cannot be a source edit performed at gate time.
///
/// Two clauses, and the second is the one "offsets are a permutation" was reaching for:
/// every derived offset equals the compiler's own `offset_of!`, **and** the two
/// declaration-order offset vectors differ. Measured on this toolchain: `repr(Rust)`
/// lays both structs out identically by field (`first` at 4, `second` at 0), so the
/// declaration-order vectors are `[4, 0]` and `[0, 4]` — a derive baking all zeros, or
/// baking by position instead of by field, collapses them into equality.
#[test]
fn swapped_declaration_order_moves_the_derived_offsets() {
    let ab = <SwapAb as Reflect>::TYPE_INFO;
    let ba = <SwapBa as Reflect>::TYPE_INFO;

    assert_eq!(ab.fields[0].name, "first");
    assert_eq!(ab.fields[1].name, "second");
    assert_eq!(ab.fields[0].offset, offset_of!(SwapAb, first));
    assert_eq!(ab.fields[1].offset, offset_of!(SwapAb, second));

    assert_eq!(ba.fields[0].name, "second");
    assert_eq!(ba.fields[1].name, "first");
    assert_eq!(ba.fields[0].offset, offset_of!(SwapBa, second));
    assert_eq!(ba.fields[1].offset, offset_of!(SwapBa, first));

    let ab_offsets = declaration_order_offsets(ab);
    let ba_offsets = declaration_order_offsets(ba);
    assert_ne!(
        ab_offsets, ba_offsets,
        "swapping the declaration order did NOT move the baked offsets ({ab_offsets:?} \
         vs {ba_offsets:?}) -- the derive is not reading the compiler's layout"
    );
}

// ────────────────────────────────── gate 4 ──────────────────────────────────

/// **Gate 4 — the drop count, both directions, over a subject that DROPS (D24).**
///
/// The gate CALLS both slots on a [`MaybeUninit`] destination it owns: C7 emits no
/// install, so a slot nobody calls is a datum written and never read — the class this
/// campaign has now found five times.
///
/// * `default_in_place` into uninitialized bytes ⇒ **+0** drops (the double-free half);
/// * the finished value's own drop, through `drop_in_place` ⇒ **exactly +1** (the leak
///   half — a count that is too low is a leak, and only an exact expectation sees it).
#[test]
fn default_in_place_writes_without_dropping_and_drop_in_place_runs_exactly_once() {
    let ti = <Owned as Reflect>::TYPE_INFO;
    let default_in_place = ti.default_in_place.expect("`Owned` derives `Default`");

    let mut slot = MaybeUninit::<Owned>::uninit();
    let p = slot.as_mut_ptr().cast::<u8>();

    let before = drops();
    // SAFETY: `p` is writable for `size_of::<Owned>()`, aligned to `align_of::<Owned>()`
    // (it is a `MaybeUninit<Owned>` this frame owns), and holds NO initialized value, so
    // no drop glue is owed on it -- exactly `default_in_place`'s documented contract.
    unsafe { default_in_place(p) };
    let after_write = drops();
    assert_eq!(
        after_write - before,
        0,
        "`default_in_place` ran drop glue on the DESTINATION: it must write into \
         uninitialized bytes only (a drop here is the double-free half)"
    );

    let before_drop = drops();
    if let Some(drop_in_place) = ti.drop_in_place {
        // SAFETY: `p` now holds the live, initialized `Owned` the call above wrote; this
        // frame owns it and never reads it again (the `MaybeUninit` is not assumed init
        // and drops nothing of its own).
        unsafe { drop_in_place(p) };
    }
    let after_drop = drops();
    assert_eq!(
        after_drop - before_drop,
        1,
        "the finished value cost {} drop(s) through `drop_in_place`, not exactly 1 -- \
         0 is a leak (the slot was baked `None` for a type that owns drop glue), and \
         more than 1 is a double free",
        after_drop - before_drop
    );
    assert_eq!(after_drop - before, 1, "one default + one drop must total exactly one drop");
}

/// The same protocol one level down: drop glue reaches through a nest.
#[test]
fn a_nested_owner_drops_exactly_once_through_the_derived_slots() {
    let ti = <NestedOwned as Reflect>::TYPE_INFO;
    let default_in_place = ti.default_in_place.expect("`NestedOwned` derives `Default`");

    let mut slot = MaybeUninit::<NestedOwned>::uninit();
    let p = slot.as_mut_ptr().cast::<u8>();

    let before = drops();
    // SAFETY: as in `default_in_place_writes_without_dropping_...`, for `NestedOwned`.
    unsafe { default_in_place(p) };
    assert_eq!(drops() - before, 0, "`default_in_place` must not drop the destination");

    let before_drop = drops();
    let drop_in_place = ti
        .drop_in_place
        .expect("`NestedOwned` owns an `Owned`, so it HAS drop glue and the slot must be Some");
    // SAFETY: `p` holds the live `NestedOwned` written above; this frame owns it and
    // never reads it again.
    unsafe { drop_in_place(p) };
    assert_eq!(drops() - before_drop, 1, "the nested owner must drop exactly once");
}

/// The drop-free half of gate 4's set: `needs_drop` is false, so the slot is `None` —
/// and `default_in_place` still works and still costs no drops.
#[test]
fn drop_free_subjects_bake_no_drop_slot_and_still_default_in_place() {
    macro_rules! check_drop_free {
        ($t:ty) => {{
            let ti = <$t as Reflect>::TYPE_INFO;
            assert!(
                ti.drop_in_place.is_none(),
                "`{}` has no drop glue, so `drop_in_place` must be None",
                ti.type_name
            );
            let default_in_place = ti.default_in_place.expect("derives Default");
            let mut slot = MaybeUninit::<$t>::uninit();
            let before = drops();
            // SAFETY: `slot` is an owned, correctly aligned, uninitialized destination of
            // exactly this type; no drop glue is owed on it.
            unsafe { default_in_place(slot.as_mut_ptr().cast::<u8>()) };
            assert_eq!(drops() - before, 0, "a drop-free default must run no drop glue");
        }};
    }
    check_drop_free!(Pod);
    check_drop_free!(NestedPod);
    check_drop_free!(ArrPack);
}

/// Gate 4's retargeted third clause: the walk's **empty** case. A fieldless struct bakes
/// `fields: &[]` and a working `default_in_place` — checked where the walk is, rather
/// than as a `default_in_place` property (C7 bakes `ptr::write(p, T::default())`, which
/// never reads `fields` at all).
#[test]
fn a_fieldless_struct_bakes_an_empty_field_list_and_a_working_default() {
    let ti = <Empty as Reflect>::TYPE_INFO;
    assert!(ti.fields.is_empty(), "a fieldless struct's field list is empty, not absent");
    let default_in_place = ti.default_in_place.expect("`Empty` derives `Default`");
    let mut slot = MaybeUninit::<Empty>::uninit();
    // SAFETY: `slot` is an owned, uninitialized `Empty` destination; a ZST write is
    // trivially in bounds and aligned, and no drop glue is owed.
    unsafe { default_in_place(slot.as_mut_ptr().cast::<u8>()) };
    assert!(validate(ti).is_ok(), "the empty case must validate clean");
}

/// `#[reflect(no_default)]` suppresses the slot **and** the `ReflectDefault` witness.
///
/// The subject is its own proof: [`NoDefaultAtAll`] implements no `Default`, so without
/// the opt-out this file would not compile at all — D20's witness would refuse it with
/// `ReflectDefault`'s `on_unimplemented` message. The rung's `Lands` carries the opt-out
/// and its gate table carried no clause for it; an emission nothing reads is the
/// dead-datum class, so the clause is added here rather than deferred to C9.
#[test]
fn no_default_opts_out_of_the_default_slot() {
    let ti = <NoDefaultAtAll as Reflect>::TYPE_INFO;
    assert!(
        ti.default_in_place.is_none(),
        "`#[reflect(no_default)]` must bake `default_in_place: None`"
    );
    assert_eq!(ti.fields.len(), 1, "the opt-out changes the default slot, not the walk");
    assert_eq!(ti.fields[0].name, "v");
}

/// `ReflectDefault` is a real, reachable item of `boyko_reflect` (D22) — not a fenced
/// sketch inside a decision. Its blanket impl covers every `Default` type.
#[test]
fn reflect_default_is_a_landed_trait_with_a_blanket_impl() {
    fn assert_reflect_default<T: ReflectDefault>() {}
    assert_reflect_default::<Pod>();
    assert_reflect_default::<Owned>();
    assert_reflect_default::<u32>();
}

// ────────────────────────────────── gate 6 ──────────────────────────────────

/// **Gate 6 — a tuple struct's field names are the decimal indices.**
///
/// Both levels are `#[repr(transparent)]` and both fields sit at offset 0 (measured), so
/// this is a naming-and-`TypeKind` gate, never an offset one.
#[test]
fn a_tuple_struct_bakes_decimal_names_and_the_tuple_struct_kind() {
    let handle = <Handle as Reflect>::TYPE_INFO;
    assert_eq!(handle.kind, TypeKind::TupleStruct);
    assert_eq!(handle.fields.len(), 1);
    assert_eq!(handle.fields[0].name, "0", "a tuple struct's field name is its index");
    assert_eq!(handle.fields[0].offset, 0);
    assert_eq!(handle.fields[0].kind, ValueKind::Prim(ScalarKind::U32));

    let slot = <Slot as Reflect>::TYPE_INFO;
    assert_eq!(slot.kind, TypeKind::TupleStruct);
    assert_eq!(slot.fields[0].name, "0");
    assert_eq!(slot.fields[0].kind, ValueKind::Nested);
    assert!(
        std::ptr::eq(slot.fields[0].nested.expect("Nested carries a child"), handle),
        "the nested edge must point at the CHILD's own static -- one stable address per \
         type is what C6's address-keyed acyclicity walk rests on"
    );
}

/// One stable address per type (D22), **within this crate** — which is all this file can
/// see, and less than the property is worth.
///
/// # This clause cannot fail, and that is recorded rather than fixed here
///
/// Its original doc said a `const` "is const-promoted afresh at each `&`-site, which
/// would give C6's Check B a new address every time it asked whether it had seen a type
/// before". **Measured 2026-08-21: false on this emission, and the reason this test looked
/// sufficient.** The derive's expansion contains exactly *one* `&__REFLECT_TYPE_INFO`, so
/// a `const` descriptor's address is stable within a crate. Substituting `const` for both
/// `static`s in `boyko_macros/src/reflect.rs` leaves every test in this file green —
/// sixteen of sixteen, this one and gate 6's two `ptr::eq` clauses included.
///
/// The property is real and violable; the **subject set** here is what cannot express it.
/// `reflect_fixture` has no `src/lib.rs`, so every annotated type it owns is a private
/// item of the very binary that reads it and no consumer exists to disagree. The
/// divergence is at the crate boundary — each crate evaluating an associated `const`
/// interns its own copy of the anonymous allocation — so the falsifying gate lives in
/// `reflect_dogfood/tests/c7_cross_crate_address.rs`, where the type is defined in a
/// library and read from a test crate.
///
/// What remains here is still worth running: it pins the *same-crate* half (a nested edge
/// and the child's own descriptor are one address), which is what C6's walk relies on
/// when a whole graph comes from one crate.
#[test]
fn a_types_descriptor_has_exactly_one_address_within_this_crate() {
    let a: *const TypeInfo = <Point as Reflect>::TYPE_INFO;
    let b: *const TypeInfo = <Point as Reflect>::TYPE_INFO;
    assert!(std::ptr::eq(a, b), "two references to TYPE_INFO must be the same address");
    assert!(
        std::ptr::eq(
            <Placement as Reflect>::TYPE_INFO.fields[0].nested.expect("`at` is Nested"),
            <Point as Reflect>::TYPE_INFO
        ),
        "a nested edge and the child's own TYPE_INFO must be one address"
    );
}

// ────────────────────────────────── gate 7 ──────────────────────────────────

/// **Gate 7 (D25) — `validate` over EVERY derived descriptor.**
///
/// C7 is the first rung whose descriptors are machine-made, and the rung's own table
/// called the coherence oracle C3 built for exactly this nowhere. Without this, a derive
/// emitting `Nested` with `nested: None`, or `Prim` with `get: Some(..)` and
/// `set: None`, is green across every other gate here.
#[test]
fn every_derived_descriptor_is_coherent() {
    for info in every_derived_descriptor() {
        if let Err(problems) = validate(info) {
            let lines: Vec<String> = problems.iter().map(ToString::to_string).collect();
            panic!("derived `{}` is INCOHERENT:\n  {}", info.type_name, lines.join("\n  "));
        }
    }
}

// ────────────────────────────────── gate 8 ──────────────────────────────────

/// **Gate 8 (D25) — the walk is index-faithful.**
///
/// D14 forbids omitting a field, because by-index access would then depend on which
/// fields were skipped. The natural implementation — `fields.iter().filter(|f| /* has a
/// ValueKind */ …)` — keeps every surviving field's name and offset **right** and shifts
/// only the INDEX, and nothing else in this table could see that.
#[test]
fn the_walk_is_index_faithful_over_an_unclassifiable_field() {
    let ti = <Padded as Reflect>::TYPE_INFO;
    assert_eq!(
        ti.fields.len(),
        3,
        "the walk dropped a field it could not classify -- `fields.len()` must equal the \
         DECLARED field count, unconditionally (D14)"
    );
    assert_eq!(ti.fields[0].name, "a");
    assert_eq!(ti.fields[0].kind, ValueKind::Prim(ScalarKind::U32));

    assert_eq!(ti.fields[1].name, "_pd");
    assert_eq!(
        ti.fields[1].kind,
        ValueKind::Opaque,
        "an unclassifiable field bakes `Opaque`, never a shorter list"
    );
    assert!(
        ti.fields[1].get.is_none()
            && ti.fields[1].set.is_none()
            && ti.fields[1].nested.is_none()
            && ti.fields[1].array.is_none(),
        "an `Opaque` field has NO accessor to call, therefore no code path (§3.2)"
    );

    assert_eq!(
        ti.fields[2].name, "b",
        "field #2 is `b` -- a walk that filtered the middle field would put `b` at index 1 \
         with its name and offset still RIGHT, which is the whole defect"
    );
    assert_eq!(ti.fields[2].offset, offset_of!(Padded, b));
}

// ───── the non-struct arm gate — MOVED TO THE CORPUS BY CORE C9 (D38) ──────
//
// `the_non_struct_arm_bakes_an_opaque_fieldless_descriptor` asserted that a
// `#[component(reflect)]` enum with two payload variants bakes `TypeKind::Opaque` with
// `fields: &[]`, and that `validate` accepts it. That was a pin on a COHERENT LIE, kept
// because an unpinned accidental behaviour is worse than a pinned wrong one. D38 refuses
// the input instead, so the same claim is now carried by a blessed `.stderr`
// (`reflect_compile_fail/data_carrying_enum_rejected.rs`) rather than by an assertion --
// moved from *"this is what it bakes"* to *"this does not compile"*.
//
// What is NOT lost: the arm still runs for a fieldless `#[repr(Int)]` enum, which stays
// ACCEPTED because its `fields: &[]` is true, until C10 replaces its `kind` with
// `TypeKind::Enum`.
//
// ⚠️ That sentence was PROSE ONLY until `reflect_pass/fieldless_repr_enum_accepted.rs`.
// Moving the subject out of this file left `has_integer_repr` returning **true** reached by
// no test in the tree: MEASURED, forcing it to `return false` unconditionally left the
// refusal corpus, this file and the census all green, so an over-broad enum refusal would
// have shipped while three documents went on claiming the shape was accepted. The claim is
// now a `t.pass()` fixture, which is where it belongs -- the arm is not exercised here any
// more, and a comment is not a gate.

/// `[T; N]` of a `Prim` bakes `ValueKind::Array` with the element kind, the element's
/// own `size_of` as the stride, and `N` (D12/D19).
#[test]
fn an_array_of_prim_bakes_its_element_kind_stride_and_length() {
    let ti = <ArrPack as Reflect>::TYPE_INFO;
    assert_eq!(ti.fields.len(), 1);
    assert_eq!(ti.fields[0].name, "data");
    assert_eq!(ti.fields[0].kind, ValueKind::Array);
    let array = ti.fields[0].array.expect("an Array field carries an ArrayInfo");
    assert_eq!(array.elem, ScalarKind::F32);
    assert_eq!(array.stride, size_of::<f32>(), "a stride from the wrong size_of reads every element but the first from the wrong address");
    assert_eq!(array.len, 4);
    assert!(
        ti.fields[0].get.is_none() && ti.fields[0].set.is_none(),
        "element access is by index (C5), never through the scalar accessors"
    );
}
