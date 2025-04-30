use crate::ecs::identifiers::primitives::{InlandArchetypeId, InlandPoolId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityInland {
    archetype_index: InlandArchetypeId,
    unit_index: InlandPoolId,
}

impl EntityInland {
    pub fn new(archetype_index: InlandArchetypeId, unit_index: InlandPoolId) -> Self {
        Self { archetype_index, unit_index }
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
    pub fn update(&mut self, archetype_index: InlandArchetypeId, unit_index: InlandPoolId) {
        self.archetype_index = archetype_index;
        self.unit_index = unit_index;
    }
}
