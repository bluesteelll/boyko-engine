//! `BundleTypeId` exhaustion is the terminal panic when the bundle-type table runs out.
//!
//! Its own integration binary, for the reason `l6_query_table_exhaustion.rs` gives for the query
//! table: driving the process-global mint counter past `MAX_BUNDLE_TYPES` is a one-way transition,
//! and any sibling test that later spawned a first-sight bundle -- every `#[derive(Bundle)]` type's
//! `static INFO` init closure calls `register_new()` exactly once -- would panic for this test's
//! reason rather than its own. The unit test that used to sit beside the dispenser
//! (`bundle_type_registry.rs`, `register_new_exhaustion_panics`) did exactly that to the lib-test
//! binary's other modules: it parked the counter one below the cap under a lock only its own module
//! took (A4b). What that test checked, this binary checks with one exception: **both edges of
//! the cap** -- exactly `MAX_BUNDLE_TYPES` ids are mintable, `0..MAX_BUNDLE_TYPES` in order, and
//! the mint after the last legal id is the panic -- the message, and that the panic is terminal:
//! every call after the first is the same panic. The exception is the saturate clamp, which no
//! binary can observe from outside the module; the query twin's module doc ("What this binary
//! cannot claim") has the measurement, and it holds for this dispenser too.
//!
//! # One test, on purpose
//!
//! The lower edge is only observable from a known counter, and the counter is known -- zero --
//! exactly once: at process start, before the first mint. This binary spawns no bundle and mints
//! nowhere but here, so the one test owns the counter from 0 to the cap. See the query twin's
//! module doc for the off-by-one (`id + 1 >= MAX`) that two counter-sharing tests would let
//! through; the deleted unit test pinned this edge with `set_next_id_for_test(MAX - 1)`, and this
//! is the same pin without the hook.
//!
//! The panic carries no `boyko-` registry code (the query twin's `B0502` is the codes-registry
//! precedent; this table predates it), so the gate is on the text: it must name the constant and
//! its value, because raising that constant is what the operator is told to do.

use std::any::Any;
use std::panic::catch_unwind;

use boyko_ecs::ecs::core::bundle::bundle_type_registry::{
    BundleTypeId, MAX_BUNDLE_TYPES, register_new,
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
fn exactly_max_bundle_types_ids_are_mintable_and_the_next_mint_is_the_terminal_panic_forever() {
    // Lower edge: every id below the cap is mintable, in order, from a counter that starts at 0.
    // A panic here is a cap that fired early; a wrong id at `expected == 0` is another mint in
    // this process before the test -- which this binary must not contain.
    for expected in 0..MAX_BUNDLE_TYPES {
        let id = match catch_unwind(register_new) {
            Ok(id) => id,
            Err(payload) => panic!(
                "mint {expected} is a legal id (below MAX_BUNDLE_TYPES = {MAX_BUNDLE_TYPES}) but \
                 panicked instead: {}",
                payload_text(&*payload)
            ),
        };
        assert_eq!(
            id,
            BundleTypeId(expected),
            "mint {expected} of {MAX_BUNDLE_TYPES} handed out {id:?}"
        );
    }

    // Upper edge: the mint after the last legal id is the terminal panic, naming the cap, its
    // value, and the recovery the operator is told about.
    let first = catch_unwind(register_new).expect_err(
        "mint MAX_BUNDLE_TYPES (one past the last legal id) must be the terminal panic",
    );
    let first_text = payload_text(&*first);
    assert!(
        first_text.contains(&format!(
            "BundleTypeId exhaustion: MAX_BUNDLE_TYPES = {MAX_BUNDLE_TYPES} reached"
        )),
        "the exhaustion panic must name the cap and its value; got: {first_text}"
    );
    assert!(
        first_text.contains("increase MAX_BUNDLE_TYPES"),
        "the exhaustion panic must tell the operator what to raise; got: {first_text}"
    );

    // Terminal: every call after the first panic is the same panic -- never a fresh id, never a
    // different message -- so a re-entry (a retried `OnceLock` init closure -- `OnceLock` does not
    // poison on panic -- or a second bundle asked for after the first died) cannot hand out an id
    // above the cap. This is the `>=` comparison at work, not the saturate store: the store is
    // unobservable from here (the query twin's module doc, "What this binary cannot claim").
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
