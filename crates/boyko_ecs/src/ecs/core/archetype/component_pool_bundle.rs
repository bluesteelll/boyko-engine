use std::ops::{Index, IndexMut};
use crate::ecs::core::component::Component;
use crate::ecs::identifiers::primitives::InlandComponentId;
use crate::ecs::memory::component_pool::ComponentPool;

struct ComponentPoolBundle {
    pools: Vec<ComponentPool>
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