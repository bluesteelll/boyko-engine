// W4 / I-NEW-4 / QV11: `EcsMaster::query::<Ref<P>, ()>()` is a compile error —
// `Ref<T>` data has `NEEDS_CHANGE_DETECTION = true`. Change-detection requires
// `Schedule` context; use `Query<D, F>` as a SystemParam.
//
// The `const _: () = ...` item below is the CHECK-time trigger (eagerly
// const-evaluated under `cargo check`, the mode `trybuild` runs). The
// `world.query::<…>()` call documents the rejected real-API shape (its inline
// `const {}` block is the codegen-time trigger, not exercised by `cargo check`).

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::{assert_query_no_change_detection, EcsMaster};
use boyko_ecs::ecs::core::iters::query::Ref;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

#[repr(C)]
struct P(u32);
impl Component for P {
    fn component_id() -> ComponentId {
        ComponentId(1)
    }
}

const _: () = assert_query_no_change_detection::<Ref<'static, P>, ()>();

fn main() {
    let mut world = EcsMaster::new();
    let _ = world.query::<Ref<'_, P>, ()>();
}
