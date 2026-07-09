// EnableTag _C2 (NARROWED — amendment A3.2/A3.3): a TUPLE of enable terms with
// NO positive archetypal term and NOT a single leaf is still rejected. The sole
// SINGLE shape `Query<(), Enabled<A>>` is now ALLOWED (candidate-seeded), but a
// tuple `(Enabled<A>, Enabled<B>)` has `IS_SOLE_SINGLE_ENABLE = false` (the tuple
// macro does not override the default), so the `(D, F)`-seam shape gate
// `assert_query_shape()` fails its narrowed `_C2` assert: v1 has no multi-tag
// resolver to bound it.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::iters::query::filter_enable::Enabled;
use boyko_ecs::ecs::core::iters::query::state::QueryDataState;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

#[repr(C)]
struct A(u32);
impl Component for A {
    fn component_id() -> ComponentId {
        ComponentId(1)
    }
}

#[repr(C)]
struct B(u32);
impl Component for B {
    fn component_id() -> ComponentId {
        ComponentId(2)
    }
}

// No positive term, not a single leaf ⇒ the narrowed _C2 assert fires.
const _: () = QueryDataState::<(), (Enabled<A>, Enabled<B>)>::assert_query_shape();

fn main() {}
