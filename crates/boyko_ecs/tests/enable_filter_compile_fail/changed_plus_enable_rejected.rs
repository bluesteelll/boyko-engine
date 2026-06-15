// EnableTag C3 (amendment A3.4): an `Enabled<T>` / `Disabled<T>` term cannot be
// combined with `Added` / `Changed` in one query. The `(D, F)`-seam shape gate
// `QueryDataState::<D, F>::assert_query_shape()` (a `pub const fn` referenced in
// a `const ITEM` context — the check-time trigger) fails its `_C3` assert: a
// point lookup applies the enable bit but not change-detection, which would
// silently mislead.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::iters::query::filter::Changed;
use boyko_ecs::ecs::core::iters::query::filter_enable::Enabled;
use boyko_ecs::ecs::core::iters::query::state::QueryDataState;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

#[repr(C)]
struct P(u32);
impl Component for P {
    fn component_id() -> ComponentId {
        ComponentId(1)
    }
}

#[repr(C)]
struct A(u32);
impl Component for A {
    fn component_id() -> ComponentId {
        ComponentId(2)
    }
}

// `const ITEM` ⇒ eagerly const-evaluated under `cargo check` ⇒ the _C3 assert
// fires at compile time.
const _: () = QueryDataState::<&P, (Changed<P>, Enabled<A>)>::assert_query_shape();

fn main() {}
