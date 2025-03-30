use boyko_utils::sparse_map::sparse_slot_map::SparseSlotMap;
use crate::ecs::core::ecs_master::archetype_bundle::ArchetypeBundle;
use crate::ecs::core::entity::entity::Entity;

pub struct EcsMaster {
    archetype_bundle: ArchetypeBundle,
    entities: Vec<Entity>
}

