use std::alloc::Layout;
use std::mem;
use std::ptr::NonNull;
use crate::ecs::core::component::Component;
use crate::ecs::memory::arena::Arena;
use crate::ecs::constants::{DEFAULT_COMPONENTS_PER_CHUNK};

/// Chunk хранит фиксированное количество компонентов одного типа
pub struct Chunk<T: Component> {
    /// Указатель на выделенную память
    data: NonNull<T>,

    /// Вместимость чанка (максимальное количество компонентов)
    capacity: usize,

    /// Текущее количество занятых слотов
    count: usize,
}

impl<T: Component> Chunk<T> {
    /// Создает новый чанк с указанной вместимостью
    pub fn new(arena: &Arena, capacity: usize) -> Self {
        // Выделяем память для массива компонентов
        // Мы должны использовать allocate_layout, так как нам нужен массив
        let layout = Layout::array::<T>(capacity).expect("Invalid array layout");
        let ptr = arena.allocate_layout(layout);

        // Приводим указатель к нужному типу
        let typed_ptr = unsafe { ptr.cast::<T>() };

        Self {
            data: typed_ptr,
            capacity,
            count: 0,
        }
    }

    /// Создает чанк с размером по умолчанию
    pub fn with_default_capacity(arena: &Arena) -> Self {
        Self::new(arena, DEFAULT_COMPONENTS_PER_CHUNK)
    }

    /// Добавляет компонент в чанк и возвращает его индекс
    pub fn add(&mut self, component: T) -> Option<usize> {
        // Проверяем, что есть место
        if self.count >= self.capacity {
            return None;
        }

        // Вычисляем адрес, куда поместить компонент
        let index = self.count;
        let ptr = unsafe { self.data.as_ptr().add(index) };

        // Размещаем компонент в памяти
        unsafe {
            std::ptr::write(ptr, component);
        }

        // Увеличиваем счетчик
        self.count += 1;

        Some(index)
    }

    pub fn set(&mut self, index: usize, component: T) -> bool {
        if index >= self.capacity {
            return false;
        }

        // Если индекс больше текущего количества,
        // мы автоматически увеличиваем счетчик
        if index >= self.count {
            self.count = index + 1;
        }

        // Указатель на место в памяти
        let ptr = unsafe { self.data.as_ptr().add(index) };

        // Размещаем компонент в памяти
        unsafe {
            // Если значение уже существует, вызываем его деструктор
            if index < self.count {
                std::ptr::drop_in_place(ptr);
            }

            std::ptr::write(ptr, component);
        }

        true
    }

    /// Получает ссылку на компонент по индексу
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.count {
            return None;
        }

        // Получаем указатель на компонент
        let ptr = unsafe { self.data.as_ptr().add(index) };

        // Преобразуем в ссылку
        unsafe {
            Some(&*ptr)
        }
    }

    /// Получает изменяемую ссылку на компонент по индексу
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.count {
            return None;
        }

        // Получаем указатель на компонент
        let ptr = unsafe { self.data.as_ptr().add(index) };

        // Преобразуем в изменяемую ссылку
        unsafe {
            Some(&mut *ptr)
        }
    }

    /// Возвращает количество компонентов в чанке
    pub fn count(&self) -> usize {
        self.count
    }

    /// Возвращает вместимость чанка
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Возвращает указатель на массив компонентов
    pub fn as_ptr(&self) -> *const T {
        self.data.as_ptr()
    }

    /// Возвращает изменяемый указатель на массив компонентов
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.data.as_ptr()
    }

    /// Получает срез всех компонентов
    pub fn as_slice(&self) -> &[T] {
        unsafe {
            std::slice::from_raw_parts(self.data.as_ptr(), self.count)
        }
    }

    /// Получает изменяемый срез всех компонентов
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        unsafe {
            std::slice::from_raw_parts_mut(self.data.as_ptr(), self.count)
        }
    }

    /// Очищает чанк, вызывая деструкторы всех компонентов
    pub fn clear(&mut self) {
        // Вызываем деструкторы для всех компонентов
        for i in 0..self.count {
            let ptr = unsafe { self.data.as_ptr().add(i) };
            unsafe {
                std::ptr::drop_in_place(ptr);
            }
        }

        // Сбрасываем счетчик
        self.count = 0;
    }

    /// Сдвигает элементы, чтобы заполнить пробел при удалении
    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.count {
            return false;
        }

        // Получаем указатель на удаляемый компонент
        let ptr = unsafe { self.data.as_ptr().add(index) };

        // Вызываем деструктор
        unsafe {
            std::ptr::drop_in_place(ptr);
        }

        // Сдвигаем все последующие элементы на одну позицию назад
        let elements_to_move = self.count - index - 1;
        if elements_to_move > 0 {
            unsafe {
                let src = self.data.as_ptr().add(index + 1);
                let dst = self.data.as_ptr().add(index);
                std::ptr::copy(src, dst, elements_to_move);
            }
        }

        // Уменьшаем счетчик
        self.count -= 1;

        true
    }

    /// Удаляет компонент, заменяя его последним (быстрее, но нарушает порядок)
    pub fn swap_remove(&mut self, index: usize) -> bool {
        if index >= self.count {
            return false;
        }

        // Получаем указатель на удаляемый компонент
        let ptr = unsafe { self.data.as_ptr().add(index) };

        // Вызываем деструктор
        unsafe {
            std::ptr::drop_in_place(ptr);
        }

        // Если это не последний элемент, заменяем его последним
        if index < self.count - 1 {
            let last_index = self.count - 1;
            let last_ptr = unsafe { self.data.as_ptr().add(last_index) };

            // Перемещаем последний элемент на место удаленного
            unsafe {
                std::ptr::copy(last_ptr, ptr, 1);
            }
        }

        // Уменьшаем счетчик
        self.count -= 1;

        true
    }
}

// Реализуем Drop, чтобы вызвать деструкторы компонентов
impl<T: Component> Drop for Chunk<T> {
    fn drop(&mut self) {
        self.clear();
    }
}