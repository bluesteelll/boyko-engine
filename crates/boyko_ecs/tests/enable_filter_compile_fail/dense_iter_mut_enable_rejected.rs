// Dense-enable plan D0: the dense fast path (`dense_iter` / `dense_iter_mut` on
// both `Query` and `QueryView`) compile-rejects an enable-bearing filter. The
// dense column is archetype-agnostic (one flat buffer across archetypes) while
// the enable bit is keyed by `(archetype, row)`, so the fast path cannot honor a
// per-row enable term — without this reject `dense_iter_mut` would `&mut`-write
// EVERY live slot, disabled rows included. Use `iter_mut()` (the archetype-walking
// cursor whose per-row `filter_fetch` enforces the bit).
//
// The reject is the shared `Query::assert_dense_iter_no_enable::<D, F>()` shape
// assert, evaluated here in a `const ITEM` context (the check-time trigger, since
// trybuild runs `cargo check` for a `compile_fail`-only suite; the in-body
// `const {}` at each method top is the codegen-time trigger for real callers).
// This case pins the `&mut Dense` / `dense_iter_mut` shape.

use boyko_ecs::ecs::core::iters::query::query::assert_dense_iter_no_enable;
use boyko_ecs::ecs::core::iters::query::filter_enable::Enabled;
use boyko_macros::Component;

/// A dense-stored component (`D::HAS_DENSE == true`).
#[derive(Component)]
#[component(storage = "dense")]
#[repr(C)]
struct Dense {
    x: f32,
}

/// A bitset enable tag (`Enabled<Tag>::CONTAINS_ENABLE_TERM == true`).
#[derive(Component)]
#[component(storage = "bitset")]
#[repr(C)]
struct Tag;

// `const ITEM` ⇒ eagerly const-evaluated under `cargo check` ⇒ the D0 assert
// fires at compile time for the `&mut Dense` (dense_iter_mut) shape.
const _: () = assert_dense_iter_no_enable::<&mut Dense, Enabled<Tag>>();

fn main() {}
