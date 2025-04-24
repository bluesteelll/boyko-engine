use crate::ecs::core::component::component_mask::ComponentMask;
use boyko_utils::bit_mask::bit_set::BitSet;


#[derive(Clone, Debug)]
pub struct ArchetypeSignature {
    /// Original 512-bit mask
    pub mask: ComponentMask,
    
    /// Level 2: 8-bit representation showing which blocks contain bits
    /// Bit i is set if blocks[i] != 0
    pub block_summary: BitSet<u8>,
    
    /// Level 3: 32-bit representation showing which 16-bit sections contain bits
    /// 4 sections per each of 8 blocks = 32 bits
    pub section_summary: BitSet<u32>,
}

impl ArchetypeSignature {
    /// Creates a new hierarchical index bitmap from a component mask
    pub fn new(mask: ComponentMask) -> Self {
        let mut block_summary = BitSet::new();
        let mut section_summary = BitSet::new();
        
        // Build hierarchical levels
        for i in 0..8 {
            let block = mask.blocks[i];
            
            // If there is at least one set bit in the block
            if !block.is_empty() {
                // Set the corresponding bit in block_summary
                block_summary.set(i);
                
                // Split the 64-bit block into 4 sections of 16 bits each
                for j in 0..4 {
                    let section_mask = BitSet::from_value(0xFFFF << (j * 16));
                    if !(block & section_mask).is_empty() {
                        // Set the corresponding bit in section_summary
                        // i*4+j gives a unique index for each of the 32 sections
                        section_summary.set(i * 4 + j);
                    }
                }
            }
        }
        
        Self {
            mask,
            block_summary,
            section_summary,
        }
    }
    
    /// Checks if the current mask contains all bits from the query mask
    #[inline]
    pub fn contains(&self, query: &ArchetypeSignature) -> bool {
        // Quick check at block level
        // If query has a block that we don't have, return false
        if !(query.block_summary & !self.block_summary).is_empty() {
            return false;
        }
        
        // Quick check at section level
        // If query has a section that we don't have, return false
        if !(query.section_summary & !self.section_summary).is_empty() {
            return false;
        }
        
        // Detailed check at block level
        // Only check blocks that exist in the query
        let mut i = 0;
        while let Some(block_idx) = query.block_summary.iter_ones().nth(i) {
            // If (q & !s) != 0, it means the query has bits that we don't have
            if !(query.mask.blocks[block_idx] & !self.mask.blocks[block_idx]).is_empty() {
                return false;
            }
            i += 1;
        }
        
        true
    }
}



