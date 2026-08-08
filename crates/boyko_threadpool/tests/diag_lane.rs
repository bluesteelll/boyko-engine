//! Rung D1 gates — the `boyko_threadpool -> boyko_diag` lane edge.
//!
//! `boyko_diag` owns one per-thread integer that both diagnostics subsystems
//! index by, and it sits *below* this crate, so it cannot discover which thread
//! is which. The pool writes the slot. These are the gates on that write, and
//! each one names the broken implementation it reds on:
//!
//! - **DG2** — `lane()` on worker `k` is `k`; an unattached thread is
//!   `LANE_UNCLAIMED`. Deleting the `set_lane` call in `worker_main` makes every
//!   worker read `LANE_UNCLAIMED`.
//! - **DG3** — the lane and the pool's own worker id agree before, during and
//!   after an `install`, **including after an `install` whose body panicked and
//!   was caught**, and including a thread that entered `install` holding a
//!   *claimed spare* lane. Moving the restore out of `InstallGuard::drop` into
//!   the normal-return path reds the panicking leg only; deriving the restored
//!   lane from `prev_worker_id` instead of saving it reds the spare leg only.
//! - **DG4** is a `const _: () = assert!(...)` in `thread_pool.rs` and cannot
//!   have a runtime leg: breaking it fails the build, so there is nothing here
//!   to assert and nothing is asserted.
//!
//! `boyko_diag` is a normal dependency of this package, so a `tests/` target can
//! name it directly.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::thread;

use boyko_diag::lane::{
    self, LANE_DISPATCHER, LANE_SPARE_BASE, LANE_UNCLAIMED, LANE_WORKER_MAX,
};
use boyko_threadpool::{
    MAX_WORKERS, ThreadPoolBuilder, WORKER_ID_DISPATCHER, WORKER_ID_UNATTACHED, current_worker_id,
};

/// DG4's subject, restated where a reader of the gates will look for it. The
/// `const` assert in `thread_pool.rs` is the gate; this only records that the
/// two constants are the same quantity and not a coincidence.
#[test]
fn lane_worker_max_equals_max_workers() {
    assert_eq!(LANE_WORKER_MAX as usize, MAX_WORKERS);
}

/// DG2 (a). A thread the pool never touched carries no lane — and, since this
/// runs after a pool has been built and torn down in other tests of the same
/// binary, it also witnesses that building a pool writes no lane on the
/// *calling* thread.
#[test]
fn unattached_thread_has_no_lane() {
    assert_eq!(current_worker_id(), WORKER_ID_UNATTACHED);
    assert_eq!(lane::lane(), LANE_UNCLAIMED);

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    assert_eq!(
        lane::lane(),
        LANE_UNCLAIMED,
        "building a pool must not label the thread that built it"
    );
    drop(pool);
    assert_eq!(lane::lane(), LANE_UNCLAIMED);
}

/// DG2 (b). Every task body observes a lane that agrees with its worker id
/// **under the documented map**, and the observation count is asserted so the
/// loop cannot pass by running nothing.
///
/// The map, not raw equality. `Scope::drop` blocks by *work stealing*, not by
/// parking: it steals from the injector and the sibling stealers and runs what
/// it takes **inline on the calling thread** (`scope.rs`, "Work-stealing wait").
/// A task that lands there legitimately executes with `current_worker_id() ==
/// WORKER_ID_DISPATCHER` and `lane() == LANE_DISPATCHER` — a *correct* pairing
/// that a raw `lane as u32 == id` check reports as a defect.
///
/// MEASURED: an earlier draft asserted raw equality, passed with zero
/// disagreements, and on a later run reported exactly one — the gate was
/// **flaky by construction**. The rate is not "some fraction": across ~24
/// further runs of this binary the dispatcher executed **zero** of the 4096
/// tasks, so the one observation is the only one, and the honest statement is
/// that the path is *documented and reachable*, not that it is *covered*. Which
/// is why the failure message below carries the offending pair rather than a
/// bare count: the next occurrence must explain itself.
#[test]
fn worker_lane_agrees_with_worker_id() {
    const WORKERS: usize = 4;
    const TASKS: usize = 4096;

    let pool = ThreadPoolBuilder::new().num_threads(WORKERS).build();

    let observed = AtomicUsize::new(0);
    let disagreements = AtomicUsize::new(0);
    let on_worker = AtomicUsize::new(0);
    let on_dispatcher = AtomicUsize::new(0);
    // Bit `k` set means worker `k` ran at least one task. Read only to assert no
    // bit lands outside the pool's worker range — *which* workers the stealer
    // happened to feed is not a property this gate may claim.
    let seen = AtomicU64::new(0);
    // First offending `(worker id, lane)` pair, packed `id << 16 | lane`.
    // `u64::MAX` is the unset sentinel and cannot collide with a real pair,
    // whose top 16 bits are always zero.
    let offender = AtomicU64::new(u64::MAX);

    pool.install(|scope| {
        for _ in 0..TASKS {
            scope.spawn(|| {
                let id = current_worker_id();
                let ln = lane::lane();
                let agrees = match id {
                    WORKER_ID_DISPATCHER => {
                        on_dispatcher.fetch_add(1, Ordering::Relaxed);
                        ln == LANE_DISPATCHER
                    }
                    WORKER_ID_UNATTACHED => false,
                    w => {
                        on_worker.fetch_add(1, Ordering::Relaxed);
                        seen.fetch_or(1u64 << w, Ordering::Relaxed);
                        u32::from(ln) == w
                    }
                };
                if !agrees {
                    disagreements.fetch_add(1, Ordering::Relaxed);
                    let packed = (u64::from(id) << 16) | u64::from(ln);
                    let _ = offender.compare_exchange(
                        u64::MAX,
                        packed,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    );
                }
                observed.fetch_add(1, Ordering::Relaxed);
            });
        }
    });

    assert_eq!(
        observed.load(Ordering::Acquire),
        TASKS,
        "invariant: every spawned task ran; a gate over an empty loop asserts nothing"
    );
    assert_eq!(
        disagreements.load(Ordering::Acquire),
        0,
        "a task body saw a lane that does not match its worker id under the \
         documented map: first offender worker_id={} lane={} \
         ({} tasks ran on a worker, {} on the dispatcher)",
        offender.load(Ordering::Acquire) >> 16,
        offender.load(Ordering::Acquire) & 0xFFFF,
        on_worker.load(Ordering::Acquire),
        on_dispatcher.load(Ordering::Acquire),
    );

    // Non-vacuity: the worker leg is the one that reds when `set_lane` is
    // deleted from `worker_main`, so at least one task must have run on a
    // worker. With 4096 tasks pushed before the dispatcher enters its drain
    // loop, and `push_task` unparking an idle worker on every push, zero here
    // would be a scheduling pathology rather than a lane defect — and the
    // message says so instead of the gate passing silently.
    assert!(
        on_worker.load(Ordering::Acquire) > 0,
        "no task ran on a worker; the worker leg of this gate observed nothing"
    );

    let mask = seen.load(Ordering::Acquire);
    assert_eq!(
        mask >> WORKERS,
        0,
        "a task reported a worker id at or above the pool's worker count"
    );
}

/// DG3, the non-unwinding half: the dispatcher labelling is applied on entry
/// and taken back on the normal return, on both slots.
#[test]
fn install_labels_the_dispatcher_and_restores_on_return() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();

    assert_eq!(current_worker_id(), WORKER_ID_UNATTACHED);
    assert_eq!(lane::lane(), LANE_UNCLAIMED);

    pool.install(|_scope| {
        assert_eq!(current_worker_id(), WORKER_ID_DISPATCHER);
        assert_eq!(lane::lane(), LANE_DISPATCHER);
    });

    assert_eq!(current_worker_id(), WORKER_ID_UNATTACHED);
    assert_eq!(lane::lane(), LANE_UNCLAIMED);
}

/// DG3, the unwinding half — the leg that reds when the lane restore is moved
/// out of `InstallGuard::drop` into the normal-return path.
///
/// The inner assertion is what keeps this from passing vacuously: without it, a
/// build in which `install` never wrote `LANE_DISPATCHER` at all would also see
/// `LANE_UNCLAIMED` afterwards and call it a restore.
#[test]
fn install_restores_the_lane_after_a_panic() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();

    let labelled_inside = AtomicUsize::new(0);
    let caught = catch_unwind(AssertUnwindSafe(|| {
        pool.install(|_scope| {
            if lane::lane() == LANE_DISPATCHER {
                labelled_inside.fetch_add(1, Ordering::Relaxed);
            }
            panic!("D1 DG3: deliberate panic inside install");
        })
    }));

    assert!(caught.is_err(), "the panic must propagate out of install");
    assert_eq!(
        labelled_inside.load(Ordering::Acquire),
        1,
        "install must have applied LANE_DISPATCHER before the panic, or the \
         restore below proves nothing"
    );
    assert_eq!(
        current_worker_id(),
        WORKER_ID_UNATTACHED,
        "the pool's own worker id is restored on the unwinding path"
    );
    assert_eq!(
        lane::lane(),
        LANE_UNCLAIMED,
        "the lane must be restored on the unwinding path too; a thread left \
         labelled LANE_DISPATCHER misattributes every later diagnostic from it"
    );
}

/// DG3, the leg that reds on deriving the restored lane from `prev_worker_id`.
///
/// A thread holding a claimed spare has `WORKER_ID_UNATTACHED` and a lane in
/// `LANE_SPARE_BASE..LANE_COUNT`. Any implementation that reconstructs the lane
/// from the worker id restores `LANE_UNCLAIMED` here and loses the spare's
/// attribution for the rest of the thread's life — while `release_lane`, which
/// reads the TLS to find the slot to free, silently frees nothing and strands
/// the spare for the process.
#[test]
fn install_restores_a_claimed_spare_lane() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();

    // On its own thread: the harness thread may already hold a lane from
    // another test in this binary, and `claim_lane` is once-per-thread.
    thread::spawn(move || {
        let spare = lane::claim_lane().expect("a fresh process has all spares free");
        assert!(
            (LANE_SPARE_BASE..lane::LANE_COUNT).contains(&spare),
            "claim_lane must return a spare index"
        );

        pool.install(|_scope| {
            assert_eq!(lane::lane(), LANE_DISPATCHER);
        });

        assert_eq!(
            lane::lane(),
            spare,
            "install must restore the SAVED lane; deriving it from the worker \
             id yields LANE_UNCLAIMED and strands the spare"
        );

        lane::release_lane();
        assert_eq!(lane::lane(), LANE_UNCLAIMED);
    })
    .join()
    .expect("spare-lane fixture thread panicked");
}
