// EnableTag D4: `Added<C>` on a bitset enable tag is compile-rejected. A bitset
// enable tag has NO per-row tick storage, so `Added` on it cannot be honored;
// rejecting the monomorphization at compile time is the correct fix rather than
// silently matching nothing (the Phase-22 D1 "compile-but-lie" lesson).
//
// The check-time trigger is the `pub const fn`
// `Added::<C>::assert_storage_supports_change_detection()` referenced in a
// `const ITEM` context, which is eagerly const-evaluated under `cargo check`
// (the mode `trybuild` runs). The fixture HAND-IMPLs `Component` with
// `STORAGE_IS_BITSET = true` because the `#[component(storage = "bitset")]`
// derive override lands in Wave 5.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::iters::query::filter::Added;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

#[repr(C)]
struct BitsetTag(u32);
impl Component for BitsetTag {
    const STORAGE_IS_BITSET: bool = true;
    fn component_id() -> ComponentId {
        ComponentId(1)
    }
}

// `const ITEM` ⇒ eagerly const-evaluated under `cargo check` ⇒ the D4 assert
// fires at compile time.
const _: () = Added::<BitsetTag>::assert_storage_supports_change_detection();

fn main() {}
