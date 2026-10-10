//! `Nested` descent, read side (CORE C6, plan §3.1): [`NestedCursor`] and
//! [`FieldValue`].
//!
//! **Descent is pointer arithmetic over a `&'static` graph.** No value tree is ever
//! materialized — that is the entire `Box<dyn>`-per-field allocation class this design
//! refuses. Descending one level is *one `add` and one pointer copy*: the child's
//! `&'static TypeInfo` already exists (the inner type's own static), so nothing is
//! built, flattened or cached. §3.3's *nested descend, depth ≥ 2* row claims **0
//! allocations per level**, and C6 gate 2 measures it at depth 2 rather than asserting
//! it at depth 1.
//!
//! # The `'a` is the validity guarantee, and it is compiler-enforced
//!
//! [`NestedCursor`] is `{ ptr, info, _pd: PhantomData<&'a ()> }`. The `'a` is not
//! decoration and it is not a documented convention: it is borrowed from the value the
//! cursor was rooted at, so a cursor **cannot** coexist with a `&mut` to that value.
//! The bare `{ptr, info}` form — a cursor with no lifetime — is what analysis M2/O3
//! calls *"deleted and never introduced"*, because it turns a use-after-free into a
//! runtime question. C6 gate 5 pins this with a `compile_fail` fixture, and C6's second
//! RED deletes the lifetime to watch that fixture start compiling.
//!
//! # What makes the arithmetic sound, and where it is proved
//!
//! `base.add(offset)` is in bounds only if the child really is inline at `offset`, and
//! the descent terminates only if the `Nested` graph is acyclic. **Neither is assumed
//! and neither is checked here**: both are [`validate`] rules (CORE D21) —
//! [`Violation::NestedNotInline`] and [`Violation::NestedCycle`] — discharged once, at
//! validation time, so the descend path carries no branch for either. Constructing a
//! cursor is `unsafe` precisely because that is where the obligation is taken on.
//!
//! [`validate`]: crate::type_info::validate
//! [`Violation::NestedNotInline`]: crate::type_info::Violation::NestedNotInline
//! [`Violation::NestedCycle`]: crate::type_info::Violation::NestedCycle

use std::marker::PhantomData;

use crate::scalar::Scalar;
use crate::type_info::{FieldInfo, TypeInfo, ValueKind};

/// A read cursor into a live value, paired with the descriptor that explains its bytes.
///
/// `Copy` and re-rootable: [`NestedCursor::descend`] returns a *new* cursor rather than
/// mutating this one, so a walk can fork at any field without bookkeeping.
///
/// # Invariant (the whole safety story, established once by [`NestedCursor::new`])
///
/// * `ptr` addresses a live, initialized instance of the type `info` describes, aligned
///   to `info.align`, valid for reads of `info.size` bytes, with provenance covering the
///   whole value;
/// * the value is not written for `'a`;
/// * `info` — and every type reachable through its `Nested` edges — passed
///   [`validate`](crate::type_info::validate), so every `Nested` offset is
///   inline-contained and the graph is acyclic (CORE D21).
///
/// Every method below is safe *because* of that invariant.
#[derive(Debug, Clone, Copy)]
pub struct NestedCursor<'a> {
    /// The value's base address. Raw rather than `&'a [u8]`: the descriptor's offsets
    /// address the whole value, and narrowing provenance to a slice of bytes would make
    /// the field arithmetic a Tree-Borrows violation at the first `add`.
    ptr: *const u8,
    /// The descriptor for the bytes at `ptr`.
    info: &'static TypeInfo,
    /// The borrow. **Load-bearing**: it is what stops a cursor from outliving the value
    /// or coexisting with a `&mut` to it (C6 gate 5).
    _pd: PhantomData<&'a ()>,
}

impl<'a> NestedCursor<'a> {
    /// Roots a cursor at `value`, explained by `info`.
    ///
    /// Takes `&'a T` rather than a raw pointer for two reasons, and both are the point
    /// of this type: the reference is where `'a` comes from — a raw-pointer constructor
    /// would leave `'a` unconstrained and the caller free to pick `'static`, which is
    /// exactly the bare-cursor shape M2/O3 refuses — and `&'a T` carries provenance over
    /// the **whole** value, which `&u8` to its first byte would not.
    ///
    /// # Safety
    ///
    /// `info` must describe `T`: `(info.type_id_fn)() == TypeId::of::<T>()`,
    /// `info.size == size_of::<T>()`, `info.align == align_of::<T>()`. And `info` must
    /// be **coherent** — [`validate`](crate::type_info::validate) returns `Ok` for it
    /// and for every type reachable through its `Nested` edges. That second clause is
    /// not paperwork: it is what makes [`descend`](Self::descend)'s `add` in bounds
    /// (Check A) and what makes a walk terminate (Check B), and it is discharged once
    /// here instead of per level.
    #[inline]
    pub unsafe fn new<T>(value: &'a T, info: &'static TypeInfo) -> Self {
        NestedCursor { ptr: std::ptr::from_ref(value).cast::<u8>(), info, _pd: PhantomData }
    }

    /// The descriptor for the bytes this cursor addresses.
    #[inline]
    pub fn type_info(&self) -> &'static TypeInfo {
        self.info
    }

    /// The fields at this level — enumeration works at **depth ≥ 1**, which is the gap
    /// a depth-0-only API leaves.
    ///
    /// A `&'static` slice read: no `Vec` is built, which is §3.3's *enumerate* row.
    #[inline]
    pub fn fields(&self) -> &'static [FieldInfo] {
        self.info.fields
    }

    /// Reads field `index` as one [`Scalar`], or `None` for an out-of-range index and
    /// for **any** non-`Prim` kind (CORE D10 / analysis FIX Mi2 — never a
    /// reinterpretation of a nested struct's first bytes).
    #[inline]
    pub fn get(&self, index: usize) -> Option<Scalar> {
        // SAFETY: the cursor invariant gives exactly `get_field`'s contract -- `ptr` is a
        // live, initialized, `info.align`-aligned instance of `info`'s type, readable for
        // `info.size` bytes with whole-value provenance, and not concurrently written for
        // `'a`.
        unsafe { self.info.get_field(self.ptr, index) }
    }

    /// Descends into field `index`, returning a cursor over the inner value.
    ///
    /// `None` for an out-of-range index, for any kind other than `Nested`, and for a
    /// `Nested` field with no descriptor. The kind is checked **first and independently
    /// of the descriptor slot**, mirroring [`TypeInfo::get_field`]'s ordering: a
    /// malformed descriptor must refuse rather than reinterpret.
    ///
    /// One `add`, one pointer copy. There is no depth counter and no cycle guard on this
    /// path *because* the graph was proved acyclic at validation time (CORE D21) — the
    /// cost of the proof is paid once, not per level.
    #[inline]
    pub fn descend(&self, index: usize) -> Option<NestedCursor<'a>> {
        let field = self.info.fields.get(index)?;
        if !matches!(field.kind, ValueKind::Nested) {
            return None;
        }
        let inner = field.nested?;
        // SAFETY: the cursor invariant includes `validate(self.info) == Ok`, whose Check A
        // proved `field.offset + inner.size <= self.info.size` with `field.offset` a
        // multiple of `inner.align` and `inner.align <= self.info.align`. So the derived
        // pointer is in bounds of the same allocation, correctly aligned for `inner`'s
        // type, and inherits the base's provenance -- and the new cursor's invariant holds
        // for `inner` exactly as this one's holds for `self.info`.
        let ptr = unsafe { self.ptr.add(field.offset) };
        Some(NestedCursor { ptr, info: inner, _pd: PhantomData })
    }

    /// Field `index` as a [`FieldValue`]: a [`Scalar`] for `Prim`, a cursor for
    /// `Nested`.
    ///
    /// `None` for the kinds whose value representation belongs to a later rung — `Array`
    /// elements are reached by index off [`ArrayInfo`](crate::type_info::ArrayInfo)
    /// (CORE C5), `Enum` at C10, `Str` at C11 — and for `Opaque`, which has no accessor
    /// at all (§3.2). **The enum carries only the arms that have a reader today**: a
    /// variant nothing constructs is the dead-datum class this campaign has recorded
    /// five instances of (CORE D9).
    #[inline]
    pub fn value(&self, index: usize) -> Option<FieldValue<'a>> {
        let field = self.info.fields.get(index)?;
        match field.kind {
            ValueKind::Prim(_) => self.get(index).map(FieldValue::Prim),
            ValueKind::Nested => self.descend(index).map(FieldValue::Nested),
            ValueKind::Array | ValueKind::Enum | ValueKind::Str | ValueKind::Opaque => None,
        }
    }
}

/// What one field *is*, for a caller that walks fields without knowing their kinds.
///
/// Two arms in CORE C6, and deliberately only two: `Prim` and `Nested` are the kinds
/// with a reader at this rung. C10's `Enum` and C11's `Str` arms land with their
/// accessors, in the commits that give them something to call.
#[derive(Debug, Clone, Copy)]
pub enum FieldValue<'a> {
    /// A primitive, already read.
    Prim(Scalar),
    /// A reflectable inner value, addressed by a cursor — **not** a materialized tree.
    Nested(NestedCursor<'a>),
}
