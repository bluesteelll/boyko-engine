//! Phase S1.5 — fn-level round-trips for the derived `serialize_fn` /
//! `deserialize_fn` glue.
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` §3.1 / §3.7. A `#[derive(Component)]` whose
//! fields are all `Wire` (`{ name: String, hp: u32, flags: Vec<u8> }`) must:
//! * classify `SerializeViaFn` and install BOTH a `serialize_fn` and a
//!   `deserialize_fn`;
//! * run `serialize_fn` into a `SaveCursor`, then `deserialize_fn` from a
//!   `LoadCursor` into a `MaybeUninit<C>`, reconstructing the original value
//!   (drop-safe — no double-free);
//! * return `Err` from `deserialize_fn` on a malformed / truncated stream, leaving
//!   `dst` uninitialized (no UB — the value is built fully before any `ptr::write`).
//!
//! These exercise the runtime `unsafe` this phase adds (`*const C` cast + the
//! `ptr::write` into uninit), so the suite is also run under Miri-TB.

use std::mem::MaybeUninit;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::{self, Serializability};
use boyko_ecs::ecs::core::serialize::{DecodeError, LoadCursor, SaveCursor};
use boyko_macros::Component;

/// An owning + bit-restricted-free component: all fields are `Wire`, so the derive
/// installs the per-element encode/decode glue (`SerializeViaFn`).
#[derive(Component, Clone, PartialEq, Debug)]
struct Unit {
    name: String,
    hp: u32,
    flags: Vec<u8>,
}

/// Resolves the installed `serialize_fn` / `deserialize_fn` for a component, forcing
/// registration first.
fn ser_de<T: Component>() -> (
    component_registry::SerializeFn,
    component_registry::DeserializeFn,
    Serializability,
) {
    let id = T::component_id().0;
    let info = component_registry::get_serialize_info(id).expect("serialize info installed");
    (
        info.serialize_fn.expect("serialize_fn installed for a Wire component"),
        info.deserialize_fn.expect("deserialize_fn installed for a Wire component"),
        info.serializability,
    )
}

#[test]
fn owning_component_classifies_via_fn_and_installs_glue() {
    let (_ser, _de, class) = ser_de::<Unit>();
    assert_eq!(
        class,
        Serializability::SerializeViaFn,
        "an owning String/Vec component must be SerializeViaFn"
    );
}

#[test]
fn fn_level_roundtrip_reconstructs_the_value() {
    let (serialize_fn, deserialize_fn, _) = ser_de::<Unit>();
    let original = Unit {
        name: "boyko".to_string(),
        hp: 0xCAFE_BABE,
        flags: vec![1, 2, 3, 255, 0],
    };

    // Encode through the derived serialize_fn into a buffer.
    let mut buf = Vec::new();
    {
        let mut sink = SaveCursor::new(&mut buf);
        // SAFETY: `&original` is a live, aligned, initialized `Unit`; the cursor is a
        // valid append-only sink (the SerializeFn contract).
        unsafe { serialize_fn((&original as *const Unit).cast::<u8>(), &mut sink) };
    }
    assert!(!buf.is_empty(), "an owning component must encode real bytes");

    // Decode through the derived deserialize_fn into uninitialized space.
    let mut slot = MaybeUninit::<Unit>::uninit();
    let mut src = LoadCursor::new(&buf);
    // SAFETY: `slot.as_mut_ptr()` is writable, uninitialized, aligned space for one
    // `Unit` (the DeserializeFn contract); on Ok it is written exactly once.
    let result = unsafe { deserialize_fn(&mut src, slot.as_mut_ptr().cast::<u8>()) };
    assert!(result.is_ok(), "well-formed bytes must decode");

    // SAFETY: `deserialize_fn` returned Ok, so `slot` holds an initialized `Unit`.
    let decoded = unsafe { slot.assume_init() };
    assert_eq!(decoded, original, "fn-level round-trip must preserve the value");
    assert_eq!(src.remaining(), 0, "the decode must consume exactly the encoded bytes");
    // `decoded` drops here (its String + Vec are freed once — no double-free, since
    // the source was moved out of the LoadCursor bytes by value, not aliased).
}

#[test]
fn deserialize_fn_rejects_truncated_stream_and_leaves_dst_uninit() {
    let (serialize_fn, deserialize_fn, _) = ser_de::<Unit>();
    let original = Unit {
        name: "truncate me".to_string(),
        hp: 42,
        flags: vec![9, 8, 7],
    };

    let mut buf = Vec::new();
    {
        let mut sink = SaveCursor::new(&mut buf);
        // SAFETY: `&original` is a live, aligned, initialized `Unit`.
        unsafe { serialize_fn((&original as *const Unit).cast::<u8>(), &mut sink) };
    }

    // Truncate the encoded bytes so a field read runs off the end. The decode must
    // fail BEFORE the `ptr::write`, leaving `slot` uninitialized — never dropped.
    let truncated = &buf[..buf.len() - 1];
    let mut slot = MaybeUninit::<Unit>::uninit();
    let mut src = LoadCursor::new(truncated);
    // SAFETY: `slot.as_mut_ptr()` is writable, uninitialized, aligned space for one
    // `Unit`; on Err `slot` is left uninitialized and must NOT be assumed-init.
    let result = unsafe { deserialize_fn(&mut src, slot.as_mut_ptr().cast::<u8>()) };
    assert!(result.is_err(), "a truncated stream must return Err");
    // `slot` falls out of scope as a `MaybeUninit` (its `Drop` is a no-op) — we never
    // `assume_init`, so no half-built `Unit` is dropped (the W5 partial-row contract;
    // Miri-TB verifies there is no read of uninitialized memory here).
    let _ = &slot;
}

#[test]
fn deserialize_fn_rejects_invalid_utf8_in_a_string_field() {
    let (_serialize_fn, deserialize_fn, _) = ser_de::<Unit>();

    // Hand-build a malformed stream: a String length prefix of 2 followed by invalid
    // UTF-8, so the `name` field read fails with InvalidBitPattern (before hp/flags).
    let mut buf = Vec::new();
    {
        let mut sink = SaveCursor::new(&mut buf);
        // String field: u32 length = 2, then two invalid UTF-8 bytes.
        2u32.to_le_bytes().iter().for_each(|&b| sink.write_bytes(&[b]));
    }
    buf.push(0xFF);
    buf.push(0xFE);

    let mut slot = MaybeUninit::<Unit>::uninit();
    let mut src = LoadCursor::new(&buf);
    // SAFETY: uninitialized aligned space for one `Unit`; left uninit on Err.
    let result = unsafe { deserialize_fn(&mut src, slot.as_mut_ptr().cast::<u8>()) };
    assert_eq!(
        result,
        Err(DecodeError::InvalidBitPattern),
        "an invalid-UTF-8 String field must be rejected"
    );
    // `slot` stays uninitialized (never `assume_init`) — no `Unit` is dropped.
    let _ = &slot;
}
