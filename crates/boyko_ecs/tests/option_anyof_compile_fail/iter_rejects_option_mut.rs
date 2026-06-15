// Task #9 (compile-fail #11): the read-only `iter()` driver rejects
// `Option<&mut T>`. `Query::iter` is bounded `D: ReadOnlyQueryData`, and
// `Option<&mut T>` is NOT `ReadOnlyQueryData` (its inner `&mut T` writes).
// Only `iter_mut()` admits it (Decision: `ReadOnlyQueryData for Option<D>
// where D: ReadOnlyQueryData`).
//
// Expected diagnostic: `Option<&mut T>: ReadOnlyQueryData` not satisfied,
// pinned to the `.iter()` call site.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

#[derive(Clone, Copy)]
#[repr(C)]
struct T(u32);

impl Component for T {
    fn component_id() -> ComponentId {
        ComponentId(760)
    }
}

fn must_not_compile(q: Query<Option<&mut T>>) {
    for _ in q.iter() {}
}

fn main() {}
