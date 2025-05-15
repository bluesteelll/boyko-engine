use std::alloc::Layout;
use std::marker::PhantomData;
use crate::ecs::memory::component_pool::ComponentPool;
use crate::ecs::core::component::Component;

/// Lightweight component pointer
#[derive(Debug, Clone, Copy)]
pub struct ComponentPtr {
    ptr: *const u8,
}

impl ComponentPtr {
    #[inline(always)]
    pub fn new(ptr: *const u8) -> Self {
        Self { ptr }
    }
    
    #[inline(always)]
    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }
    
    #[inline(always)]
    pub unsafe fn as_ref<T>(&self) -> &T {
        &*(self.ptr as *const T)
    }
}

/// Mutable component pointer
#[derive(Debug, Clone, Copy)]
pub struct ComponentMutPtr {
    ptr: *mut u8,
}

impl ComponentMutPtr {
    #[inline(always)]
    pub fn new(ptr: *mut u8) -> Self {
        Self { ptr }
    }
    
    #[inline(always)]
    pub fn as_mut_ptr(&self) -> *mut u8 {
        self.ptr
    }
    
    #[inline(always)]
    pub unsafe fn as_mut<T>(&self) -> &mut T {
        &mut *(self.ptr as *mut T)
    }
}

/// Immutable sparse iterator implementing Iterator trait
pub struct ComponentPoolSparseIter {
    pointers: Box<[ComponentPtr]>,
    current: usize,
    component_id: usize,
    layout: Layout,
}

impl ComponentPoolSparseIter {
    pub fn new(pool: &ComponentPool, indices: &[usize]) -> Self {
        let component_id = pool.component_id();
        let layout = pool.component_layout();
        
        let pointers: Box<[ComponentPtr]> = indices
            .iter()
            .filter_map(|&idx| pool.get_raw(idx).map(ComponentPtr::new))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        
        Self {
            pointers,
            current: 0,
            component_id,
            layout,
        }
    }
    
    #[inline(always)]
    pub fn component_id(&self) -> usize {
        self.component_id
    }
    
    #[inline(always)]
    pub fn reset(&mut self) {
        self.current = 0;
    }
    
    /// Create a typed iterator adapter
    pub fn typed<T: Component>(self) -> TypedComponentIter<T> {
        debug_assert_eq!(T::component_id(), self.component_id);
        TypedComponentIter {
            inner: self,
            _phantom: PhantomData,
        }
    }
}

// Implement Iterator for immutable version
impl Iterator for ComponentPoolSparseIter {
    type Item = ComponentPtr;
    
    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        if self.current < self.pointers.len() {
            let ptr = self.pointers[self.current];
            self.current += 1;
            Some(ptr)
        } else {
            None
        }
    }
    
    #[inline(always)]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.pointers.len().saturating_sub(self.current);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for ComponentPoolSparseIter {}

/// Mutable sparse iterator implementing Iterator trait
pub struct ComponentPoolSparseIterMut {
    pointers: Box<[ComponentMutPtr]>,
    current: usize,
    component_id: usize,
    layout: Layout,
}

impl ComponentPoolSparseIterMut {
    pub fn new(pool: &mut ComponentPool, indices: &[usize]) -> Self {
        let component_id = pool.component_id();
        let layout = pool.component_layout();
        
        let pointers: Box<[ComponentMutPtr]> = indices
            .iter()
            .filter_map(|&idx| pool.get_raw_mut(idx).map(ComponentMutPtr::new))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        
        Self {
            pointers,
            current: 0,
            component_id,
            layout,
        }
    }
    
    #[inline(always)]
    pub fn component_id(&self) -> usize {
        self.component_id
    }
    
    #[inline(always)]
    pub fn reset(&mut self) {
        self.current = 0;
    }
    
    /// Create a typed mutable iterator adapter
    pub fn typed_mut<T: Component>(self) -> TypedComponentIterMut<T> {
        debug_assert_eq!(T::component_id(), self.component_id);
        TypedComponentIterMut {
            inner: self,
            _phantom: PhantomData,
        }
    }
}

// Implement Iterator for mutable version
impl Iterator for ComponentPoolSparseIterMut {
    type Item = ComponentMutPtr;
    
    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        if self.current < self.pointers.len() {
            let ptr = self.pointers[self.current];
            self.current += 1;
            Some(ptr)
        } else {
            None
        }
    }
    
    #[inline(always)]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.pointers.len().saturating_sub(self.current);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for ComponentPoolSparseIterMut {}