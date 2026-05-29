// Phase 15 — `#[derive(SystemSet)]` on a generic type must be rejected. Sets
// are keyed by `TypeId::of::<S>()`; a generic set would mint a fresh id per
// monomorphisation.
//
// Expected diagnostic: "SystemSet derive does not support generics (Phase 9
// scope)".

use boyko_macros::SystemSet;

#[derive(SystemSet)]
struct G<T> {
    _marker: std::marker::PhantomData<T>,
}

fn main() {}
