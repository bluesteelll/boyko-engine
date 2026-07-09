// Task #9 (compile-fail #12): `AnyOf` arms are sealed (Decision 3) — `()` is
// NOT an `AnyOfArm`. Its `matches_component_set` is unconditionally `true`,
// which would break the OR's >=1-member trim.
//
// Expected diagnostic: `(): AnyOfArm` not satisfied.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::data::AnyOf;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

#[derive(Clone, Copy)]
#[repr(C)]
struct A(u32);
impl Component for A {
    fn component_id() -> ComponentId {
        ComponentId(764)
    }
}

fn must_not_compile(q: Query<AnyOf<((), &A)>>) {
    for _ in q.iter() {}
}

fn main() {}
