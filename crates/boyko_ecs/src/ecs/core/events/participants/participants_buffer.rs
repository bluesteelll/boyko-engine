use std::alloc::Layout;
use std::ptr::NonNull;
use crate::ecs::memory::arena::Arena;
use crate::ecs::core::events::event::EventId;
use crate::ecs::core::events::participants::participants::Participants;

/// Type-erased buffer for storing event participants
pub struct ParticipantBuffer {
    /// Event ID this buffer is for
    event_id: EventId,
    
    /// Layout of the participants structure
    layout: Layout,
    
    /// Raw data storage
    data: Vec<u8>,
    
    /// Number of participant sets stored
    count: usize,
    
    /// Size of each participant set in bytes
    participant_size: usize,
}

impl ParticipantBuffer {
    /// Creates a new participant buffer
    pub fn new<P: Participants>(event_id: EventId) -> Self {
        let layout = P::layout();
        Self {
            event_id,
            layout,
            data: Vec::new(),
            count: 0,
            participant_size: layout.size(),
        }
    }
    
    /// Creates a new participant buffer with capacity
    pub fn with_capacity<P: Participants>(event_id: EventId, capacity: usize) -> Self {
        let layout = P::layout();
        let participant_size = layout.size();
        Self {
            event_id,
            layout,
            data: Vec::with_capacity(capacity * participant_size),
            count: 0,
            participant_size,
        }
    }
    
    /// Adds participants to the buffer
    pub fn push<P: Participants>(&mut self, participants: &P) -> usize {
        let bytes = participants.to_bytes();
        debug_assert_eq!(bytes.len(), self.participant_size);
        
        let index = self.count;
        self.data.extend_from_slice(&bytes);
        self.count += 1;
        index
    }
    
    /// Adds raw participant bytes
    pub fn push_raw(&mut self, bytes: &[u8]) -> Option<usize> {
        if bytes.len() != self.participant_size {
            return None;
        }
        
        let index = self.count;
        self.data.extend_from_slice(bytes);
        self.count += 1;
        Some(index)
    }
    
    /// Gets participants at index
    pub unsafe fn get<P: Participants>(&self, index: usize) -> Option<P> {
        if index >= self.count {
            return None;
        }
        
        let offset = index * self.participant_size;
        let bytes = &self.data[offset..offset + self.participant_size];
        P::from_bytes(bytes)
    }
    
    /// Gets raw bytes at index
    pub fn get_raw(&self, index: usize) -> Option<&[u8]> {
        if index >= self.count {
            return None;
        }
        
        let offset = index * self.participant_size;
        Some(&self.data[offset..offset + self.participant_size])
    }
    
    /// Clears all participants
    pub fn clear(&mut self) {
        self.data.clear();
        self.count = 0;
    }
    
    /// Returns the number of participant sets
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }
    
    /// Checks if empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    
    /// Returns the event ID
    #[inline]
    pub fn event_id(&self) -> EventId {
        self.event_id
    }
}