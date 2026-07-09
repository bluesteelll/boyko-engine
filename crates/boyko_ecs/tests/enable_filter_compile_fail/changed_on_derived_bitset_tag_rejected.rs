// EnableTag D4 reachable via the REAL derive (Step 10): `Changed<C>` on a
// `#[component(storage = "bitset")]` tag is compile-rejected. The polarity twin
// of `added_on_derived_bitset_tag_rejected.rs` — the `STORAGE_IS_BITSET` const
// comes from the Wave-5 derive emission, not a hand-impl, so the D4 reject fires
// for an ordinary derived enable tag.

use boyko_ecs::ecs::core::iters::query::filter::Changed;
use boyko_macros::Component;

#[derive(Component)]
#[component(storage = "bitset")]
struct Stunned;

const _: () = Changed::<Stunned>::assert_storage_supports_change_detection();

fn main() {}
