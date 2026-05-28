// Phase X.A Wave 7 Step 7B — `Mut<'_, T>` is NOT a `ChunkedQueryData`.
//
// Symmetric counterpart to `ref_data_rejected.rs`. `Mut<'_, T>` exposes a
// per-row `Drop`-guard that bumps the row's `changed` tick on write; the
// chunked path elides per-row state (plan §4.3) — slice-level access cannot
// carry that guard. The user should use raw `&mut T` (no change-detection)
// inside `for_each_chunk` or fall back to per-row `Query::iter_mut`.
//
// Expected diagnostic: `Mut<'_, T>: ChunkedQueryData` not satisfied, pinned
// to the `for_each_chunk` call site.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::data::Mut;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

#[derive(Clone, Copy)]
#[repr(C)]
struct T(u32);

impl Component for T {
    fn component_id() -> ComponentId {
        ComponentId(905)
    }
}

fn must_not_compile(mut q: Query<Mut<'_, T>>) {
    q.for_each_chunk(|_slice| {});
}

fn main() {}
