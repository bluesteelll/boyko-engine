use std::any::TypeId;
use std::mem::MaybeUninit;
use crate::ecs::core::events::event::EventId;
use crate::ecs::core::events::parameters::parameters::Parameters;

/// Type-erased buffer for storing event parameters.
///
/// The buffer carries a [`TypeId`] that records the concrete `P` used at construction
/// time. Every typed read (`get`) asserts that the caller passes the same `P`, catching
/// bugs in macro-generated glue code or test helpers that bypass the `#[event]` macro.
/// In release builds the assertion compiles away; the invariant is upheld by the macro,
/// which always ties a `ParametersBuffer` to a single concrete `Parameters` type.
pub struct ParametersBuffer {
    /// Event ID this buffer is for.
    event_id: EventId,

    /// `TypeId` of the concrete `Parameters` type this buffer was constructed for.
    /// Used in debug builds to detect type confusion (Q-019).
    type_id: TypeId,

    /// Raw data storage. `MaybeUninit<u8>` makes padding-byte writes sound
    /// under Miri — the buffer never interprets the bytes as initialized `u8`.
    data: Vec<MaybeUninit<u8>>,

    /// Number of parameter sets stored.
    count: usize,

    /// Size of each parameter set in bytes.
    parameters_size: usize,
}

impl ParametersBuffer {
    /// Creates a new parameters buffer for the concrete type `P`.
    ///
    /// The `TypeId` of `P` is recorded and checked on every `get` call in debug
    /// builds, defending against type confusion when buffers are accessed through
    /// type-erased code paths.
    pub fn new<P: Parameters>(event_id: EventId) -> Self {
        Self {
            event_id,
            type_id: TypeId::of::<P>(),
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
            type_id: TypeId::of::<P>(),
            data: Vec::with_capacity(capacity * parameters_size),
            count: 0,
            parameters_size,
        }
    }

    /// Adds parameters to the buffer. Returns the index of the inserted entry.
    pub fn push<P: Parameters>(&mut self, parameters: &P) -> usize {
        debug_assert_eq!(
            TypeId::of::<P>(),
            self.type_id,
            "ParametersBuffer::push called with wrong Parameters type (Q-019)"
        );
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
    /// `P` must be the exact type supplied when the buffer was constructed (i.e. the
    /// same `P` passed to [`new`](Self::new) or [`with_capacity`](Self::with_capacity)).
    /// In debug builds a [`TypeId`] assertion enforces this; in release builds the
    /// invariant is upheld by the `#[event]` macro, which always binds the buffer to a
    /// single concrete type.
    pub unsafe fn get<P: Parameters>(&self, index: usize) -> Option<P> {
        debug_assert_eq!(
            TypeId::of::<P>(),
            self.type_id,
            "ParametersBuffer::get called with wrong Parameters type (Q-019)"
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::events::parameters::parameters::Parameters;

    // --- stub types for tests ---

    #[derive(Clone, Copy, PartialEq, Debug)]
    struct ParamA {
        damage: f32,
        multiplier: f32,
    }

    impl Parameters for ParamA {}

    /// Same size as `ParamA` (8 bytes) but a distinct type —
    /// exactly the "size match but type mismatch" hazard Q-019 guards against.
    #[derive(Clone, Copy, PartialEq, Debug)]
    struct ParamB {
        speed: f32,
        friction: f32,
    }

    impl Parameters for ParamB {}

    const DUMMY_EVENT_ID: EventId = 0;

    // --- tests ---

    /// Correct round-trip: construct for TypeA, push TypeA, read as TypeA.
    #[test]
    fn test_push_and_get_correct_type() {
        let mut buf = ParametersBuffer::new::<ParamA>(DUMMY_EVENT_ID);
        let val = ParamA {
            damage: 10.0,
            multiplier: 1.5,
        };
        let idx = buf.push(&val);
        assert_eq!(idx, 0);
        // SAFETY: ParamA matches the buffer's construction type.
        let result = unsafe { buf.get::<ParamA>(0) };
        assert_eq!(result, Some(val));
    }

    /// Out-of-bounds read returns `None` for the correct type.
    #[test]
    fn test_get_out_of_bounds_returns_none() {
        let mut buf = ParametersBuffer::new::<ParamA>(DUMMY_EVENT_ID);
        let val = ParamA {
            damage: 5.0,
            multiplier: 2.0,
        };
        buf.push(&val);
        // SAFETY: ParamA matches; index 1 is out of bounds.
        let result = unsafe { buf.get::<ParamA>(1) };
        assert!(result.is_none());
    }

    /// Type mismatch on `get` must panic in debug builds (Q-019).
    ///
    /// `ParamA` and `ParamB` are both 8 bytes — they would silently pass a size-only
    /// check. The `TypeId` assertion is the only defence.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "ParametersBuffer::get called with wrong Parameters type (Q-019)")]
    fn test_get_wrong_type_panics_in_debug() {
        let mut buf = ParametersBuffer::new::<ParamA>(DUMMY_EVENT_ID);
        let val = ParamA {
            damage: 1.0,
            multiplier: 1.0,
        };
        buf.push(&val);
        // SAFETY (test intent): we intentionally pass the wrong type to exercise
        // the debug_assert. The UB that would follow never occurs because the assert fires.
        let _ = unsafe { buf.get::<ParamB>(0) };
    }

    /// Type mismatch on `push` must panic in debug builds.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "ParametersBuffer::push called with wrong Parameters type (Q-019)")]
    fn test_push_wrong_type_panics_in_debug() {
        let mut buf = ParametersBuffer::new::<ParamA>(DUMMY_EVENT_ID);
        let b_val = ParamB {
            speed: 3.0,
            friction: 0.1,
        };
        // push<ParamB> on a buffer constructed for ParamA → TypeId mismatch → assert.
        buf.push::<ParamB>(&b_val);
    }
}
