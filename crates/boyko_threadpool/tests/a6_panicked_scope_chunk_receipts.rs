//! Defect A6, validation row **P7** — a scope whose task panicked returns every chunk.
//!
//! # What is under test
//!
//! `Scope::drop` frees the scope's bump block (`free_all`) immediately after the join and BEFORE
//! anything that can unwind — the re-raise of a captured task panic, and since A6 the
//! already-panicking branch that discards the payload instead. That ordering is what makes the
//! block's leak-freedom structural rather than a `Drop` impl. This binary counts chunk-class
//! allocations on the three panic shapes A6 touches and asserts the net count is back to zero after
//! each one.
//!
//! # Why this is a separate binary and not a second `#[test]` in `block_allocation_receipts.rs`
//!
//! The design names that file as P7's home, because its instrument already reads the chunk class.
//! But that file states a rule that a second test would break: **exactly one `#[test]`, because the
//! counters are process-global and any intra-binary parallelism would race them** — its cell-class
//! receipts assert EXACT counts, and libtest's own allocations around a sibling test's completion
//! can land in any bucket. So P7 carries its own copy of the instrument, reduced to the two chunk
//! counters it reads, in its own process. The chunk predicate is the same one, for the same reason:
//! `block.rs`'s growth rule emits only `(CHUNK0 << e, CHUNK_ALIGN)`, and nothing else in the process
//! allocates at `(2^k >= 4096, 64)` (the positive control there, R0b, is what makes that an
//! observation).
//!
//! # The instrument is NOT installed under Miri
//!
//! Copied from `block_allocation_receipts.rs`, whose header carries the measurement: Miri
//! interprets a custom global allocator instead of replacing it, and std's Windows `System` then
//! frees an over-aligned block through a pointer Tree Borrows rejects. So the allocator is
//! `cfg(not(miri))`, the test is `cfg_attr(miri, ignore = ...)`, and [`refuse_under_miri`] turns a
//! rotted attribute into a loud panic instead of a green over frozen counters.
//!
//! # What makes this test RED
//!
//! - **M10** (move `self.block.free_all()` below the resume in `Scope::drop`): the resume path
//!   leaves the scope with its chunks still allocated, so `CHUNK_LIVE` reads positive after the
//!   single-panic, every-task-panics and nested phases.
//! - Any phase allocating NO chunk (`CHUNK_ALLOCS < 2`): the balance that follows would be green
//!   from emptiness, so it is asserted first, per phase.
//! - A phase whose unwind does not reach the owner at all (`Ok` instead of `Err`), or reaches it with
//!   a payload that is not this phase's marker.
//! - M7 (the already-panicking branch removed) ABORTS this process in the owner-and-task phase,
//!   before any assertion — a crash, not a message; that mutation's named gate is
//!   `a6_panic_propagation.rs`'s `owner_and_task_panicking_together_resume_on_owner_without_abort`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind, panic_any};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use boyko_threadpool::ThreadPoolBuilder;
use boyko_threadpool::__layout_receipt::{CHUNK_ALIGN, CHUNK0};

// =========================================================================
// The instrument: two counters, no map.
// =========================================================================

/// **CUMULATIVE.** `+= 1` on an alloc matching the chunk predicate. Reset per phase.
static CHUNK_ALLOCS: AtomicUsize = AtomicUsize::new(0);
/// **NET.** `+= 1` on a chunk-class alloc, `-= 1` on a chunk-class dealloc. Never reset, so a
/// phase's balance cannot be fooled by a chunk allocated before a reset and freed after it.
static CHUNK_LIVE: AtomicIsize = AtomicIsize::new(0);

/// The chunk-class predicate — `block_allocation_receipts.rs`'s, verbatim.
#[cfg_attr(miri, allow(dead_code))]
#[inline]
fn is_chunk(layout: Layout) -> bool {
    layout.align() == CHUNK_ALIGN && layout.size().is_power_of_two() && layout.size() >= CHUNK0
}

/// The measuring allocator. Two atomics, then `System`; nothing in here allocates.
///
/// `realloc` and `alloc_zeroed` are deliberately NOT overridden: `GlobalAlloc`'s default bodies
/// route through `self.alloc` / `self.dealloc`, so they are counted.
#[cfg_attr(miri, allow(dead_code))]
struct ChunkCountingAlloc;

// SAFETY: every method forwards its arguments UNCHANGED to `System`, which is a correct
//   `GlobalAlloc`; the counter updates are atomic writes to `static` integers and allocate nothing,
//   so no method re-enters the allocator and none of them can observe or produce a pointer that
//   `System` did not. The size/alignment contract is therefore `System`'s, unmodified.
unsafe impl GlobalAlloc for ChunkCountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if is_chunk(layout) {
            CHUNK_ALLOCS.fetch_add(1, Ordering::AcqRel);
            CHUNK_LIVE.fetch_add(1, Ordering::AcqRel);
        }
        // SAFETY: `layout` is forwarded exactly as received, so it still satisfies `alloc`'s only
        //   precondition (non-zero size) — the caller's obligation, discharged by the caller.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if is_chunk(layout) {
            CHUNK_LIVE.fetch_sub(1, Ordering::AcqRel);
        }
        // SAFETY: `ptr` and `layout` are forwarded exactly as received. The caller guarantees `ptr`
        //   came from this allocator under this `layout`, and every pointer this allocator returned
        //   came from `System` under the same `layout`, so `System`'s precondition holds.
        unsafe { System.dealloc(ptr, layout) }
    }
}

// NOT INSTALLED UNDER MIRI — see the header. The `cfg` is the load-bearing half: an `#[ignore]`
// alone would leave the allocator installed for libtest's own teardown.
#[cfg(not(miri))]
#[global_allocator]
static CHUNK_COUNTING_ALLOC: ChunkCountingAlloc = ChunkCountingAlloc;

/// Refuse to run under Miri, where the allocator above is absent and every counter reads zero —
/// so the `CHUNK_LIVE == 0` balances would be green over an empty bucket and the anti-vacuity
/// floors would red for a reason that is not a regression.
#[cfg(miri)]
fn refuse_under_miri() {
    panic!(
        "the chunk-counting allocator is `cfg(not(miri))` (see this file's header), so every \
         counter this test reads is frozen at zero. A test that needs it must carry \
         #[cfg_attr(miri, ignore = ...)]"
    );
}

/// The native arm of [`refuse_under_miri`]: nothing to refuse.
#[cfg(not(miri))]
fn refuse_under_miri() {}

// =========================================================================
// Fixtures.
// =========================================================================

/// The typed payload every deliberate panic here carries, so a caught payload is provably OURS and
/// not a `debug_assert!` inside the pool.
#[derive(Debug, PartialEq, Eq)]
struct P7Marker(usize);

/// Tasks per scope in the single-panic and nested phases. At a stride of at least 88 bytes the
/// first chunk (4096 B) holds fewer than 47 cells, so 200 cells need at least two chunks — and the
/// per-phase `CHUNK_ALLOCS >= 2` floor makes that an observation rather than this comment.
const WIDE: usize = 200;

/// Tasks per scope in the every-task-panics and owner-panics phases: still past one chunk.
const NARROW: usize = 64;

/// The index that panics in the one-panic phases — past the first chunk's cells, so the panicking
/// task's own cell is not in chunk 0.
const PANIC_AT: usize = 137;

/// One scoped body type for every phase: 72 bytes of ballast, read so RFC 2229 captures it by
/// value, plus the index. `panic_at == usize::MAX` means "never"; `panic_all` means "always".
fn body(i: usize, panic_at: usize, panic_all: bool) -> impl FnOnce() + Send + 'static {
    let ballast = [0u8; 72];
    move || {
        std::hint::black_box(&ballast);
        if panic_all || i == panic_at {
            panic_any(P7Marker(i));
        }
    }
}

/// The phase verdict every panic phase shares. Snapshot-then-assert, so no `println!` or formatting
/// runs between the scope's return and the read.
fn assert_phase(
    phase: &str,
    caught: Result<(), Box<dyn Any + Send>>,
    accept: impl Fn(usize) -> bool,
    chunks: usize,
    live: isize,
) {
    println!("[P7 {phase}] CHUNK_ALLOCS (cumulative) = {chunks}  CHUNK_LIVE (net) = {live}");
    let payload = match caught {
        Ok(()) => panic!(
            "P7 {phase}: the scope RETURNED normally, so the panic never unwound out of it and this \
             phase measured nothing about the unwinding path"
        ),
        Err(payload) => payload,
    };
    match payload.downcast_ref::<P7Marker>() {
        Some(P7Marker(i)) => assert!(
            accept(*i),
            "P7 {phase}: the owner received P7Marker({i}), which is not this phase's panic"
        ),
        None => panic!(
            "P7 {phase}: the owner received a panic that is not a P7Marker -- something other than \
             the deliberate body panicked"
        ),
    }
    assert!(
        chunks >= 2,
        "P7 {phase}: the scope allocated {chunks} chunk-class blocks; at least 2 were required, or \
         the CHUNK_LIVE balance below is green from an empty (or single-chunk) bucket"
    );
    assert_eq!(
        live, 0,
        "P7 {phase}: CHUNK_LIVE (NET) must be back to 0 once the panic has left the scope. \
         `Scope::drop` frees the block immediately after the join and BEFORE the re-raise (or the \
         already-panicking discard); a positive reading means the block was not freed on this \
         unwinding path (M10: `free_all` moved below the resume)"
    );
}

/// Spins until `flag` is set, bounded, so a task that never started is a failure with a message.
fn wait_for(flag: &AtomicBool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !flag.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline, "P7 fixture: the task that sets the flag never ran");
        std::thread::yield_now();
    }
}

// =========================================================================
// The one test.
// =========================================================================

/// P7 — every chunk a scope allocated is back once a task's panic has left it, on each unwinding
/// shape A6 touches: a re-raised panic, a scope where every task panicked (so the losers go through
/// `discard_payload`), an owner and a task panicking together (the already-panicking branch), and a
/// nested scope joined by a worker.
#[test]
#[cfg_attr(
    miri,
    ignore = "instrument: reads a `#[global_allocator]` that is `cfg(not(miri))` because Miri interprets a custom global allocator and std's Windows `System` then frees an over-aligned block through a pointer Tree Borrows rejects (`block_allocation_receipts.rs`'s header carries the measurement); under Miri every counter stays 0 and `refuse_under_miri()` panics with that cause instead. Runs natively."
)]
fn a_scope_whose_task_panicked_returns_every_chunk() {
    refuse_under_miri();

    // Silence ONLY the deliberate `P7Marker` panics (up to 64 of them per phase); anything else —
    // an assertion inside the pool, a double fault — still reaches the default hook.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if info.payload().downcast_ref::<P7Marker>().is_none() {
            default_hook(info);
        }
    }));

    let pool = ThreadPoolBuilder::new().num_threads(4).build();

    // Warm the pool: worker threads exist and crossbeam's buffers have grown before any phase.
    pool.install(|s| {
        for i in 0..WIDE {
            s.spawn(body(i, usize::MAX, false));
        }
    });
    assert_eq!(
        CHUNK_LIVE.load(Ordering::Acquire),
        0,
        "P7 warm-up: a scope that did NOT panic must return its chunks too, or every phase below \
         starts from a non-zero balance"
    );

    // ── Phase 1: one task of 200 panics; the owner re-raises it. ─────────────────────────────
    CHUNK_ALLOCS.store(0, Ordering::Release);
    let caught = catch_unwind(AssertUnwindSafe(|| {
        pool.install(|s| {
            for i in 0..WIDE {
                s.spawn(body(i, PANIC_AT, false));
            }
        });
    }));
    let chunks = CHUNK_ALLOCS.load(Ordering::Acquire);
    let live = CHUNK_LIVE.load(Ordering::Acquire);
    assert_phase("one-task-panics", caught, |i| i == PANIC_AT, chunks, live);

    // ── Phase 2: every task panics; one payload is re-raised, the rest are discarded. ─────────
    CHUNK_ALLOCS.store(0, Ordering::Release);
    let caught = catch_unwind(AssertUnwindSafe(|| {
        pool.install(|s| {
            for i in 0..NARROW {
                s.spawn(body(i, usize::MAX, true));
            }
        });
    }));
    let chunks = CHUNK_ALLOCS.load(Ordering::Acquire);
    let live = CHUNK_LIVE.load(Ordering::Acquire);
    assert_phase("every-task-panics", caught, |i| i < NARROW, chunks, live);

    // ── Phase 3: the owner panics while a task's payload is pending (already-panicking branch).
    CHUNK_ALLOCS.store(0, Ordering::Release);
    let task_panicked = Arc::new(AtomicBool::new(false));
    let caught = catch_unwind(AssertUnwindSafe(|| {
        pool.install(|s| {
            for i in 0..NARROW {
                let flag = Arc::clone(&task_panicked);
                let inner = body(i, 5, false);
                s.spawn(move || {
                    if i == 5 {
                        flag.store(true, Ordering::Release);
                    }
                    inner();
                });
            }
            wait_for(&task_panicked);
            // The OWNER's panic: its marker is outside the tasks' index range, so the verdict can
            // tell which payload reached the caller.
            panic_any(P7Marker(usize::MAX));
        });
    }));
    let chunks = CHUNK_ALLOCS.load(Ordering::Acquire);
    let live = CHUNK_LIVE.load(Ordering::Acquire);
    assert_phase("owner-and-task-panic", caught, |i| i == usize::MAX, chunks, live);

    // ── Phase 4: a nested scope on a worker; its inner task panics; the worker joiner re-raises
    //    into the outer task, and the outer scope re-raises on the owner.
    CHUNK_ALLOCS.store(0, Ordering::Release);
    let inner_pool = Arc::clone(&pool);
    let caught = catch_unwind(AssertUnwindSafe(|| {
        pool.install(|outer| {
            outer.spawn(move || {
                inner_pool.scope(|inner| {
                    for i in 0..WIDE {
                        inner.spawn(body(i, PANIC_AT, false));
                    }
                });
            });
        });
    }));
    let chunks = CHUNK_ALLOCS.load(Ordering::Acquire);
    let live = CHUNK_LIVE.load(Ordering::Acquire);
    assert_phase("nested-scope-panics", caught, |i| i == PANIC_AT, chunks, live);

    // ── Final balance, with the pool gone. ────────────────────────────────────────────────────
    drop(pool);
    let _ = std::panic::take_hook();
    let final_live = CHUNK_LIVE.load(Ordering::Acquire);
    println!("[P7 final] CHUNK_LIVE (net, whole process, never reset) = {final_live}");
    assert_eq!(
        final_live, 0,
        "P7: every chunk-class allocation this process made must have been freed by the end"
    );
}
