// EnableTag M1: `Disabled<T>` inside `Or<>` is a compile error — the polarity
// twin of `or_enabled_rejected.rs`. `Disabled` does not implement the sealed
// `OrComposable` marker, so `Or<(Disabled<A>, ..)>` fails to implement
// `QueryFilter`.

use boyko_ecs::ecs::core::iters::query::filter::{Or, QueryFilter, With};
use boyko_ecs::ecs::core::iters::query::filter_enable::Disabled;
use boyko_macros::Component;

#[derive(Component)]
#[repr(C)]
struct A(u32);

#[derive(Component)]
#[repr(C)]
struct B(u32);

fn requires_filter<F: QueryFilter>() {}

fn main() {
    requires_filter::<Or<(Disabled<A>, With<B>)>>();
}
