use std::alloc::Layout;
use std::ptr::NonNull;
use crate::ecs::memory::arena::Arena;
use crate::ecs::core::events::event::{Event, EventId, ErasedEvent};
use crate::ecs::core::events::event_registry;

/// Pool for storing events of a specific type
/// Similar to ComponentPool but for events
pub struct EventPool {
    /// Event ID for this pool
    event_id: EventId,
    
    /// Memory layout of the event
    layout: Layout,
    
    /// Buffer for storing events
    buffer: NonNull<u8>,
    
    /// Current number of events
    count: usize,
    
    /// Maximum capacity
    capacity: usize,
    
    /// Size of each event in bytes
    event_size: usize,
}

impl EventPool {
    /// Creates a new EventPool for a specific event type
    pub fn new<E: Event>(arena: &Arena, capacity: usize) -> Self {
        let event_id = E::event_id();
        let layout = E::layout();
        let event_size = layout.size();
        
        // Allocate buffer for all events
        let buffer_size = capacity * event_size;
        let buffer_layout = unsafe {
            Layout::from_size_align_unchecked(buffer_size, layout.align())
        };
        
        let buffer = arena.allocate_layout(buffer_layout);
        
        Self {
            event_id,
            layout,
            buffer,
            count: 0,
            capacity,
            event_size,
        }
    }
    
    /// Creates a new EventPool using event ID
    pub fn new_with_id(arena: &Arena, event_id: EventId, capacity: usize) -> Option<Self> {
        let event_info = event_registry::get_event_info(event_id)?;
        let layout = event_info.layout;
        let event_size = layout.size();
        
        // Allocate buffer for all events
        let buffer_size = capacity * event_size;
        let buffer_layout = unsafe {
            Layout::from_size_align_unchecked(buffer_size, layout.align())
        };
        
        let buffer = arena.allocate_layout(buffer_layout);
        
        Some(Self {
            event_id,
            layout,
            buffer,
            count: 0,
            capacity,
            event_size,
        })
    }
    
    /// Pushes an event to the pool
    pub fn push<E: Event>(&mut self, event: E) -> Option<usize> {
        if E::event_id() != self.event_id {
            return None; // Wrong event type
        }
        
        if self.count >= self.capacity {
            return None; // Pool is full
        }
        
        let index = self.count;
        let offset = index * self.event_size;
        
        unsafe {
            let dst = self.buffer.as_ptr().add(offset);
            std::ptr::write(dst as *mut E, event);
        }
        
        self.count += 1;
        Some(index)
    }
    
    /// Pushes raw event bytes to the pool
    pub fn push_raw(&mut self, event_bytes: &[u8]) -> Option<usize> {
        if event_bytes.len() != self.event_size {
            return None; // Wrong size
        }
        
        if self.count >= self.capacity {
            return None; // Pool is full
        }
        
        let index = self.count;
        let offset = index * self.event_size;
        
        unsafe {
            let dst = self.buffer.as_ptr().add(offset);
            std::ptr::copy_nonoverlapping(event_bytes.as_ptr(), dst, self.event_size);
        }
        
        self.count += 1;
        Some(index)
    }
    
    /// Gets an event by index
    pub fn get<E: Event>(&self, index: usize) -> Option<&E> {
        if E::event_id() != self.event_id {
            return None; // Wrong event type
        }
        
        if index >= self.count {
            return None; // Out of bounds
        }
        
        let offset = index * self.event_size;
        unsafe {
            let ptr = self.buffer.as_ptr().add(offset) as *const E;
            Some(&*ptr)
        }
    }
    
    /// Gets a mutable event by index
    pub fn get_mut<E: Event>(&mut self, index: usize) -> Option<&mut E> {
        if E::event_id() != self.event_id {
            return None; // Wrong event type
        }
        
        if index >= self.count {
            return None; // Out of bounds
        }
        
        let offset = index * self.event_size;
        unsafe {
            let ptr = self.buffer.as_ptr().add(offset) as *mut E;
            Some(&mut *ptr)
        }
    }
    
    /// Gets raw pointer to event bytes
    pub fn get_raw(&self, index: usize) -> Option<*const u8> {
        if index >= self.count {
            return None;
        }
        
        let offset = index * self.event_size;
        unsafe {
            Some(self.buffer.as_ptr().add(offset))
        }
    }
    
    /// Removes an event using swap remove
    pub fn swap_remove(&mut self, index: usize) -> bool {
        if index >= self.count {
            return false;
        }
        
        if index < self.count - 1 {
            // Swap with last
            let last_index = self.count - 1;
            let removed_offset = index * self.event_size;
            let last_offset = last_index * self.event_size;
            
            unsafe {
                let removed_ptr = self.buffer.as_ptr().add(removed_offset);
                let last_ptr = self.buffer.as_ptr().add(last_offset);
                
                // Copy last to removed position
                std::ptr::copy_nonoverlapping(last_ptr, removed_ptr as *mut u8, self.event_size);
            }
        }
        
        self.count -= 1;
        true
    }
    
    /// Clears all events from the pool
    pub fn clear(&mut self) {
        // Drop all events properly
        unsafe {
            // Note: This assumes events don't need explicit drop
            // For events with Drop impl, we'd need to call drop properly
        }
        
        self.count = 0;
    }
    
    /// Gets the number of events in the pool
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }
    
    /// Checks if the pool is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    
    /// Gets the capacity of the pool
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    
    /// Gets the event ID for this pool
    #[inline]
    pub fn event_id(&self) -> EventId {
        self.event_id
    }
    
    /// Creates an iterator over events
    pub fn iter<E: Event>(&self) -> EventPoolIter<E> {
        EventPoolIter {
            pool: self,
            current: 0,
            _phantom: std::marker::PhantomData,
        }
    }
}

/// Iterator over events in an EventPool
pub struct EventPoolIter<'a, E: Event> {
    pool: &'a EventPool,
    current: usize,
    _phantom: std::marker::PhantomData<E>,
}

impl<'a, E: Event> Iterator for EventPoolIter<'a, E> {
    type Item = &'a E;
    
    fn next(&mut self) -> Option<Self::Item> {
        if self.current >= self.pool.count {
            return None;
        }
        
        let event = self.pool.get::<E>(self.current)?;
        self.current += 1;
        Some(event)
    }
}
