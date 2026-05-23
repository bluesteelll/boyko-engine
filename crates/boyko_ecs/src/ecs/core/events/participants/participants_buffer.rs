use std::alloc::Layout;
use std::mem::MaybeUninit;
use crate::ecs::core::events::event::EventId;
use crate::ecs::core::events::participants::participants::Participants;

/// Type-erased buffer for storing event participants.
pub struct ParticipantBuffer {
    /// Event ID this buffer is for.
    event_id: EventId,

    /// Layout of the participants structure.
    layout: Layout,

    /// Raw data storage. `MaybeUninit<u8>` makes padding-byte writes sound
    /// under Miri — the buffer never interprets the bytes as initialized `u8`.
    data: Vec<MaybeUninit<u8>>,

    /// Number of participant sets stored.
    count: usize,

    /// Size of each participant set in bytes.
    participant_size: usize,
}

impl ParticipantBuffer {
    /// Creates a new participant buffer.
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

    /// Creates a new participant buffer with pre-allocated capacity.
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

    /// Adds participants to the buffer. Returns the index of the inserted entry.
    pub fn push<P: Participants>(&mut self, participants: &P) -> usize {
        debug_assert_eq!(std::mem::size_of::<P>(), self.participant_size);
        let index = self.count;
        let old_len = self.data.len();
        self.data
            .resize(old_len + self.participant_size, MaybeUninit::uninit());
        // SAFETY:
        // (1) `participants` is a valid `&P` for `size_of::<P>()` bytes (borrowed reference).
        // (2) The destination is the freshly-reserved tail region of `self.data` with exactly
        //     `participant_size` bytes (the `resize` above extended it).
        // (3) `MaybeUninit<u8>` accepts any byte pattern including padding, so copying
        //     padding bytes from `*participants` is sound.
        // (4) Regions cannot overlap: `participants` is borrowed externally, the destination
        //     is inside the internal Vec allocation.
        unsafe {
            std::ptr::copy_nonoverlapping(
                (participants as *const P).cast::<u8>(),
                self.data.as_mut_ptr().add(old_len).cast::<u8>(),
                self.participant_size,
            );
        }
        self.count += 1;
        index
    }

    /// Gets participants at the given index.
    ///
    /// # Safety
    /// `P` must match the type used at the corresponding `push` call. The buffer
    /// is type-erased (Q-019 tracks adding a `TypeId` check; deferred to Phase 4b).
    pub unsafe fn get<P: Participants>(&self, index: usize) -> Option<P> {
        if index >= self.count {
            return None;
        }
        debug_assert_eq!(std::mem::size_of::<P>(), self.participant_size);
        let offset = index * self.participant_size;
        // SAFETY: `offset + participant_size <= self.data.len()` by construction —
        // `count` is incremented only after a successful `push` that grew `data`.
        // Caller's invariant ensures the bytes at this offset are a valid bit-pattern
        // for `P`. `read_unaligned` does not require alignment of the source pointer.
        unsafe {
            let src = self.data.as_ptr().add(offset).cast::<P>();
            Some(std::ptr::read_unaligned(src))
        }
    }

    /// Clears all participants.
    pub fn clear(&mut self) {
        self.data.clear();
        self.count = 0;
    }

    /// Returns the number of participant sets stored.
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
