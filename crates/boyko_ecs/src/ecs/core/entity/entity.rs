use boyko_utils::identifiers::slot::Slot;
use crate::ecs::identifiers::primitives::EntityId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entity {
    pub id: EntityId,
    pub generation: usize,
}

impl Entity {
    #[inline]
    pub fn new(id: EntityId, generation: usize) -> Self {
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
    pub fn generation(&self) -> usize {
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



impl From<Slot> for Entity {
    fn from(slot: Slot) -> Self {
        Entity {
            id: slot.index(),
            generation: slot.generation(),
        }
    }
}

impl From<Entity> for Slot {
    fn from(val: Entity) -> Self {
        Slot::new(val.id(), val.generation())
    }
}