use crate::ecs::identifiers::primitives::{InlandArchetypeId, InlandPoolId, Generation};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityInland {
    archetype_index: InlandArchetypeId,
    unit_index: InlandPoolId,
    generation: Generation,
}

impl EntityInland {
    pub fn new(archetype_index: InlandArchetypeId, unit_index: InlandPoolId, generation: Generation) -> Self {
        Self { archetype_index, unit_index, generation }
    }
    
    #[inline]
    pub fn archetype_index(&self) -> InlandArchetypeId {
        self.archetype_index
    }
    
    #[inline]
    pub fn unit_index(&self) -> InlandPoolId {
        self.unit_index
    }
    
    #[inline]
    pub fn set_archetype_index(&mut self, archetype_index: InlandArchetypeId) {
        self.archetype_index = archetype_index;
    }
    
    #[inline]
    pub fn set_unit_index(&mut self, unit_index: InlandPoolId) {
        self.unit_index = unit_index;
    }

     #[inline]
    pub fn set_generation(&mut self, generation: Generation) {
        self.generation = generation;
    }
    
    #[inline]
    pub fn increment_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    #[inline]
    pub fn update(&mut self, archetype_index: InlandArchetypeId, unit_index: InlandPoolId) {
        self.archetype_index = archetype_index;
        self.unit_index = unit_index;
    }
}
