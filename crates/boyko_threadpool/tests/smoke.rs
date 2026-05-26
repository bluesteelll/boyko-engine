//! Wave 1 smoke tests for `boyko_threadpool`.
//!
//! These exercise the bare minimum surface so the foundation is known-good
//! before Waves 2-6 layer on top: pool construction, drop, empty install,
//! single task, many tasks. Per the Phase 9 plan §14 Step 4 acceptance
//! criteria (install/scope smoke + panic propagation; nested scope already
//! covered by `scope::tests`).

use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_threadpool::ThreadPoolBuilder;

#[test]
fn pool_builds_and_drops() {
    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    assert_eq!(pool.num_threads(), 4);
    drop(pool);
}

#[test]
fn pool_install_empty_scope() {
    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    pool.install(|_scope| {});
}

#[test]
fn pool_install_one_task_runs() {
    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    let counter = AtomicUsize::new(0);
    pool.install(|scope| {
        scope.spawn(|| {
            counter.fetch_add(1, Ordering::Relaxed);
        });
    });
    assert_eq!(counter.load(Ordering::Acquire), 1);
}

#[test]
fn pool_install_many_tasks_runs_all() {
    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    let counter = AtomicUsize::new(0);
    pool.install(|scope| {
        for _ in 0..1000 {
            scope.spawn(|| {
                counter.fetch_add(1, Ordering::Relaxed);
            });
        }
    });
    assert_eq!(counter.load(Ordering::Acquire), 1000);
}

#[test]
fn pool_install_returns_value() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let v: u64 = pool.install(|_scope| 0xDEAD_BEEF);
    assert_eq!(v, 0xDEAD_BEEF);
}
