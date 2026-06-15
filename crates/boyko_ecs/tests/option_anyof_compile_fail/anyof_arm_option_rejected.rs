// Task #9 (compile-fail #12): `AnyOf` arms are sealed (Decision 3) — only
// real-component leaves (`&T`, `&mut T`, `Ref<T>`, `Mut<T>`) implement the
// sealed `AnyOfArm` marker. `Option<&B>` does NOT, because its
// `matches_component_set` is unconditionally `true`, which would break the
// OR's >=1-member trim by matching the whole world.
//
// Expected diagnostic: `Option<&B>: AnyOfArm` not satisfied (the `AnyOf`
// QueryData impl requires every arm `Di: AnyOfArm`).

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::data::AnyOf;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

#[derive(Clone, Copy)]
#[repr(C)]
struct A(u32);
impl Component for A {
    fn component_id() -> ComponentId {
        ComponentId(762)
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
struct B(u32);
impl Component for B {
    fn component_id() -> ComponentId {
        ComponentId(763)
    }
}

fn must_not_compile(q: Query<AnyOf<(&A, Option<&B>)>>) {
    for _ in q.iter() {}
}

fn main() {}
