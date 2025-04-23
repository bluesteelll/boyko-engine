use crate::ecs::identifiers::primitives::ComponentId;
/// 512-bit component mask
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(align(32))] 
pub struct ComponentMask {
    pub blocks: [u64; 8],
}


impl ComponentMask {
    pub fn new() -> Self {
        Self { blocks: [0; 8] }
    }
    
    #[inline]
    pub fn set(&mut self, component_id: ComponentId) {
        let block = (component_id / 64) % 8;
        let bit = component_id % 64;
        self.blocks[block] |= 1u64 << bit;
    }
    
    #[inline]
    pub fn unset(&mut self, component_id: ComponentId) {
        let block = (component_id / 64) % 8;
        let bit = component_id % 64;
        self.blocks[block] &= !(1u64 << bit);
    }
    
    #[inline]
    pub fn contains(&self, component_id: ComponentId) -> bool {
        let block = (component_id / 64) % 8;
        let bit = component_id % 64;
        (self.blocks[block] & (1u64 << bit)) != 0
    }
    
    pub fn from_components(components: &[ComponentId]) -> Self {
        let mut mask = Self::new();
        for &comp_id in components {
            mask.set(comp_id);
        }
        mask
    }
}