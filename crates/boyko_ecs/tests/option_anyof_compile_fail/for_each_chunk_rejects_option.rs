// Task #9 (compile-fail #14): `for_each_chunk` rejects `Option<&T>`
// (Decision 5). The chunked path dispatches through `ChunkedQueryData`
// (`fetch_chunk -> &[T]`, whole-archetype slices, no per-row gating). Per-row
// `Option` gating is incompatible with slice chunking, so `Option<D>` does
// NOT implement `ChunkedQueryData`.
//
// Expected diagnostic: `Option<&T>: ChunkedQueryData` not satisfied, pinned
// to the `for_each_chunk` call site.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

#[derive(Clone, Copy)]
#[repr(C)]
struct T(u32);

impl Component for T {
    fn component_id() -> ComponentId {
        ComponentId(765)
    }
}

fn must_not_compile(mut q: Query<Option<&T>>) {
    q.for_each_chunk(|_slice| {});
}

fn main() {}
