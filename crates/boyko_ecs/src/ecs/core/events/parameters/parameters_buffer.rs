use std::alloc::Layout;
use crate::ecs::core::events::event::EventId;
use crate::ecs::core::events::parameters::parameters::Parameters;

/// Type-erased buffer for storing event parameters
pub struct ParametersBuffer {
    /// Event ID this buffer is for
    event_id: EventId,
    
    /// Layout of the parameters structure
    layout: Layout,
    
    /// Raw data storage
    data: Vec<u8>,
    
    /// Number of parameter sets stored
    count: usize,
    
    /// Size of each parameter set in bytes
    parameters_size: usize,
}

impl ParametersBuffer {
    /// Creates a new parameters buffer
    pub fn new<P: Parameters>(event_id: EventId) -> Self {
        let layout = P::layout();
        Self {
            event_id,
            layout,
            data: Vec::new(),
            count: 0,
            parameters_size: layout.size(),
        }
    }
    
    /// Creates a new parameters buffer with capacity
    pub fn with_capacity<P: Parameters>(event_id: EventId, capacity: usize) -> Self {
        let layout = P::layout();
        let parameters_size = layout.size();
        Self {
            event_id,
            layout,
            data: Vec::with_capacity(capacity * parameters_size),
            count: 0,
            parameters_size,
        }
    }
    
    /// Adds parameters to the buffer
    pub fn push<P: Parameters>(&mut self, parameters: &P) -> usize {
        let bytes = parameters.to_bytes();
        debug_assert_eq!(bytes.len(), self.parameters_size);
        
        let index = self.count;
        self.data.extend_from_slice(&bytes);
        self.count += 1;
        index
    }
    
    /// Adds raw parameter bytes
    pub fn push_raw(&mut self, bytes: &[u8]) -> Option<usize> {
        if bytes.len() != self.parameters_size {
            return None;
        }
        
        let index = self.count;
        self.data.extend_from_slice(bytes);
        self.count += 1;
        Some(index)
    }
    
    /// Gets parameters at index
    pub unsafe fn get<P: Parameters>(&self, index: usize) -> Option<P> {
        if index >= self.count {
            return None;
        }
        
        let offset = index * self.parameters_size;
        let bytes = &self.data[offset..offset + self.parameters_size];
        P::from_bytes(bytes)
    }
    
    /// Gets raw bytes at index
    pub fn get_raw(&self, index: usize) -> Option<&[u8]> {
        if index >= self.count {
            return None;
        }
        
        let offset = index * self.parameters_size;
        Some(&self.data[offset..offset + self.parameters_size])
    }
    
    /// Clears all parameters
    pub fn clear(&mut self) {
        self.data.clear();
        self.count = 0;
    }
    
    /// Returns the number of parameter sets
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