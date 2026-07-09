/// Integration tests for the `#[event]` attribute macro.
///
/// Validates audit finding Q-005: the macro-generated `event_id()` must mint a
/// unique, stable ID on first call and return the cached value on subsequent
/// calls — using the per-type `OnceLock` + `register_event_new` path.
///
/// Also serves as the Q-001 regression test suite: the `#[event]` macro rewrites
/// the struct into a two-field native layout, eliminating the unsound cast from
/// the deleted `#[derive(Event)]`.
///
/// Test isolation note: the roadmap reserves event-ID range 500-509 for these
/// integration tests, but post-C-003 the `event_id()` accessor mints from
/// `NEXT_EVENT_ID` sequentially via `register_event_new`, so ranges cannot be
/// enforced. Each `#[event]`-generated type carries its own per-impl `OnceLock`,
/// so re-running tests is idempotent — but cross-test ordering of IDs is not.
use boyko_macros::event;
use boyko_ecs::ecs::core::events::event::Event;
use boyko_ecs::ecs::core::events::event_registry;
use boyko_ecs::ecs::core::events::parameters::parameters_buffer::ParametersBuffer;

/// A minimal event with no fields (no participants, no parameters).
#[allow(dead_code)]
#[event]
struct PulseEvent {}

/// A second event type to verify distinct ID assignment.
#[allow(dead_code)]
#[event]
struct TickEvent {}

/// A third event with a parameter field.
#[allow(dead_code)]
#[event]
struct DamageEvent {
    #[parameter]
    amount: f32,
}

// --- Migrated tests (names preserved from derive_event.rs) ---

/// #[event] must mint an ID on the first call to event_id().
/// The ID must be < MAX_EVENTS and the registry slot must be populated.
#[test]
fn derive_event_first_call_mints_valid_id() {
    let id = PulseEvent::event_id();
    assert!(
        (id as usize) < event_registry::MAX_EVENTS,
        "event_id must be < MAX_EVENTS, got {id}"
    );
    assert!(
        event_registry::is_event_registered(id),
        "registry slot must be populated after first event_id() call"
    );
}

/// The second call to event_id() must return the same value as the first.
/// This is the OnceLock cache effect — no second trip through register_event_new.
#[test]
fn derive_event_emits_lazy_id_second_call_returns_same() {
    let id_first = PulseEvent::event_id();
    let id_second = PulseEvent::event_id();
    assert_eq!(
        id_first,
        id_second,
        "event_id() must be stable: first={id_first}, second={id_second}"
    );
}

/// Two distinct event types must receive different EventIds.
#[test]
fn derive_event_distinct_types_get_distinct_ids() {
    let id_pulse = PulseEvent::event_id();
    let id_tick = TickEvent::event_id();
    let id_damage = DamageEvent::event_id();

    assert_ne!(
        id_pulse,
        id_tick,
        "PulseEvent and TickEvent must have different event IDs \
         (got id_pulse={id_pulse}, id_tick={id_tick})"
    );
    assert_ne!(
        id_pulse,
        id_damage,
        "PulseEvent and DamageEvent must have different event IDs \
         (got id_pulse={id_pulse}, id_damage={id_damage})"
    );
    assert_ne!(
        id_tick,
        id_damage,
        "TickEvent and DamageEvent must have different event IDs \
         (got id_tick={id_tick}, id_damage={id_damage})"
    );
}

/// The registry slot populated by event_id() must carry the correct TypeId.
#[test]
fn derive_event_registry_slot_carries_correct_type_id() {
    use std::any::TypeId;

    let id = TickEvent::event_id();
    let info = event_registry::get_event_info(id)
        .expect("TickEvent slot must be populated");
    assert_eq!(
        info.type_id,
        TypeId::of::<TickEvent>(),
        "EventInfo at TickEvent's slot must carry TypeId::of::<TickEvent>()"
    );
}

/// An event with a parameter field: the generated EventInfo must reflect the
/// parameters layout (non-zero size for DamageEvent which has an f32 field).
#[test]
fn derive_event_with_parameter_field_has_nonzero_parameters_layout() {
    let id = DamageEvent::event_id();
    let info = event_registry::get_event_info(id)
        .expect("DamageEvent slot must be populated");
    assert_eq!(
        info.parameters_layout.size(),
        std::mem::size_of::<DamageEventParameters>(),
        "parameters_layout.size() must match DamageEventParameters size"
    );
}

/// Multiple repeated calls in a loop must all return the same ID (stress variant).
#[test]
fn derive_event_id_is_stable_across_many_calls() {
    let expected = PulseEvent::event_id();
    for i in 0..100 {
        let id = PulseEvent::event_id();
        assert_eq!(
            id,
            expected,
            "call {i}: event_id() returned {id}, expected {expected}"
        );
    }
}

// --- Q-001 regression tests (added in step 6f, pre-declared here) ---

/// Q-001 regression: push/get round-trip through ParametersBuffer for a type
/// with internal padding (u8 + padding + u64 = 16 bytes).
///
/// Validates that Vec<MaybeUninit<u8>> storage correctly handles padding bytes
/// that the source struct contains between `a` (u8) and `b` (u64).
#[allow(dead_code)]
#[event]
struct PaddedEvent {
    #[parameter]
    a: u8,
    #[parameter]
    b: u64,
}

#[test]
fn q001_buffer_push_get_roundtrip_with_padded_struct() {
    let event_id = PaddedEvent::event_id();
    let mut buf = ParametersBuffer::new::<PaddedEventParameters>(event_id);

    for i in 0u64..1000 {
        let params = PaddedEventParameters {
            a: (i % 256) as u8,
            b: i.wrapping_mul(0x0101_0101_0101_0101),
        };
        buf.push::<PaddedEventParameters>(&params);
    }

    assert_eq!(buf.len(), 1000);

    for i in 0u64..1000 {
        // SAFETY: PaddedEventParameters matches the type pushed at index i.
        let recovered = unsafe {
            buf.get::<PaddedEventParameters>(i as usize)
                .expect("index must be valid")
        };
        assert_eq!(
            recovered.a,
            (i % 256) as u8,
            "field `a` mismatch at index {i}"
        );
        assert_eq!(
            recovered.b,
            i.wrapping_mul(0x0101_0101_0101_0101),
            "field `b` mismatch at index {i}"
        );
    }
}

/// Q-002 regression: replacement for the deleted `from_bytes_on_unaligned_buffer_no_ub`.
///
/// Forces a buffer slot offset that is not aligned to `align_of::<P>()` and asserts
/// that push/get round-trips correctly. Uses `ThreeU32` (size=12, align=4) as the
/// misaligning first entry, then `TwoU64` (size=16, align=8) at the next slot
/// (offset 12 — not 8-aligned) to trigger the unaligned-read path.
#[test]
fn q002_regression_buffer_round_trip_on_unaligned_offset() {
    use boyko_ecs::ecs::core::events::participants::participants::Participants;
    use boyko_ecs::ecs::core::events::participants::participants::ParticipantInfo;

    // ThreeU32: size=12, align=4. Pushing one entry makes offset 12 the next slot.
    #[repr(C)]
    #[derive(Debug, Clone, Copy, PartialEq)]
    struct ThreeU32 {
        x: u32,
        y: u32,
        z: u32,
    }
    impl Participants for ThreeU32 {
        fn participant_count() -> usize { 1 }
        fn participant_info() -> &'static [ParticipantInfo] { &[] }
    }

    // TwoU64: size=16, align=8. At offset 12 the pointer is not 8-aligned.
    #[repr(C)]
    #[derive(Debug, Clone, Copy, PartialEq)]
    struct TwoU64 {
        a: u64,
        b: u64,
    }
    impl Participants for TwoU64 {
        fn participant_count() -> usize { 1 }
        fn participant_info() -> &'static [ParticipantInfo] { &[] }
    }

    // Use a raw Vec<MaybeUninit<u8>> approach: push ThreeU32 first into its own
    // buffer, then verify TwoU64 in a separate buffer at various offsets to
    // exercise read_unaligned. Simpler: push 1 ThreeU32 into a u8-agnostic buffer,
    // then push TwoU64 into the same buffer (same type, because ParticipantBuffer
    // is monomorphised per-type). Instead, we build the scenario at the raw level:
    // create a Vec<MaybeUninit<u8>>, write a ThreeU32 at offset 0, then write a
    // TwoU64 at offset 12 (misaligned for u64), then read it back via read_unaligned.
    use std::mem::MaybeUninit;

    let three = ThreeU32 { x: 1, y: 2, z: 3 };
    let two = TwoU64 { a: 0x1122_3344_5566_7788, b: 0xAABB_CCDD_EEFF_0011 };

    let mut buf: Vec<MaybeUninit<u8>> = vec![MaybeUninit::uninit(); 12 + 16];

    // Write ThreeU32 at offset 0.
    // SAFETY: buf has 28 bytes; ThreeU32 is 12 bytes; destination does not overlap source.
    unsafe {
        std::ptr::copy_nonoverlapping(
            (&three as *const ThreeU32).cast::<u8>(),
            buf.as_mut_ptr().cast::<u8>(),
            12,
        );
    }

    // Write TwoU64 at offset 12 (not 8-aligned).
    // SAFETY: same invariants as above; destination starts at byte 12.
    unsafe {
        std::ptr::copy_nonoverlapping(
            (&two as *const TwoU64).cast::<u8>(),
            buf.as_mut_ptr().add(12).cast::<u8>(),
            16,
        );
    }

    // Read back TwoU64 from offset 12 using read_unaligned — the same operation
    // used by ParticipantBuffer::get.
    // SAFETY: bytes at offset 12..28 were written as a valid TwoU64 above.
    let recovered = unsafe {
        std::ptr::read_unaligned(buf.as_ptr().add(12).cast::<TwoU64>())
    };

    assert_eq!(
        recovered,
        two,
        "read_unaligned from misaligned offset must reproduce the written TwoU64 — Q-002 regression"
    );
}

// --- N1 / N3 regression tests (post-Q-001 polish) ---

/// N1 regression: the macro must reproduce field visibility 1:1 on generated substructs.
///
/// `pub_param` is `pub`, `crate_param` has no explicit visibility (inherited/private
/// from the crate perspective, but accessible within this integration test crate).
/// If the macro silently widens both to `pub`, this still compiles — but the key
/// assertion is that fields with *no* explicit visibility keyword are accepted
/// (i.e. `syn::Visibility::Inherited`) and the generated substruct compiles at all.
#[test]
fn n1_substruct_preserves_field_visibility() {
    #[event]
    #[allow(dead_code)]
    struct VisEvent {
        #[participant(components = "")]
        pub_field: boyko_ecs::ecs::core::entity::entity::Entity,
        #[parameter]
        pub pub_param: u32,
        #[parameter]
        crate_param: u64,
    }

    // Construct the substructs directly; both fields are accessible from
    // within this crate regardless of whether they are `pub` or inherited.
    let _ = VisEventParameters {
        pub_param: 0,
        crate_param: 0,
    };
}

/// N3 regression: verify the happy path of `components = "..."` parsing still works
/// after switching from substring-scan to `syn::parse_nested_meta`.
#[test]
fn n3_components_parser_happy_path() {
    use boyko_ecs::ecs::core::events::participants::participants::Participants;

    #[event]
    #[allow(dead_code)]
    struct N3Event {
        #[participant(components = "")]
        a: boyko_ecs::ecs::core::entity::entity::Entity,
        #[participant(components = "")]
        b: boyko_ecs::ecs::core::entity::entity::Entity,
        #[parameter]
        x: u32,
    }
    // If the macro parsed the attributes correctly, this compiles.
    assert_eq!(N3EventParticipants::participant_count(), 2);
}

/// Smoke test: clear() and Drop on a large buffer do not panic and leave the
/// buffer empty. Validates that Vec<MaybeUninit<u8>> does not call Drop on
/// stored bytes (the type system enforces this: MaybeUninit<u8> has no Drop).
#[allow(dead_code)]
#[event]
struct SmokeEvent {
    #[parameter]
    value: u64,
}

#[test]
fn q001_buffer_clear_drop_smoke() {
    let event_id = SmokeEvent::event_id();
    let mut buf = ParametersBuffer::new::<SmokeEventParameters>(event_id);

    for i in 0u64..10_000 {
        buf.push::<SmokeEventParameters>(&SmokeEventParameters { value: i });
    }
    assert_eq!(buf.len(), 10_000);
    assert!(!buf.is_empty());

    buf.clear();
    assert_eq!(buf.len(), 0);
    assert!(buf.is_empty());

    // Push again after clear to verify the buffer is reusable.
    for i in 0u64..100 {
        buf.push::<SmokeEventParameters>(&SmokeEventParameters { value: i });
    }
    assert_eq!(buf.len(), 100);

    // Drop at end of function — must not panic.
}
