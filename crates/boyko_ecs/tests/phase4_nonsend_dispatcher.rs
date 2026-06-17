//! Phase 4 Seam 2 (D6 + CR-A + CR-B + IM-4) — behavioral test for the
//! NonSend resource SystemParam surface.
//!
//! Guards two facts (IM-4 — the Phase-8cd/14b missed-forwarder lesson):
//!
//! 1. **The param is actually fetched** (the silent-no-op class): a system
//!    taking `NonSendResMut<R>` mutates a counter in `R`; after
//!    `Schedule::run` the counter is observably changed. If the tuple `apply`
//!    forwarder dropped the param (or `get_param` were never wired), the
//!    counter would stay at its initial value.
//! 2. **The system runs on the dispatcher thread** (CR-A): the body records
//!    `thread::current().id()`; it must equal the `Schedule::run`-calling
//!    thread's id — proving a NonSend system resolves `CpuExclusive` and runs
//!    dispatcher-solo, never on a worker. Other concurrent CPU systems run in
//!    the same schedule to make the dispatcher-vs-worker distinction real.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::{self, ThreadId};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::resources::resource::NonSendResource;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::NonSendResMut;
use boyko_threadpool::ThreadPoolBuilder;

/// `!Send` resource: the raw `*const u8` makes the type `!Send` (no auto
/// impl), exactly the FFI-handle shape NonSend resources exist for. The body
/// mutates `counter` and records the thread it ran on.
struct NonSendProbe {
    counter: u32,
    observed_thread: Option<ThreadId>,
    _not_send: *const u8,
}

impl NonSendResource for NonSendProbe {}

/// Shared counter incremented by the concurrent CPU systems, so the schedule
/// has real worker-eligible work running alongside the dispatcher-only
/// NonSend system.
static CONCURRENT_HITS: AtomicUsize = AtomicUsize::new(0);

#[test]
fn nonsend_system_runs_on_dispatcher_and_observes_resource() {
    CONCURRENT_HITS.store(0, Ordering::Relaxed);

    // The thread that calls `Schedule::run` is the dispatcher thread (the
    // executor main loop runs inside `pool.install` on the calling thread,
    // and exclusive systems run inline on it).
    let caller_thread = thread::current().id();

    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    let mut world = EcsMaster::new();

    world.insert_non_send_resource(NonSendProbe {
        counter: 0,
        observed_thread: None,
        _not_send: std::ptr::null(),
    });

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));

    // The NonSend system: mutate the counter + record the running thread.
    builder.add_system(|mut probe: NonSendResMut<NonSendProbe>| {
        probe.counter += 1;
        probe.observed_thread = Some(thread::current().id());
    });

    // A handful of plain concurrent CPU systems running in the same schedule.
    // They declare no conflicting access, so the scheduler is free to run
    // them on workers concurrently with each other (the NonSend system is
    // CpuExclusive and serializes against all of them).
    for _ in 0..8 {
        builder.add_system(|| {
            CONCURRENT_HITS.fetch_add(1, Ordering::Relaxed);
        });
    }

    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    let probe = world.non_send_resource::<NonSendProbe>();

    // (a) the param was actually fetched + mutated (guards the silent-no-op).
    assert_eq!(
        probe.counter, 1,
        "NonSendResMut param must be fetched and its counter mutated exactly once"
    );

    // (b) the body ran on the dispatcher = the Schedule::run-calling thread.
    assert_eq!(
        probe.observed_thread,
        Some(caller_thread),
        "a NonSend system must run on the dispatcher thread (CpuExclusive), \
         never on a worker"
    );

    // Sanity: every concurrent CPU system ran too (the schedule actually
    // executed the full set, so the dispatcher-only result is not a
    // degenerate single-system schedule).
    assert_eq!(
        CONCURRENT_HITS.load(Ordering::Relaxed),
        8,
        "all concurrent CPU systems must have run"
    );
}

/// Compile-gate: `NonSendProbe` is `!Send` (the raw pointer interior), yet it
/// implements [`NonSendResource`] — the whole point of the trait carrying no
/// `Send + Sync` bound. The `assert_nonsend_resource` turbofish fails to
/// compile only if the bound is re-introduced. Simultaneously assert
/// `EcsMaster` stays `Send + Sync` after the NonSend slab field addition
/// (SEND1 non-regression / SEND10).
#[test]
fn nonsend_trait_accepts_non_send_type_and_send1_holds() {
    fn assert_nonsend_resource<T: NonSendResource>() {}
    fn assert_send_sync<T: Send + Sync>() {}

    assert_nonsend_resource::<NonSendProbe>();
    // SEND10 / CR-A: the type-erased NonSend slab does not weaken SEND1.
    assert_send_sync::<EcsMaster>();

    // Static witness that the probe really is `!Send` (a raw pointer field):
    // if a future edit accidentally made it `Send`, this `const` block would
    // still compile, so we document the intent rather than rely on a
    // negative-impl detector (not available on stable). The behavioral test
    // above is the load-bearing dispatcher-only proof.
    let _ = std::ptr::null::<u8>();
}

#[test]
fn nonsend_resource_round_trips_through_facade() {
    let mut world = EcsMaster::new();
    assert!(
        !world.contains_non_send_resource::<NonSendProbe>(),
        "empty world must not contain the NonSend resource"
    );

    world.insert_non_send_resource(NonSendProbe {
        counter: 5,
        observed_thread: None,
        _not_send: std::ptr::null(),
    });
    assert!(world.contains_non_send_resource::<NonSendProbe>());
    assert_eq!(world.non_send_resource::<NonSendProbe>().counter, 5);

    world.non_send_resource_mut::<NonSendProbe>().counter = 9;
    assert_eq!(world.non_send_resource::<NonSendProbe>().counter, 9);

    let removed = world
        .remove_non_send_resource::<NonSendProbe>()
        .expect("remove must return the value");
    assert_eq!(removed.counter, 9);
    assert!(!world.contains_non_send_resource::<NonSendProbe>());
}
