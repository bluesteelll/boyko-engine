use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_macros::event;

#[event]
struct ConflictingMarkers {
    #[participant(components = "")]
    #[parameter]
    confused: Entity,
}

fn main() {}
