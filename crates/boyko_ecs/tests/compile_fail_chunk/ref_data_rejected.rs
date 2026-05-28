// Phase X.A Wave 7 Step 7B — `Ref<'_, T>` is NOT a `ChunkedQueryData`.
//
// `Ref<'_, T>` (Phase 10 change-detection wrapper) exposes per-row
// `last_changed` / `added` tick metadata; the chunked path elides per-row
// state (plan §4.3) — there is no way to materialise a `&[Ref<'_, T>]` slice
// over storage that does not store one `Ref` header per row. The CD-trait
// gate at `D: ChunkedQueryData` excludes `Ref<'_, T>` accordingly.
//
// Expected diagnostic: `Ref<'_, T>: ChunkedQueryData` not satisfied, pinned
// to the `for_each_chunk` call site.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::data::Ref;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

#[derive(Clone, Copy)]
#[repr(C)]
struct T(u32);

impl Component for T {
    fn component_id() -> ComponentId {
        ComponentId(904)
    }
}

fn must_not_compile(mut q: Query<Ref<'_, T>>) {
    q.for_each_chunk(|_slice| {});
}

fn main() {}
