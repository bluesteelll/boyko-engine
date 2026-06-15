// W4 / I-NEW-4 / QV11: `EcsMaster::query::<&P, Changed<P>>()` is a compile
// error — `Changed<C>` in the filter slot has `NEEDS_CHANGE_DETECTION = true`
// (NCD3 propagation covers `F::NEEDS_CHANGE_DETECTION`). Change-detection
// requires `Schedule` context; use `Query<D, F>` as a SystemParam.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::{assert_query_no_change_detection, EcsMaster};
use boyko_ecs::ecs::core::iters::query::Changed;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

#[repr(C)]
struct P(u32);
impl Component for P {
    fn component_id() -> ComponentId {
        ComponentId(1)
    }
}

const _: () = assert_query_no_change_detection::<&P, Changed<P>>();

fn main() {
    let mut world = EcsMaster::new();
    let _ = world.query::<&P, Changed<P>>();
}
