use crate::ecs::core::ecs_master::archetype_bundle::ArchetypeBundle;

pub struct EcsMaster {
    archetypes: ArchetypeBundle,
    free_entity_ids: Vec<usize>
}

impl EcsMaster {
    fn create_entity()
}