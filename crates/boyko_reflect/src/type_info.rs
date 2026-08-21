//! The reflection data model (CORE C3, taxonomy §3): [`TypeInfo`], [`FieldInfo`] and
//! the four descriptors they point at.
//!
//! **The model must be shown expressible BEFORE a macro is asked to emit it.** Nothing
//! here is generated; C3's gates walk `TYPE_INFO` statics written by hand
//! (`tests/c3_type_info.rs`), and C7's derive inherits a shape that is already pinned by
//! an independent comparison target rather than by its own output.
//!
//! # What the model deliberately does NOT carry
//!
//! * **No `stable_name`** (CORE D8) — it already exists as `SerializeInfo.stable_name`
//!   in the kernel, with a *serialization wire format* on the other end; a second
//!   declaration would be a drift pair. The save key is read from
//!   `get_serialize_info(id)`.
//! * **No `serialize` slot, no `debug_fmt` slot** (CORE D9) — neither has a reader in
//!   CORE's scope, and this campaign has recorded five instances of the *dead datum*
//!   class. A datum lands in the same commit as its first reader.
//!
//! # Every accessor is an `Option` (CORE D10/D20)
//!
//! There is no poison stub. "This kind has no such accessor" is a value the type system
//! carries, not a function that exists to be never called — a panicking stub would panic
//! in the release-editor build that should have *refused*, and a stub returning a zero
//! [`Scalar`] is the silent-garbage defect (analysis FIX Mi2) this shape exists to
//! refuse. `default_in_place` is no longer the one exception (D20): `None` is a real
//! state with a real consumer, ECS's `add_default` answering `Err(Refusal::NoDefault)`.
//!
//! # Coherence is CHECKED, not assumed
//!
//! [`validate`] walks a descriptor and returns every rule violation it finds, one
//! [`Problem`] per violation, over a match with **no wildcard arm** — a new
//! [`ValueKind`] fails to compile until it is classified. That exhaustiveness is the
//! whole reason the rules live in code rather than in this comment.

use std::any::TypeId;
use std::fmt;

use crate::scalar::{Scalar, ScalarKind};

// ───────────────────────────── the taxonomy (§3) ────────────────────────────

/// What a field's bytes mean, and therefore which descriptor slot on
/// [`FieldInfo`] is the live one (taxonomy §3).
///
/// [`Scalar`]'s tag doubles as the guard on `set` (CORE D11), so the `Prim` arm's
/// payload is not extra cost.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueKind {
    /// A primitive readable as one [`Scalar`]: `get`/`set` are the live slots.
    Prim(ScalarKind),
    /// `[T; N]` where `T` is a `Prim` — CORE D12/D19. `array` is the live slot;
    /// element access is by index (CORE C5), never through `get`.
    Array,
    /// A field whose own type is reflectable: `nested` is the live slot, and
    /// descent is pointer arithmetic over a `&'static` graph (§3.1).
    Nested,
    /// A fieldless `#[repr(Int)]` enum: `enum_info` is the live slot (CORE C10).
    Enum,
    /// A `String` field — built LAST (CORE D13), and until C11 lands its accessor
    /// pair a `Str` field is structurally accessorless.
    Str,
    /// Not a `Prim`, not an array of `Prim`, not a reflectable nested type, not a
    /// fieldless `#[repr(Int)]` enum (§3.2).
    ///
    /// **Unreachable from the derive except via `#[reflect(skip)]`** (CORE
    /// D14/D15): an un-skipped `Opaque` field is a hard, spanned derive error,
    /// because the wire format is shared with the shipped `boyko_serialize` and
    /// silent omission is unacceptable. That asymmetry is deliberate. An `Opaque`
    /// field has **no accessor to call, therefore no code path, therefore no
    /// allocation** — in v1 there is no `Opaque` path at all, only an `Opaque`
    /// label. *If v2 ever gives it a payload-producing accessor, §3.3's allocation
    /// audit must be re-run against it, and this sentence is where the next author
    /// sees that before the allocator does.*
    Opaque,
}

/// The shape of the whole reflected type (taxonomy §3).
///
/// `Enum` is a real top-level case, not only a field case: `Visibility`
/// (`boyko_scene/src/render_caps.rs`) is a fieldless `#[repr(u8)]` enum that **is** a
/// Component, so `fields` is `&[]` and the value is reached through
/// [`TypeInfo::enum_info`] instead (CORE C10).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeKind {
    /// A named-field struct.
    Struct,
    /// A tuple struct; field names are the decimal indices.
    TupleStruct,
    /// A fieldless `#[repr(Int)]` enum.
    Enum,
    /// A type the model cannot describe (§3.2).
    Opaque,
}

/// The `[T; N]`-of-`Prim` descriptor (CORE D12, built out at C5).
///
/// Offset + stride + count, all `const`: zero alloc, no drop, no Tree-Borrows
/// exposure. Arrays *of arrays* are v2 (CORE D19); the named exclusion is
/// `csm_config.rs`'s `view_proj: [[f32;4];4]`, which is refused rather than
/// silently flattened.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArrayInfo {
    /// The element's scalar kind — `T` is a `Prim` and only that (D19).
    pub elem: ScalarKind,
    /// `size_of::<T>()`. Pinned against `offset_of!`-derived spacing at C5 gate 4,
    /// because a `stride` from the wrong `size_of` is the derive bug that reads
    /// every element but the first from the wrong address.
    pub stride: usize,
    /// `N`.
    pub len: usize,
}

/// The integer representation a fieldless enum's discriminant is stored in.
///
/// Load-bearing on the read side: an `Ix` repr sign-extends out of
/// [`VariantInfo::discr_bits`], exactly as [`Scalar`]'s signed kinds do.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnumRepr {
    /// `#[repr(u8)]`.
    U8,
    /// `#[repr(u16)]`.
    U16,
    /// `#[repr(u32)]`.
    U32,
    /// `#[repr(u64)]`.
    U64,
    /// `#[repr(i8)]`, sign-extended.
    I8,
    /// `#[repr(i16)]`, sign-extended.
    I16,
    /// `#[repr(i32)]`, sign-extended.
    I32,
    /// `#[repr(i64)]`, sign-extended.
    I64,
}

/// One variant of a fieldless `#[repr(Int)]` enum.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VariantInfo {
    /// The variant's Rust name — the inspector's label and the wire key.
    pub name: &'static str,
    /// The discriminant **already narrowed to the repr width** (analysis FIX
    /// C2/O1): baked as the repr's own bit pattern, never as a lossy
    /// `i128 as u64` performed at the call site.
    pub discr_bits: u64,
}

/// A fieldless `#[repr(Int)]` enum's variant table (CORE C10).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnumInfo {
    /// Which integer the discriminant is stored in, and whether reads sign-extend.
    pub repr: EnumRepr,
    /// The variants, in declaration order.
    pub variants: &'static [VariantInfo],
}

// ─────────────────────────── the descriptors ────────────────────────────────

/// One field of a reflected type: where it is, what it means, and how (or whether)
/// it can be read and written.
///
/// Every slot beyond `name`/`offset`/`type_id_fn`/`kind` is conditional on `kind`,
/// and the conditions are [`validate`]'s rules rather than prose.
#[repr(C)]
#[derive(Debug)]
pub struct FieldInfo {
    /// The field's Rust name.
    ///
    /// **Load-bearing, not strippable:** it is the deserialize key. A build that
    /// drops it does not lose a diagnostic, it loses the ability to read a save.
    pub name: &'static str,
    /// Byte offset from the containing value's base, baked from
    /// `core::mem::offset_of!` (CORE F12 — the idiom is load-bearing engine-wide,
    /// including as `const _: () = assert!(offset_of!(…) == N)` layout pins).
    pub offset: usize,
    /// The field type's [`TypeId`], behind an `fn` pointer because `TypeId::of` is
    /// **not** `const` and therefore cannot be a baked static field.
    pub type_id_fn: fn() -> TypeId,
    /// What the bytes mean, and which slot below is live.
    pub kind: ValueKind,
    /// Reads the field as one [`Scalar`]. `Some` only for `Prim`.
    ///
    /// # Safety
    ///
    /// The pointer handed to this fn must be `base.add(offset)` for a live,
    /// initialized instance of the type this `FieldInfo` was baked for.
    pub get: Option<unsafe fn(*const u8) -> Scalar>,
    /// Writes one [`Scalar`] into the field, returning `false` — **before touching
    /// memory** — when the scalar's kind does not match the field's (CORE D11: a
    /// release `bool`, never a `debug_assert!`). `Some` only for `Prim`.
    ///
    /// # Safety
    ///
    /// Same contract as [`FieldInfo::get`], with write permission.
    pub set: Option<unsafe fn(*mut u8, Scalar) -> bool>,
    /// The inner type's own `TYPE_INFO`. `Some` only for `Nested` — and it is a
    /// *pointer*, never a flattened path table, which is why derive-time recursion
    /// is depth 1 and no proc-macro recursion exists (§3.1).
    pub nested: Option<&'static TypeInfo>,
    /// The variant table. `Some` only for `Enum`.
    pub enum_info: Option<&'static EnumInfo>,
    /// The element descriptor. `Some` only for `Array`.
    pub array: Option<ArrayInfo>,
}

/// A reflected type's whole descriptor: the value one `REFLECT` slot publishes.
#[repr(C)]
#[derive(Debug)]
pub struct TypeInfo {
    /// `std::any::type_name` — **diagnostics only**. It is not stable across
    /// compilations and is never a save key (CORE D8).
    pub type_name: &'static str,
    /// The type's [`TypeId`], behind an `fn` pointer (`TypeId::of` is not `const`).
    pub type_id_fn: fn() -> TypeId,
    /// `size_of::<T>()`.
    pub size: usize,
    /// `align_of::<T>()`.
    pub align: usize,
    /// The fields, in declaration order, **including `#[reflect(skip)]` ones**
    /// (CORE D14: an inspector that shows nothing for a field is honest; one that
    /// shows a shorter list is lying — and a shorter list would make the by-index
    /// API's indices depend on which fields were skipped).
    ///
    /// `&'static`, baked: enumeration is a slice read, never a `Vec` build.
    pub fields: &'static [FieldInfo],
    /// The shape of the type itself.
    pub kind: TypeKind,
    /// The variant table — `Some` **iff** `kind == TypeKind::Enum` (CORE C10).
    pub enum_info: Option<&'static EnumInfo>,
    /// Writes `T::default()` into the pointed-at bytes. `None` when the type has no
    /// `Default` (CORE D20) — a real state with a real consumer, not a hole.
    ///
    /// # Safety
    ///
    /// The pointer must be writable for `size`, aligned to `align`, and must not
    /// already hold an initialized value whose drop glue is owed.
    pub default_in_place: Option<unsafe fn(*mut u8)>,
    /// Runs `T`'s drop glue in place. `None` when `T: Copy` / has no glue.
    ///
    /// # Safety
    ///
    /// The pointer must hold a live, initialized `T` that the caller owns and will
    /// not read again.
    pub drop_in_place: Option<unsafe fn(*mut u8)>,
}

impl TypeInfo {
    /// Reads field `index` as one [`Scalar`] — the scalar read API (CORE D10).
    ///
    /// Returns `None` for an out-of-range index, for **any non-`Prim` kind**, and
    /// for a `Prim` whose `get` slot is empty. The kind is checked **first and
    /// independently of the accessor slot**: a malformed descriptor (a `Nested`
    /// field carrying a scalar getter — exactly C3's first RED mutation) must
    /// refuse rather than reinterpret a nested struct's first bytes as an `f32`.
    /// That ordering is what makes this gate independent of [`validate`]'s instead
    /// of a second reading of it.
    ///
    /// # Safety
    ///
    /// `base` must point at a live, initialized instance of the type this
    /// `TypeInfo` describes, aligned to [`TypeInfo::align`], valid for reads of
    /// [`TypeInfo::size`] bytes, with provenance covering the whole value — the
    /// same contract `offset_of!`-derived field arithmetic needs to stay in
    /// bounds. The value must not be concurrently written.
    #[inline]
    pub unsafe fn get_field(&self, base: *const u8, index: usize) -> Option<Scalar> {
        let field = self.fields.get(index)?;
        if !matches!(field.kind, ValueKind::Prim(_)) {
            return None;
        }
        let get = field.get?;
        // SAFETY: the caller guarantees `base` is a live, initialized, `align`-aligned
        // instance of this type, valid for `size` bytes with whole-value provenance;
        // `field.offset` is an `offset_of!` of that same type, so `base.add(offset)` is
        // in bounds, field-aligned and inherits the caller's provenance. `get` was baked
        // for this field's type, and the kind check above proved the field is a `Prim`,
        // so the accessor reads exactly the scalar it was installed for.
        Some(unsafe { get(base.add(field.offset)) })
    }
}

// ────────────────────────── coherence validation ────────────────────────────

/// A single coherence rule broken by one descriptor slot ([`validate`]).
///
/// Each variant names the *rule*, not the symptom, so a red says which invariant
/// the descriptor violated rather than which assertion happened to notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Violation {
    /// `Prim` without a `get` accessor: the field claims to be scalar-readable and
    /// is not.
    PrimWithoutGet,
    /// `Prim` without a `set` accessor.
    PrimWithoutSet,
    /// `Prim` carrying a `nested` descriptor.
    PrimWithNestedDescriptor,
    /// `Prim` carrying an `array` descriptor.
    PrimWithArrayDescriptor,
    /// `Nested` without the inner type's `TypeInfo` — nothing to descend into.
    NestedWithoutTypeInfo,
    /// `Nested` carrying a scalar accessor: a struct is not a [`Scalar`], and an
    /// accessor here would reinterpret its first bytes (analysis FIX Mi2).
    NestedWithScalarAccessor,
    /// `Array` without an [`ArrayInfo`] — no stride, no length, no element access.
    ArrayWithoutArrayInfo,
    /// `Array` carrying a scalar accessor: elements are reached by index (C5), not
    /// through `get`.
    ArrayWithScalarAccessor,
    /// `Enum` without an [`EnumInfo`] — no variant table.
    EnumWithoutEnumInfo,
    /// `Str` carrying any accessor. The `String` arm is built LAST (CORE D13) and
    /// its accessor pair does not exist yet, so a `Str` field is structurally
    /// accessorless; C11 replaces this rule in the same commit that gives `Str`
    /// something to call.
    StrWithAccessor,
    /// `Opaque` carrying any accessor — §3.2: an `Opaque` field has no accessor,
    /// therefore no code path, therefore no allocation.
    OpaqueWithAccessor,
    /// `TypeKind::Enum` without an [`EnumInfo`] on the type.
    TypeIsEnumWithoutEnumInfo,
    /// An [`EnumInfo`] on a type whose `kind` is not `Enum` — the "`Some` **iff**"
    /// half that a one-directional check would miss.
    TypeEnumInfoOnNonEnum,
}

/// One coherence failure, located.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Problem {
    /// The offending field's index, or `None` for a rule about the type itself.
    pub field_index: Option<usize>,
    /// The offending field's name, or the type's name for a type-level rule.
    pub name: &'static str,
    /// Which rule was broken.
    pub violation: Violation,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.field_index {
            Some(i) => write!(f, "field #{i} `{}`: {:?}", self.name, self.violation),
            None => write!(f, "type `{}`: {:?}", self.name, self.violation),
        }
    }
}

/// Checks a descriptor's kind/accessor coherence (CORE C3 gate 1).
///
/// Returns **every** violation found, not the first — a descriptor with two broken
/// slots should report two, so a "fix" that silences one does not read as green.
///
/// The per-kind match carries **no wildcard arm**: adding a [`ValueKind`] fails to
/// compile here until it is classified, which is the property that keeps the rules
/// from rotting behind a new arm.
///
/// `Vec` is the natural shape for the error path and is not a hot-path allocation:
/// this is a dev-time descriptor check, run by tests and by the derive's own
/// fixtures, never on a frame.
pub fn validate(info: &TypeInfo) -> Result<(), Vec<Problem>> {
    let mut problems = Vec::new();

    let type_level = |v: Violation| Problem {
        field_index: None,
        name: info.type_name,
        violation: v,
    };
    match (info.kind, info.enum_info.is_some()) {
        (TypeKind::Enum, false) => problems.push(type_level(Violation::TypeIsEnumWithoutEnumInfo)),
        (TypeKind::Struct | TypeKind::TupleStruct | TypeKind::Opaque, true) => {
            problems.push(type_level(Violation::TypeEnumInfoOnNonEnum));
        }
        _ => {}
    }

    for (index, field) in info.fields.iter().enumerate() {
        let mut push = |v: Violation| {
            problems.push(Problem {
                field_index: Some(index),
                name: field.name,
                violation: v,
            });
        };
        let has_scalar_accessor = field.get.is_some() || field.set.is_some();

        match field.kind {
            ValueKind::Prim(_) => {
                if field.get.is_none() {
                    push(Violation::PrimWithoutGet);
                }
                if field.set.is_none() {
                    push(Violation::PrimWithoutSet);
                }
                if field.nested.is_some() {
                    push(Violation::PrimWithNestedDescriptor);
                }
                if field.array.is_some() {
                    push(Violation::PrimWithArrayDescriptor);
                }
            }
            ValueKind::Nested => {
                if field.nested.is_none() {
                    push(Violation::NestedWithoutTypeInfo);
                }
                if has_scalar_accessor {
                    push(Violation::NestedWithScalarAccessor);
                }
            }
            ValueKind::Array => {
                if field.array.is_none() {
                    push(Violation::ArrayWithoutArrayInfo);
                }
                if has_scalar_accessor {
                    push(Violation::ArrayWithScalarAccessor);
                }
            }
            ValueKind::Enum => {
                if field.enum_info.is_none() {
                    push(Violation::EnumWithoutEnumInfo);
                }
            }
            ValueKind::Str => {
                if has_scalar_accessor {
                    push(Violation::StrWithAccessor);
                }
            }
            ValueKind::Opaque => {
                if has_scalar_accessor || field.nested.is_some() || field.array.is_some() {
                    push(Violation::OpaqueWithAccessor);
                }
            }
        }
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}
