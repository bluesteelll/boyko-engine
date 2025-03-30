use super::component_pool_bundle::ComponentPoolBundle;
use crate::ecs::identifiers::primitives::InlandUnitId;
use boyko_utils::sparse_map::sparse_slot_map::SparseSlotMap;
use crate::ecs::core::entity::entity::Entity;

pub struct Archetype {
    pool_bundle: ComponentPoolBundle,
    entities: Vec<Entity>
}


impl Archetype {
    fn get_unit()
}