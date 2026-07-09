//! I2 gate — documentation of `#[derive(Actionlike)]` compile-fail cases
//! (plan §14 I2: "compile-fail on non-enum / on `COUNT > 256`").
//!
//! `trybuild` is **not** an in-house dependency (the engine path forbids adding
//! third-party crates for this), so the compile-fail matrix cannot be asserted
//! by an in-tree harness. Instead each rejected case is enumerated here, the
//! emitted diagnostic is documented, and the tester verifies them out-of-band
//! by compiling each snippet with `rustc` (see the test report). The derive
//! source is `boyko_macros/src/lib.rs::actionlike_macro` +
//! `actionlike_variant_kind`.
//!
//! Rejected shapes and the exact compiler message each must emit:
//!
//! | Shape | Diagnostic |
//! |-------|------------|
//! | `#[derive(Actionlike)] struct S;` | "Actionlike can only be derived for a fieldless enum" |
//! | `#[derive(Actionlike)] union U {…}` | "Actionlike can only be derived for a fieldless enum" |
//! | data-carrying variant `enum E { V(u8) }` | "Actionlike variants must be fieldless (no stable dense index otherwise)" |
//! | generic enum `enum E<T> { … }` | "Actionlike derive does not support generics (the action set must be a fixed enum)" |
//! | empty enum `enum E {}` | "Actionlike enum must declare at least one variant (COUNT == 0 is unusable)" |
//! | unknown kind `#[actionlike(Foo)] V` | "unknown action kind `Foo` (expected Button, Axis1D, or Axis2D)" |
//! | `COUNT > 256` (257 variants) | "Actionlike enum exceeds BitSet256 capacity (256 actions max)" |
//!
//! This file deliberately contains **no** invalid derives — it only documents
//! them so the test crate keeps compiling. The negative cases are exercised by
//! the report's `rustc` runs.

/// A compile-time positive control: a valid derive in this same file proves the
/// derive path is reachable from the test crate (the negative cases differ only
/// by the rejected shape, verified out-of-band).
#[derive(boyko_input::Actionlike, Clone, Copy, PartialEq, Eq, Debug)]
enum _Valid {
    A,
    #[actionlike(Axis2D)]
    B,
}

#[test]
fn valid_derive_compiles_as_control() {
    use boyko_input::Actionlike;
    assert_eq!(_Valid::COUNT, 2);
    assert_eq!(_Valid::A.index(), 0);
}
