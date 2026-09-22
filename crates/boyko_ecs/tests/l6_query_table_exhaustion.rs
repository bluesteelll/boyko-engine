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
//! (A4b). What that test checked, this binary checks with one exception: **both edges of the
//! cap** -- exactly `MAX_QUERY_TYPES` ids are mintable, `0..MAX_QUERY_TYPES` in order, and the
//! mint after the last legal id is the panic -- the code, the message, and that the panic is
//! terminal: every call after the first is the same panic.
//!
//! # What this binary cannot claim: the saturate clamp
//!
//! The deleted unit test also read the private counter after the panic and pinned it at
//! `MAX_QUERY_TYPES` -- the dispenser's saturate store. From outside the module that store is
//! unobservable: with `id >= MAX_QUERY_TYPES` as the cap check, a counter left unclamped at
//! `MAX + 1, MAX + 2, ..` produces the identical terminal panic, so the re-entry leg below pins
//! the comparison, not the store. Measured 2026-09-21 (the A4b retest): both `store(MAX_*,
//! Relaxed)` lines deleted, both binaries 1/1 green. The clamp is therefore unpinned on this
//! line; its only failure mode is a wrap after `usize::MAX - MAX_QUERY_TYPES` re-entries into a
//! process that is already dying. Pinning it again needs either a read hook in the shipped
//! library or the cap check factored over a caller-supplied counter -- a change to the dispenser
//! body, not to this file.
//!
//! # One test, on purpose
//!
//! The lower edge ("the 1024th mint succeeds and hands out `QueryTypeId(1023)`") is only
//! observable from a known counter, and the counter is known -- zero -- exactly once: at process
//! start, before the first mint. This binary's only mints are this test's, so the one test owns
//! the counter from 0 to the cap. Two tests sharing the counter could each still see the panic,
//! but neither could say which mint was the last legal one, and a cap that is off by one in the
//! downward direction (`id + 1 >= MAX_QUERY_TYPES`, cap 1023) would be green: the message prints
//! the constant, not the id, so the upper-edge legs cannot tell 1023 from 1024. The deleted unit
//! test pinned that edge with `set_next_id_for_test(MAX - 1); assert_eq!(register_new(), MAX - 1)`;
//! this is the same pin without the hook.
//!
//! **This is also the gate on `PanicCode`'s `Display`.** L6 replaced the string literal
//! `"boyko-B0502: …"` with the registry constant so the identifier reaches the walker's CODE
//! stream; the `boyko-B0502` substring assertion below is what proves the rendered text did not
//! move with it. Delete the `Display` impl's `boyko-` prefix, or write the code as an inline
//! `{B0502}` format argument, and this reds.

use std::any::Any;
use std::panic::catch_unwind;

use boyko_ecs::ecs::core::iters::query::query_type_registry::{
    MAX_QUERY_TYPES, QueryTypeId, register_new,
};

/// The panic payload as text, whichever of the two payload types `panic!` produced.
fn payload_text(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&'static str>().copied())
        .unwrap_or("<non-string panic payload>")
}

#[test]
fn exactly_max_query_types_ids_are_mintable_and_the_next_mint_is_b0502_forever() {
    // Lower edge: every id below the cap is mintable, in order, from a counter that starts at 0.
    // A panic here is a cap that fired early; a wrong id at `expected == 0` is another mint in
    // this process before the test -- which this binary must not contain.
    for expected in 0..MAX_QUERY_TYPES {
        let id = match catch_unwind(register_new) {
            Ok(id) => id,
            Err(payload) => panic!(
                "mint {expected} is a legal id (below MAX_QUERY_TYPES = {MAX_QUERY_TYPES}) but \
                 panicked instead: {}",
                payload_text(&*payload)
            ),
        };
        assert_eq!(
            id,
            QueryTypeId(expected),
            "mint {expected} of {MAX_QUERY_TYPES} handed out {id:?}"
        );
    }

    // Upper edge: the mint after the last legal id is the terminal panic, with the registry code,
    // the cap's name and value, and the recovery the operator is told about.
    let first = catch_unwind(register_new)
        .expect_err("mint MAX_QUERY_TYPES (one past the last legal id) must be the terminal panic");
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

    // Terminal: every call after the first panic is the same panic -- never a fresh id, never a
    // different message -- so a re-entry (a retried init closure, a second shape asked for after
    // the first died) cannot hand out an id above the cap. This is the `>=` comparison at work,
    // not the saturate store: the store is unobservable from here (module doc, "What this binary
    // cannot claim").
    for attempt in 0..3 {
        let again = match catch_unwind(register_new) {
            Ok(id) => {
                panic!("re-entry {attempt} after exhaustion minted {id:?} instead of panicking")
            }
            Err(payload) => payload,
        };
        let again_text = payload_text(&*again);
        assert_eq!(
            again_text, first_text,
            "re-entry {attempt} must be the same terminal panic"
        );
    }
}
