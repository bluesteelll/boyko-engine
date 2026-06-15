// EnableTag M1: `Enabled<T>` inside `Or<>` is a compile error. `Enabled` does
// not implement the sealed `OrComposable` marker, so `Or<(Enabled<A>, ..)>`
// fails to implement `QueryFilter` (the `Or` impl requires every element to be
// `OrComposable`). `Or` folds a non-archetypal per-row test against an
// archetypal element's unconditional `true`, which would leak disabled rows —
// hence the type-level reject.

use boyko_ecs::ecs::core::iters::query::filter::{Or, QueryFilter, With};
use boyko_ecs::ecs::core::iters::query::filter_enable::Enabled;
use boyko_macros::Component;

#[derive(Component)]
#[repr(C)]
struct A(u32);

#[derive(Component)]
#[repr(C)]
struct B(u32);

fn requires_filter<F: QueryFilter>() {}

fn main() {
    requires_filter::<Or<(Enabled<A>, With<B>)>>();
}
