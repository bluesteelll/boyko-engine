use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_macros::event;

#[event]
struct MissingMarker {
    #[participant(components = "")]
    a: Entity,
    unmarked_field: u32,
}

fn main() {}
