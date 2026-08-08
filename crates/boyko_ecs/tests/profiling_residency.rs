//! `G23a` — resident memory is bounded and allocated once, over three measurement domains.
//!
//! # Why this is its own binary
//!
//! It installs a `#[global_allocator]`. An integration test is a separate binary, so the counting
//! wrapper measures this file's work and nothing else — inside the crate's own test binary it
//! would be counting nine hundred unrelated tests' allocations at the same time.
//!
//! # Two departures from the gate as written, both narrowings and both stated
//!
//! **1. Domain 1 asserts ZERO, not "> 0".** The gate says *"each domain > 0 (two-sided — a stub
//! that allocates nothing fails)"*, which is the right instinct applied to the wrong domain. The
//! std-allocator domain exists to catch the profiler reaching for the heap; the design's whole
//! claim is that it never does. Asserting `> 0` there would demand an allocation in order to prove
//! there are none. The two-sidedness the clause wants is kept, and moved to where it can hold:
//! domains 2 and 3 are asserted `> 0`, so a stub that reserves nothing and links no static fails.
//!
//! **2. Domain 3 is measured with `size_of`, not with `section_report`.** The gate names
//! `section_report{LANES, REGISTRY}.total` as domain 3's bytes — but `section_report` shells out to
//! `llvm-readobj`, and this box has no `llvm-readobj`, `objdump` or `nm` on `PATH` under the active
//! `stable-x86_64-pc-windows-gnu` toolchain (re-measured this rung; unchanged since the substrate
//! measured it). The gate's own rule is that tool absence is a **RED, never a SKIP**, so under the
//! literal reading this row cannot be green on this machine at all.
//!
//! The two things being conflated are separable. The tool proves **`.bss` residency** — that the
//! image carries no raw data for a symbol. This gate needs the symbol's **bytes**, and the bytes of
//! a `static` array are a compile-time constant that `size_of` gives exactly, with no toolchain and
//! no shell-out. So the bound is measured here and is exact; the residency claim is **not made
//! here** and stays `G22a`'s, where it remains RED for want of the tool. That RED is pre-existing
//! and is not this rung's to clear — `rustup component add llvm-tools` is a D0 line item that was
//! never taken.
//!
//! # The counter is PER-THREAD, and the first version of it was not
//!
//! **MEASURED, and it is the reason this note exists.** The first draft counted into a process-wide
//! `AtomicUsize`. Both tests in this file then failed, reporting 136 B for `Profiler::new()` and
//! 11 753 B for `arm` — figures that have nothing to do with the profiler. `libtest` runs the two
//! tests on separate threads, and a global counter read before and after a call reports *whatever
//! the whole process allocated in that interval*, which included the other test's harness.
//!
//! A direct probe settled it: with the two loads adjacent, `Profiler::new()`, `clock::calibrate()`,
//! a first `warn!`, a second `warn!` and `arm` each measured **exactly 0**.
//!
//! The failure mode worth keeping is not the red. It is that the same instrument would have gone
//! **green by luck** had the scheduler placed the two tests further apart — a gate whose verdict
//! depends on thread timing is not measuring its subject. A per-thread counter answers the question
//! that was actually asked: *did this call, on this thread, reach for the heap*.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use boyko_ecs::ecs::core::profiling::{ArmOutcome, Profiler, ProfilerConfig};

thread_local! {
    /// Bytes this thread's allocations have asked for. `const`-initialised and `Copy`, so the slot
    /// needs no lazy setup and registers no destructor — either would re-enter the allocator from
    /// inside the allocator.
    static ALLOCATED: Cell<usize> = const { Cell::new(0) };
}

/// This thread's total. `0` if TLS is already torn down, which cannot happen on a live test thread
/// and must not panic if it somehow does.
fn allocated() -> usize {
    ALLOCATED.try_with(Cell::get).unwrap_or(0)
}

/// The counting wrapper. Only `alloc`/`alloc_zeroed`/`realloc`'s growth are counted: the question
/// is *"did this call reach for the heap"*, and a free tells us about an allocation some earlier
/// call made.
struct Counting;

/// Charge `n` bytes to the calling thread, ignoring a torn-down TLS.
#[inline]
fn charge(n: usize) {
    let _ = ALLOCATED.try_with(|c| c.set(c.get() + n));
}

// SAFETY: every method forwards to `System` with the caller's own layout and pointer, unchanged.
//   The accounting touches a `Cell<usize>` in `const`-initialised thread-local storage: no lazy
//   initialisation and no destructor, so it cannot re-enter the allocator, and `try_with` makes a
//   torn-down TLS a zero rather than a panic inside an allocation.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        charge(layout.size());
        // SAFETY: `layout` is the caller's, forwarded unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` and `layout` are the caller's, forwarded unchanged.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        charge(layout.size());
        // SAFETY: as `alloc`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        charge(new_size.saturating_sub(layout.size()));
        // SAFETY: as `dealloc`, plus `new_size` forwarded unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// The `dev` profile's armed budget, derived here rather than quoted, because a budget quoted from
/// a document is a number nothing checks.
///
/// At `LANE_COUNT = 80`, `REGION_CAPACITY = 1024`, `zone_stride = ENGINE_ZONE_SLOTS = 4096` and
/// `WINDOW = 121`:
///
/// | Term | Bytes |
/// |---|---|
/// | sample slab `80 × 2 × 1024 × 24` | 3 932 160 |
/// | `total` `4096 × 121 × 8` | 3 964 928 |
/// | `count` + `min` + `max` `3 × 4096 × 121 × 4` | 5 947 392 |
/// | `label` `4096 × 121 × 1` | 495 616 |
/// | frames + begins `121 × 32 + 121 × 8` | 4 840 |
/// | `LANES` `80 × 256` + `REGISTRY` `4096 × 8` | 53 248 |
/// | **total** | **≈ 14.7 MiB** |
///
/// 16 MiB leaves ~1.3 MiB of headroom for the section padding and the reservation's granule
/// rounding. It is a `dev` figure and says nothing about a shipped title: the shipping row is
/// 1 280 KiB and depends on per-profile constants (`ENGINE_ZONE_SLOTS = 256`,
/// `REGION_CAPACITY = 128`) that arrive with the single build axis at J1. **Until then no
/// shipping-budget claim is made here**, and quoting one would be a number taken on a geometry the
/// binary does not have.
const DEV_ARMED_BUDGET_BYTES: usize = 16 * 1024 * 1024;

/// The three domains, summed, against the budget — and each domain asserted in the direction it
/// can actually be wrong in.
///
/// # The showable RED
///
/// Raise `LANE_COUNT` in `boyko_diag::lane` from 80 to 256: the sample slab becomes
/// `256 × 2 × 1024 × 24` = 12 582 912 B (+8.65 MiB) and `LANES` becomes 65 536 B, so the sum
/// reaches ~23.0 MiB and crosses 16 MiB. Nothing else in the row moves. Run and confirmed at
/// implementation.
#[test]
fn the_armed_footprint_is_bounded_and_the_store_never_touches_the_heap() {
    let mut p = Profiler::new();

    let before = allocated();
    let outcome = p.arm(ProfilerConfig::default());
    let std_bytes = allocated() - before;

    assert_eq!(outcome, ArmOutcome::Armed, "this binary's first arm must create the reservation");

    let reserved = Profiler::reserved_bytes();
    let statics = boyko_diag::sample::lanes_bytes() + boyko_diag::profiling_abi::registry_bytes();

    // Domain 2 and domain 3 are the two-sided half: a stub that reserves nothing, or one that
    // links no static, fails here.
    assert!(reserved > 0, "domain 2 measured nothing — was anything reserved at all?");
    assert!(statics > 0, "domain 3 measured nothing — are the transports linked?");

    // Domain 1, asserted in the only direction it can be wrong in. See the module docs.
    assert_eq!(
        std_bytes, 0,
        "the profiling store reached for the heap during arm ({std_bytes} B). Every byte it owns \
         is either the reservation or a `.bss` static, by construction — a heap allocation here \
         means a `Vec`, a `Box` or a `String` got in."
    );

    let total = std_bytes + reserved + statics;
    assert!(
        total <= DEV_ARMED_BUDGET_BYTES,
        "the armed footprint is {total} B, over the {DEV_ARMED_BUDGET_BYTES} B dev budget \
         (reservation {reserved}, statics {statics})"
    );

    // And it is allocated ONCE: a second arm at the live geometry adds nothing to any domain.
    let before_second = allocated();
    assert_eq!(p.arm(ProfilerConfig::default()), ArmOutcome::Rearmed);
    assert_eq!(
        allocated() - before_second,
        0,
        "a re-arm allocated"
    );
    assert_eq!(Profiler::reserved_bytes(), reserved, "a re-arm reserved more address space");
}

/// A disarmed store costs **nothing** in any domain: no reservation, no heap, no page.
///
/// This is the flag-off half of the residency claim, and it is the one a shipped title actually
/// pays. It runs before the arm test in a fresh process only by accident of ordering, so it
/// asserts about its own `Profiler` rather than about `reserved_bytes`, which is process-wide.
#[test]
fn a_store_that_never_arms_allocates_nothing() {
    let before = allocated();
    let p = Profiler::new();
    let after = allocated();
    assert!(!p.is_armed());
    assert_eq!(p.zone_stride(), 0);
    assert_eq!(after - before, 0, "constructing a disarmed store allocated");
}
