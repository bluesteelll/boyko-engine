use std::collections::{BTreeMap, HashMap};

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

    start_map: HashMap<usize, usize>,
    end_map: HashMap<usize, usize>,

    // Общее количество активных блоков
    size: usize,
}

impl MemFreeBlockMaster {
    pub fn new() -> Self {
        Self {
            blocks: Vec::with_capacity(1024),
            free_ind: Vec::new(),
            mem_size_tree: BTreeMap::new(),
            start_map: HashMap::new(),
            end_map: HashMap::new(),
            size: 0,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            blocks: Vec::with_capacity(capacity),
            free_ind: Vec::with_capacity(capacity / 4),  // Резервируем место для ~25% свободных индексов
            mem_size_tree: BTreeMap::new(),
            start_map: HashMap::with_capacity(capacity),
            end_map: HashMap::with_capacity(capacity),
            size: 0,
        }
    }

    #[inline(always)]
    fn add_block(&mut self, block: MemFreeBlock) -> usize {
        if let Some(index) = self.free_ind.pop() {
            self.blocks[index] = block;
            index
        } else {
            let index = self.blocks.len();
            self.blocks.push(block);
            index
        }
    }

    /// Adding a memory block with possible merging of adjacent blocks
    pub fn insert(&mut self, mut block: MemFreeBlock){
        debug_assert!(block.size() != 0);

        block = self.try_merge_remove(block);

        let index = self.add_block(block);
        let size = block.size();

        self.start_map.insert(block.start, index);
        self.end_map.insert(block.end, index);

        self.mem_size_tree.entry(size)
            .or_insert_with(Vec::new)
            .push(index);

        self.size += 1;
    }

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
        if let Some(indices) = self.mem_size_tree.get_mut(&size) {
            if let Some(pos) = indices.iter().position(|&idx| idx == index) {
                indices.swap_remove(pos);

                if indices.is_empty() {
                    self.mem_size_tree.remove(&size);
                }
            }
        }

        self.free_ind.push(index);

        self.size -= 1;
    }

    pub fn find_best_fit(&self, min_size: usize) -> Option<MemFreeBlock> {
        // Найти первую запись, где размер >= min_size
        self.mem_size_tree.range(min_size..)
            .next()
            .and_then(|(_, indices)| indices.first().map(|&idx| self.blocks[idx]))
    }


    /// Returns start address
    pub fn allocate(&mut self, size: usize) -> Option<usize> {
        if size == 0 {
            return None;
        }

        let (block_index, block) = self.find_best_fit_with_index(size)?;

        self.remove_block_index(block_index);

        // If remainder available insert it to the pool
        let remainder_size = block.size() - size;
        if remainder_size > 0 {
            let remainder = MemFreeBlock::new(block.start + size, block.end);
            self.insert(remainder);
        }

        Some(block.start)
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
        let mut new_mem_size_tree = BTreeMap::new();
        let mut new_start_map = HashMap::with_capacity(self.size);
        let mut new_end_map = HashMap::with_capacity(self.size);
        let mut index_map = HashMap::with_capacity(self.size);

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
                index_map.insert(old_index, new_index);
            }

            new_mem_size_tree.insert(*size, new_indices);
        }

        self.blocks = new_blocks;
        self.mem_size_tree = new_mem_size_tree;
        self.start_map = new_start_map;
        self.end_map = new_end_map;
        self.free_ind.clear();
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

    #[test]
    fn test_best_fit() {
        let mut set = MemFreeBlockMaster::new();

        // Добавляем блоки разного размера
        set.insert(MemFreeBlock::new(100, 200)); // размер 100
        set.insert(MemFreeBlock::new(300, 350)); // размер 50
        set.insert(MemFreeBlock::new(500, 600)); // размер 100
        set.insert(MemFreeBlock::new(700, 760)); // размер 60

        // Проверяем best-fit
        let block = set.find_best_fit(60).unwrap();
        assert_eq!(block.size(), 60);

        let block = set.find_best_fit(70).unwrap();
        assert_eq!(block.size(), 100);
    }

    #[test]
    fn test_merge_blocks() {
        let mut set = MemFreeBlockMaster::new();

        // Добавляем несмежные блоки
        set.insert(MemFreeBlock::new(100, 200));
        set.insert(MemFreeBlock::new(300, 400));
        assert_eq!(set.len(), 2);

        // Добавляем блок, смежный с существующими
        set.insert(MemFreeBlock::new(200, 300));

        // Проверяем, что все три блока слились в один
        assert_eq!(set.len(), 1);

        let block = set.find_best_fit(1).unwrap();
        assert_eq!(block.start, 100);
        assert_eq!(block.end, 400);
    }

    #[test]
    fn test_allocate_with_remainder() {
        let mut set = MemFreeBlockMaster::new();

        // Добавляем блок
        set.insert(MemFreeBlock::new(100, 200)); // размер 100

        // Выделяем часть блока
        let addr = set.allocate(40).unwrap();
        assert_eq!(addr, 100);

        // Проверяем, что остаток добавлен обратно
        assert_eq!(set.len(), 1);
        let remainder = set.find_best_fit(1).unwrap();
        assert_eq!(remainder.size(), 60);
        assert_eq!(remainder.start, 140);
    }

    #[test]
    fn test_reuse_slots() {
        let mut set = MemFreeBlockMaster::new();

        // Добавляем несколько блоков
        set.insert(MemFreeBlock::new(100, 200));
        set.insert(MemFreeBlock::new(300, 400));
        set.insert(MemFreeBlock::new(500, 600));

        // Удаляем средний блок
        set.allocate(100); // Удаляет блок по размеру (100, 200)
        set.allocate(100); // Удаляет блок по размеру (500, 600)

        // Проверяем статистику
        let stats = set.get_memory_stats();
        assert_eq!(stats.active_blocks, 1);
        assert_eq!(stats.free_slots, 2);

        // Добавляем новый блок и проверяем, что он использует свободный слот
        set.insert(MemFreeBlock::new(700, 800));

        let stats = set.get_memory_stats();
        assert_eq!(stats.active_blocks, 2);
        assert_eq!(stats.free_slots, 1);
    }

    #[test]
    fn test_defragmentation() {
        let mut set = MemFreeBlockMaster::new();

        // Добавляем много блоков и затем удаляем некоторые из них
        for i in 0..100 {
            set.insert(MemFreeBlock::new(i * 1000, i * 1000 + 500));
        }

        // Удаляем половину блоков
        for _ in 0..50 {
            set.allocate(500);
        }

        let before = set.get_memory_stats();
        assert_eq!(before.active_blocks, 50);
        assert_eq!(before.free_slots, 50);

        // Выполняем дефрагментацию
        set.defragment();

        let after = set.get_memory_stats();
        assert_eq!(after.active_blocks, 50);
        assert_eq!(after.free_slots, 0);
        assert_eq!(after.total_blocks, 50);
    }
}