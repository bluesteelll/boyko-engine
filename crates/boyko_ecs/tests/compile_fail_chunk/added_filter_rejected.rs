// Phase X.A Wave 7 Step 7B — `Added<C>` is NOT an `ArchetypalQueryFilter`.
//
// Symmetric counterpart to `changed_filter_rejected.rs`. `Added<C>` requires
// per-row added-tick state (component-creation tick comparison against the
// system's `last_run`); the chunked path elides per-row state by design (plan
// §3). The user should fall back to `Query::iter` / `Query::iter_mut`.
//
// Expected diagnostic: `Added<U>: ArchetypalQueryFilter` not satisfied,
// pinned to the `for_each_chunk` call site.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::filter::Added;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

#[derive(Clone, Copy)]
#[repr(C)]
struct T(u32);

#[derive(Clone, Copy)]
#[repr(C)]
struct U(u32);

impl Component for T {
    fn component_id() -> ComponentId {
        ComponentId(902)
    }
}

impl Component for U {
    fn component_id() -> ComponentId {
        ComponentId(903)
    }
}

fn must_not_compile(mut q: Query<&T, Added<U>>) {
    q.for_each_chunk(|_slice: &[T]| {});
}

fn main() {}
