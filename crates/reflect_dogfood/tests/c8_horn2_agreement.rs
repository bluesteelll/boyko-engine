//! **CORE C8 gate 5 (D16 / D30) — Horn 2's price: the two GENERATORS agree on a field
//! vocabulary.**
//!
//! # What Horn 2 bought, and what it therefore owes
//!
//! D16's Horn 2 keeps `#[derive(Bindable)]`'s `u8` field ids and
//! `#[component(reflect)]`'s field table as **two independently generated** views of one
//! struct, rather than making one a projection of the other. The saving is real (no
//! `boyko-reflect` edge in `boyko_ui`, no reflect surface in a shipped crate); the debt is
//! that nothing structural forces the two vocabularies to stay the same, and this gate is
//! the payment.
//!
//! # Why the red is a GENERATOR mutation and not a source edit (D30)
//!
//! The gate was first specified as a drift test — *"rename a field on one side only"* —
//! and that mutation **does not exist**. There is no "one side": a single struct
//! definition feeds both derives, both read the same `syn::Field::ident`
//! (`bindable.rs:36-40,:54`; `reflect.rs:172-175`), and neither parses a rename —
//! `attributes(bind)` is declared at `lib.rs:559` and never read.
//!
//! Nor does C9's `#[reflect(skip)]` supply the divergence later. **D14 forbids exactly
//! that**: `skip` emits an `Opaque` `FieldInfo` and *does not omit the field*, chosen so
//! the by-index API's indices cannot depend on which fields were skipped. So the reflect
//! index equals the declaration index always, `Bindable`'s id equals the declaration index
//! always, and the two agree **by construction, permanently** — for as long as both
//! generators keep computing them that way. That last clause is the measurand, so the red
//! is a mutation of a generator. **Both forms were applied and OBSERVED at C8's landing:**
//!
//! * reverse `bindable.rs:53`'s `ids` to `(0..field_count as u8).rev()` — `zeta` becomes
//!   index 0 to reflect and id 3 to `Bindable`;
//! * suffix the name `reflect.rs`'s `field_info` bakes. D30 cites `:190` for this, which
//!   is the `FieldInfo {` line and *one of four arms*; the mutation was applied one level
//!   up at the `name_str` computation (`reflect.rs:172-175`), which is the single site
//!   feeding all four arms — reflect then bakes `zeta_`, and `field_id` resolves it to
//!   `None`.
//!
//! # Why `FIELD_COUNT` is a separate clause
//!
//! The name loop is driven by the **reflect** descriptor, because `Bindable` exposes no
//! name enumeration at all — only `FIELD_COUNT` and `field_id(name)`
//! (`boyko_ui/src/binding/bindable.rs:23-46`). A name that reflect *stopped emitting* is
//! therefore invisible to a name-driven loop: it iterates a shorter list and every
//! surviving entry still agrees. The cardinality clause is what sees it.
//!
//! # The invocation is part of the gate
//!
//! ```text
//! cargo test -p reflect-dogfood --features reflect-dogfood/reflect --test c8_horn2_agreement
//! ```
//!
//! Feature-off this file compiles to nothing and exits 0 — vacuous on the green side and
//! on every red side. The output must read `running [1-9]`.
#![cfg(feature = "reflect")]

use boyko_macros::{Bindable, Component};
use boyko_reflect::Reflect;
use boyko_ui::binding::Bindable as BindableTrait;

/// The subject: one struct, both derives, four fields in a deliberately non-alphabetical
/// declaration order.
///
/// Non-alphabetical on purpose. If either generator ever sorted its fields — a plausible
/// "tidy" change, and one that would leave a declaration-ordered subject green — the two
/// vocabularies would diverge, and only an order the sort would *change* can see it.
/// Four fields rather than two so that a reversal (`(0..n).rev()`) cannot be mistaken for
/// a swap of a symmetric pair.
///
/// The field types are all `Prim` scalars because `Bindable::value_field` emits
/// `self.<field> as f32`, which is a cast, not a conversion — a nested or opaque field
/// would fail to compile on the `Bindable` side and the subject would never reach this
/// gate's question.
#[derive(Component, Bindable, Default, Debug, Clone, Copy)]
#[component(reflect)]
#[repr(C)]
pub struct Horn2Subject {
    /// Declared first; alphabetically last.
    pub zeta: f32,
    /// Declared second; alphabetically first.
    pub alpha: u32,
    /// Declared third.
    pub middle: i16,
    /// Declared fourth; a second `f32` so the type multiset is not a discriminator either.
    pub omega: f32,
}

/// **The gate.** For every name in the reflect descriptor, `Bindable::field_id` returns
/// that name's reflect index — and the two vocabularies have the same cardinality.
#[test]
fn the_two_generators_agree_on_every_field_name_and_on_the_count() {
    let info = <Horn2Subject as Reflect>::TYPE_INFO;

    // The cardinality clause FIRST: it is the one that survives a reflect side which
    // stopped emitting a name, and running it first keeps that failure from being reported
    // as "every name agreed".
    assert_eq!(
        <Horn2Subject as BindableTrait>::FIELD_COUNT as usize,
        info.fields.len(),
        "the two generators disagree on how many fields `Horn2Subject` has: Bindable says \
         {}, reflect says {}. Because `Bindable` exposes no name enumeration, the \
         name-by-name loop below is driven by the reflect descriptor alone -- so a field \
         reflect stopped emitting would leave every remaining name agreeing and this is \
         the only clause that can see it.",
        <Horn2Subject as BindableTrait>::FIELD_COUNT,
        info.fields.len(),
    );

    for (reflect_index, field) in info.fields.iter().enumerate() {
        let bind_id = <Horn2Subject as BindableTrait>::field_id(field.name).unwrap_or_else(|| {
            panic!(
                "reflect bakes a field named `{}` (index {reflect_index}) that \
                 `Bindable::field_id` does not resolve at all. The two generators read the \
                 same `syn::Field::ident`, so a name known to one and not the other means \
                 one of them is transforming the identifier -- Horn 2's whole premise is \
                 that neither does.",
                field.name
            )
        });
        assert_eq!(
            bind_id as usize, reflect_index,
            "field `{}` is index {reflect_index} to reflect and id {bind_id} to Bindable. \
             The by-index reflect API and the `u8` bind id would then address DIFFERENT \
             fields of the same struct under one name -- which is the cost D16 accepted \
             when it kept the two views independent, and this gate is what pays it.",
            field.name
        );
    }
}

/// The agreement is about the *declaration* order specifically, pinned by name so a
/// generator that silently sorted could not satisfy it.
///
/// Without this, "they agree" is satisfiable by two generators that sort identically —
/// consistent, and wrong in the same direction, which is the failure a mutual-agreement
/// test is least able to see.
#[test]
fn both_vocabularies_are_declaration_order() {
    let info = <Horn2Subject as Reflect>::TYPE_INFO;
    let declared = ["zeta", "alpha", "middle", "omega"];
    let baked: Vec<&str> = info.fields.iter().map(|f| f.name).collect();
    assert_eq!(
        baked, declared,
        "the reflect descriptor's field order is not `Horn2Subject`'s DECLARATION order. \
         Two generators that both sorted would still agree with each other, so the \
         agreement clause cannot see this -- the literal order has to be pinned somewhere."
    );
    for (i, name) in declared.iter().enumerate() {
        assert_eq!(
            <Horn2Subject as BindableTrait>::field_id(name),
            Some(i as u8),
            "Bindable resolves `{name}` to something other than its declaration index {i}"
        );
    }
}
