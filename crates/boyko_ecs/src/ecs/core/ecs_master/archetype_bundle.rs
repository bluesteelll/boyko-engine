use boyko_utils::sparse_map::sparse_map::SparseMap;
use boyko_utils::identifiers::slot::Slot;
use boyko_utils::sparse_map::sparse_slot_map::SparseSlotMap;
use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::identifiers::primitives::InlandArchetypeId;

type EntitySlot = Slot;
//TODO: fix by implementing generic SparseSlotMap
pub struct ArchetypeBundle {
    sparse_map: SparseSlotMap<InlandArchetypeId>,
    archetypes: Vec<Archetype>
}