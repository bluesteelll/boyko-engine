//! `BundleTypeId` exhaustion is the terminal panic when the bundle-type table runs out.
//!
//! Its own integration binary, for the reason `l6_query_table_exhaustion.rs` gives for the query
//! table: driving the process-global mint counter past `MAX_BUNDLE_TYPES` is a one-way transition,
//! and any sibling test that later spawned a first-sight bundle -- every `#[derive(Bundle)]` type's
//! `static INFO` init closure calls `register_new()` exactly once -- would panic for this test's
//! reason rather than its own. The unit test that used to sit beside the dispenser
//! (`bundle_type_registry.rs`, `register_new_exhaustion_panics`) did exactly that to the lib-test
//! binary's other modules: it parked the counter one below the cap under a lock only its own module
//! took (A4b). What that test checked, this binary checks: the trigger (the mint past
//! `MAX_BUNDLE_TYPES`), the message, and -- from outside the module, where the counter itself is
//! private -- the one observable consequence of its saturate clamp, in
//! [`exhaustion_is_terminal_every_call_after_the_first_is_the_same_panic`].
//!
//! The panic carries no `boyko-` registry code (the query twin's `B0502` is the codes-registry
//! precedent; this table predates it), so the gate is on the text: it must name the constant and
//! its value, because raising that constant is what the operator is told to do.

use std::any::Any;
use std::panic::catch_unwind;

use boyko_ecs::ecs::core::bundle::bundle_type_registry::{MAX_BUNDLE_TYPES, register_new};

#[test]
#[should_panic(expected = "BundleTypeId exhaustion: MAX_BUNDLE_TYPES = ")]
fn exhaustion_is_the_terminal_panic_when_the_table_is_exhausted() {
    // One past the cap. `register_new` saturates the counter before panicking, so the loop cannot
    // run it further even if the panic were caught.
    for _ in 0..=MAX_BUNDLE_TYPES {
        let _ = register_new();
    }
}

/// The panic payload as text, whichever of the two payload types `panic!` produced.
fn payload_text(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&'static str>().copied())
        .unwrap_or("<non-string panic payload>")
}

/// Every call after the terminal panic is the same terminal panic -- never a fresh id, never a
/// different message. This is what the dispenser's saturate clamp buys from the outside: the
/// counter is pinned at the cap, so a re-entry (a retried `OnceLock` init closure -- `OnceLock`
/// does not poison on panic -- or a second bundle asked for after the first died) cannot walk it
/// past the cap or hand out an id above it.
///
/// Order-independent with the `#[should_panic]` test above: both drive the same monotonic counter
/// to the cap, and both accept the exhaustion panic at whichever call reaches it first.
#[test]
fn exhaustion_is_terminal_every_call_after_the_first_is_the_same_panic() {
    let first = catch_unwind(|| {
        for _ in 0..=MAX_BUNDLE_TYPES {
            let _ = register_new();
        }
    })
    .expect_err("MAX_BUNDLE_TYPES + 1 mints must reach the terminal panic");
    let first_text = payload_text(&*first);
    assert!(
        first_text.contains(&format!("BundleTypeId exhaustion: MAX_BUNDLE_TYPES = {MAX_BUNDLE_TYPES} reached")),
        "the exhaustion panic must name the cap and its value; got: {first_text}"
    );
    assert!(
        first_text.contains("increase MAX_BUNDLE_TYPES"),
        "the exhaustion panic must tell the operator what to raise; got: {first_text}"
    );

    for attempt in 0..3 {
        let again = match catch_unwind(register_new) {
            Ok(id) => panic!("re-entry {attempt} after exhaustion minted {id:?} instead of panicking"),
            Err(payload) => payload,
        };
        let again_text = payload_text(&*again);
        assert_eq!(
            again_text, first_text,
            "re-entry {attempt} must be the same terminal panic"
        );
    }
}
