// KE3: `Query::contains` refuses a DENSE `D` at compile time. Dense-store
// membership needs a data fetch, so a `contains` that answered anyway would be
// answering a DIFFERENT question from `get` — silently, and only for dense
// queries. Use `get(e).is_some()` / `get_mut(e).is_some()` instead.
//
// Pinned through the shared `assert_contains_not_dense` in a `const ITEM`, the
// same shape the `dense_iter*` cases use and for the same reason: this suite runs
// `cargo check`, which never codegens a generic body, so the in-body `const {}`
// alone could not be fired by any fixture here.

use boyko_ecs::ecs::core::iters::query::query::assert_contains_not_dense;
use boyko_macros::Component;

/// A dense leaf (`D::HAS_DENSE == true`).
#[derive(Component)]
#[component(storage = "dense")]
#[repr(C)]
struct Dense {
    x: f32,
}

// `const ITEM` ⇒ eagerly const-evaluated under `cargo check` ⇒ the KE3 refusal
// fires for a dense `D`.
const _: () = assert_contains_not_dense::<&Dense>();

fn main() {}
