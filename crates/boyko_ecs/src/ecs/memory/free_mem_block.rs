use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MemFreeBlock {
    pub start: usize,
    pub end: usize,
}

impl MemFreeBlock {
    #[inline(always)]
    pub fn new(start: usize, end: usize) -> Self {
        debug_assert!(end > start, "Block size should be positive");
        Self { start, end }
    }

    #[inline(always)]
    pub fn size(&self) -> usize {
        self.end - self.start
    }
}

pub struct MemFreeBlockMaster {
    blocks: Vec<MemFreeBlock>,

    free_ind: Vec<usize>,

    mem_size_tree: BTreeMap<usize, Vec<usize>>,

    start_map: BTreeMap<usize, usize>,
    end_map: BTreeMap<usize, usize>,

    /// Parallel to `blocks`: stores the position of each block within its
    /// size bucket in `mem_size_tree`. `usize::MAX` sentinel means the slot
    /// is currently free (in `free_ind`) and has no bucket position.
    pos_in_size_vec: Vec<usize>,

    // Total number of active blocks
    size: usize,
}

impl MemFreeBlockMaster {
    pub fn new() -> Self {
        Self::with_capacity(1024)
    }

    pub fn new_init(arena_size: usize) -> Self {
        let mut block_master = Self::with_capacity(1024);
        block_master.insert(MemFreeBlock::new(0, arena_size));
        block_master
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            blocks: Vec::with_capacity(capacity),
            free_ind: Vec::with_capacity(capacity / 4),
            mem_size_tree: BTreeMap::new(),
            start_map: BTreeMap::new(),
            end_map: BTreeMap::new(),
            pos_in_size_vec: Vec::with_capacity(capacity),
            size: 0,
        }
    }

    #[inline(always)]
    fn add_block(&mut self, block: MemFreeBlock) -> usize {
        if let Some(index) = self.free_ind.pop() {
            self.blocks[index] = block;
            // pos_in_size_vec[index] remains usize::MAX (sentinel) from the previous remove;
            // insert() will overwrite it with the real bucket position.
            index
        } else {
            let index = self.blocks.len();
            self.blocks.push(block);
            // Push sentinel; insert() overwrites with the real position immediately after.
            self.pos_in_size_vec.push(usize::MAX);
            index
        }
    }

    /// Adding a memory block with possible merging of adjacent blocks
    pub fn insert(&mut self, mut block: MemFreeBlock) {
        debug_assert!(block.size() != 0);

        block = self.try_merge_remove(block);

        let index = self.add_block(block);
        let size = block.size();

        self.start_map.insert(block.start, index);
        self.end_map.insert(block.end, index);

        let bucket = self.mem_size_tree.entry(size)
            .or_insert_with(Vec::new);
        bucket.push(index);
        // Record this block's position inside its bucket for O(1) removal.
        self.pos_in_size_vec[index] = bucket.len() - 1;

        self.size += 1;

        debug_assert!(self.debug_invariants(), "MemFreeBlockMaster invariants violated after insert");
    }

    #[inline]
    fn try_merge_remove(&mut self, mut block: MemFreeBlock) -> MemFreeBlock {

        if let Some(&left_index) = self.end_map.get(&block.start) {
            let left_block = self.blocks[left_index];

            self.remove_block_index(left_index);

            block.start = left_block.start;
        }

        if let Some(&right_index) = self.start_map.get(&block.end) {
            let right_block = self.blocks[right_index];

            self.remove_block_index(right_index);

            block.end = right_block.end;
        }

        block
    }

    fn remove_block_index(&mut self, index: usize) {
        let block = self.blocks[index];

        self.start_map.remove(&block.start);
        self.end_map.remove(&block.end);

        let size = block.size();

        // O(1) removal via reverse index: look up position directly instead of scanning.
        let pos = self.pos_in_size_vec[index];
        debug_assert!(pos != usize::MAX, "removing already-freed slot (double free or sentinel not set)");

        if let Some(bucket) = self.mem_size_tree.get_mut(&size) {
            bucket.swap_remove(pos);

            // If swap_remove moved the tail element into `pos`, update its reverse index.
            if pos < bucket.len() {
                let moved_idx = bucket[pos];
                self.pos_in_size_vec[moved_idx] = pos;
            }

            if bucket.is_empty() {
                self.mem_size_tree.remove(&size);
            }
        }

        // Mark this slot as free in the reverse-index vec.
        self.pos_in_size_vec[index] = usize::MAX;

        self.free_ind.push(index);

        self.size -= 1;
    }

    pub fn find_best_fit(&self, min_size: usize) -> Option<MemFreeBlock> {
        // Find the first entry where size >= min_size
        self.mem_size_tree.range(min_size..)
            .next()
            .and_then(|(_, indices)| indices.first().map(|&idx| self.blocks[idx]))
    }


    /// Returns start address
    pub fn allocate(&mut self, size: usize) -> Option<MemFreeBlock> {
        if size == 0 {
            return None;
        }

        let (block_index, block) = self.find_best_fit_with_index(size)?;

        self.remove_block_index(block_index);

        // If there is a remainder, return it back to the pool
        let remainder_size = block.size() - size;
        if remainder_size > 0 {
            let remainder = MemFreeBlock::new(block.start + size, block.end);
            self.insert(remainder);

            // insert() already fires debug_assert!(debug_invariants()) internally.
            // Return only the requested portion of the block
            return Some(MemFreeBlock::new(block.start, block.start + size));
        }

        // Return the entire block if it fits the requested size exactly
        debug_assert!(self.debug_invariants(), "MemFreeBlockMaster invariants violated after allocate");
        Some(block)
    }

    /// Allocates an aligned memory block
    pub fn allocate_aligned(&mut self, size: usize, align: usize) -> Option<MemFreeBlock> {
        if size == 0 {
            return None;
        }

        // Search for a block accounting for the maximum possible alignment
        let required_size = size + align - 1;
        let (block_index, block) = self.find_best_fit_with_index(required_size)?;

        self.remove_block_index(block_index);

        // Compute the aligned start address
        let aligned_start = crate::ecs::memory::utils::align_up(block.start, align);

        // Create the aligned block
        let aligned_block = MemFreeBlock::new(aligned_start, aligned_start + size);

        // If alignment created a gap at the start, return it to the pool
        if aligned_start > block.start {
            self.insert(MemFreeBlock::new(block.start, aligned_start));
        }

        // If there is a remainder after the allocated memory, return it to the pool
        let aligned_end = aligned_start + size;
        if block.end > aligned_end {
            self.insert(MemFreeBlock::new(aligned_end, block.end));
        }

        // insert() fires debug_assert!(debug_invariants()) for each spill path above;
        // assert here for the no-spill case and for clarity at function exit.
        debug_assert!(self.debug_invariants(), "MemFreeBlockMaster invariants violated after allocate_aligned");
        Some(aligned_block)
    }

    fn find_best_fit_with_index(&self, min_size: usize) -> Option<(usize, MemFreeBlock)> {
        self.mem_size_tree.range(min_size..)
            .next()
            .and_then(|(_, indices)| {
                indices.first().map(|&idx| (idx, self.blocks[idx]))
            })
    }


    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }


    pub fn total_free_size(&self) -> usize {
        self.mem_size_tree.iter()
            .map(|(size, indices)| size * indices.len())
            .sum()
    }

    pub fn get_by_index(&self, index: usize) -> Option<MemFreeBlock> {
        if index >= self.size {
            return None;
        }

        let mut current_idx = 0;

        for (_, indices) in self.mem_size_tree.iter() {
            if current_idx + indices.len() > index {
                // Have found the right size range
                let idx_in_vec = index - current_idx;
                let block_index = indices[idx_in_vec];
                return Some(self.blocks[block_index]);
            }
            current_idx += indices.len();
        }

        None
    }

    pub fn get_memory_stats(&self) -> MemoryStats {
        MemoryStats {
            active_blocks: self.size,
            total_blocks: self.blocks.len(),
            free_slots: self.free_ind.len(),
            total_memory: self.total_free_size(),
        }
    }

    pub fn defragment(&mut self) {
        if self.free_ind.is_empty() {
            return;
        }

        let mut new_blocks = Vec::with_capacity(self.size);
        let mut new_mem_size_tree: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        let mut new_start_map = BTreeMap::new();
        let mut new_end_map = BTreeMap::new();

        // Iterate through the size tree and create a new vector of blocks
        for (size, indices) in &self.mem_size_tree {
            let mut new_indices = Vec::with_capacity(indices.len());

            for &old_index in indices {
                let block = self.blocks[old_index];
                let new_index = new_blocks.len();

                new_blocks.push(block);
                new_indices.push(new_index);
                new_start_map.insert(block.start, new_index);
                new_end_map.insert(block.end, new_index);
            }

            new_mem_size_tree.insert(*size, new_indices);
        }

        // Rebuild pos_in_size_vec from scratch: all active slots get their real
        // bucket position; no free slots remain after defragment (free_ind is cleared).
        let mut new_pos_in_size_vec = vec![usize::MAX; new_blocks.len()];
        for (_, bucket) in &new_mem_size_tree {
            for (pos, &block_idx) in bucket.iter().enumerate() {
                new_pos_in_size_vec[block_idx] = pos;
            }
        }

        self.blocks = new_blocks;
        self.mem_size_tree = new_mem_size_tree;
        self.start_map = new_start_map;
        self.end_map = new_end_map;
        self.free_ind.clear();
        self.pos_in_size_vec = new_pos_in_size_vec;

        debug_assert!(self.debug_invariants(), "MemFreeBlockMaster invariants violated after defragment");
    }

    /// Checks structural invariants that must hold at every stable point.
    ///
    /// Invariants:
    /// - `start_map` and `end_map` contain the same number of entries.
    /// - Both maps contain exactly `self.size` entries (active block count).
    /// - Every slot is either active (tracked by maps) or free (`free_ind`):
    ///   `self.size + self.free_ind.len() == self.blocks.len()`.
    /// - `blocks.len() == pos_in_size_vec.len()` (CR-1).
    /// - Active slots (not in `free_ind`) have `pos_in_size_vec[i] != usize::MAX`
    ///   and `mem_size_tree[blocks[i].size()][pos_in_size_vec[i]] == i`.
    /// - Free slots (in `free_ind`) have `pos_in_size_vec[i] == usize::MAX`.
    ///
    /// Compiled in both debug and non-debug builds; `debug_assert!` callers
    /// ensure the body is elided in release. Returns `true` when all hold.
    pub(crate) fn debug_invariants(&self) -> bool {
        // Basic size accounting
        if self.start_map.len() != self.end_map.len() {
            return false;
        }
        if self.start_map.len() != self.size {
            return false;
        }
        if self.size + self.free_ind.len() != self.blocks.len() {
            return false;
        }
        // CR-1: reverse-index vec must stay parallel to blocks.
        if self.blocks.len() != self.pos_in_size_vec.len() {
            return false;
        }

        // Verify sentinel values for free slots.
        for &free_slot in &self.free_ind {
            if self.pos_in_size_vec[free_slot] != usize::MAX {
                return false;
            }
        }

        // Verify reverse-index accuracy for active slots.
        let free_set: std::collections::HashSet<usize> = self.free_ind.iter().copied().collect();
        for i in 0..self.blocks.len() {
            if free_set.contains(&i) {
                continue;
            }
            // Active slot: pos must not be sentinel, and must point back correctly.
            let pos = self.pos_in_size_vec[i];
            if pos == usize::MAX {
                return false;
            }
            let size = self.blocks[i].size();
            let Some(bucket) = self.mem_size_tree.get(&size) else {
                return false;
            };
            if pos >= bucket.len() || bucket[pos] != i {
                return false;
            }
        }

        true
    }

    /// Test-only accessor: returns the raw `pos_in_size_vec` value for a block index.
    /// `usize::MAX` means the slot is free (sentinel).
    #[cfg(test)]
    pub(crate) fn pos_in_size_vec_for_test(&self, idx: usize) -> usize {
        self.pos_in_size_vec[idx]
    }
}

pub struct MemoryStats {
    pub active_blocks: usize,
    pub total_blocks: usize,
    pub free_slots: usize,
    pub total_memory: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1. new_init produces a single block covering [0, arena_size).
    #[test]
    fn master_new_init_single_block_covers_arena() {
        let master = MemFreeBlockMaster::new_init(1024);
        assert_eq!(master.len(), 1);
        let block = master.find_best_fit(1).expect("must have one block");
        assert_eq!(block.start, 0);
        assert_eq!(block.end, 1024);
        assert!(master.debug_invariants());
    }

    // 2. Two disjoint blocks are not merged.
    #[test]
    fn master_insert_two_disjoint_blocks_no_merge() {
        let mut master = MemFreeBlockMaster::new();
        master.insert(MemFreeBlock::new(0, 100));
        master.insert(MemFreeBlock::new(200, 300));
        assert_eq!(master.len(), 2);
        assert!(master.debug_invariants());
        // Both offsets must be indexed.
        assert!(master.start_map.contains_key(&0));
        assert!(master.start_map.contains_key(&200));
    }

    // 3. Insert [0,100) then [100,200): left-neighbor merge -> [0,200).
    #[test]
    fn master_insert_merges_left_neighbor() {
        let mut master = MemFreeBlockMaster::new();
        master.insert(MemFreeBlock::new(0, 100));
        master.insert(MemFreeBlock::new(100, 200));
        assert_eq!(master.len(), 1);
        assert!(master.debug_invariants());
        let block = master.find_best_fit(1).expect("one merged block");
        assert_eq!(block.start, 0);
        assert_eq!(block.end, 200);
    }

    // 4. Insert [100,200) then [0,100): right-neighbor merge -> [0,200).
    #[test]
    fn master_insert_merges_right_neighbor() {
        let mut master = MemFreeBlockMaster::new();
        master.insert(MemFreeBlock::new(100, 200));
        master.insert(MemFreeBlock::new(0, 100));
        assert_eq!(master.len(), 1);
        assert!(master.debug_invariants());
        let block = master.find_best_fit(1).expect("one merged block");
        assert_eq!(block.start, 0);
        assert_eq!(block.end, 200);
    }

    // 5. Insert [0,100), [200,300), [100,200): both-sides merge -> [0,300).
    #[test]
    fn master_insert_merges_both_neighbors() {
        let mut master = MemFreeBlockMaster::new();
        master.insert(MemFreeBlock::new(0, 100));
        master.insert(MemFreeBlock::new(200, 300));
        master.insert(MemFreeBlock::new(100, 200));
        assert_eq!(master.len(), 1);
        assert!(master.debug_invariants());
        let block = master.find_best_fit(1).expect("one merged block");
        assert_eq!(block.start, 0);
        assert_eq!(block.end, 300);
    }

    // 6. allocate(100) on a single 100-byte block leaves the pool empty.
    #[test]
    fn master_allocate_exact_size_removes_block() {
        let mut master = MemFreeBlockMaster::new();
        master.insert(MemFreeBlock::new(0, 100));
        let result = master.allocate(100);
        assert!(result.is_some());
        assert_eq!(master.len(), 0);
        assert!(master.debug_invariants());
    }

    // 7. allocate(40) on a 100-byte block splits and re-inserts a 60-byte remainder.
    #[test]
    fn master_allocate_smaller_splits_remainder() {
        let mut master = MemFreeBlockMaster::new();
        master.insert(MemFreeBlock::new(0, 100));
        let allocated = master.allocate(40).expect("must succeed");
        assert_eq!(allocated.start, 0);
        assert_eq!(allocated.end, 40);
        assert_eq!(master.len(), 1);
        assert!(master.debug_invariants());
        let remainder = master.find_best_fit(1).expect("remainder block");
        assert_eq!(remainder.start, 40);
        assert_eq!(remainder.end, 100);
    }

    // 8. allocate_aligned on [10,200) with size=64, align=64 creates head and tail spills.
    #[test]
    fn master_allocate_aligned_creates_head_and_tail_spill() {
        let mut master = MemFreeBlockMaster::new();
        // Block [10, 200): total 190 bytes.
        master.insert(MemFreeBlock::new(10, 200));
        // Request 64 bytes at 64-byte alignment.
        let result = master.allocate_aligned(64, 64);
        let block = result.expect("must find an aligned block");
        // Aligned start must be 64.
        assert_eq!(block.start, 64);
        assert_eq!(block.end, 128);
        assert_eq!(block.start % 64, 0);
        // Two spill blocks: [10, 64) and [128, 200).
        assert_eq!(master.len(), 2);
        assert!(master.debug_invariants());
    }

    // 9. Allocate then re-insert the same range coalesces back to the original block.
    #[test]
    fn master_allocate_then_insert_coalesces_back() {
        let mut master = MemFreeBlockMaster::new();
        master.insert(MemFreeBlock::new(0, 1024));
        let allocated = master.allocate(64).expect("must succeed");
        // Return the block.
        master.insert(allocated);
        assert_eq!(master.len(), 1);
        assert!(master.debug_invariants());
        let restored = master.find_best_fit(1).expect("restored block");
        assert_eq!(restored.start, 0);
        assert_eq!(restored.end, 1024);
    }

    // 10. find_best_fit picks the smallest block that satisfies the request.
    #[test]
    fn master_find_best_fit_picks_smallest_sufficient() {
        let mut master = MemFreeBlockMaster::new();
        // Insert three disjoint blocks with sizes 100, 200, 300.
        master.insert(MemFreeBlock::new(0, 100));
        master.insert(MemFreeBlock::new(1000, 1200));   // size 200
        master.insert(MemFreeBlock::new(2000, 2300));   // size 300
        let best = master.find_best_fit(150).expect("must find a 200-byte block");
        assert_eq!(best.size(), 200);
    }

    // 11. remove_block_index removes an entry from both maps; free_ind grows.
    #[test]
    fn master_remove_block_index_keeps_maps_synchronized() {
        let mut master = MemFreeBlockMaster::new();
        master.insert(MemFreeBlock::new(0, 100));
        let free_before = master.free_ind.len();
        // Allocate clears the block via remove_block_index internally.
        master.allocate(100).expect("must succeed");
        // Maps must not contain the freed offsets.
        assert!(!master.start_map.contains_key(&0));
        assert!(!master.end_map.contains_key(&100));
        // The slot was returned to free_ind.
        assert_eq!(master.free_ind.len(), free_before + 1);
        assert!(master.debug_invariants());
    }

    // 12. defragment after insert+remove compacts free slots.
    #[test]
    fn master_defragment_compacts_free_slots() {
        let mut master = MemFreeBlockMaster::new();
        // Insert several blocks, then allocate some to create free slots.
        master.insert(MemFreeBlock::new(0, 100));
        master.insert(MemFreeBlock::new(200, 300));
        master.insert(MemFreeBlock::new(400, 500));
        master.allocate(100).expect("first alloc");
        // free_ind must be non-empty now.
        assert!(!master.free_ind.is_empty());
        master.defragment();
        assert!(master.free_ind.is_empty(), "defragment must empty free_ind");
        assert!(master.debug_invariants());
    }

    // 13. Stress: 65 536 non-overlapping inserts must not panic and keep len() consistent.
    // Run via: cargo test -- --ignored
    #[test]
    #[ignore]
    fn master_stress_btreemap_handles_64k_inserts_no_panic() {
        let n = 65_536usize;
        let mut master = MemFreeBlockMaster::with_capacity(n);
        for i in 0..n {
            // Gap of 100 between each block ensures no merging.
            let start = i * 200;
            master.insert(MemFreeBlock::new(start, start + 100));
        }
        assert_eq!(master.len(), n);
        assert!(master.debug_invariants());
    }

    // 14. Positive invariant test: debug_invariants() returns true after each op.
    #[test]
    fn master_invariant_positive_after_each_op() {
        let mut master = MemFreeBlockMaster::new_init(4096);
        assert!(master.debug_invariants(), "after new_init");

        master.insert(MemFreeBlock::new(5000, 5100));
        assert!(master.debug_invariants(), "after insert");

        let _alloc = master.allocate(64);
        assert!(master.debug_invariants(), "after allocate");

        let _aligned = master.allocate_aligned(64, 64);
        assert!(master.debug_invariants(), "after allocate_aligned");

        master.insert(MemFreeBlock::new(6000, 6100));
        master.allocate(100).expect("setup for defragment");
        master.defragment();
        assert!(master.debug_invariants(), "after defragment");
    }

    // 15. Functional regression: O(1) reverse-index path is exercised and invariants hold
    //     throughout 1000 same-size inserts + 500 allocations.
    #[test]
    fn master_remove_uses_reverse_index_o1() {
        let n = 1_000usize;
        let mut master = MemFreeBlockMaster::with_capacity(n);

        // Insert 1000 disjoint same-size (100-byte) blocks.
        for i in 0..n {
            let start = i * 200; // gap of 100 ensures no merging
            master.insert(MemFreeBlock::new(start, start + 100));
        }
        assert!(master.debug_invariants(), "after 1000 inserts");

        // Allocate 500 of them, asserting invariants on each removal.
        for _ in 0..500 {
            let block = master.allocate(100).expect("must allocate from same-size pool");
            let _ = block;
            assert!(master.debug_invariants(), "invariant violated during alloc");
        }

        assert_eq!(master.len(), 500, "500 blocks should remain");
    }

    // 16. Swap_remove mid-bucket: reverse index of the moved tail element is updated.
    //     Also tests removing the LAST element (pos == bucket.len() post-swap_remove => no fix-up).
    #[test]
    fn master_insert_then_remove_middle_of_bucket() {
        let mut master = MemFreeBlockMaster::with_capacity(16);

        // Insert 5 same-size (64-byte) blocks at disjoint addresses.
        // Using addresses spaced >64 apart to prevent merging.
        let starts = [0usize, 200, 400, 600, 800];
        for &s in &starts {
            master.insert(MemFreeBlock::new(s, s + 64));
        }
        assert_eq!(master.len(), 5);
        assert!(master.debug_invariants(), "after 5 inserts");

        // Allocate the middle block (at start=400, end=464).
        // We force-remove it by calling allocate(64) enough times to reach it,
        // but find_best_fit always picks the first in the bucket. Instead, we
        // directly use allocate() which removes the first-in-bucket block each time.
        // Allocate 3 blocks total: removes index 0, 1, 2 in bucket order.
        let _b0 = master.allocate(64).expect("alloc 1");
        assert!(master.debug_invariants(), "after alloc 1 (removes bucket[0])");

        let _b1 = master.allocate(64).expect("alloc 2");
        assert!(master.debug_invariants(), "after alloc 2 (removes bucket[0] again, tail moved)");

        let _b2 = master.allocate(64).expect("alloc 3");
        assert!(master.debug_invariants(), "after alloc 3 (removes bucket[0] again, tail moved)");

        // 2 blocks remain. Allocate the last one (removes last element, no fix-up path).
        let _b3 = master.allocate(64).expect("alloc 4 (second-to-last)");
        assert!(master.debug_invariants(), "after alloc 4 (second-to-last)");

        // Now only 1 block remains. Removing it: pos == 0, after swap_remove bucket is empty
        // (pos == bucket.len() after removal), so no fix-up is needed.
        let _b4 = master.allocate(64).expect("alloc 5 (last element)");
        assert!(master.debug_invariants(), "after alloc 5 (last element, no fix-up case)");
        assert_eq!(master.len(), 0, "all blocks consumed");
    }

    // 17. defragment rebuilds pos_in_size_vec consistently; alloc works after defragment.
    #[test]
    fn master_defragment_preserves_reverse_index() {
        let mut master = MemFreeBlockMaster::with_capacity(16);

        // Insert and partially allocate to create free slots.
        for i in 0..6usize {
            master.insert(MemFreeBlock::new(i * 200, i * 200 + 64));
        }
        // Allocate 3: creates 3 free slots in blocks vec.
        let _a = master.allocate(64).expect("alloc 1");
        let _b = master.allocate(64).expect("alloc 2");
        let _c = master.allocate(64).expect("alloc 3");

        assert!(!master.free_ind.is_empty(), "free slots must exist before defragment");
        assert!(master.debug_invariants(), "before defragment");

        master.defragment();

        assert!(master.free_ind.is_empty(), "free_ind must be empty after defragment");
        assert!(master.debug_invariants(), "after defragment");
        assert_eq!(master.pos_in_size_vec.len(), master.blocks.len(),
            "pos_in_size_vec must parallel blocks after defragment");

        // All active blocks must have valid (non-sentinel) positions.
        for i in 0..master.blocks.len() {
            assert_ne!(master.pos_in_size_vec_for_test(i), usize::MAX,
                "active block {} must not have sentinel pos after defragment", i);
        }

        // Allocate again after defragment to confirm pool is still operational.
        let post = master.allocate(64).expect("alloc after defragment");
        assert_eq!(post.size(), 64);
        assert!(master.debug_invariants(), "after alloc post-defragment");
    }

    // 18. After allocate, the freed block's pos_in_size_vec entry is usize::MAX (sentinel).
    #[test]
    fn master_free_slot_has_sentinel_pos() {
        let mut master = MemFreeBlockMaster::with_capacity(8);

        master.insert(MemFreeBlock::new(0, 64));
        master.insert(MemFreeBlock::new(200, 264));

        // Find which block index corresponds to the first bucket entry.
        // After inserting two same-size disjoint blocks, both are in the same bucket.
        // The block vec indices are 0 and 1 (insertion order).
        // allocate(64) removes block at bucket[0] — which is blocks index 0.
        let freed_block = master.allocate(64).expect("alloc must succeed");
        let _ = freed_block;

        // Block index 0 was freed; its pos_in_size_vec entry must be usize::MAX.
        assert_eq!(
            master.pos_in_size_vec_for_test(0),
            usize::MAX,
            "freed slot must have sentinel usize::MAX in pos_in_size_vec"
        );

        // Block index 1 is still active; its pos must be a valid bucket position (0).
        assert_eq!(
            master.pos_in_size_vec_for_test(1),
            0,
            "remaining active block must be at bucket position 0 after the head was removed"
        );

        assert!(master.debug_invariants());
    }

    // 19. Stress: 64k same-size alloc/free roundtrip. #[ignore] for CI speed.
    #[test]
    #[ignore]
    fn master_stress_same_size_64k_alloc_free() {
        use std::collections::VecDeque;

        let n = 64_000usize;
        let block_size = 64usize;
        let stride = block_size * 2; // gap prevents merging

        let mut master = MemFreeBlockMaster::with_capacity(n);

        // Insert all blocks upfront.
        for i in 0..n {
            master.insert(MemFreeBlock::new(i * stride, i * stride + block_size));
        }
        assert_eq!(master.len(), n);
        assert!(master.debug_invariants(), "after initial inserts");

        // Alloc all, collect in a queue.
        let mut freed: VecDeque<MemFreeBlock> = VecDeque::with_capacity(n);
        for _ in 0..n {
            let b = master.allocate(block_size).expect("alloc must succeed");
            freed.push_back(b);
        }
        assert_eq!(master.len(), 0, "all blocks consumed");

        // Re-insert all (simulated "random" return order via queue — deterministic).
        // Each re-insert may coalesce with adjacent previously returned blocks.
        while let Some(b) = freed.pop_front() {
            master.insert(b);
            assert!(master.debug_invariants(), "invariant violated during bulk re-insert");
        }

        assert!(master.debug_invariants(), "after full roundtrip");
    }
}
