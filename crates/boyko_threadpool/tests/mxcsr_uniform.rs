//! G-FP (physics lever L5) — every worker of a pool runs under one MXCSR, the one a freshly
//! spawned thread starts with, and that one is the IEEE default Rust's floating-point model
//! assumes.
//!
//! # Why this is a gate
//!
//! The parallel narrowphase (`boyko_physics/src/narrowphase/dispatch.rs`) collides a step's pairs
//! on whichever workers steal its chunks, and claims the manifolds are bit-identical to the serial
//! loop's for any worker count and any steal order. Same binary, IEEE operations only — so the
//! claim rests on one more thing: every thread that runs a chunk computes under the same SSE
//! control state. MXCSR's rounding control (bits 13–14), flush-to-zero (bit 15) and
//! denormals-are-zero (bit 6) change results, and they are per thread. No source in the tree writes
//! them (a grep for `setcsr|ldmxcsr|FLUSH_ZERO|DENORMALS_ZERO` over `crates`, `src` and `tests`
//! finds only this comment), so every worker starts with the state its OS gives a new thread.
//! This test pins that: a worker that ran under another state — a pool that set FTZ in
//! `worker_main`, say — turns it red.
//!
//! It compares the workers with each other and with a thread spawned by `std::thread::spawn`, and
//! requires that state's three result-changing fields to be the IEEE default (round to nearest,
//! no flush, no denormals-as-zero). It deliberately does not mutate the spawner's MXCSR to show a
//! red: on Linux a thread inherits its creator's MXCSR, so such a mutation reds only on Windows,
//! and setting MXCSR is outside Rust's floating-point model.
//!
//! x86_64 only (MXCSR is an SSE register), and not under Miri, which runs no inline assembly and
//! no pool.

#![cfg(all(target_arch = "x86_64", not(miri)))]

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use boyko_threadpool::{ThreadPoolBuilder, current_worker_id};

/// Workers in the pool under test.
const WORKERS: usize = 4;

/// Tasks spawned per round; each spins long enough that the idle workers steal some.
const TASKS_PER_ROUND: usize = 16 * WORKERS;

/// Each task's spin, so the round's tasks spread over the workers.
const TASK_SPIN: Duration = Duration::from_micros(200);

/// Rounds before a worker that never ran a task is reported.
const MAX_ROUNDS: usize = 200;

/// MXCSR's result-changing fields: flush-to-zero (bit 15), rounding control (bits 13–14) and
/// denormals-are-zero (bit 6). All clear is IEEE round-to-nearest with gradual underflow.
const RESULT_FIELDS: u32 = (1 << 15) | (0b11 << 13) | (1 << 6);

/// A slot no MXCSR value can hold (MXCSR is 32 bits), meaning "this worker has not reported".
const UNSEEN: u64 = u64::MAX;

/// The calling thread's MXCSR.
fn mxcsr() -> u32 {
    let mut csr: u32 = 0;
    // SAFETY: `stmxcsr` stores the 32-bit MXCSR to the address in the register it is given, here
    //   a live, 4-byte-aligned local `u32`, and touches no other memory, no stack and no flag.
    //   It needs only SSE, which every x86_64 processor has.
    unsafe {
        core::arch::asm!(
            "stmxcsr [{p}]",
            p = in(reg) &raw mut csr,
            options(nostack, preserves_flags),
        );
    }
    csr
}

#[test]
fn every_worker_runs_under_the_default_mxcsr() {
    let fresh = std::thread::spawn(mxcsr).join().expect("the probe thread returns");
    assert_eq!(
        fresh & RESULT_FIELDS,
        0,
        "a new thread's MXCSR {fresh:#06x} is not IEEE round-to-nearest with gradual underflow"
    );
    assert_eq!(mxcsr(), fresh, "the test thread's MXCSR differs from a new thread's");

    let pool = ThreadPoolBuilder::new().num_threads(WORKERS).build();
    let seen: [AtomicU64; WORKERS] = [const { AtomicU64::new(UNSEEN) }; WORKERS];
    let mut rounds = 0;
    while seen.iter().any(|s| s.load(Ordering::Relaxed) == UNSEEN) {
        assert!(
            rounds < MAX_ROUNDS,
            "after {MAX_ROUNDS} rounds of {TASKS_PER_ROUND} tasks some worker never ran one: {:?}",
            seen.iter().map(|s| s.load(Ordering::Relaxed)).collect::<Vec<_>>()
        );
        rounds += 1;
        pool.install(|scope| {
            for _ in 0..TASKS_PER_ROUND {
                scope.spawn(|| {
                    let id = current_worker_id() as usize;
                    // The dispatcher also runs tasks while it joins; it is the test thread,
                    // checked above. Only pool workers report here.
                    if id < WORKERS {
                        // Relaxed: the reader is this test after `install` returns, whose join
                        // orders every task's effects before it.
                        seen[id].store(u64::from(mxcsr()), Ordering::Relaxed);
                    }
                    let start = Instant::now();
                    while start.elapsed() < TASK_SPIN {
                        std::hint::spin_loop();
                    }
                });
            }
        });
    }
    for (worker, slot) in seen.iter().enumerate() {
        let csr = slot.load(Ordering::Relaxed);
        assert_eq!(
            csr,
            u64::from(fresh),
            "worker {worker} runs under MXCSR {csr:#06x}, a new thread under {fresh:#06x}: chunks \
             stolen by different workers would round differently"
        );
    }
}
