//! CORE C9 / D38 — a data-carrying enum as the component ITSELF, refused at the enum item.
//!
//! This shape was **accepted** until C9. `boyko_macros`'s `codegen` sent every enum and
//! every union down one arm and baked `TypeKind::Opaque` with `fields: &[]` — a coherent
//! descriptor asserting that a type with two payload variants has no fields.
//! `boyko_reflect::validate` returns `Ok` on it, because *"has no fields"* is structurally
//! well-formed; only a reader who knows the subject can tell it is a lie.
//!
//! # This fixture IS the migrated gate
//!
//! `tests/c7_derive_bake.rs`'s `NonStruct` and its
//! `the_non_struct_arm_bakes_an_opaque_fieldless_descriptor` pinned exactly that lie, and
//! that test's own doc said *"C10 replaces this test rather than deleting it"*. C9 replaced
//! it four rungs early, and the replacement is a blessed `.stderr` rather than a deletion:
//! the same claim, moved from *"this is what it bakes"* to *"this does not compile"*.
//!
//! The variants are the original's, deliberately: a fieldless enum would make `fields: &[]`
//! look like a tautology, while `Something(u32)` and `Named { x: f32 }` make the baked
//! *"this type has no fields"* a substantive claim about a type that has some.

use boyko_macros::Component;

/// The subject: one tuple payload, one named payload.
#[derive(Component, Default)]
#[component(reflect)]
#[repr(u8)]
pub enum NonStruct {
    /// The `Default` variant.
    #[default]
    Nothing,
    /// A tuple payload the descriptor would not describe.
    Something(u32),
    /// A named payload the descriptor would not describe.
    Named {
        /// Not reachable through the descriptor.
        x: f32,
    },
}

fn main() {}
