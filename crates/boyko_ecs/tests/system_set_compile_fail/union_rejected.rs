// Phase 15 — `#[derive(SystemSet)]` on a union must be rejected. A marker has
// no use for a union; only unit structs and fieldless enums are accepted.
//
// Expected diagnostic: "SystemSet can only be derived for unit structs or
// fieldless enums".

use boyko_macros::SystemSet;

#[derive(SystemSet)]
union U {
    a: u32,
    b: f32,
}

fn main() {}
