// Phase 8.5 Step 9 — `#[derive(Bundle)]` on a struct with a non-Component
// field must be rejected via the per-field `where T: Component` bound the
// derive emits. The error surfaces from the trait bound check, not from
// the macro itself — but the location is still pinned to the struct so
// users can see which field is at fault.

use boyko_macros::Bundle;

#[derive(Bundle)]
struct B {
    x: u32,
}

fn main() {}
