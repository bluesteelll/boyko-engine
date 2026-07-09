// W4 / I-NEW-4 / QV11: `EcsMaster::query::<&P, Added<P>>()` is a compile error —
// `Added<C>` in the filter slot has `NEEDS_CHANGE_DETECTION = true` (the
// `Changed` polarity twin). Change-detection requires `Schedule` context; use
// `Query<D, F>` as a SystemParam.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::{assert_query_no_change_detection, EcsMaster};
use boyko_ecs::ecs::core::iters::query::Added;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

#[repr(C)]
struct P(u32);
impl Component for P {
    fn component_id() -> ComponentId {
        ComponentId(1)
    }
}

const _: () = assert_query_no_change_detection::<&P, Added<P>>();

fn main() {
    let mut world = EcsMaster::new();
    let _ = world.query::<&P, Added<P>>();
}
