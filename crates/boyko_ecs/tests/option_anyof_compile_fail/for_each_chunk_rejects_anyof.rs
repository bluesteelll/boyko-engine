// Task #9 (compile-fail #14): `for_each_chunk` rejects `AnyOf<..>`
// (Decision 5). Same rationale as the `Option<&T>` reject: `AnyOf` does NOT
// implement `ChunkedQueryData` — its per-arm OR gating is incompatible with
// whole-archetype slice chunking.
//
// Expected diagnostic: `AnyOf<(&A, &B)>: ChunkedQueryData` not satisfied,
// pinned to the `for_each_chunk` call site.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::data::AnyOf;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

#[derive(Clone, Copy)]
#[repr(C)]
struct A(u32);
impl Component for A {
    fn component_id() -> ComponentId {
        ComponentId(766)
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
struct B(u32);
impl Component for B {
    fn component_id() -> ComponentId {
        ComponentId(767)
    }
}

fn must_not_compile(mut q: Query<AnyOf<(&A, &B)>>) {
    q.for_each_chunk(|_slice| {});
}

fn main() {}
