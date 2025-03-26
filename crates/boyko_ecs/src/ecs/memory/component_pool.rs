use std::any::TypeId;
use std::marker::PhantomData;
use std::mem::size_of;
use std::ptr::NonNull;
use crate::ecs::core::component::Component;
use crate::ecs::memory::arena::Arena;
use crate::ecs::memory::chunk::Chunk;
use crate::ecs::constants::{
    DEFAULT_CHUNKS_PER_POOL,
    DEFAULT_COMPONENTS_PER_CHUNK,
    TINY_COMPONENTS_PER_CHUNK,
    SMALL_COMPONENTS_PER_CHUNK,
    MEDIUM_COMPONENTS_PER_CHUNK,
    LARGE_COMPONENTS_PER_CHUNK,
    TINY_COMPONENT_THRESHOLD,
    SMALL_COMPONENT_THRESHOLD,
    MEDIUM_COMPONENT_THRESHOLD
};

/// Struct for indexing inland chunk
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentIndex {
    /// Chunk index
    pub id_chunk: u32,
    /// Chunk inland index
    pub id_inland: u32,
}

impl ComponentIndex {
    pub fn new(id_chunk: usize, id_inland: usize) -> Self {
        Self {
            id_chunk: id_chunk as u32,
            id_inland: id_inland as u32,
        }
    }
}

/// Type-erased ComponentPool interface
pub trait IComponentPool {
    fn component_type_id(&self) -> TypeId;
    
    fn component_size(&self) -> usize;

    /// Count active components in pool
    fn count(&self) -> usize;
    
    fn capacity(&self) -> usize;
    
    fn swap_remove(&mut self, index: ComponentIndex) -> bool;
}

/// Static component pool handling components of specific type
pub struct ComponentPool<T: Component> {
    arena: NonNull<Arena>,
    
    chunks: Vec<Chunk<T>>,
    
    count: usize,
    
    capacity: usize,

    free_slots: Vec<ComponentIndex>,

    /// Component type marker
    _marker: PhantomData<T>,
}

impl<T: Component> ComponentPool<T> {
    pub fn new(arena: &Arena, chunks_per_pool: usize, components_per_chunk: usize) -> Self {
        Self {
            arena: NonNull::from(arena),
            chunks: Vec::with_capacity(chunks_per_pool),
            count: 0,
            capacity: components_per_chunk,
            free_slots: Vec::new(),
            _marker: PhantomData,
        }
    }

    pub fn with_default_sizes(arena: &Arena) -> Self {
        let components_per_chunk = Self::get_optimal_chunk_capacity();
        Self::new(arena, DEFAULT_CHUNKS_PER_POOL, components_per_chunk)
    }

    fn get_optimal_chunk_capacity() -> usize {
        let size = size_of::<T>();
        if size <= TINY_COMPONENT_THRESHOLD {
            TINY_COMPONENTS_PER_CHUNK
        } else if size <= SMALL_COMPONENT_THRESHOLD {
            SMALL_COMPONENTS_PER_CHUNK
        } else if size <= MEDIUM_COMPONENT_THRESHOLD {
            MEDIUM_COMPONENTS_PER_CHUNK
        } else {
            LARGE_COMPONENTS_PER_CHUNK
        }
    }


    pub fn add(&mut self, component: T) -> Option<ComponentIndex> {
        // Сначала проверяем свободные слоты
        if let Some(slot) = self.free_slots.pop() {
            let chunk = &mut self.chunks[slot.id_chunk as usize];
            chunk.set(slot.id_inland as usize, component);
            self.count += 1;
            return Some(slot);
        }

        // Check top chunk if it is existed and not full
        if !self.chunks.is_empty() {
            let last_chunk_index = self.chunks.len() - 1;
            let last_chunk = &mut self.chunks[last_chunk_index];

            if last_chunk.count() < self.capacity {
                let id_inland = last_chunk.add(component).unwrap();
                self.count += 1;
                return Some(ComponentIndex::new(last_chunk_index, id_inland));
            }
        }

        // If free slots not available create new
        let arena = unsafe { &*self.arena.as_ptr() };
        let mut new_chunk = Chunk::<T>::new(arena, self.capacity);
        let id_inland = new_chunk.add(component).unwrap();
        let id_chunk = self.chunks.len();
        self.chunks.push(new_chunk);
        self.count += 1;

        Some(ComponentIndex::new(id_chunk, id_inland))
    }

    pub fn get(&self, index: ComponentIndex) -> Option<&T> {
        if index.id_chunk as usize >= self.chunks.len() {
            return None;
        }

        let chunk = &self.chunks[index.id_chunk as usize];
        chunk.get(index.id_inland as usize)
    }

    pub fn get_mut(&mut self, index: ComponentIndex) -> Option<&mut T> {
        if index.id_chunk as usize >= self.chunks.len() {
            return None;
        }

        let chunk = &mut self.chunks[index.id_chunk as usize];
        chunk.get_mut(index.id_inland as usize)
    }

    pub fn swap_remove(&mut self, index: ComponentIndex) -> bool {
        if index.id_chunk as usize >= self.chunks.len() {
            return false;
        }

        let chunk = &mut self.chunks[index.id_chunk as usize];

        // Try to use swap_remove for chunk
        if chunk.swap_remove(index.id_inland as usize) {
            // Если чанк теперь пуст и это не единственный чанк, можно удалить его
            if chunk.count() == 0 && self.chunks.len() > 1 {
                // Удаляем чанк
                self.chunks.swap_remove(index.id_chunk as usize);

                // Корректируем индексы в free_slots, если удаленный чанк был не последним
                if (index.id_chunk as usize) < self.chunks.len() {
                    // Найти все слоты, относящиеся к последнему чанку (который переместился)
                    let last_chunk_index = self.chunks.len();

                    for slot in self.free_slots.iter_mut() {
                        if slot.id_chunk == last_chunk_index as u32 {
                            slot.id_chunk = index.id_chunk;
                        }
                    }
                }
            } else {
                // Добавляем освободившийся слот
                self.free_slots.push(index);
            }

            self.count -= 1;
            return true;
        }

        false
    }
}

impl<T: Component> IComponentPool for ComponentPool<T> {
    fn component_type_id(&self) -> TypeId {
        TypeId::of::<T>()
    }

    fn component_size(&self) -> usize {
        size_of::<T>()
    }

    fn count(&self) -> usize {
        self.count
    }

    fn capacity(&self) -> usize {
        self.chunks.len() * self.capacity
    }

    fn swap_remove(&mut self, index: ComponentIndex) -> bool {
        self.swap_remove(index)
    }
}

// Фабрика компонентных пулов для создания type-erased пулов
pub struct ComponentPoolFactory;

impl ComponentPoolFactory {
    pub fn create<T: Component + 'static>(arena: &Arena) -> Box<dyn IComponentPool> {
        Box::new(ComponentPool::<T>::with_default_sizes(arena))
    }
}

