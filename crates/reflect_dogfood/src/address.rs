//! **CORE C7 / D22 — the cross-crate address subject.** Two annotated types, and two
//! readers that materialize their descriptors *in this crate*.
//!
//! # The property
//!
//! `#[component(reflect)]` emits a free `static __REFLECT_TYPE_INFO` and points
//! `<T as Reflect>::TYPE_INFO` at it by reference. A `static` has **one address for the
//! whole process**: the crate that defines the type emits the symbol, and every consumer
//! links against that one definition. `tests/c7_cross_crate_address.rs` reads that
//! address from both sides of a crate boundary and requires the two to be equal.
//!
//! # Why the readers are `#[inline(never)]`
//!
//! The observation is only cross-crate if the read is **codegen'd in this crate**. A
//! cross-crate-inlined reader would evaluate the descriptor reference in the *consumer's*
//! codegen unit, which is precisely the side of the boundary the gate is trying to hold
//! fixed — a `const` emission inlined into its consumer would then agree with the
//! consumer's own read and the gate would pass while the property was broken. The
//! attribute is the instrument, not an optimization decision.
//!
//! # What is actually at stake, and why it is urgent rather than tidy
//!
//! This is the first rung whose output can break a **C6** obligation. C6's Check B — the
//! acyclicity walk in `boyko_reflect::type_info` — identifies types **by address**
//! (`ptr::from_ref` + `ptr::eq` over its `path` and `done` arrays). Two addresses for one
//! type means Check B silently stops recognizing a type it has already seen: its cycle
//! test and its memoization both degrade into a walk that recognizes nothing, and
//! `validate` keeps returning `Ok`, so nothing reds.
//!
//! It goes live at **CORE C8**'s install seam and at **ECS EG8**, both of which read a
//! descriptor from a crate other than the one that defined it. A defect introduced here
//! would therefore first become observable two rungs downstream of the change that caused
//! it — which is why the gate is at C7, on C7's emission, rather than left to the rung
//! that would trip over it.
//!
//! # The reason the emission is a `static`, corrected
//!
//! An associated `const` compiles and is wrong — but **not for the reason this campaign
//! wrote down at five sites** (`boyko_macros/src/reflect.rs`, `boyko_reflect/src/reflect.rs`,
//! `reflect_fixture/tests/c7_derive_bake.rs`, and `docs/REFLECTION-PLAN-CORE.md` twice; all
//! five corrected in the same change as this module, because a repair that reads the
//! sentence and not the paragraph arguing for it is how this tree has introduced new rot
//! before). The claim was that a `const` is "const-promoted afresh at
//! each `&`-site". It is not, on this emission: the derive's expansion contains exactly
//! **one** `&__REFLECT_TYPE_INFO`, so *within a crate* a `const` descriptor's address is
//! perfectly stable, and every same-crate `ptr::eq` check in the campaign's existing
//! tables is blind to the substitution (measured: the `static` → `const` mutation leaves
//! all sixteen of `reflect_fixture`'s `c7_derive_bake` tests green, both `ptr::eq` clauses
//! included).
//!
//! The divergence is at the **crate boundary**. An associated `const`'s value is
//! re-materialized by whichever crate *evaluates* it, so the promoted allocation behind
//! `&__REFLECT_TYPE_INFO` is re-interned per consumer: the defining crate gets one copy
//! and each downstream crate gets its own. That is a property no single-crate test can
//! see, which is why this subject had to be built here.

use boyko_macros::Component;
use boyko_reflect::Reflect;
use boyko_reflect::type_info::TypeInfo;

/// The leaf of the cross-crate probe — one `Prim` field.
///
/// Defined **in the library crate**, unlike every other annotated type this campaign has
/// so far: `reflect_fixture`'s subjects are private items of integration-test binaries,
/// so no consumer of theirs exists to disagree about an address.
#[derive(Component, Default, Debug, Clone, Copy, PartialEq)]
#[component(reflect)]
#[repr(C)]
pub struct ProbeLeaf {
    /// One POD field, so the walk has a real (non-ZST) subject.
    pub value: u32,
}

/// The root of the cross-crate probe: a `Nested` edge onto [`ProbeLeaf`] plus a `Prim`.
///
/// The nested edge is the half that matters. `ProbeRoot`'s field table is baked **here**,
/// so its `nested` pointer is this crate's view of `ProbeLeaf`'s descriptor; the consumer
/// test reads `<ProbeLeaf as Reflect>::TYPE_INFO` for itself. Comparing the two is
/// literally the `ptr::eq(edge, descriptor)` that C6's Check B performs, evaluated across
/// a crate boundary.
#[derive(Component, Default, Debug, Clone, Copy, PartialEq)]
#[component(reflect)]
#[repr(C)]
pub struct ProbeRoot {
    /// The depth-1 nest — the field whose baked `nested` pointer the gate compares.
    pub leaf: ProbeLeaf,
    /// A `Prim` beside it, so the root is not a one-field transparent wrapper.
    pub tag: u32,
}

/// [`ProbeLeaf`]'s descriptor address **as this crate sees it**.
///
/// `#[inline(never)]` is load-bearing — see this module's header: a reader inlined into
/// its consumer would report the *consumer's* materialization and the gate would compare
/// one side against itself.
#[inline(never)]
pub fn probe_leaf_type_info_in_defining_crate() -> &'static TypeInfo {
    <ProbeLeaf as Reflect>::TYPE_INFO
}

/// [`ProbeRoot`]'s descriptor address **as this crate sees it**. `#[inline(never)]` for
/// the reason [`probe_leaf_type_info_in_defining_crate`] states.
#[inline(never)]
pub fn probe_root_type_info_in_defining_crate() -> &'static TypeInfo {
    <ProbeRoot as Reflect>::TYPE_INFO
}
