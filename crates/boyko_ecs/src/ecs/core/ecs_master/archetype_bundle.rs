use boyko_utils::sparse_map::sparse_map::SparseMap;
use boyko_utils::identifiers::slot::Slot;
use boyko_utils::sparse_map::sparse_slot_map::SparseSlotMap;
use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::component::Component;
use crate::ecs::identifiers::primitives::{ArchetypeId, InlandArchetypeId};

type EntitySlot = Slot;

pub struct ArchetypeBundle {
    sparse_map: SparseMap<InlandArchetypeId>,
    archetypes: Vec<Archetype>
}

impl ArchetypeBundle {
    pub fn get_archetype (&self, index: ArchetypeId) {

    }
}