//! L6 — `boyko-B0502` is the terminal panic when the query-type table runs out.
//!
//! Its own integration binary, because filling the process-global mint counter past the cap is a
//! one-way transition: any sibling test that later asked for a `QueryTypeId` would panic for this
//! test's reason rather than its own. See `l6_query_table_high_water.rs`, which burns 768 slots
//! for the same reason at one remove.
//!
//! This binary is also the ONLY exhaustion gate. The unit test that used to sit beside the
//! dispenser (`query_type_registry.rs`, `register_new_exhaustion_panics`) parked the counter one
//! below the cap under a lock that only its own module took, while seven other src/ test modules
//! in the same lib-test binary minted first-sight `(D, F)` shapes through `world.query::<D, F>()`
//! -- a sibling that minted inside that window redded with `boyko-B0502`, one lib run in eleven
//! (A4b). What that test checked, this binary checks: the trigger (the mint past
//! `MAX_QUERY_TYPES`), the code, the message, and -- from outside the module, where the counter
//! itself is private -- the one observable consequence of its saturate clamp, in
//! [`b0502_is_terminal_every_call_after_the_first_is_the_same_panic`].
//!
//! **This is also the gate on `PanicCode`'s `Display`.** L6 replaced the string literal
//! `"boyko-B0502: …"` with the registry constant so the identifier reaches the walker's CODE
//! stream; the `expected` substring below is what proves the rendered text did not move with it.
//! Delete the `Display` impl's `boyko-` prefix, or write the code as an inline `{B0502}` format
//! argument, and this reds.

use std::any::Any;
use std::panic::catch_unwind;

use boyko_ecs::ecs::core::iters::query::query_type_registry::{MAX_QUERY_TYPES, register_new};

#[test]
#[should_panic(expected = "boyko-B0502")]
fn b0502_is_the_terminal_panic_when_the_table_is_exhausted() {
    // One past the cap. `register_new` saturates the counter before panicking, so the loop cannot
    // run it further even if the panic were caught.
    for _ in 0..=MAX_QUERY_TYPES {
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
/// counter is pinned at the cap, so a re-entry (a retried init closure, a second shape asked for
/// after the first died) cannot walk it past the cap or hand out an id above it.
///
/// Order-independent with the `#[should_panic]` test above: both drive the same monotonic counter
/// to the cap, and both accept `boyko-B0502` at whichever call reaches it first.
#[test]
fn b0502_is_terminal_every_call_after_the_first_is_the_same_panic() {
    let first = catch_unwind(|| {
        for _ in 0..=MAX_QUERY_TYPES {
            let _ = register_new();
        }
    })
    .expect_err("MAX_QUERY_TYPES + 1 mints must reach the terminal panic");
    let first_text = payload_text(&*first);
    assert!(
        first_text.contains("boyko-B0502"),
        "the exhaustion panic must carry the registry code; got: {first_text}"
    );
    assert!(
        first_text.contains(&format!("MAX_QUERY_TYPES = {MAX_QUERY_TYPES} reached")),
        "the exhaustion panic must name the cap and its value; got: {first_text}"
    );
    assert!(
        first_text.contains("big_query_table"),
        "the exhaustion panic must name the feature that raises the cap; got: {first_text}"
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
