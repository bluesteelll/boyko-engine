use boyko_utils::sparse_map::sparse_slot_map::SparseSlotMap;
use crate::ecs::core::ecs_master::archetype_bundle::ArchetypeBundle;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId, EntityId};

pub struct EcsMaster {
    archetype_bundle: ArchetypeBundle,
    entities: Vec<Entity>
}

impl EcsMaster {
    fn new(){
        todo!()
    }

    fn craft_entity(self, archetype_id: ArchetypeId){
        // Create entity by archetype index (insert into archetyp)
        todo!()
    }

    fn remove_entity(self, entity: Entity) {
        todo!()
    }

    fn get_unit_by_component(self, entity: Entity, component_id: ComponentId){
        let archetype = self.archetype_bundle[entity];
        // Etc
        todo!()
    }

    fn get_unit_by_component_mut(self, entity: Entity, component_id: ComponentId){
        let archetype = self.archetype_bundle[entity];
        // Etc
        todo!()
    }
    fn add_archetype(self){
        todo!()
        // Create archetype from component list
    }
    fn remove_archetype(self, archetype_id: ArchetypeId){
        todo!()
    }

}
