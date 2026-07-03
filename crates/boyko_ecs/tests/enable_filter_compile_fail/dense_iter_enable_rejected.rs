// Dense-enable plan D0 (read-only twin): the dense fast path rejects an
// enable-bearing filter for a READ-ONLY dense leaf (`&Dense`) too. Same rationale
// as the `dense_iter_mut` twin — the archetype-agnostic dense column cannot honor
// a per-row enable term. Use `iter()` instead. Pins the `&Dense` / `dense_iter`
// shape via the shared `assert_dense_iter_no_enable` in a `const ITEM` context.

use boyko_ecs::ecs::core::iters::query::query::assert_dense_iter_no_enable;
use boyko_ecs::ecs::core::iters::query::filter_enable::Enabled;
use boyko_macros::Component;

/// Read-only dense leaf (`D::HAS_DENSE == true`).
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

// `const ITEM` ⇒ eagerly const-evaluated under `cargo check` ⇒ the D0 assert
// fires for the `&Dense` (dense_iter) shape.
const _: () = assert_dense_iter_no_enable::<&Dense, Enabled<Tag>>();

fn main() {}
