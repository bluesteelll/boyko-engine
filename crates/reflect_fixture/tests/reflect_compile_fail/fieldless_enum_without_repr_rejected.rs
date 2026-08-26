//! CORE C9 — a fieldless enum with no `#[repr(Int)]`, refused at the enum item.
//!
//! Analysis FIX Mi3: without an integer repr there is no guaranteed discriminant width,
//! and C10 will bake the discriminant's **byte** into a descriptor a serializer reads. A
//! silent `Opaque` here would be the same coherent lie the two D38 rows refuse.
//!
//! `#[repr(C)]` does not satisfy the rule either: a `repr(C)` enum's discriminant is the
//! platform's `int`, which is target-dependent rather than guaranteed.
//!
//! # The window this row deliberately leaves open
//!
//! A fieldless enum that DOES carry `#[repr(u8)]` is **accepted** — gated by the twin
//! `reflect_pass/fieldless_repr_enum_accepted.rs`, without which nothing in the tree
//! reached `has_integer_repr`'s `true` — and until C10 it bakes `TypeKind::Opaque`, a
//! silent `Opaque`. The difference is that `fields: &[]` is *true* for a fieldless enum:
//! the descriptor is incomplete, not false. §5 lets C9 land before C10, so it is recorded.

use boyko_macros::Component;

/// The subject: three variants, no payloads, no integer repr.
#[derive(Component, Default)]
#[component(reflect)]
pub enum NoRepr {
    /// The `Default` variant.
    #[default]
    Idle,
    /// Second.
    Running,
    /// Third.
    Done,
}

fn main() {}
