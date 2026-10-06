//! SR (3): a pool scope opened inside a region block is refused in debug builds
//! (`02-SR-DESIGN.md` §1.5; the scaling design §6.1's "no nested scope inside a region block").
//!
//! It is a LIVENESS rule, not hygiene. A join inside a block runs whatever task its queues hand
//! it — including a still-unstarted helper task of the very region the block belongs to. That
//! helper then waits for the region's next publish, which participant 0 cannot make while it is
//! inside the block that is joining: the region hangs (here: until the test policy's wait bound).
//!
//! The gate builds exactly that state. The pool's one worker is held by a gated task, so the
//! region's helper task stays queued in the global injector; participant 0 (the dispatcher) runs an
//! inline item whose block opens a nested `pool.scope`. With the guard, `Scope::new` refuses at once
//! with its message. Without it, the nested join takes the queued helper task, the helper's wait
//! passes the 2 s bound, and the payload is `RegionWaitBound` — the red this file recorded before
//! the guard existed.
//!
//! Debug profile only: the guard is a `debug_assert!`; release builds carry nothing.

mod region_common;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use boyko_threadpool::RegionWaitBound;

use region_common::{Frame, Route, Stages, TestPolicy, hold_workers, pool, run_region};

#[cfg(debug_assertions)]
#[test]
fn a_scope_opened_inside_a_region_block_is_refused_in_debug() {
    let pool = pool(1);
    let (_, release) = hold_workers(&pool, 1);
    // Item 0: entry 0, inline (its block opens the nested scope). Item 1: entry 1, two blocks.
    let mut frame = Frame::new(2, 2, 8);
    frame.set_table::<TestPolicy>(&[1, 2], &[0, 1], 2, 0);
    let stages = Arc::new(Stages::new(&frame).with_nested_scope(0));
    let ran = run_region::<TestPolicy, false>(&pool, Route::External, frame, stages, 2);
    release.store(true, Ordering::Release);
    let payload = ran.result.as_ref().expect_err("a nested scope inside a region block must not pass");
    let msg = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or_else(|| {
            panic!(
                "the payload is not the guard's message: bound {:?}",
                payload.downcast_ref::<RegionWaitBound>()
            )
        });
    assert!(msg.contains("inside a region block"), "the payload is {msg:?}");
    assert_eq!(ran.stages.nested_returned.load(Ordering::Relaxed), 0, "the nested scope never ran");
}

/// Release: no guard is compiled, and the hazard the guard exists for is what happens — the nested
/// join runs the region's own queued helper, whose wait reaches the test bound. Kept as a test so
/// the release target asserts the documented behaviour instead of reading `running 0 tests`.
#[cfg(not(debug_assertions))]
#[test]
fn without_the_guard_the_nested_join_runs_its_own_regions_helper() {
    let pool = pool(1);
    let (_, release) = hold_workers(&pool, 1);
    let mut frame = Frame::new(2, 2, 8);
    frame.set_table::<TestPolicy>(&[1, 2], &[0, 1], 2, 0);
    let stages = Arc::new(Stages::new(&frame).with_nested_scope(0));
    let ran = run_region::<TestPolicy, false>(&pool, Route::External, frame, stages, 2);
    release.store(true, Ordering::Release);
    let payload = ran.result.as_ref().expect_err("the nested join deadlocks the region until the bound");
    let bound = payload.downcast_ref::<RegionWaitBound>().unwrap_or_else(|| {
        panic!("the payload is not the wait bound: {:?}", payload.downcast_ref::<String>())
    });
    assert_eq!(bound.participant, 1, "the helper run inside the nested join hit the bound");
}
