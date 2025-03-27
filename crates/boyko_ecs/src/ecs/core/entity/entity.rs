use crate::ecs::identifiers::primitives::EntityId;

pub struct Entity {
    pub id: EntityId,
    pub generation: u16,
}

impl Entity {
    #[inline]
    pub fn new(id: EntityId, generation: u16) -> Self {
        Self { id, generation }
    }

    /// Creates a new entity with the specified ID and generation 0
    #[inline]
    pub fn with_id(id: EntityId) -> Self {
        Self { id, generation: 0 }
    }

    #[inline]
    pub fn id(&self) -> EntityId {
        self.id
    }

    #[inline]
    pub fn generation(&self) -> u16 {
        self.generation
    }

    #[inline]
    pub fn increment_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// Checks if this entity is the same as the other entity
    #[inline]
    pub fn is_same(&self, other: &Entity) -> bool {
        self.id == other.id && self.generation == other.generation
    }
}

impl PartialEq for Entity {
    fn eq(&self, other: &Self) -> bool {
        self.is_same(other)
    }
}

impl Eq for Entity {}

impl Copy for Entity {}

impl Clone for Entity {
    fn clone(&self) -> Self {
        *self
    }
}