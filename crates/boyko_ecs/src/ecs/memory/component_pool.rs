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

/// Структура для индексации компонента внутри пула
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentIndex {
    /// Индекс чанка
    pub chunk_index: u32,
    /// Индекс внутри чанка
    pub index_in_chunk: u32,
}

impl ComponentIndex {
    pub fn new(chunk_index: usize, index_in_chunk: usize) -> Self {
        Self {
            chunk_index: chunk_index as u32,
            index_in_chunk: index_in_chunk as u32,
        }
    }
}

/// Интерфейс для type-erased компонентного пула
pub trait IComponentPool {
    /// Возвращает TypeId компонентов в пуле
    fn component_type_id(&self) -> TypeId;

    /// Возвращает размер компонента в байтах
    fn component_size(&self) -> usize;

    /// Возвращает общее количество компонентов в пуле
    fn count(&self) -> usize;

    /// Возвращает вместимость пула
    fn capacity(&self) -> usize;

    /// Удаляет компонент по индексу
    fn swap_remove(&mut self, index: ComponentIndex) -> bool;
}

/// Компонентный пул, который хранит компоненты типа T в чанках
pub struct ComponentPool<T: Component> {
    /// Арена, из которой аллоцируются чанки
    arena: NonNull<Arena>,

    /// Чанки с компонентами
    chunks: Vec<Chunk<T>>,

    /// Количество компонентов в пуле
    count: usize,

    /// Вместимость чанка
    chunk_capacity: usize,

    /// Индексы свободных слотов в частично заполненных чанках
    /// (индекс чанка, индекс внутри чанка)
    free_slots: Vec<ComponentIndex>,

    /// Маркер для указания типа компонентов в пуле
    _marker: PhantomData<T>,
}

impl<T: Component> ComponentPool<T> {
    /// Создает новый компонентный пул с указанными размерами
    pub fn new(arena: &Arena, chunks_per_pool: usize, components_per_chunk: usize) -> Self {
        Self {
            arena: NonNull::from(arena),
            chunks: Vec::with_capacity(chunks_per_pool),
            count: 0,
            chunk_capacity: components_per_chunk,
            free_slots: Vec::new(),
            _marker: PhantomData,
        }
    }

    /// Создает компонентный пул с учетом размера компонента
    pub fn with_default_sizes(arena: &Arena) -> Self {
        let components_per_chunk = Self::get_optimal_chunk_capacity();
        Self::new(arena, DEFAULT_CHUNKS_PER_POOL, components_per_chunk)
    }

    /// Выбирает оптимальный размер чанка в зависимости от размера компонента
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

    /// Добавляет компонент в пул и возвращает его индекс
    pub fn add(&mut self, component: T) -> Option<ComponentIndex> {
        // Сначала проверяем свободные слоты
        if let Some(slot) = self.free_slots.pop() {
            let chunk = &mut self.chunks[slot.chunk_index as usize];
            chunk.set(slot.index_in_chunk as usize, component);
            self.count += 1;
            return Some(slot);
        }

        // Проверяем последний чанк, если он есть и не заполнен
        if !self.chunks.is_empty() {
            let last_chunk_index = self.chunks.len() - 1;
            let last_chunk = &mut self.chunks[last_chunk_index];

            if last_chunk.count() < self.chunk_capacity {
                let index_in_chunk = last_chunk.add(component).unwrap();
                self.count += 1;
                return Some(ComponentIndex::new(last_chunk_index, index_in_chunk));
            }
        }

        // Если нет свободных слотов или последний чанк заполнен, создаем новый чанк
        let arena = unsafe { &*self.arena.as_ptr() };
        let mut new_chunk = Chunk::<T>::new(arena, self.chunk_capacity);
        let index_in_chunk = new_chunk.add(component).unwrap();
        let chunk_index = self.chunks.len();
        self.chunks.push(new_chunk);
        self.count += 1;

        Some(ComponentIndex::new(chunk_index, index_in_chunk))
    }

    /// Получает ссылку на компонент по индексу
    pub fn get(&self, index: ComponentIndex) -> Option<&T> {
        if index.chunk_index as usize >= self.chunks.len() {
            return None;
        }

        let chunk = &self.chunks[index.chunk_index as usize];
        chunk.get(index.index_in_chunk as usize)
    }

    /// Получает изменяемую ссылку на компонент по индексу
    pub fn get_mut(&mut self, index: ComponentIndex) -> Option<&mut T> {
        if index.chunk_index as usize >= self.chunks.len() {
            return None;
        }

        let chunk = &mut self.chunks[index.chunk_index as usize];
        chunk.get_mut(index.index_in_chunk as usize)
    }

    /// Удаляет компонент путем перемещения последнего компонента на его место
    pub fn swap_remove(&mut self, index: ComponentIndex) -> bool {
        if index.chunk_index as usize >= self.chunks.len() {
            return false;
        }

        let chunk = &mut self.chunks[index.chunk_index as usize];

        // Пытаемся использовать swap_remove у чанка
        if chunk.swap_remove(index.index_in_chunk as usize) {
            // Если чанк теперь пуст и это не единственный чанк, можно удалить его
            if chunk.count() == 0 && self.chunks.len() > 1 {
                // Удаляем чанк
                self.chunks.swap_remove(index.chunk_index as usize);

                // Корректируем индексы в free_slots, если удаленный чанк был не последним
                if (index.chunk_index as usize) < self.chunks.len() {
                    // Найти все слоты, относящиеся к последнему чанку (который переместился)
                    let last_chunk_index = self.chunks.len();

                    for slot in self.free_slots.iter_mut() {
                        if slot.chunk_index == last_chunk_index as u32 {
                            slot.chunk_index = index.chunk_index;
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
        self.chunks.len() * self.chunk_capacity
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

