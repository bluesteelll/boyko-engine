// Dense-enable plan D0 (Disabled polarity twin): the dense fast-path reject fires
// for `Disabled<Tag>` exactly as for `Enabled<Tag>` — both set
// `CONTAINS_ENABLE_TERM == true`, and neither can be honored by the
// archetype-agnostic dense column. This pins the polarity coverage so a future
// change that special-cased only `Enabled` cannot silently re-open the `Disabled`
// dense-iter leak. Shared `assert_dense_iter_no_enable` in a `const ITEM` context.

use boyko_ecs::ecs::core::iters::query::query::assert_dense_iter_no_enable;
use boyko_ecs::ecs::core::iters::query::filter_enable::Disabled;
use boyko_macros::Component;

/// Dense-stored `D` (`D::HAS_DENSE == true`).
#[derive(Component)]
#[component(storage = "dense")]
#[repr(C)]
struct Dense {
    x: f32,
}

/// Bitset enable tag.
#[derive(Component)]
#[component(storage = "bitset")]
#[repr(C)]
struct Tag;

// `const ITEM` ⇒ the D0 assert fires for the `Disabled<Tag>` polarity too.
const _: () = assert_dense_iter_no_enable::<&mut Dense, Disabled<Tag>>();

fn main() {}
