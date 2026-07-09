// EnableTag: `Enabled<T>` cannot be used with the chunk API. `for_each_chunk`
// requires `F: ArchetypalQueryFilter` (one slice per archetype, no per-row
// gate); `Enabled` is a non-archetypal per-row filter and does NOT implement
// `ArchetypalQueryFilter`, so the call fails the bound.

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::filter_enable::Enabled;
use boyko_macros::Component;

#[derive(Component)]
#[repr(C)]
struct P(u32);

#[derive(Component)]
#[repr(C)]
struct A(u32);

fn main() {
    let mut ecs = EcsMaster::new();
    ecs.query::<&P, Enabled<A>>()
        .for_each_chunk(|_slice: &[P]| {});
}
