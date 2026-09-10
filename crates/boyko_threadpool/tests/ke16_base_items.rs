//! App-11's `ThreadPool::parked_mask` (`KE16-DESIGN-APP.md` §10).
//!
//! Three properties of `parked_mask` are pinned here rather than inside any one
//! instrument: a quiescent pool marks every worker, no bit outside
//! `[0, worker_count)` is ever set, and a worker inside a task body is not
//! marked.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

/// Builds a pool and waits, bounded, for every worker to be parked.
fn quiescent_pool(workers: u32) -> Arc<ThreadPool> {
    let pool = ThreadPoolBuilder::new()
        .num_threads(workers as usize)
        .build();
    let deadline = Instant::now() + Duration::from_secs(10);
    while pool.parked_mask().count_ones() < workers && Instant::now() < deadline {
        std::thread::yield_now();
    }
    pool
}

/// App-11: a quiescent pool reports every worker parked. This is the receipt
/// the occupancy instruments read as "who was asleep while this wave ran", so
/// an accessor that answered `0` for a sleeping pool would make every such
/// column silently empty.
#[test]
fn parked_mask_sets_one_bit_per_worker_when_the_pool_is_quiescent() {
    let workers = 4u32;
    let pool = quiescent_pool(workers);
    assert_eq!(
        pool.parked_mask(),
        (1u64 << workers) - 1,
        "every worker of an unloaded pool must be parked and marked"
    );
}

/// App-11: no bit above the worker count is ever set — the receipt indexes
/// workers, and a bit outside `[0, worker_count)` would name a worker that does
/// not exist.
#[test]
fn parked_mask_never_sets_a_bit_above_the_worker_count() {
    let workers = 4u32;
    let pool = quiescent_pool(workers);
    assert_eq!(
        pool.parked_mask() >> workers,
        0,
        "the idle bitset must be confined to this pool's workers"
    );
}

/// App-11: a worker inside a task body is NOT marked idle. Each of the `4 * W`
/// tasks blocks until released, so a worker that starts one never returns to
/// the poll loop; `started == W` therefore proves all `W` are inside bodies,
/// and the mask must be empty at that instant.
#[test]
fn parked_mask_is_empty_while_every_worker_runs_a_task() {
    let workers = 4u32;
    let pool = quiescent_pool(workers);
    let release = Arc::new(AtomicBool::new(false));
    let started = Arc::new(AtomicU32::new(0));

    for _ in 0..workers * 4 {
        let release = Arc::clone(&release);
        let started = Arc::clone(&started);
        pool.spawn(move || {
            started.fetch_add(1, Ordering::AcqRel);
            while !release.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
        });
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    while started.load(Ordering::Acquire) < workers && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let observed_mask = pool.parked_mask();
    let observed_started = started.load(Ordering::Acquire);
    release.store(true, Ordering::Release);

    assert_eq!(
        observed_started, workers,
        "not every worker entered a body within the deadline; the reading below would be \
         about a pool that was still partly idle"
    );
    assert_eq!(
        observed_mask, 0,
        "a worker inside a task body has already unmarked itself"
    );
}
