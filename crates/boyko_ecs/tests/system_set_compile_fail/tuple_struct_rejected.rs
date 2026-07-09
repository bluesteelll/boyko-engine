// Phase 15 — `#[derive(SystemSet)]` on a tuple struct with a field must be
// rejected. A per-instance set would imply a per-value identity the
// `(TypeId, discriminant)` key cannot represent; only unit structs are valid.
//
// Expected diagnostic: "SystemSet derive requires a unit struct (no fields)".

use boyko_macros::SystemSet;

#[derive(SystemSet)]
struct T(u32);

fn main() {}
