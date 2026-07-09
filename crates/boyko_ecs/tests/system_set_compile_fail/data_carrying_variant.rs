// Phase 15 — a `#[derive(SystemSet)]` enum with a data-carrying variant must
// be rejected. Only fieldless variants have a stable type-level identity that
// the `(TypeId, discriminant)` interning key can represent.
//
// Expected diagnostic: "SystemSet enum variants must be unit variants (no
// fields)".

use boyko_macros::SystemSet;

#[derive(SystemSet)]
enum E {
    A,
    V(u32),
}

fn main() {}
