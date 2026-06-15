// EnableTag D4: `Changed<C>` on a bitset enable tag is compile-rejected. Same
// reasoning as the `Added<C>` twin — a bitset enable tag has no per-row tick
// storage, so change detection on it is meaningless and is rejected at compile
// time (the Phase-22 D1 "compile-but-lie" lesson).
//
// The check-time trigger is the `pub const fn`
// `Changed::<C>::assert_storage_supports_change_detection()` referenced in a
// `const ITEM` context. The fixture HAND-IMPLs `Component` with
// `STORAGE_IS_BITSET = true` (the `#[component(storage = "bitset")]` derive
// override lands in Wave 5).

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::iters::query::filter::Changed;
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
const _: () = Changed::<BitsetTag>::assert_storage_supports_change_detection();

fn main() {}
