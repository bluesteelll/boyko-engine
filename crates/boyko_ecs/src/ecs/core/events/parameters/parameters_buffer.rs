//! Type-erased buffer for event **parameters**.
//!
//! This is a thin alias over the shared [`ErasedKindBuffer`] generic; all
//! storage logic lives there. The [`ParametersBuffer`] name is preserved so the
//! `#[event]` attribute path and existing tests keep compiling unchanged.

use crate::ecs::core::events::erased_buffer::{ErasedKindBuffer, ParametersKind};

/// Type-erased buffer for storing event parameters.
///
/// Alias of [`ErasedKindBuffer<ParametersKind>`]. See the generic for the full
/// contract (Q-019 type-confusion guard, `MaybeUninit` padding soundness).
pub type ParametersBuffer = ErasedKindBuffer<ParametersKind>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::events::event::EventId;
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
    ///
    /// Carries its only consumers' `#[cfg(debug_assertions)]` (both Q-019 panic tests
    /// are gated that way, since `debug_assert!` is what they exercise), so a release
    /// build has no user for it and `-D warnings` reds. The cfg is structural: a future
    /// non-debug consumer fails to compile rather than silently reviving dead code.
    #[cfg(debug_assertions)]
    #[derive(Clone, Copy, PartialEq, Debug)]
    struct ParamB {
        speed: f32,
        friction: f32,
    }

    #[cfg(debug_assertions)]
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
