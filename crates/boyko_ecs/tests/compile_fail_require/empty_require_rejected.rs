// Feature 1 — an empty `#[require()]` is rejected (list at least one required
// component).
//
// Expected diagnostic: "empty #[require(...)]: list at least one required
// component ...".

use boyko_macros::Component;

#[derive(Component)]
#[require()]
#[repr(C)]
struct Bad(u32);

fn main() {}
