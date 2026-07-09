// W4 / I-NEW-4 / QV11: `EcsMaster::query::<Mut<P>, ()>()` is a compile error —
// `Mut<T>` data has `NEEDS_CHANGE_DETECTION = true` (the `Ref` polarity twin).
// Change-detection requires `Schedule` context; use `Query<D, F>` as a
// SystemParam.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::{assert_query_no_change_detection, EcsMaster};
use boyko_ecs::ecs::core::iters::query::Mut;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

#[repr(C)]
struct P(u32);
impl Component for P {
    fn component_id() -> ComponentId {
        ComponentId(1)
    }
}

const _: () = assert_query_no_change_detection::<Mut<'static, P>, ()>();

fn main() {
    let mut world = EcsMaster::new();
    let _ = world.query::<Mut<'_, P>, ()>();
}
