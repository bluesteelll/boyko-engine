use std::any::TypeId;
use std::mem::MaybeUninit;
use crate::ecs::core::events::event::EventId;
use crate::ecs::core::events::participants::participants::Participants;

/// Type-erased buffer for storing event participants.
///
/// The buffer carries a [`TypeId`] that records the concrete `P` used at construction
/// time. Every typed read (`get`) asserts that the caller passes the same `P`, catching
/// bugs in macro-generated glue code or test helpers that bypass the `#[event]` macro.
/// In release builds the assertion compiles away; the invariant is upheld by the macro,
/// which always ties a `ParticipantBuffer` to a single concrete `Participants` type.
pub struct ParticipantBuffer {
    /// Event ID this buffer is for.
    event_id: EventId,

    /// `TypeId` of the concrete `Participants` type this buffer was constructed for.
    /// Used in debug builds to detect type confusion (Q-019).
    type_id: TypeId,

    /// Raw data storage. `MaybeUninit<u8>` makes padding-byte writes sound
    /// under Miri — the buffer never interprets the bytes as initialized `u8`.
    data: Vec<MaybeUninit<u8>>,

    /// Number of participant sets stored.
    count: usize,

    /// Size of each participant set in bytes.
    participant_size: usize,
}

impl ParticipantBuffer {
    /// Creates a new participant buffer for the concrete type `P`.
    ///
    /// The `TypeId` of `P` is recorded and checked on every `get` call in debug
    /// builds, defending against type confusion when buffers are accessed through
    /// type-erased code paths.
    pub fn new<P: Participants>(event_id: EventId) -> Self {
        Self {
            event_id,
            type_id: TypeId::of::<P>(),
            data: Vec::new(),
            count: 0,
            participant_size: P::layout().size(),
        }
    }

    /// Creates a new participant buffer with pre-allocated capacity.
    pub fn with_capacity<P: Participants>(event_id: EventId, capacity: usize) -> Self {
        let participant_size = P::layout().size();
        Self {
            event_id,
            type_id: TypeId::of::<P>(),
            data: Vec::with_capacity(capacity * participant_size),
            count: 0,
            participant_size,
        }
    }

    /// Adds participants to the buffer. Returns the index of the inserted entry.
    pub fn push<P: Participants>(&mut self, participants: &P) -> usize {
        debug_assert_eq!(
            TypeId::of::<P>(),
            self.type_id,
            "ParticipantBuffer::push called with wrong Participants type (Q-019)"
        );
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
    /// `P` must be the exact type supplied when the buffer was constructed (i.e. the
    /// same `P` passed to [`new`](Self::new) or [`with_capacity`](Self::with_capacity)).
    /// In debug builds a [`TypeId`] assertion enforces this; in release builds the
    /// invariant is upheld by the `#[event]` macro, which always binds the buffer to a
    /// single concrete type.
    pub unsafe fn get<P: Participants>(&self, index: usize) -> Option<P> {
        debug_assert_eq!(
            TypeId::of::<P>(),
            self.type_id,
            "ParticipantBuffer::get called with wrong Participants type (Q-019)"
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::events::participants::participants::{ParticipantInfo, Participants};

    // --- stub types for tests ---

    #[derive(Clone, Copy, PartialEq, Debug)]
    struct ParticipantA {
        x: u64,
        y: u64,
    }

    impl Participants for ParticipantA {
        fn participant_count() -> usize {
            1
        }
        fn participant_info() -> &'static [ParticipantInfo] {
            &[]
        }
    }

    /// Same size as `ParticipantA` (16 bytes) but a different type —
    /// exactly the "size match but type mismatch" hazard Q-019 guards against.
    #[derive(Clone, Copy, PartialEq, Debug)]
    struct ParticipantB {
        a: u64,
        b: u64,
    }

    impl Participants for ParticipantB {
        fn participant_count() -> usize {
            1
        }
        fn participant_info() -> &'static [ParticipantInfo] {
            &[]
        }
    }

    const DUMMY_EVENT_ID: EventId = 0;

    // --- tests ---

    /// Correct round-trip: construct for TypeA, push TypeA, read as TypeA.
    #[test]
    fn test_push_and_get_correct_type() {
        let mut buf = ParticipantBuffer::new::<ParticipantA>(DUMMY_EVENT_ID);
        let val = ParticipantA { x: 42, y: 99 };
        let idx = buf.push(&val);
        assert_eq!(idx, 0);
        // SAFETY: TypeA matches the buffer's construction type.
        let result = unsafe { buf.get::<ParticipantA>(0) };
        assert_eq!(result, Some(val));
    }

    /// Out-of-bounds read returns `None` for the correct type.
    #[test]
    fn test_get_out_of_bounds_returns_none() {
        let mut buf = ParticipantBuffer::new::<ParticipantA>(DUMMY_EVENT_ID);
        let val = ParticipantA { x: 1, y: 2 };
        buf.push(&val);
        // SAFETY: TypeA matches; index 1 is out of bounds.
        let result = unsafe { buf.get::<ParticipantA>(1) };
        assert!(result.is_none());
    }

    /// Type mismatch on `get` must panic in debug builds (Q-019).
    ///
    /// `ParticipantA` and `ParticipantB` are both 16 bytes — they would silently
    /// pass a size-only check. The `TypeId` assertion is the only defence.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "ParticipantBuffer::get called with wrong Participants type (Q-019)")]
    fn test_get_wrong_type_panics_in_debug() {
        let mut buf = ParticipantBuffer::new::<ParticipantA>(DUMMY_EVENT_ID);
        let val = ParticipantA { x: 7, y: 8 };
        buf.push(&val);
        // SAFETY (test intent): we intentionally pass the wrong type to exercise
        // the debug_assert. The UB that would follow never occurs because the assert fires.
        let _ = unsafe { buf.get::<ParticipantB>(0) };
    }

    /// Type mismatch on `push` must panic in debug builds.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "ParticipantBuffer::push called with wrong Participants type (Q-019)")]
    fn test_push_wrong_type_panics_in_debug() {
        // Construct for ParticipantA, attempt push of ParticipantB.
        // We need a buffer typed as ParticipantA but then call push<ParticipantB>.
        // Achieve this by constructing through the raw fields — not possible without
        // unsafe, so we use a small helper that produces the mismatch via the public API.
        //
        // The easiest way: construct for A, then call push<B>. The TypeId stored is A;
        // the assert compares against TypeId::of::<B>() and fires.
        let mut buf = ParticipantBuffer::new::<ParticipantA>(DUMMY_EVENT_ID);
        // We need to call push<ParticipantB>, which requires a &ParticipantB value.
        // Transmute is intentional here — this is a test verifying that the
        // debug_assert fires *before* any unsound byte copy occurs.
        let b_val = ParticipantB { a: 1, b: 2 };
        // Re-interpret the buffer's type parameter by going through a raw-pointer cast
        // is not needed: we can simply call push::<ParticipantB> directly since the
        // function is generic. The type_id field will mismatch → assert fires.
        //
        // Unfortunately Rust's type system won't let us call buf.push::<ParticipantB>
        // when buf was constructed as ParticipantBuffer::new::<ParticipantA>, because
        // push<P: Participants> is generic — the monomorphization with P=ParticipantB
        // is entirely valid at the call site. The check is runtime (debug_assert).
        // We must call the function to trigger it.
        buf.push::<ParticipantB>(&b_val);
    }
}
