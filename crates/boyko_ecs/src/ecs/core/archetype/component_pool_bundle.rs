use std::ops::{Index, IndexMut};
use crate::ecs::core::component::Component;
use crate::ecs::identifiers::primitives::{ChunkId, InlandComponentId};
use crate::ecs::memory::component_pool::ComponentPool;
use boyko_utils::sparse_map::sparse_map::SparseMap;
pub struct ComponentPoolBundle {
    pools: Vec<ComponentPool>,
    sparse_indexes: SparseMap<ChunkId>,
}

impl Index<InlandComponentId> for ComponentPoolBundle {
    type Output = ComponentPool;

    fn index(&self, index: InlandComponentId) -> &Self::Output {
        &self.pools[index as usize]
    }
}

impl IndexMut<InlandComponentId> for ComponentPoolBundle {
    fn index_mut(&mut self, index: InlandComponentId) -> &mut Self::Output {

        &mut self.pools[index as usize]
    }
}

//TODO: ComponentPoolBundle iterator