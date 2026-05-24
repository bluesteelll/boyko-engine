/* use crate::ecs::core::events::event::Event;
use crate::ecs::core::events::event_pool::EventPool;
use crate::ecs::core::memory::arena::Arena;

/// Bundle for multiple event pools
pub struct EventPoolBundle {
    /// Maps event IDs to their pools
    pools: Vec<Option<EventPool>>,

    /// Raw provenance pointer to the arena for memory allocation.
    /// Use `*const Arena` (not `NonNull<Arena>`) — Phase 3a Miri retag fix.
    arena: *const Arena,
}

impl EventPoolBundle {
    /// Creates a new EventPoolBundle
    pub fn new(arena: &Arena) -> Self {
        Self {
            pools: vec![None; 256], // Max 256 event types
            // SAFETY: `arena` is a shared reference; raw pointer preserves provenance.
            arena: &raw const *arena,
        }
    }

    /// Creates a pool for a specific event type
    pub fn create_pool<E: Event>(&mut self, capacity: usize) -> bool {
        let event_id = E::event_id() as usize;

        if event_id >= self.pools.len() {
            return false;
        }

        if self.pools[event_id].is_some() {
            return false; // Pool already exists
        }

        // SAFETY: `self.arena` was minted from a live `Box<Arena>`-owned
        // allocation (Phase 3a raw provenance contract). The reborrow is
        // scoped to this call and does not escape.
        let arena = unsafe { &*self.arena };
        let pool = EventPool::new::<E>(arena, capacity);
        self.pools[event_id] = Some(pool);
        
        true
    }
    
    /// Gets a pool by event type
    pub fn get_pool<E: Event>(&self) -> Option<&EventPool> {
        let event_id = E::event_id() as usize;
        self.pools.get(event_id)?.as_ref()
    }
    
    /// Gets a mutable pool by event type
    pub fn get_pool_mut<E: Event>(&mut self) -> Option<&mut EventPool> {
        let event_id = E::event_id() as usize;
        self.pools.get_mut(event_id)?.as_mut()
    }
    
    /// Pushes an event to the appropriate pool
    pub fn push_event<E: Event>(&mut self, event: E) -> Option<usize> {
        self.get_pool_mut::<E>()?.push(event)
    }
    
    /// Processes all events of a specific type
    pub fn process_events<E: Event, F>(&self, mut handler: F) 
    where
        F: FnMut(&E)
    {
        if let Some(pool) = self.get_pool::<E>() {
            for event in pool.iter::<E>() {
                handler(event);
            }
        }
    }
    
    /// Clears all events from all pools
    pub fn clear_all(&mut self) {
        for pool in &mut self.pools {
            if let Some(p) = pool {
                p.clear();
            }
        }
    }
} */



 //TODO: rework