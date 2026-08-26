//! CORE C9 / D20 — a reflected type with no `Default`, refused by a NAMED trait bound.
//!
//! # This row is not a `compile_error!`, and cannot be
//!
//! A proc macro sees tokens, not trait impls, so *"this type has no `Default`"* is not a
//! fact the derive can learn. `#[component(reflect)]` bakes `TypeInfo::default_in_place`
//! from `Default` (an inspector's "Add Component" needs it), so the requirement is real;
//! what carries it is `boyko_reflect::ReflectDefault` and its
//! `#[diagnostic::on_unimplemented]`, asserted through a witness the derive emits spanned
//! at the user's own type name.
//!
//! That is this tree's existing answer for the class — `boyko_ecs`'s
//! `query/chunked_data.rs` and `query/filter.rs` both carry one, the second with a blessed
//! `compile_fail` fixture pinning its message.
//!
//! # Why it still has a `REFUSALS` row
//!
//! The defect D20 exists to close is precisely that a hidden `T: Default` bound is
//! **structurally invisible** to a census keyed on `REFUSALS`. So it gets a row, marked
//! *message-only*, and the census asserts its bytes are byte-identical to the
//! `on_unimplemented` `message = "…"` in `crates/boyko_reflect/src/reflect.rs` — the only
//! instrument available, since `boyko_macros` must never gain an edge to `boyko_reflect`
//! (D17) and the two strings therefore cannot share a const.
//!
//! The `.stderr` here pins `ReflectDefault`'s message, **not** rustc's generic E0277 text,
//! which is the whole point of the row. C9's third RED deletes the `on_unimplemented` and
//! watches this file's bytes become the generic ones.
//!
//! The way out is `#[reflect(no_default)]` — `reflect_pass/no_default_accepted.rs`.

use boyko_macros::Component;

/// The subject: one describable field, and deliberately no `Default`.
#[derive(Component)]
#[component(reflect)]
pub struct NoDefaultImpl {
    /// A `Prim`, so nothing else about this type is refusable.
    pub value: u32,
}

fn main() {}
