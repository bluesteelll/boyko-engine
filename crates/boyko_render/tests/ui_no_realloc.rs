//! GUI P5a NO-REALLOC gate — the UI render scratch (the `Vec<UiInstance>` pack + the
//! `(stack, idx)` key lane) does NOT reallocate frame-to-frame in steady state.
//!
//! P5a budget (Decision 6 / A1): "Zero per-frame allocation, zero per-frame realloc in
//! steady state … one preallocated CPU pack buffer; capacity grows pow2 only on
//! overflow." Once the scratch is warmed to a stable capacity, a steady-state frame is
//! `clear()` + `extend` + an in-place `sort_unstable_by_key` (over the total
//! `(stack, idx)` key, so no timsort merge buffer) — capacity persists, so the global
//! allocator must observe ZERO allocations across repeated frames at the same N.
//!
//! A counting global allocator (installed for THIS test binary only) makes the
//! invariant a hard assert, not a hopeful comment. The steady-state window brackets
//! the allocation counter around the warmed frames so the first-frame / capacity-
//! crossing allocations (legitimately excluded from the steady-state guarantee) do not
//! mask a regression.

// Test harness, not an engine path: the `SERIAL` `Mutex<()>` serializes the three tests because
// the counting global allocator's counters + ARMED flag are PROCESS-GLOBAL while libtest runs
// tests on parallel threads. Test-only scaffolding, never linked into a shipping build.
#![allow(clippy::disallowed_types)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use boyko_render::ui::{pack_ui_instance, PackInput, UiInstance, UiRenderScratch};

/// A pass-through allocator that counts allocations while ARMED. Disarmed by default
/// so test-harness / setup allocations are ignored; the steady-state window arms it.
struct CountingAlloc;

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static REALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static ARMED: AtomicBool = AtomicBool::new(false);

/// Serializes the three tests: the counters + ARMED flag are PROCESS-GLOBAL while
/// libtest runs the tests on parallel threads — a sibling test's setup allocations
/// landing inside another test's armed window is a counter race (a real flake seen
/// under full-workspace machine load). Each test holds this for its whole body.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

// SAFETY: every method forwards verbatim to the system allocator (same ptr/layout
// contract); the only added work is two relaxed atomic increments when ARMED, which
// does not affect allocation correctness.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            REALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

const OPAQUE_RED: u32 = 0xFF_00_00_FF;

fn rect(i: usize) -> PackInput {
    PackInput {
        rect: [i as f32, (i * 2) as f32, 10.0, 10.0],
        color: OPAQUE_RED,
        border_color: 0,
        corner_radius: [0.0; 4],
        border_width: [0.0; 4],
        clip: Some([0.0, 0.0, 1000.0, 1000.0]),
        text_uv: None,
    }
}

/// Packs `n` rects into the scratch (clear + extend) and sorts in place — exactly one
/// steady-state frame's CPU build (minus the GPU memcpy, which targets a mapped slot,
/// not the heap).
fn build_frame(scratch: &mut UiRenderScratch, gather: &mut Vec<UiInstance>, n: usize) {
    scratch.pack.clear();
    scratch.keys.clear();
    for i in 0..n {
        scratch.pack.push(pack_ui_instance(&rect(i), 1.5));
        // Reverse stack so the sort actually permutes (worst case for the gather).
        scratch.keys.push(((n - i) as u32, i as u32));
    }
    scratch.sort_by_stack(gather);
}

/// THE GATE (`no_realloc_verified`): the persistent scratch buffers — the
/// `Vec<UiInstance>` pack, the `(stack, idx)` key lane, and the gather buffer — do NOT
/// reallocate frame-to-frame in steady state. This is the literal Decision-6 invariant
/// ("one preallocated CPU pack buffer; capacity grows pow2 only on overflow"): once
/// warmed, `clear()` + `extend` reuse the storage and the capacities are byte-stable.
#[test]
fn ui_render_scratch_does_not_realloc_in_steady_state() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    const N: usize = 4096;
    let mut scratch = UiRenderScratch::default();
    let mut gather: Vec<UiInstance> = Vec::new();

    // --- Warm-up: the first frames legitimately allocate the pack/key/gather Vecs and
    //     grow them to the steady-state capacity. Excluded from the guarantee. ---
    for _ in 0..3 {
        build_frame(&mut scratch, &mut gather, N);
    }

    let cap_pack = scratch.pack.capacity();
    let cap_keys = scratch.keys.capacity();
    let cap_gather = gather.capacity();
    assert!(cap_pack >= N && cap_keys >= N && cap_gather >= N, "warmed to >= N capacity");

    // --- Steady state: arm the counter and run many frames at the SAME N. ---
    ARMED.store(true, Ordering::Relaxed);
    ALLOC_COUNT.store(0, Ordering::Relaxed);
    REALLOC_COUNT.store(0, Ordering::Relaxed);

    for _ in 0..64 {
        build_frame(&mut scratch, &mut gather, N);
    }

    ARMED.store(false, Ordering::Relaxed);

    let reallocs = REALLOC_COUNT.load(Ordering::Relaxed);

    // Capacities must be byte-identical to the warmed values (no hidden grow of the
    // persistent buffers — the swap rotates two same-capacity buffers).
    assert_eq!(scratch.pack.capacity(), cap_pack, "pack capacity must not change in steady state");
    assert_eq!(scratch.keys.capacity(), cap_keys, "key-lane capacity must not change");
    assert_eq!(gather.capacity(), cap_gather, "gather capacity must not change");

    assert_eq!(
        reallocs, 0,
        "the UI scratch persistent buffers must NOT realloc frame-to-frame in steady state (got {reallocs})"
    );
}

/// The persistent buffers stay alloc-free even when steady frames are SMALLER than the
/// warmed capacity (clear() keeps capacity; extend stays under it). The unstable sort
/// is in place and allocation-free at any N, so this frame is fully allocation-free.
#[test]
fn ui_render_scratch_steady_state_smaller_n_reuses_capacity() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    const WARM_N: usize = 2048;
    const STEADY_N: usize = 16;
    let mut scratch = UiRenderScratch::default();
    let mut gather: Vec<UiInstance> = Vec::new();

    for _ in 0..3 {
        build_frame(&mut scratch, &mut gather, WARM_N);
    }

    ARMED.store(true, Ordering::Relaxed);
    ALLOC_COUNT.store(0, Ordering::Relaxed);
    REALLOC_COUNT.store(0, Ordering::Relaxed);

    for _ in 0..64 {
        build_frame(&mut scratch, &mut gather, STEADY_N);
    }

    ARMED.store(false, Ordering::Relaxed);

    assert_eq!(
        REALLOC_COUNT.load(Ordering::Relaxed),
        0,
        "smaller steady-state frames must not realloc"
    );
    assert_eq!(
        ALLOC_COUNT.load(Ordering::Relaxed),
        0,
        "small steady-state frames must not allocate (the unstable sort is in place)"
    );
}

/// THE PER-FRAME ZERO-ALLOCATION gate for the sort: at N well above stdlib's
/// insertion-sort cutoff (~20), `UiRenderScratch::sort_by_stack` must perform ZERO
/// heap allocations per frame. This previously failed: the original `sort_by_key`
/// (stable timsort) heap-allocated an n/2 merge buffer on EACH call — one alloc per
/// frame at large N — which violated plan A1's "zero alloc" sort budget. The fix is
/// `sort_unstable_by_key`: sound here because the `(stack, idx)` key is a TOTAL order
/// (the append index is unique), so the unstable result equals the stable one while
/// the in-place pattern-defeating quicksort allocates nothing. This test asserts the
/// corrected contract (`allocs == 0` across many large-N frames).
#[test]
fn ui_render_scratch_sort_is_alloc_free_per_frame_at_large_n() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    const N: usize = 4096;
    let mut scratch = UiRenderScratch::default();
    let mut gather: Vec<UiInstance> = Vec::new();
    for _ in 0..3 {
        build_frame(&mut scratch, &mut gather, N);
    }

    ARMED.store(true, Ordering::Relaxed);
    ALLOC_COUNT.store(0, Ordering::Relaxed);

    const FRAMES: usize = 64;
    for _ in 0..FRAMES {
        build_frame(&mut scratch, &mut gather, N);
    }

    ARMED.store(false, Ordering::Relaxed);

    let allocs = ALLOC_COUNT.load(Ordering::Relaxed);
    assert_eq!(
        allocs, 0,
        "the UI sort must be allocation-free per frame at N={N} \
         (sort_unstable_by_key over the total (stack, idx) key allocates nothing) — got {allocs}"
    );
}
