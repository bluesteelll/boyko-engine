// EnableTag D4 reachable via the REAL derive (Step 10): `Added<C>` on a
// `#[component(storage = "bitset")]` tag is compile-rejected. This is the twin of
// `added_on_bitset_tag_rejected.rs` (which hand-impls `Component` with
// `STORAGE_IS_BITSET = true`) — here the const comes from the Wave-5 derive
// emission, proving the D4 reject fires for an ordinary derived enable tag.
//
// The check-time trigger is the `pub const fn`
// `Added::<C>::assert_storage_supports_change_detection()` referenced in a
// `const ITEM`, eagerly const-evaluated under `cargo check` (the mode `trybuild`
// runs).

use boyko_ecs::ecs::core::iters::query::filter::Added;
use boyko_macros::Component;

#[derive(Component)]
#[component(storage = "bitset")]
struct Stunned;

const _: () = Added::<Stunned>::assert_storage_supports_change_detection();

fn main() {}
