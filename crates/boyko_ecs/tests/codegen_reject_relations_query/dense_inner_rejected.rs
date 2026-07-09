// C — a DENSE inner component in `Related<R, &DenseComp>` must NOT compile: the
// join reads the FK target's TABLE columns directly and does not resolve a
// `DenseStore` per row, so a dense inner is const-rejected at monomorphisation
// (`Related::init_state`'s `const { assert!(!D::HAS_DENSE) }`).
//
// Expected diagnostic: a DENSE inner component is not supported in v1 — query the
// dense component on the target via a separate dense query.

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::hierarchy::ChildOf;
use boyko_ecs::ecs::core::iters::query::relation::Related;
use boyko_macros::Component;

/// A dense-storage component — invalid as a `Related` inner in v1.
#[derive(Component, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct DenseBody {
    v: u32,
}

fn main() {
    let mut world = EcsMaster::new();
    // Building the query monomorphises `Related::init_state`, whose const block
    // asserts `!D::HAS_DENSE`. `DenseBody` is dense ⇒ the assertion fails.
    let _view = world.query::<Related<ChildOf, &DenseBody>, ()>();
}
