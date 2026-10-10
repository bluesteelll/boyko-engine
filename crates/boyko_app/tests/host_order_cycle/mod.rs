//! Shared by the R4-frame-order `host_orders_*` gates.
//!
//! Each gate builds the shipped `EnginePlugins` composition, declares the REVERSE of one shipped
//! ordering edge on top of it, and expects `finish()` to reject the result with a `boyko-B9001`
//! ordering cycle. [`assert_finish_rejects_cycle`] runs that `finish()` and checks the cycle the
//! panic names.
//!
//! # Why the cycle is compared as a set
//!
//! `boyko-B9001` alone matches ANY ordering cycle, so the gates compare the systems the cycle
//! names, which ties the panic to the edge under test. But the ORDER `finish()` lists them in is
//! the order Tarjan's traversal pops them, and an edge that changes nothing about which systems
//! form the cycle can still change that order. MEASURED while writing these gates: with the
//! complete list pinned as a `should_panic(expected = ...)` string, deleting the
//! `InstancePackSet` edge turned the `VisibilitySet` gate red — its cycle still held the same six
//! systems, listed in a different order. A gate that goes red on a reordering it does not guard is
//! noise, so the members are compared as a set, and the count `finish()` prints is checked
//! against the list.
//!
//! Each expected entry is the name `finish()` prints: the system's path with its set memberships
//! appended (`name [in: SetA, SetB]`). A system joining or leaving a set therefore changes the
//! entry, which is intended — the memberships are half of what a set edge orders.

use std::panic::{AssertUnwindSafe, catch_unwind};

use boyko_ecs::App;

/// The text `ScheduleBuildError::OrderingCycle` renders before the count.
const CYCLE_PREFIX: &str = "boyko-B9001: schedule contains a cycle of ";

/// Runs `app.finish()`, which must panic with a `boyko-B9001` ordering cycle whose systems are
/// exactly `expected`, in any order.
///
/// # Panics
///
/// If `finish()` returns (nothing in the composition orders the pair the reverse edge names, i.e.
/// the shipped edge under test is gone), if it panics with anything other than an ordering cycle,
/// or if the cycle names a different set of systems than `expected`.
pub fn assert_finish_rejects_cycle(app: &mut App, expected: &[&str]) {
    let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
        app.finish();
    })) else {
        panic!(
            "finish() accepted the reverse edge: nothing in the shipped composition orders these \
             systems the other way, so the shipped edge under test is gone. Expected a cycle of \
             {expected:#?}"
        );
    };
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("<a non-string panic payload>");
    let Some(rest) = message.strip_prefix(CYCLE_PREFIX) else { unrecognised(message) };
    let Some((count, list)) = rest.split_once(" systems: [\"") else { unrecognised(message) };
    let Some(list) = list.strip_suffix("\"]") else { unrecognised(message) };

    let mut got: Vec<&str> = list.split("\", \"").collect();
    assert_eq!(
        count.parse::<usize>().ok(),
        Some(got.len()),
        "the cycle's printed count and its list disagree: {message}"
    );
    let mut want = expected.to_vec();
    got.sort_unstable();
    want.sort_unstable();
    assert_eq!(
        got, want,
        "the reverse edge closed a cycle through a different set of systems than the shipped edge \
         should (sorted; the message was: {message})"
    );
}

/// The failure for a `finish()` panic that is not an ordering cycle.
fn unrecognised(message: &str) -> ! {
    panic!("finish() panicked, but not with an ordering cycle: {message}")
}
