//! Type-erased buffer for event **participants**.
//!
//! This is a thin alias over the shared [`ErasedKindBuffer`] generic; all
//! storage logic lives there. The [`ParticipantBuffer`] name is preserved so the
//! `#[event]` attribute path and existing tests keep compiling unchanged.

use crate::ecs::core::events::erased_buffer::{ErasedKindBuffer, ParticipantsKind};

/// Type-erased buffer for storing event participants.
///
/// Alias of [`ErasedKindBuffer<ParticipantsKind>`]. See the generic for the full
/// contract (Q-019 type-confusion guard, `MaybeUninit` padding soundness).
pub type ParticipantBuffer = ErasedKindBuffer<ParticipantsKind>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::events::event::EventId;
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
        // Construct for ParticipantA, then call push::<ParticipantB>. The stored
        // `TypeId` is A; the assert compares against `TypeId::of::<B>()` and fires
        // before any unsound byte copy occurs.
        let mut buf = ParticipantBuffer::new::<ParticipantA>(DUMMY_EVENT_ID);
        let b_val = ParticipantB { a: 1, b: 2 };
        buf.push::<ParticipantB>(&b_val);
    }
}
