//! Stage 3b's allocation receipts — R0, R0b, R1, R2 and R4' of
//! `taskrep_plan5.md` §3, in ONE binary with ONE `#[test]`.
//!
//! ## Why this file exists
//!
//! Stage 3b removes the per-task `alloc`/`dealloc` pair from the scoped-task
//! path by giving each `Scope` a bump allocator (`src/block.rs`, landed in 3a,
//! wired in 3b). Everything in here exists so that "the allocation is gone" is
//! an **observation** rather than a claim, and so that the observation cannot
//! come back green from an empty bucket.
//!
//! ## The rule this file obeys, without exception
//!
//! A receipt names a size class **this design produces**, or it does not exist.
//! Every scoped push in the default build lands in a crossbeam `Injector`,
//! which allocates and frees `Block`s cyclically on the push/steal path, so
//! anything phrased as a process `total_allocs`, an "exactly 2", or a global
//! alloc/dealloc balance would be an assertion about crossbeam-deque's
//! internals — permanently red or flaky. The two classes this design owns:
//!
//! | class | predicate | produced by |
//! |---|---|---|
//! | **cell** | exact `(size, align)` of the cell under test | `task/detached.rs`'s `alloc_cell` |
//! | **chunk** | `align == 64 && size.is_power_of_two() && size >= 4096` | `block.rs`'s growth site, and nothing else |
//!
//! The chunk predicate is exact **by construction**: `block.rs`'s growth rule
//! emits only `(CHUNK0 << e, 64)` and `debug_assert`s that at the producer.
//! Nothing else in the process is expected to hit it — crossbeam's
//! `Block`/`Buffer` are align 8, `CachePadded` is align 128 on x86_64, and the
//! `ScopeShared` box contains a `CachePadded<AtomicUsize>` so it is align 128
//! too. **That is an argument, not a guarantee**, which is why it is backed by
//! a positive control ([R0b]) whose failure direction is RED.
//!
//! ## The instrument
//!
//! A `#[global_allocator]` that delegates to `System` and keeps **four
//! counters and no map**: a map inside `GlobalAlloc` re-enters the allocator it
//! is measuring. `CELL_ALLOCS` and `CHUNK_ALLOCS` are **cumulative**,
//! `CHUNK_LIVE` is **net**, `OTHER` is recorded and printed and never
//! asserted. Every assertion site names the counter it reads, so no assert is
//! ambiguous about which of cumulative/net it is stating.
//!
//! Exactly **one `#[test]`**, because the counters are process-global and any
//! intra-binary parallelism would race them. There is deliberately **no
//! file-scope `#![cfg(...)]`**: that is this repository's catalogued way to
//! produce `running 0 tests` and exit 0, which is a vacuous pass rather than a
//! pass.
//!
//! Layout facts come from [`boyko_threadpool::__layout_receipt`], whose
//! constants are independent literals pinned to the real types by
//! `const _: () = assert!(..)` next to those types. `ScopedCell` /
//! `DetachedCell` / `ScopeBlock` are crate-private and an integration binary
//! cannot name them, so the strides here are **derived** from those constants
//! plus a measured `size_of_val` of the body — and the derivation's
//! preconditions are asserted rather than assumed.
//!
//! ## The instrument is NOT INSTALLED UNDER MIRI, and that is a measurement
//!
//! The `#[global_allocator]` below is `#[cfg(not(miri))]`, which copies
//! `src/block.rs`'s device (stage 3a) for the reason 3a measured. Miri
//! *interprets* a custom global allocator instead of replacing it, so the
//! delegate becomes interpreted code — and std's Windows `System` frees an
//! OVER-ALIGNED block through a pointer it walks backwards out of the caller's
//! provenance (`HeapFree(heap, 0, block)`, `std/src/sys/alloc/windows.rs:199`),
//! which Tree Borrows rejects. 3a's measurement, on
//! `nightly-x86_64-pc-windows-gnu` under the repo's pinned
//! `-Zmiri-tree-borrows`: without its recording module `8 passed; 0 failed`;
//! with the allocator installed unconditionally, UB reported — **and the report
//! survives deleting every `record()` call**, so the cause is the delegation
//! and not the tape. This binary makes over-aligned allocations of its own —
//! every `ScopeShared` box contains a `CachePadded<AtomicUsize>`, align 128 on
//! x86_64, which is the same fact the chunk-predicate argument above rests on —
//! so the report would land during harness teardown with `ScopeBlock` nowhere
//! on the stack.
//!
//! This binary needs the guard MORE than `block.rs` does, because it is in the
//! deciding gate's scope: `.github/workflows/ci.yml:258-261` runs
//! `cargo +nightly miri test --all-targets … -p boyko-threadpool` as a required
//! job. After stage 3b the scoped cell outlives the release RMW, so what
//! carries the soundness argument's last clause is an execution gate over
//! `run_scoped` under exactly that Miri run — and a red raised at harness
//! teardown by a std-internal issue is indistinguishable, from the
//! exit-condition receipts, from a red raised by `run_scoped`. Installing this
//! allocator unconditionally is therefore not caution; it destroys the one
//! signal the campaign is deciding on.
//!
//! The guard is a THREE-part device, and each part closes a hole the others
//! leave open:
//!
//! 1. `#[cfg(not(miri))]` on the `#[global_allocator]` static. This is the
//!    load-bearing part: `#[ignore]` alone would not help, because the
//!    allocator would still be **installed** and libtest would still allocate
//!    through it at teardown.
//! 2. `#[cfg_attr(miri, ignore = "…")]` on the one `#[test]`. The reason string
//!    is not optional — `tests/ignore_reasons_census.rs` fails the build on a
//!    bare `#[ignore]`, which is this repository's third way to make a check
//!    disappear after `unsafe` and `#[allow(clippy::disallowed_types)]`, and
//!    the same rule governs all three: write down why, at the site.
//! 3. [`refuse_under_miri`] at the top of the test body. Part 1 alone leaves a
//!    test that RUNS under Miri with every counter frozen at zero, and what
//!    that costs was MEASURED rather than argued — a `--cfg miri` build of this
//!    binary with the guard removed, forced with `--ignored`, does not report a
//!    green: it reds at **R0**, `CELL_ALLOCS = 0`, accusing `alloc_cell` of
//!    having left the detached path. So the state part 3 prevents is a
//!    **misattributed red**, raised inside the one gate the campaign is
//!    deciding on and naming a regression that did not happen. The `== 0`
//!    receipts under R0 (R1a, R1b, `n == 0`, R4') *would* have been green over
//!    an empty bucket; they are simply never reached — which makes the
//!    non-vacuity a property of **phase order**, not of the file, and leaves it
//!    one reorder, one `--exact` filter or one deleted R0 away from being lost.
//!    The guard is what does not depend on that ordering: reaching the body
//!    under Miri is a loud panic naming the real cause.
//!
//! None of this is the file-scope `#![cfg(…)]` trap named above. That one
//! deletes the test from the binary and reports `running 0 tests` with exit 0;
//! this one leaves the test compiled and unconditional, and Miri lists it as
//! `ignored` with the reason attached.
//!
//! ## What makes each receipt RED
//!
//! Stated once here and again at each site, because a test whose red is not
//! nameable is a claim rather than a gate.
//!
//! - **R0** (cell class, `== N`): red low if `alloc_cell` leaves the detached
//!   spawn path; red high if anything else in the process allocates at the
//!   cell class's exact `(size, align)` during the wave; red at 0 if the
//!   derived cell class is wrong — which is precisely the state in which R1
//!   would be *vacuously* green, and the reason R0 runs first.
//! - **R0b** (chunk class, `== 1`): red at 0 if the scope's block never
//!   allocates (the wiring is absent, or `emplace` was bypassed); red at 2+ if
//!   the growth rule stopped fitting the wave in chunk 0, or if something else
//!   in the process allocates at `(2^k >= 4096, 64)`.
//! - **R1** (cell class, `== 0`): red at exactly n if `alloc_cell` is back on
//!   the scoped path — a revert, a "just this once" fallback, a `MAX_CHUNKS`
//!   fallback; red at `0 < k < n` for a partial bypass.
//! - **R2** (chunk class, `== predicted_chunks(n)`): red at 0 if the block is
//!   bypassed; red off-by-k if `block.rs`'s growth rule drifts from the
//!   re-derivation below. The re-derivation itself is pinned against hand
//!   arithmetic by `const _: () = assert!(..)`, so a drift *there* is a build
//!   failure and not a moving expectation.
//! - **R4'** (chunk class, net `== 0`): red if any chunk-class allocation is
//!   outstanding at test end — `free_all` skipped on some path, or a chunk
//!   recorded in `bases` but not counted in `n_chunks`. Its non-vacuity is
//!   carried by R0b and R2: if no chunk was ever allocated they are red, so a
//!   green R4' is never green from emptiness.
//!
//! The cell class is deliberately **not** in R4'. R1 already asserts zero
//! there, and the detached cells R0 creates are freed asynchronously in
//! `run_detached`, so asserting their net would re-import a join that does not
//! exist: `ThreadPool::spawn` returns `()` and registers nothing, so it has no
//! completion accounting (see `tests/ke16_w_wake_completion.rs`'s
//! `w_b_a_fire_and_forget_wave_into_a_parked_pool_runs_every_task_exactly_once`,
//! whose drain shape this file reuses).
//!
//! [R0b]: #

use std::alloc::{GlobalAlloc, Layout, System};
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use boyko_threadpool::ThreadPoolBuilder;
use boyko_threadpool::__layout_receipt::{
    CHUNK0, CHUNK_ALIGN, DETACHED_CELL_ALIGN, DETACHED_CELL_HEADER, MAX_CHUNKS,
    SCOPED_CELL_ALIGN, SCOPED_CELL_HEADER,
};

// =========================================================================
// Layout facts, re-pinned locally.
//
// The constants above are read from the crate so that a drift between the
// crate and this file is impossible. The literals below then pin the crate's
// values to the numbers the committed expectations were computed BY HAND from
// (see `R2_ROWS`): without them a change to `CHUNK0` would silently move every
// prediction in this file and the receipts would keep agreeing with themselves.
// Drift is a BUILD failure, not a re-measured number.
// =========================================================================
const _: () = assert!(CHUNK0 == 4096);
const _: () = assert!(CHUNK_ALIGN == 64);
const _: () = assert!(MAX_CHUNKS == 32);
const _: () = assert!(SCOPED_CELL_HEADER == 16);
const _: () = assert!(SCOPED_CELL_ALIGN == 8);
const _: () = assert!(DETACHED_CELL_HEADER == 8);
const _: () = assert!(DETACHED_CELL_ALIGN == 8);

// =========================================================================
// The two body types, chosen so that ONE `(size, align)` pair serves both the
// positive control and the binding receipt.
//
// `ScopedCell<F>` and `DetachedCell<G>` must land in the SAME bucket, because
// R0 arms that bucket through the detached path (which 3b does not touch) and
// R1 then asserts zero in it through the scoped path (which 3b does touch).
// Two different buckets would leave R1 asserting over a class nothing produces
// any more — green from emptiness, which is the failure mode R0 exists to
// forbid.
//
//   scoped body  F: 72 bytes, align 1  =>  ScopedCell<F>   = 16 + 72 = 88, align 8
//   detached body G: 80 bytes, align 8  =>  DetachedCell<G> =  8 + 80 = 88, align 8
//
// `G` is 8 bytes wider than `F` because its header is 8 bytes narrower: a
// fire-and-forget cell has no `ScopeShared` address to carry.
//
// Both bodies are CLOSURES, whose layout the language does not guarantee, so
// the test MEASURES both with `size_of_val` / `align_of_val` and asserts the
// measurement before it derives anything from it.
// =========================================================================

/// `size_of::<F>()` for the scoped body — the ballast is the whole capture.
const SCOPED_BODY_SIZE: usize = 72;
/// `align_of::<F>()` for the scoped body: a `[u8; N]` capture and nothing else.
const SCOPED_BODY_ALIGN: usize = 1;
/// The detached body's ballast. Plus its `Arc` (8 B) that is `SCOPED_BODY_SIZE + 8`.
const DETACHED_BODY_BALLAST: usize = 72;
/// `size_of::<G>()` for the detached body: ballast plus one `Arc` pointer.
const DETACHED_BODY_SIZE: usize = DETACHED_BODY_BALLAST + 8;
/// `align_of::<G>()`, set by the `Arc` pointer.
const DETACHED_BODY_ALIGN: usize = 8;

/// The cell class both paths must land in: `(88, 8)`.
///
/// Derived, not measured, because an integration binary cannot name
/// `ScopedCell`. The derivation `size == header + body` is exact under two
/// conditions, both `const _`-pinned below: `align_of::<F>() <=
/// SCOPED_CELL_ALIGN` (so the cell's alignment is the header's, not the body's,
/// and the body needs no leading pad — the header is already a multiple of 8)
/// and `size_of::<F>() % SCOPED_CELL_ALIGN == 0` (so the cell needs no tail
/// pad). The one term neither a constant nor pinnable — the closure's actual
/// layout — is MEASURED by the test's probes before anything reads this.
const CELL_SIZE_EXPECTED: usize = SCOPED_CELL_HEADER + SCOPED_BODY_SIZE;
/// The alignment half of the cell bucket. A bucket is the PAIR; pinning size
/// alone leaves the other half free to move, and a class that moved would
/// leave R1 counting over an empty bucket.
const CELL_ALIGN_EXPECTED: usize = SCOPED_CELL_ALIGN;

const _: () = assert!(CELL_SIZE_EXPECTED == 88);
const _: () = assert!(DETACHED_CELL_HEADER + DETACHED_BODY_SIZE == CELL_SIZE_EXPECTED);
const _: () = assert!(DETACHED_CELL_ALIGN == CELL_ALIGN_EXPECTED);

/// The block's per-cell stride for the scoped body: `size_of::<ScopedCell<F>>()`.
///
/// Cells never need padding BETWEEN them — `bases[i]` is `CHUNK_ALIGN`-aligned
/// and `size_of::<T>()` is a multiple of `align_of::<T>()` for every Rust type
/// — so the stride is the cell size and nothing more.
const STRIDE: usize = CELL_SIZE_EXPECTED;

/// `required(T) = size_of::<T>() + align_of::<T>().saturating_sub(CHUNK_ALIGN)`.
///
/// The `saturating_sub` term is the leading pad the first cell of a chunk can
/// need, and it is ZERO here (and for AVX2's 32 and AVX-512's 64), which is why
/// this engine's ISA baseline costs nothing in the block.
const REQUIRED: usize = STRIDE + CELL_ALIGN_EXPECTED.saturating_sub(CHUNK_ALIGN);

const _: () = assert!(STRIDE == 88);
const _: () = assert!(REQUIRED == 88);

// The two cell-size derivations' remaining preconditions: NO TAIL PAD on
// either cell. `header % align == 0` and `body % align == 0` together give
// `size_of::<Cell<Body>>() == header + body`, which is what lets an
// integration binary that cannot NAME `ScopedCell` still state its bucket.
// Compile-time, because every term is a constant — a runtime check over
// constants is a check that already had its answer.
const _: () = assert!(
    SCOPED_CELL_HEADER.is_multiple_of(SCOPED_CELL_ALIGN),
    "ScopedCell's header must be a multiple of its alignment for `size = header + body`"
);
const _: () = assert!(
    SCOPED_BODY_SIZE.is_multiple_of(SCOPED_CELL_ALIGN),
    "the scoped body's size must be a multiple of the cell alignment, or the cell carries a tail \
     pad the derived size does not account for"
);
const _: () = assert!(
    DETACHED_CELL_HEADER.is_multiple_of(DETACHED_CELL_ALIGN),
    "DetachedCell's header must be a multiple of its alignment for `size = header + body`"
);
const _: () = assert!(
    DETACHED_BODY_SIZE.is_multiple_of(DETACHED_CELL_ALIGN),
    "the detached body's size must be a multiple of the cell alignment"
);
// The BODY alignments are the other precondition — the derivation assumes the
// cell's alignment is its HEADER's, which holds only while the body's own
// alignment does not exceed it. The bodies are closures, so these two
// constants are CLAIMS about them and the test MEASURES both with
// `align_of_val` before it derives anything.
const _: () = assert!(SCOPED_BODY_ALIGN <= SCOPED_CELL_ALIGN);
const _: () = assert!(DETACHED_BODY_ALIGN <= DETACHED_CELL_ALIGN);

// =========================================================================
// Wave sizes.
// =========================================================================

/// R0's detached wave. Small: the receipt is an exact count, not a load test.
const R0_TASKS: usize = 64;

/// R0b's scoped wave — sized so the WHOLE payload provably fits chunk 0.
const R0B_TASKS: usize = 32;

// R0b's premise, at build time: `n * stride` plus a worst-case leading pad of
// `CHUNK_ALIGN` must fit `CHUNK0`, or `CHUNK_ALLOCS == 1` is not the right
// prediction and the positive control would be asserting the wrong number.
const _: () = assert!(
    R0B_TASKS * STRIDE + CHUNK_ALIGN <= CHUNK0,
    "R0b's wave must fit chunk 0, or its expected chunk count is not 1"
);

/// R1's scoped wave, through both spawn entry points.
const R1_TASKS: usize = 256;

// =========================================================================
// The instrument: four counters, no map.
// =========================================================================

/// Cell-class discriminant, size half. Set by the test per phase.
///
/// Initialised to `usize::MAX` rather than 0 so that an UNSET class matches
/// nothing: a `Layout` of size 0 never reaches `GlobalAlloc::alloc` (it is a
/// precondition violation), but 0 as a sentinel would still be one edit away
/// from matching, and `usize::MAX` cannot be a live allocation size.
static CELL_SIZE: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Cell-class discriminant, alignment half. Set by the test per phase.
static CELL_ALIGN: AtomicUsize = AtomicUsize::new(0);

/// **CUMULATIVE.** `+= 1` on an alloc matching `(CELL_SIZE, CELL_ALIGN)`.
/// Never decremented — a cell dealloc is not counted, deliberately, because
/// the detached cells R0 creates are freed asynchronously and R4' therefore
/// does not state a cell balance.
static CELL_ALLOCS: AtomicUsize = AtomicUsize::new(0);
/// **CUMULATIVE.** `+= 1` on an alloc matching the chunk predicate.
static CHUNK_ALLOCS: AtomicUsize = AtomicUsize::new(0);
/// **NET.** `+= 1` on a chunk-class alloc, `-= 1` on a chunk-class dealloc.
///
/// `AtomicIsize` rather than `AtomicUsize` — the one place this file departs
/// from §3's letter, and only in the type: a net counter that went negative
/// would wrap to ~2^64 in a `usize` and the failure message would name a
/// number nobody can read. Both spellings are RED in the same cases; this one
/// prints why. It is deliberately NEVER reset, so R4' is a whole-process
/// balance and cannot be fooled by a chunk allocated before a reset and freed
/// after it.
static CHUNK_LIVE: AtomicIsize = AtomicIsize::new(0);
/// Everything else — recorded and PRINTED, never asserted. libtest's own
/// machinery, `Arc`s, crossbeam's blocks and buffers, the formatter, the
/// panic hook, the OS thread stacks all land here.
static OTHER: AtomicUsize = AtomicUsize::new(0);

/// The chunk-class predicate, exact and by construction (`block.rs`'s growth
/// rule emits only `(CHUNK0 << e, CHUNK_ALIGN)`).
///
/// Its only caller is [`ReceiptAlloc`]'s `GlobalAlloc` impl, which is unreached
/// under Miri — hence the same scoped allow the struct carries.
#[cfg_attr(miri, allow(dead_code))]
#[inline]
fn is_chunk(layout: Layout) -> bool {
    layout.align() == CHUNK_ALIGN
        && layout.size().is_power_of_two()
        && layout.size() >= CHUNK0
}

/// The cell-class predicate: an EXACT `(size, align)` match, never a range.
/// A range would let crossbeam's `Block` (1520 B, align 8) or a `Buffer<Task>`
/// (a power-of-two multiple of 16 B, align 8) into the bucket.
///
/// Unreached under Miri for the same reason as [`is_chunk`].
#[cfg_attr(miri, allow(dead_code))]
#[inline]
fn is_cell(layout: Layout) -> bool {
    layout.size() == CELL_SIZE.load(Ordering::Relaxed)
        && layout.align() == CELL_ALIGN.load(Ordering::Relaxed)
}

/// The measuring allocator.
///
/// No map, no `Vec`, no lock, no formatting: the body touches four atomics and
/// then `System`. Anything that allocated in here would re-enter the allocator
/// it is measuring.
///
/// `realloc` and `alloc_zeroed` are deliberately NOT overridden. `GlobalAlloc`'s
/// default bodies route through `self.alloc` / `self.dealloc`, so they are
/// counted; an override that forwarded straight to `System` would bypass the
/// counters and open a hole in exactly the direction that reads green.
///
/// The allow is scoped to `miri` and to this item, not re-taken over the file:
/// under Miri the `#[global_allocator]` below is `cfg`'d out (see the header),
/// so nothing constructs this and a trait impl alone does not count as a use.
/// A file-wide allow would also hide the next item that genuinely stopped being
/// reachable, and a native `dead_code` here would still be a real finding.
#[cfg_attr(miri, allow(dead_code))]
struct ReceiptAlloc;

// SAFETY: every method forwards its arguments UNCHANGED to `System`, which is
//   a correct `GlobalAlloc`; the counter updates before each forward are
//   atomic reads/writes of `static` integers and allocate nothing, so no
//   method re-enters the allocator and none of them can observe or produce a
//   pointer that `System` did not. The block-size/alignment contract is
//   therefore `System`'s, unmodified.
unsafe impl GlobalAlloc for ReceiptAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // Chunk first: the two predicates are disjoint (88 is not a power of
        // two >= 4096), so the order is not load-bearing for correctness — it
        // is load-bearing for readability of the `else` arm.
        if is_chunk(layout) {
            CHUNK_ALLOCS.fetch_add(1, Ordering::AcqRel);
            CHUNK_LIVE.fetch_add(1, Ordering::AcqRel);
        } else if is_cell(layout) {
            CELL_ALLOCS.fetch_add(1, Ordering::AcqRel);
        } else {
            OTHER.fetch_add(1, Ordering::AcqRel);
        }
        // SAFETY: `layout` is forwarded exactly as received, so it still
        //   satisfies `alloc`'s only precondition (non-zero size) — the
        //   caller's obligation, discharged by the caller.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if is_chunk(layout) {
            CHUNK_LIVE.fetch_sub(1, Ordering::AcqRel);
        }
        // SAFETY: `ptr` and `layout` are forwarded exactly as received. The
        //   caller guarantees `ptr` came from this allocator under this
        //   `layout`, and every pointer this allocator ever returned came from
        //   `System` under the same `layout`, so `System`'s precondition holds.
        unsafe { System.dealloc(ptr, layout) }
    }
}

// NOT INSTALLED UNDER MIRI — the header's third section states the measurement
// and why this binary needs it more than `src/block.rs` does. The `cfg` is the
// load-bearing half: an `#[ignore]` alone leaves the allocator INSTALLED, and
// libtest allocates through it at teardown whether a test ran or not.
#[cfg(not(miri))]
#[global_allocator]
static RECEIPT_ALLOC: ReceiptAlloc = ReceiptAlloc;

/// Refuse to run the receipts under Miri.
///
/// The allocator above is not installed there, so every counter stays at zero
/// for the whole run. MEASURED (a `--cfg miri` build of this binary, this call
/// removed, forced with `--ignored`): the run reds at R0 with `CELL_ALLOCS = 0`
/// — a red that accuses `alloc_cell` of leaving the detached path, in the one
/// gate that decides `run_scoped`, for a regression that did not happen. The
/// `== 0` receipts below R0 (R1a, R1b, `n == 0`, R4') would have been green
/// over an EMPTY bucket and are saved only by R0 running first, so their
/// non-vacuity is an ordering property. This call is the part that does not
/// depend on the ordering: a test that reaches it under Miri is one whose
/// `#[cfg_attr(miri, ignore = ...)]` has rotted, and that attribute is the only
/// way this binary's Miri skip can rot.
///
/// A `cfg`-split pair rather than `assert!(!cfg!(miri), ..)`, because the latter
/// is an assertion on a constant and the crate's clippy gate denies those.
#[cfg(miri)]
fn refuse_under_miri() {
    panic!(
        "the receipt allocator is `cfg(not(miri))` (see this file's header: delegating to std's \
         Windows `System` reds this crate's own Miri gate from inside `HeapFree`), so every \
         counter this test reads is frozen at zero: R0 would red as a FICTIONAL `alloc_cell` \
         regression, and R1/`n == 0`/R4' would be green over an EMPTY bucket. This panic is the \
         real cause, stated once. A test that needs the allocator must carry \
         #[cfg_attr(miri, ignore = ...)]"
    );
}

/// The native arm of [`refuse_under_miri`]: nothing to refuse.
#[cfg(not(miri))]
fn refuse_under_miri() {}

/// Zero the two CUMULATIVE counters and the unasserted `OTHER`.
///
/// `CHUNK_LIVE` is NOT touched: it is the net counter R4' reads at test end,
/// and resetting it mid-run would let a chunk allocated in one phase and freed
/// in the next drive it negative for reasons that are not a leak.
fn reset_cumulative() {
    CELL_ALLOCS.store(0, Ordering::Release);
    CHUNK_ALLOCS.store(0, Ordering::Release);
    OTHER.store(0, Ordering::Release);
}

/// Arm the cell bucket. Both halves, because a bucket is the pair.
fn arm_cell_class(size: usize, align: usize) {
    CELL_SIZE.store(size, Ordering::Release);
    CELL_ALIGN.store(align, Ordering::Release);
}

// =========================================================================
// R2's prediction — an INDEPENDENT re-derivation of §2.1's growth rule.
//
// Deliberately NOT called out of `block.rs`. A model that asked the code for
// its own rule would assert `x == x` and survive any change to either side.
// =========================================================================

/// How many chunks a wave of `n` cells of `stride` bytes costs.
///
/// The growth rule, re-derived:
///
/// ```text
/// required(T) = size_of::<T>() + align_of::<T>().saturating_sub(CHUNK_ALIGN)
/// e_i         = max(i, smallest e such that (CHUNK0 << e) >= required(T))
/// cap_i       = CHUNK0 << e_i
/// ```
///
/// The per-chunk **remainder** is modelled, and it is the part a naive
/// `total_bytes / capacity` division gets wrong: a cell that does not fit in
/// the tail of chunk `i` is not split across chunks, it starts chunk `i + 1`
/// and the tail is abandoned. Hence
///
/// ```text
/// cells_in_chunk_i = floor(cap_i / stride)
/// ```
///
/// A `const fn` on purpose: it lets the committed row values below be pinned
/// against this model at BUILD time, so a drift between the re-derivation and
/// the hand arithmetic cannot present itself as a re-measured number.
const fn predicted_chunks(n: usize, stride: usize, required: usize) -> usize {
    if n == 0 {
        // The block is lazy: `ScopeBlock::new` allocates nothing and the first
        // `emplace` is what grows chunk 0.
        return 0;
    }
    let mut min_e: u32 = 0;
    while (CHUNK0 << min_e) < required {
        min_e += 1;
    }
    let mut placed = 0usize;
    let mut i = 0usize;
    while placed < n {
        // `max(i, min_e)`: the `i` term is the doubling, the `min_e` term is
        // the fit, and taking the larger is what keeps one oversized cell from
        // resetting the growth curve.
        let e = if (i as u32) > min_e { i as u32 } else { min_e };
        let cap = CHUNK0 << e;
        let cells = cap / stride;
        assert!(cells > 0, "a chunk grown for required(T) admits at least one T");
        placed += cells;
        i += 1;
    }
    i
}

/// `(n, chunks)` — the committed prediction, computed BY HAND at `stride = 88`.
///
/// ```text
/// cap:         4096  8192  16384  32768  65536
/// cells:         46    93    186    372    744      (floor(cap / 88))
/// cumulative:    46   139    325    697   1441
/// ```
///
/// so `n = 1` -> 1, `n = 16` -> 1, `n = 64` -> 2 (46 < 64 <= 139),
/// `n = 1024` -> 5 (697 < 1024 <= 1441).
const R2_ROWS: [(usize, usize); 4] = [(1, 1), (16, 1), (64, 2), (1024, 5)];

// The hand arithmetic against the model, at build time. If the model drifts
// these stop compiling; if `block.rs` drifts, the RUN goes red instead.
const _: () = assert!(predicted_chunks(R2_ROWS[0].0, STRIDE, REQUIRED) == R2_ROWS[0].1);
const _: () = assert!(predicted_chunks(R2_ROWS[1].0, STRIDE, REQUIRED) == R2_ROWS[1].1);
const _: () = assert!(predicted_chunks(R2_ROWS[2].0, STRIDE, REQUIRED) == R2_ROWS[2].1);
const _: () = assert!(predicted_chunks(R2_ROWS[3].0, STRIDE, REQUIRED) == R2_ROWS[3].1);
// And the per-chunk cell counts themselves, so the table above is checkable
// rather than decorative.
const _: () = assert!(4096 / STRIDE == 46);
const _: () = assert!(8192 / STRIDE == 93);
const _: () = assert!(16384 / STRIDE == 186);
const _: () = assert!(32768 / STRIDE == 372);
const _: () = assert!(65536 / STRIDE == 744);
// `n == 0` is the lazy-block edge case, stated in the model as well as in the
// run.
const _: () = assert!(predicted_chunks(0, STRIDE, REQUIRED) == 0);

// =========================================================================
// Bodies.
// =========================================================================

/// The scoped body `F`: 72 bytes of ballast and nothing else.
///
/// One function so that every phase spawns ONE closure type and therefore one
/// stride. The ballast is READ in the body, not merely mentioned: under RFC
/// 2229 an unused capture is not captured at all, and the cell would shrink
/// out of the class the receipts are stated over. `move` is what makes the
/// capture by value despite the body only borrowing it.
fn scoped_body() -> impl FnOnce() + Send + 'static {
    let ballast = [0u8; SCOPED_BODY_SIZE];
    move || {
        std::hint::black_box(&ballast);
    }
}

/// The scoped body that panics — same ballast, hence the same stride, so the
/// `n = 1` chunk prediction is the same arithmetic as everywhere else.
fn panicking_body() -> impl FnOnce() + Send + 'static {
    let ballast = [0u8; SCOPED_BODY_SIZE];
    move || {
        std::hint::black_box(&ballast);
        panic!("{PANIC_MARKER}");
    }
}

/// Marker for the deliberate panic, so the caught payload can be identified as
/// OURS and not, say, a `debug_assert!` inside the pool.
const PANIC_MARKER: &str = "block_allocation_receipts: deliberate body panic";

/// The detached body `G`: 72 bytes of ballast plus the completion counter.
///
/// The counter is how the wave is drained. `ThreadPool::spawn` has no
/// completion accounting and no join, so there is nothing else to wait on —
/// and dropping the pool to force the drain would be the one recovery this
/// crate documents as a pool lifetime rather than a quantum.
fn detached_body(done: Arc<AtomicUsize>) -> impl FnOnce() + Send + 'static {
    let ballast = [0u8; DETACHED_BODY_BALLAST];
    move || {
        std::hint::black_box(&ballast);
        done.fetch_add(1, Ordering::AcqRel);
    }
}

/// Spin until `done` reaches `want`, with the pool deliberately kept ALIVE.
///
/// The shape is `ke16_w_wake_completion.rs`'s
/// `w_b_a_fire_and_forget_wave_into_a_parked_pool_runs_every_task_exactly_once`:
/// waiting by dropping the pool would unpark every worker
/// (`thread_pool.rs`'s `shutdown_and_join`, which the pool's `Drop` runs) and
/// so recover the very strand a fire-and-forget wave can suffer, so the wave is
/// awaited on its own counter instead.
///
/// The deadline is a diagnostic, not a gate: a strand here means the test
/// panics with a legible message instead of hanging until the harness is
/// killed.
fn drain(done: &AtomicUsize, want: usize) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while done.load(Ordering::Acquire) < want {
        if Instant::now() > deadline {
            panic!(
                "the fire-and-forget wave did not drain in 60 s: {} of {want} tasks ran. \
                 This is a liveness failure of the wave, not an allocation receipt — see \
                 tests/ke16_w_wake_completion.rs for the row that owns it.",
                done.load(Ordering::Acquire)
            );
        }
        std::thread::yield_now();
    }
}

/// Best-effort text of a caught panic payload, for the assertion message.
fn panic_text(payload: &(dyn Any + Send)) -> &str {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.as_str()
    } else {
        "<panic payload was neither &str nor String>"
    }
}

// =========================================================================
// The one test.
// =========================================================================

/// R0, R0b, R1, R2, R4' plus the two edge cases §3 names at this level.
///
/// One test, because the counters are process-global. The phases are ordered:
/// the positive controls run BEFORE the receipt that depends on them, which is
/// what turns "the bucket is right and armed" from an assumption into an
/// observation.
#[test]
#[cfg_attr(miri, ignore = "instrument: reads a `#[global_allocator]` that is `cfg(not(miri))` (see this file's header) because Miri interprets a custom global allocator rather than replacing it, and std's Windows `System` then frees an over-aligned block through a backwards-walked pointer (`HeapFree`) that Tree Borrows rejects — landing UB at harness teardown with `ScopeBlock` nowhere on the stack, i.e. reddening the very gate that decides `run_scoped`. Under Miri every counter stays 0, which MEASURED does not read green but reds R0 as a fictional `alloc_cell` regression while R1/`n == 0`/R4''s `== 0` receipts go vacuous below it; `refuse_under_miri()` at the top of the body panics with the real cause instead, so this attribute cannot rot into a misattributed red. Runs natively.")]
fn the_scoped_task_path_allocates_no_cell_and_the_block_accounts_for_every_chunk() {
    // First statement in the body, before any counter is read: under Miri the
    // instrument is absent and every phase below would be measuring nothing.
    refuse_under_miri();

    // -----------------------------------------------------------------
    // Phase -1: measure the bodies, then derive the cell class from the
    // measurement. Nothing below may assume a closure layout.
    // -----------------------------------------------------------------
    let done = Arc::new(AtomicUsize::new(0));

    {
        let probe = scoped_body();
        assert_eq!(
            std::mem::size_of_val(&probe),
            SCOPED_BODY_SIZE,
            "the scoped body's capture must be exactly {SCOPED_BODY_SIZE} B, or the cell class \
             this file derives is not the class `alloc_cell` would produce and R1 would assert \
             zero over an EMPTY bucket"
        );
        assert_eq!(
            std::mem::align_of_val(&probe),
            SCOPED_BODY_ALIGN,
            "the scoped body must be align {SCOPED_BODY_ALIGN}: the derivation \
             `cell = header + body` holds only while `align_of::<F>() <= SCOPED_CELL_ALIGN`"
        );
    }
    {
        let probe = panicking_body();
        assert_eq!(
            std::mem::size_of_val(&probe),
            SCOPED_BODY_SIZE,
            "the panicking body must share the scoped body's stride, so the panic phase's \
             one-chunk prediction is the same arithmetic as every other phase"
        );
    }
    {
        let probe = detached_body(Arc::clone(&done));
        assert_eq!(
            std::mem::size_of_val(&probe),
            DETACHED_BODY_SIZE,
            "the detached body's capture must be exactly {DETACHED_BODY_SIZE} B, or R0 arms a \
             DIFFERENT bucket from the one R1 asserts zero in — which is the whole point of R0"
        );
        assert_eq!(
            std::mem::align_of_val(&probe),
            DETACHED_BODY_ALIGN,
            "the detached body must be align {DETACHED_BODY_ALIGN}, or `DetachedCell<G>`'s \
             alignment is not `DETACHED_CELL_ALIGN` and the bucket's other half moves"
        );
    }

    // The derivations' remaining preconditions — no tail pad on either cell,
    // and neither body's alignment exceeding its header's — are `const _`
    // pins next to the constants themselves, so they are build failures rather
    // than run failures. What CANNOT be pinned at build time is the closures'
    // measured layout, which is what the three probes above just checked.

    arm_cell_class(CELL_SIZE_EXPECTED, CELL_ALIGN_EXPECTED);
    println!(
        "[receipt] cell class = ({CELL_SIZE_EXPECTED}, {CELL_ALIGN_EXPECTED})  \
         chunk class = (2^k >= {CHUNK0}, {CHUNK_ALIGN})  stride = {STRIDE}  required = {REQUIRED}"
    );

    // -----------------------------------------------------------------
    // Phase 0: build and WARM the pool.
    //
    // The worker threads must already exist, crossbeam's `Worker` buffers must
    // already have grown (they only ever grow), and every first-touch
    // allocation on both task paths must already have happened — otherwise a
    // one-off allocation from the warm-up lands in a measured phase and reads
    // as a contributor. The warm-up runs the SAME shapes the receipts do:
    // R1's scoped wave and a small detached wave.
    // -----------------------------------------------------------------
    let pool = ThreadPoolBuilder::new().num_threads(4).build();

    pool.install(|s| {
        for _ in 0..R1_TASKS {
            s.spawn(scoped_body());
        }
    });
    pool.install(|s| {
        s.spawn_batch(R1_TASKS, (0..R1_TASKS).map(|_| scoped_body()));
    });
    {
        const WARM_DETACHED: usize = 8;
        let base = done.load(Ordering::Acquire);
        for _ in 0..WARM_DETACHED {
            pool.spawn(detached_body(Arc::clone(&done)));
        }
        drain(&done, base + WARM_DETACHED);
    }

    // -----------------------------------------------------------------
    // R0 — cell-class positive control. RUNS FIRST of the receipts.
    //
    // `ThreadPool::spawn` is unchanged by 3b and still routes through
    // `alloc_cell`, synchronously on the spawner, so the count is complete the
    // instant the loop ends and NO join is needed (there is none to invent:
    // `ThreadPool::spawn` has no completion accounting).
    //
    // RED: `< N` if `alloc_cell` left the detached path or the derived class is
    // wrong (0 is the vacuity case R1 would otherwise inherit); `> N` if
    // anything else in the process allocates at exactly (88, 8) during the
    // wave. Either direction is a fact about the bucket, which is what this
    // receipt is for.
    // -----------------------------------------------------------------
    let r0_base = done.load(Ordering::Acquire);
    reset_cumulative();
    for _ in 0..R0_TASKS {
        pool.spawn(detached_body(Arc::clone(&done)));
    }
    // Snapshot BEFORE anything that could allocate — including `println!`.
    let r0_cells = CELL_ALLOCS.load(Ordering::Acquire);
    let r0_other = OTHER.load(Ordering::Acquire);
    println!("[R0 ] CELL_ALLOCS (cumulative) = {r0_cells}  expected {R0_TASKS}  OTHER = {r0_other}");
    assert_eq!(
        r0_cells, R0_TASKS,
        "R0: CELL_ALLOCS (CUMULATIVE) must be exactly {R0_TASKS} after {R0_TASKS} detached \
         spawns. Below it, `alloc_cell` is no longer on the detached path or the cell class \
         ({CELL_SIZE_EXPECTED}, {CELL_ALIGN_EXPECTED}) is not the one it produces — and a zero \
         here means R1's `== 0` would have been green over an EMPTY bucket. Above it, something \
         else in this process allocates at that exact pair and every cell-class receipt in this \
         file is contaminated."
    );

    // Drain with the pool kept alive, so no task is stranded into pool
    // teardown and no cell dealloc lands in a later phase.
    drain(&done, r0_base + R0_TASKS);
    assert_eq!(
        done.load(Ordering::Acquire),
        r0_base + R0_TASKS,
        "R0's wave must run each body once. A sanity check, not an exactly-once oracle — that \
         is `ke16_w_wake_completion.rs`'s job."
    );

    // -----------------------------------------------------------------
    // R0b — chunk-class positive control.
    //
    // One scope whose whole payload provably fits chunk 0. This arms the chunk
    // bucket the way R0 arms the cell bucket, and it converts "nothing else in
    // this process allocates at (2^k >= 4096, 64)" from an assumption into an
    // observation whose failure is RED.
    //
    // RED: 0 if the block never grows (the wiring is absent, or `emplace` was
    // bypassed); 2+ if the wave stopped fitting chunk 0 or something else
    // contributes to the chunk class.
    // -----------------------------------------------------------------
    // R0b's premise — `R0B_TASKS * STRIDE + CHUNK_ALIGN <= CHUNK0` — is a
    // `const _` pin next to `R0B_TASKS`, so a wave resized past chunk 0 fails
    // to BUILD instead of quietly expecting the wrong chunk count.
    reset_cumulative();
    pool.install(|s| {
        for _ in 0..R0B_TASKS {
            s.spawn(scoped_body());
        }
    });
    let r0b_chunks = CHUNK_ALLOCS.load(Ordering::Acquire);
    let r0b_live = CHUNK_LIVE.load(Ordering::Acquire);
    println!(
        "[R0b] CHUNK_ALLOCS (cumulative) = {r0b_chunks}  expected 1  \
         CHUNK_LIVE (net, process) = {r0b_live}"
    );
    assert_eq!(
        r0b_chunks, 1,
        "R0b: CHUNK_ALLOCS (CUMULATIVE) must be exactly 1 for a {R0B_TASKS}-task scope whose \
         payload fits chunk 0. Zero means the scope's block never allocated — the bump is not \
         wired, or the scoped path bypasses `emplace`, and then R2's exact predictions and R4''s \
         balance are both stated over an EMPTY bucket. More than one means either the growth rule \
         no longer fits {R0B_TASKS} cells in {CHUNK0} B, or something outside `block.rs` allocates \
         at (2^k >= {CHUNK0}, {CHUNK_ALIGN})."
    );

    // -----------------------------------------------------------------
    // R1 — the binding receipt, through both spawn entry points.
    //
    // RED: exactly n if `alloc_cell::<ScopedCell<F>>()` is back on the scoped
    // path — a revert, a "just this once" fallback, a `MAX_CHUNKS` fallback;
    // `0 < k < n` for a partial bypass.
    //
    // It cannot false-red on chunk growth (the predicates are disjoint) nor on
    // the injector (`Block` is 1520 B / align 8, `Buffer<Task>` is a
    // power-of-two multiple of 16 B / align 8, and the match is exact).
    //
    // Both entry points are asserted with the SAME constant: `Scope::spawn_batch`
    // is `for f in bodies { self.spawn(f) }` (`src/scope.rs`), so every body it
    // takes reaches the same `prepare` -> `new_scoped` -> `emplace` seam
    // `Scope::spawn` reaches.
    // -----------------------------------------------------------------
    reset_cumulative();
    pool.install(|s| {
        for _ in 0..R1_TASKS {
            s.spawn(scoped_body());
        }
    });
    let r1_spawn_cells = CELL_ALLOCS.load(Ordering::Acquire);
    let r1_spawn_chunks = CHUNK_ALLOCS.load(Ordering::Acquire);
    println!(
        "[R1a] Scope::spawn       CELL_ALLOCS (cumulative) = {r1_spawn_cells}  expected 0  \
         (CHUNK_ALLOCS = {r1_spawn_chunks})"
    );
    assert_eq!(
        r1_spawn_cells, 0,
        "R1a: CELL_ALLOCS (CUMULATIVE) must be 0 for a {R1_TASKS}-task scope spawned through \
         `Scope::spawn`. A reading of exactly {R1_TASKS} is the whole regression this receipt \
         exists for — `alloc_cell` back on the scoped path; anything between 1 and {R1_TASKS} is \
         a partial bypass. R0 above proves this bucket is the right one and is armed, so a zero \
         here is a clean run and not an empty bucket."
    );

    reset_cumulative();
    pool.install(|s| {
        s.spawn_batch(R1_TASKS, (0..R1_TASKS).map(|_| scoped_body()));
    });
    let r1_batch_cells = CELL_ALLOCS.load(Ordering::Acquire);
    let r1_batch_chunks = CHUNK_ALLOCS.load(Ordering::Acquire);
    println!(
        "[R1b] Scope::spawn_batch CELL_ALLOCS (cumulative) = {r1_batch_cells}  expected 0  \
         (CHUNK_ALLOCS = {r1_batch_chunks})"
    );
    assert_eq!(
        r1_batch_cells, 0,
        "R1b: CELL_ALLOCS (CUMULATIVE) must be 0 for a {R1_TASKS}-body wave spawned through \
         `Scope::spawn_batch`. Same regression, other entry point: one constant serves both arms \
         because both reach `prepare` -> `new_scoped` -> `emplace`."
    );

    // -----------------------------------------------------------------
    // R2 — chunk count as an EXACT prediction.
    //
    // The count is a function of BYTES, not of task count, and the test
    // asserts the function. `predicted_chunks` is this file's own
    // re-derivation of the growth rule; the row literals are the hand
    // arithmetic; `block.rs` is the third party. All three must agree.
    //
    // RED: 0 if the block is bypassed; off-by-k if `block.rs`'s growth rule
    // drifted from the re-derivation — and it SHOULD go red then, because the
    // plan's memory table is derived from that formula.
    // -----------------------------------------------------------------
    for (n, expected) in R2_ROWS {
        // Model against hand arithmetic, again at run time so the row is
        // legible in the output and not only in a `const _`.
        assert_eq!(
            predicted_chunks(n, STRIDE, REQUIRED),
            expected,
            "R2: this file's re-derivation of the growth rule disagrees with the committed hand \
             arithmetic at n = {n}. One of the two is wrong; neither is `block.rs`."
        );

        reset_cumulative();
        pool.install(|s| {
            for _ in 0..n {
                s.spawn(scoped_body());
            }
        });
        let measured = CHUNK_ALLOCS.load(Ordering::Acquire);
        println!(
            "[R2 ] n = {n:>4}  CHUNK_ALLOCS (cumulative) = {measured}  predicted {expected}"
        );
        assert_eq!(
            measured, expected,
            "R2: a {n}-task scope at stride {STRIDE} must cost exactly {expected} chunk-class \
             allocations. Zero means the block was bypassed entirely (jointly with R1 that is \
             the full bypass signature). Any other value means `block.rs`'s growth rule has \
             drifted from this file's independent re-derivation of it: caps \
             4096/8192/16384/32768/65536 holding 46/93/186/372/744 cells, cumulative \
             46/139/325/697/1441."
        );
    }

    // -----------------------------------------------------------------
    // Edge case: n == 0. The block stays LAZY.
    //
    // RED: any nonzero reading — `ScopeBlock::new` started allocating, or
    // `Scope::new` grew a chunk eagerly, and every empty-scope dispatch round
    // in the scheduler pays for a chunk it does not use.
    // -----------------------------------------------------------------
    reset_cumulative();
    pool.install(|_s| {});
    let empty_chunks = CHUNK_ALLOCS.load(Ordering::Acquire);
    let empty_cells = CELL_ALLOCS.load(Ordering::Acquire);
    println!(
        "[n=0] CHUNK_ALLOCS (cumulative) = {empty_chunks}  expected 0  \
         (CELL_ALLOCS = {empty_cells})"
    );
    assert_eq!(
        empty_chunks, 0,
        "n == 0: a scope that spawns nothing must make no chunk-class allocation — the block is \
         lazy and the first `emplace` is what grows chunk 0."
    );

    // -----------------------------------------------------------------
    // Edge case: a body that PANICS.
    //
    // The scope captures the panic and re-raises it at drop, and `free_all`
    // runs BEFORE the re-raise, so the chunks are gone by the time the unwind
    // leaves `install`.
    //
    // RED: `Ok(())` if the panic was not propagated at all; a wrong marker if
    // something else panicked; `CHUNK_ALLOCS != 1` if the scope allocated no
    // chunk (which would make the balance below vacuous — the anti-vacuity
    // guard for this phase); `CHUNK_LIVE != 0` if `free_all` is skipped on the
    // unwinding path.
    // -----------------------------------------------------------------
    reset_cumulative();
    let caught = catch_unwind(AssertUnwindSafe(|| {
        pool.install(|s| {
            s.spawn(panicking_body());
        });
    }));
    let panic_chunks = CHUNK_ALLOCS.load(Ordering::Acquire);
    let panic_live = CHUNK_LIVE.load(Ordering::Acquire);
    println!(
        "[pnc] CHUNK_ALLOCS (cumulative) = {panic_chunks}  expected 1  \
         CHUNK_LIVE (net, process) = {panic_live}  expected 0  caught = {}",
        caught.is_err()
    );
    let payload = caught.expect_err(
        "the scope must re-raise the body's captured panic on the calling thread; an `Ok` here \
         means the panic was swallowed and this phase measured nothing about the unwinding path",
    );
    assert!(
        panic_text(&*payload).contains(PANIC_MARKER),
        "the caught panic must be the body's own (marker {PANIC_MARKER:?}), not an unrelated \
         failure inside the pool; got {:?}",
        panic_text(&*payload)
    );
    assert_eq!(
        panic_chunks, 1,
        "the panicking scope must still have allocated its one chunk at stride {STRIDE}; a zero \
         makes the CHUNK_LIVE assertion below vacuous"
    );
    assert_eq!(
        panic_live, 0,
        "CHUNK_LIVE (NET) must be back to 0 after the panic unwound out of the scope: \
         `Scope::drop` runs `free_all` immediately after the join and BEFORE `resume_unwind`, so \
         nothing can panic between the join and the free and leak-safety is structural rather \
         than a `Drop` impl. A nonzero reading means `free_all` is skipped on the unwinding path."
    );

    // -----------------------------------------------------------------
    // R4' — chunk balance, at test end.
    //
    // The pool is dropped first, so every worker is joined and no scope can
    // still be open. `CHUNK_LIVE` is a whole-process NET that was never reset,
    // so this is a balance over every chunk this run ever allocated.
    //
    // RED: nonzero if `free_all` is skipped on any path, or if a chunk was
    // recorded in `bases` but not counted in `n_chunks` (the loop bound
    // `free_all` iterates to).
    //
    // NOT vacuous: R0b and R2 above assert that chunks WERE allocated, so a
    // green here cannot come from a bucket nothing ever entered.
    //
    // The cell class is deliberately absent: R1 asserts zero allocations
    // there, and R0's detached cells are freed asynchronously in
    // `run_detached`, so a cell balance would re-import a join that does not
    // exist. The stronger instrument for the cell class is Miri WITHOUT
    // `-Zmiri-ignore-leaks`.
    // -----------------------------------------------------------------
    drop(pool);
    let final_live = CHUNK_LIVE.load(Ordering::Acquire);
    let final_other = OTHER.load(Ordering::Acquire);
    println!(
        "[R4'] CHUNK_LIVE (net, whole process, never reset) = {final_live}  expected 0  \
         OTHER (recorded, never asserted) = {final_other}"
    );
    assert_eq!(
        final_live, 0,
        "R4': every chunk-class allocation this process made must have been freed. A positive \
         reading is a leaked chunk — `free_all` skipped on some path, or a chunk recorded in \
         `bases` past `n_chunks`. A negative reading is NARROWER than a layout mismatch, and \
         deliberately stated as such: this counter is NET, so a chunk freed at the wrong \
         power-of-two SIZE still satisfies `is_chunk` and the two events cancel to zero. What a \
         negative reading does catch is a free whose layout crosses the class boundary — a \
         different alignment, a non-power-of-two size, or a size below {CHUNK0}. Layout mismatch \
         proper is decided by Miri, which compares each dealloc's layout against its alloc's: the \
         recipe is `src/task/mod.rs`'s teardown note, run over `src/task/scoped.rs`'s and \
         `src/block.rs`'s own tests with leak checking ON, where this file's allocator is absent."
    );
}
