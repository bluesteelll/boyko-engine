use std::mem::MaybeUninit;
use crate::ecs::core::events::event::EventId;
use crate::ecs::core::events::parameters::parameters::Parameters;

/// Type-erased buffer for storing event parameters.
pub struct ParametersBuffer {
    /// Event ID this buffer is for.
    event_id: EventId,

    /// Raw data storage. `MaybeUninit<u8>` makes padding-byte writes sound
    /// under Miri — the buffer never interprets the bytes as initialized `u8`.
    data: Vec<MaybeUninit<u8>>,

    /// Number of parameter sets stored.
    count: usize,

    /// Size of each parameter set in bytes.
    parameters_size: usize,
}

impl ParametersBuffer {
    /// Creates a new parameters buffer.
    pub fn new<P: Parameters>(event_id: EventId) -> Self {
        Self {
            event_id,
            data: Vec::new(),
            count: 0,
            parameters_size: P::layout().size(),
        }
    }

    /// Creates a new parameters buffer with pre-allocated capacity.
    pub fn with_capacity<P: Parameters>(event_id: EventId, capacity: usize) -> Self {
        let parameters_size = P::layout().size();
        Self {
            event_id,
            data: Vec::with_capacity(capacity * parameters_size),
            count: 0,
            parameters_size,
        }
    }

    /// Adds parameters to the buffer. Returns the index of the inserted entry.
    pub fn push<P: Parameters>(&mut self, parameters: &P) -> usize {
        debug_assert_eq!(std::mem::size_of::<P>(), self.parameters_size);
        let index = self.count;
        let old_len = self.data.len();
        self.data
            .resize(old_len + self.parameters_size, MaybeUninit::uninit());
        // SAFETY:
        // (1) `parameters` is a valid `&P` for `size_of::<P>()` bytes (borrowed reference).
        // (2) The destination is the freshly-reserved tail region of `self.data` with exactly
        //     `parameters_size` bytes (the `resize` above extended it).
        // (3) `MaybeUninit<u8>` accepts any byte pattern including padding, so copying
        //     padding bytes from `*parameters` is sound.
        // (4) Regions cannot overlap: `parameters` is borrowed externally, the destination
        //     is inside the internal Vec allocation.
        unsafe {
            std::ptr::copy_nonoverlapping(
                (parameters as *const P).cast::<u8>(),
                self.data.as_mut_ptr().add(old_len).cast::<u8>(),
                self.parameters_size,
            );
        }
        self.count += 1;
        index
    }

    /// Gets parameters at the given index.
    ///
    /// # Safety
    /// `P` must match the type used at the corresponding `push` call. The buffer
    /// is type-erased (Q-019 tracks adding a `TypeId` check; deferred to Phase 4b).
    pub unsafe fn get<P: Parameters>(&self, index: usize) -> Option<P> {
        if index >= self.count {
            return None;
        }
        debug_assert_eq!(std::mem::size_of::<P>(), self.parameters_size);
        let offset = index * self.parameters_size;
        // SAFETY: `offset + parameters_size <= self.data.len()` by construction —
        // `count` is incremented only after a successful `push` that grew `data`.
        // Caller's invariant ensures the bytes at this offset are a valid bit-pattern
        // for `P`. `read_unaligned` does not require alignment of the source pointer.
        unsafe {
            let src = self.data.as_ptr().add(offset).cast::<P>();
            Some(std::ptr::read_unaligned(src))
        }
    }

    /// Clears all parameters.
    pub fn clear(&mut self) {
        self.data.clear();
        self.count = 0;
    }

    /// Returns the number of parameter sets stored.
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Returns `true` if the buffer contains no entries.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns the event ID this buffer belongs to.
    #[inline]
    pub fn event_id(&self) -> EventId {
        self.event_id
    }
}
