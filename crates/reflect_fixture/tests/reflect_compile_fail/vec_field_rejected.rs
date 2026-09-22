//! CORE C9 — a field the v1 kind table classifies `Opaque`, refused at THE FIELD.
//!
//! D15: the descriptor's wire is shared with the shipped `boyko_serialize`, so a field
//! whose bytes the model cannot describe must not be silently absent from it.
//!
//! # This ONE row refuses every standard indirection (D34, measured)
//!
//! `Vec<T>`, `Box<T>`, `Option<T>`, `PhantomData<T>`, `&T` and raw pointers all reach the
//! same fallthrough in `boyko_macros::reflect::field_info` — after `scalar_kind`, the
//! array arm and `is_nested_path` have all declined — and all bake `ValueKind::Opaque`.
//! C9 as first written carried a *second* row enumerating them; it was struck and merged
//! here, because two rows reaching one verdict at one span is one rule with two names, and
//! the second one's red could never distinguish itself from this one's.
//!
//! The way out is `#[reflect(skip)]`, and it is not hypothetical: the accepting twin
//! `reflect_pass/vec_field_skip_accepted.rs` is this same type with the attribute.

use boyko_macros::Component;

/// The subject: one un-skipped `Vec` field beside a describable one, so the refusal is
/// visibly about the FIELD rather than about the type.
#[derive(Component, Default)]
#[component(reflect)]
pub struct HasVecField {
    /// Describable — a `Prim`, and not what is refused.
    pub tag: u32,
    /// Not describable, and not skipped.
    pub items: Vec<u32>,
}

fn main() {}
