// Phase X.A Wave 7 Step 7B — `Or<(With<A>, Changed<B>)>: !ArchetypalQueryFilter`.
//
// The tuple `(F0, F1, …)` implements `ArchetypalQueryFilter` iff every
// element does (plan §5.2 / `filter.rs:1730-1751`). `Or<F>` propagates the
// inner tuple's archetypal-ness via `unsafe impl<F: ArchetypalQueryFilter>
// ArchetypalQueryFilter for Or<F>` (filter.rs:1718). `Changed<B>` is NOT
// archetypal — therefore `(With<A>, Changed<B>)` is not archetypal, and
// `Or<(With<A>, Changed<B>)>` inherits the rejection.
//
// Expected diagnostic: `Or<(With<A>, Changed<B>)>: ArchetypalQueryFilter`
// not satisfied (with the underlying `Changed<B>: !ArchetypalQueryFilter`
// trail), pinned to the `for_each_chunk` call site.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::iters::query::{Query, With};
use boyko_ecs::ecs::core::iters::query::filter::{Changed, Or};
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

#[derive(Clone, Copy)]
#[repr(C)]
struct T(u32);

#[derive(Clone, Copy)]
#[repr(C)]
struct A(u32);

#[derive(Clone, Copy)]
#[repr(C)]
struct B(u32);

impl Component for T {
    fn component_id() -> ComponentId {
        ComponentId(906)
    }
}
impl Component for A {
    fn component_id() -> ComponentId {
        ComponentId(907)
    }
}
impl Component for B {
    fn component_id() -> ComponentId {
        ComponentId(908)
    }
}

fn must_not_compile(mut q: Query<&T, Or<(With<A>, Changed<B>)>>) {
    q.for_each_chunk(|_slice: &[T]| {});
}

fn main() {}
