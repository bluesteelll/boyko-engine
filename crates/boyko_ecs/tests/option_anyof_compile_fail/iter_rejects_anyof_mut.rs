// Task #9 (compile-fail #11): the read-only `iter()` driver rejects
// `AnyOf<(&mut A,)>`. `AnyOf<(D0, ..)>::IS_READ_ONLY` AND-folds each arm;
// a `&mut A` arm makes it `false`, so `AnyOf<(&mut A,)>` is NOT
// `ReadOnlyQueryData` and the `D: ReadOnlyQueryData` bound on `iter()` fails.
//
// Expected diagnostic: `AnyOf<(&mut A,)>: ReadOnlyQueryData` not satisfied,
// pinned to the `.iter()` call site.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::data::AnyOf;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

#[derive(Clone, Copy)]
#[repr(C)]
struct A(u32);

impl Component for A {
    fn component_id() -> ComponentId {
        ComponentId(761)
    }
}

fn must_not_compile(q: Query<AnyOf<(&mut A,)>>) {
    for _ in q.iter() {}
}

fn main() {}
