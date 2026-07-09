// Phase X.A Wave 7 Step 7B — `Changed<C>` is NOT an `ArchetypalQueryFilter`.
//
// `Query<&T, Changed<U>>::for_each_chunk(...)` requires `F:
// ArchetypalQueryFilter` per plan §3 (the chunked path elides per-row tick
// state — `Changed<U>` needs that state). The bound is enforced at the
// `for_each_chunk` declaration site; the user should fall back to per-row
// `Query::iter` / `Query::iter_mut` for change-detection-bearing iteration.
//
// Expected diagnostic: `Changed<U>: ArchetypalQueryFilter` not satisfied,
// pinned to the `for_each_chunk` call site via
// `#[diagnostic::on_unimplemented]` on the marker trait.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::filter::Changed;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

#[derive(Clone, Copy)]
#[repr(C)]
struct T(u32);

#[derive(Clone, Copy)]
#[repr(C)]
struct U(u32);

impl Component for T {
    fn component_id() -> ComponentId {
        ComponentId(900)
    }
}

impl Component for U {
    fn component_id() -> ComponentId {
        ComponentId(901)
    }
}

fn must_not_compile(mut q: Query<&T, Changed<U>>) {
    q.for_each_chunk(|_slice: &[T]| {});
}

fn main() {}
