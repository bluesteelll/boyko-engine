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
//! One test here is NOT about allocation: `ui_nine_slice_emitter_pushes_in_d4_order`
//! gates the ORDER half of S4's G4-4. It lives here because the no-realloc gate
//! beside it already drives `emit_ui_node_records` with the same input, and
//! because the emitter's push order is observable on no other route — see that
//! test's own doc.
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

use boyko_render::ui::{
    emit_ui_node_records, pack_ui_instance, PackInput, UiImageInput, UiInstance, UiNineSliceInput,
    UiRenderScratch, FLAG_TEXTURED, UI_NINE_SLICE_REGIONS,
};

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
        image: None,
        nine_slice: None,
    }
}

/// A nine-sliced, imaged `PackInput` — the S4 expansion's worst case (ten records
/// per node). `border_uv` is the component's own equal-thirds default and
/// `border_px` is a real inset, so every region has positive extent.
fn nine_sliced(i: usize) -> PackInput {
    let mut input = rect(i);
    input.rect = [i as f32, (i * 2) as f32, 96.0, 96.0];
    input.image = Some(UiImageInput {
        slot: 1,
        uv: [0.0, 0.0, 1.0, 1.0],
        tint: 0xFF_FF_FF_FF,
    });
    input.nine_slice = Some(UiNineSliceInput {
        border_px: [16.0, 24.0, 16.0, 24.0],
        border_uv: [1.0 / 3.0; 4],
        mode: 0,
        fill_center: true,
    });
    input
}

/// One steady-state frame of the S4 expansion, built through the PRODUCTION
/// emitter (`emit_ui_node_records`) rather than a hand-rolled pack loop, then
/// sorted in place — the G4-4 subject.
fn build_nine_sliced_frame(
    scratch: &mut UiRenderScratch,
    gather: &mut Vec<UiInstance>,
    n: usize,
) {
    scratch.pack.clear();
    scratch.keys.clear();
    for i in 0..n {
        let before = scratch.pack.len();
        let emitted = emit_ui_node_records(&nine_sliced(i), 1.5, &mut scratch.pack);
        debug_assert_eq!(emitted, scratch.pack.len() - before);
        for k in 0..emitted {
            // Reverse stack so the sort actually permutes; the append index is
            // the record's own position, which is what makes the key TOTAL.
            scratch.keys.push(((n - i) as u32, (before + k) as u32));
        }
    }
    scratch.sort_by_stack(gather);
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

    // --- UI-ADVANCED S4, gate G4-4's UPPER bound (red mutation M4-d). ---
    //
    // Capacity STABILITY is not capacity DISCIPLINE: the warm-up check above is a
    // lower bound (`cap >= N`), the armed window compares against the warmed
    // value, and a setup-time `reserve` of the worst case is set once and
    // allocates nothing inside the window — so all three pass while the scratch
    // permanently holds several times the memory the frame needs. `UiRenderScratch`
    // is a `Resource`; that growth never comes back.
    //
    // The bound lives on THIS rect-only frame and not on the nine-sliced one
    // below, and that is structural rather than a coincidence of N: the tempting
    // wrong fix reserves the STRIDE per node (11) while a sliced node EMITS ten,
    // so `11N < 2 × 10N` always holds and the assert could never fire there. Here
    // `emitted == N`, so the same reserve overshoots 5.5× and the mutation is a
    // red like the others.
    //
    // The bound is on the PAIR of rotating buffers, and that was MEASURED rather
    // than assumed. `sort_by_stack` ends in `core::mem::swap(&mut self.pack,
    // gather)`, so a reserve made ONCE at `UiRenderScratch::default()` sits in
    // `scratch.pack` on even frames and in `gather` on odd ones. Bounding
    // `scratch.pack` alone — which is what the rung's own text prescribed — read
    // 4 096 with the 22 528-row reserve parked in `gather`: the mutation applied
    // and the assert stayed GREEN. A bound the swap parity can hide from is a red
    // that cannot fire.
    let emitted = N;
    let held = scratch.pack.capacity().max(gather.capacity());
    assert!(
        held < 2 * emitted,
        "the pack lane must not carry a worst-case reserve: {held} rows of capacity across \
         the two rotating buffers (pack {} / gather {}) for a frame that emitted {emitted} \
         records — a setup-time `reserve` of the staging budget is permanent, because the \
         scratch is a `Resource`",
        scratch.pack.capacity(),
        gather.capacity()
    );
}

/// UI-ADVANCED S4, gate G4-4: the nine-slice expansion drives the PRODUCTION
/// emitter — `emit_ui_node_records`, the same free function `gather_into_staging`
/// reaches through — and the persistent scratch still does not reallocate in
/// steady state.
///
/// It does NOT extend [`build_frame`] above, and that is the point: that helper
/// calls `pack_ui_instance` directly and pushes its keys by hand, so "extending
/// it" would re-implement the expansion policy inside the test and gate the test
/// against itself. S4 exposes the expansion as a callable seam and this gate
/// calls it.
///
/// Fewer steady frames than the rect-only gate (16 vs 64): each frame here packs
/// ten times the records, and the property under test is capacity STABILITY,
/// which a warmed capacity either holds or breaks on the first frame.
#[test]
fn ui_nine_slice_expansion_does_not_realloc_in_steady_state() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    const N: usize = 4096;
    let mut scratch = UiRenderScratch::default();
    let mut gather: Vec<UiInstance> = Vec::new();

    for _ in 0..3 {
        build_nine_sliced_frame(&mut scratch, &mut gather, N);
    }

    let cap_pack = scratch.pack.capacity();
    let cap_keys = scratch.keys.capacity();
    let cap_gather = gather.capacity();
    let emitted = scratch.pack.len();
    assert_eq!(
        emitted,
        N * (1 + UI_NINE_SLICE_REGIONS as usize),
        "every node emitted its background plus every region — the count is DERIVED, so a \
         rung that adds a sub code moves it here too"
    );
    assert!(
        cap_pack >= emitted && cap_keys >= emitted && cap_gather >= emitted,
        "warmed to >= the emitted record count"
    );

    ARMED.store(true, Ordering::Relaxed);
    ALLOC_COUNT.store(0, Ordering::Relaxed);
    REALLOC_COUNT.store(0, Ordering::Relaxed);

    for _ in 0..16 {
        build_nine_sliced_frame(&mut scratch, &mut gather, N);
    }

    ARMED.store(false, Ordering::Relaxed);

    let reallocs = REALLOC_COUNT.load(Ordering::Relaxed);
    let allocs = ALLOC_COUNT.load(Ordering::Relaxed);

    assert_eq!(scratch.pack.capacity(), cap_pack, "pack capacity must not change");
    assert_eq!(scratch.keys.capacity(), cap_keys, "key-lane capacity must not change");
    assert_eq!(gather.capacity(), cap_gather, "gather capacity must not change");
    assert_eq!(
        reallocs, 0,
        "the nine-slice expansion must not realloc the scratch in steady state (got {reallocs})"
    );
    assert_eq!(
        allocs, 0,
        "…and `emit_ui_node_records` must not allocate per call: its sub-code scratch is a \
         stack array, not a `Vec` (got {allocs})"
    );
}

/// UI-ADVANCED S4, gate G4-4's **ORDER** half: `emit_ui_node_records` appends a
/// node's records in D4's per-node emission order — the node's own background
/// first, then the nine regions row-major (TL, T, TR, L, C, R, BL, B, BR).
///
/// # Why the seam needs its own order gate, and why this file is where it goes
///
/// The emitter's doc states the order as its contract, and until this assert
/// NOTHING read it. The sibling route (`gather_into_staging`) is immune to the
/// same defect by construction and therefore cannot stand in: it pushes the
/// `(stack, node * UI_RECORDS_PER_NODE + sub)` key and SORTS on it, so the sorted
/// stream is in sub-CODE order no matter what order the codes were pushed in —
/// `ui_node_sub_codes` emitting `[0, 9, …, 1]` produces a byte-identical staged
/// frame there. This route does not sort; it pushes straight into the caller's
/// sink in `subs[..n]` order, so here — and only here — the push order IS the
/// output order. (MEASURED: `out.swap(1, 9)` in `ui_node_sub_codes` leaves the
/// whole `boyko-render` suite green without this test, and reds it with.)
///
/// It lives beside the no-realloc gate because that gate already drives this exact
/// seam with this exact input, so the two share `nine_sliced` and the emitter call
/// rather than growing a second scene. The SERIAL lock is taken for the same
/// reason the other three take it: the allocation counters are process-global and
/// this test allocates.
#[test]
fn ui_nine_slice_emitter_pushes_in_d4_order() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    const SCALE: f32 = 1.5;

    /// Region names, so a failure says WHICH slice moved.
    const REGION: [&str; 9] = ["TL", "T", "TR", "L", "C", "R", "BL", "B", "BR"];
    // The destination grid `nine_sliced(0)` produces, AUTHORED rather than
    // recomputed from the pack's own formula: a 96×96 rect at the origin inset by
    // `[16, 24, 16, 24]` gives columns 16/64/16 and rows 24/48/24, each scaled by
    // SCALE (1.5) → 24/96/24 and 36/72/36. A region's ORIGIN is unique across the
    // nine, which is what makes "record i is region r" decidable from geometry
    // alone (two corners share a SIZE; none share a corner).
    const COL_X: [f32; 3] = [0.0, 24.0, 120.0];
    const COL_W: [f32; 3] = [24.0, 96.0, 24.0];
    const ROW_Y: [f32; 3] = [0.0, 36.0, 108.0];
    const ROW_H: [f32; 3] = [36.0, 72.0, 36.0];

    let input = nine_sliced(0);
    let mut sink: Vec<UiInstance> = Vec::new();
    let emitted = emit_ui_node_records(&input, SCALE, &mut sink);

    assert_eq!(
        emitted,
        1 + UI_NINE_SLICE_REGIONS as usize,
        "background + every region — the count is DERIVED, like the no-realloc gate's"
    );
    assert_eq!(sink.len(), emitted, "the return value counts what was pushed");

    // Sub 0 opens the block: D4 paints the rect BEFORE what sits on it.
    assert_eq!(
        sink[0].flags & FLAG_TEXTURED,
        0,
        "record 0 must be the node's own untextured BACKGROUND, not a slice"
    );
    assert_eq!(
        sink[0].size_px,
        [96.0 * SCALE, 96.0 * SCALE],
        "record 0 covers the node's whole rect"
    );

    for r in 0..UI_NINE_SLICE_REGIONS as usize {
        let rec = &sink[1 + r];
        let (col, row) = (r % 3, r / 3);
        assert_ne!(
            rec.flags & FLAG_TEXTURED,
            0,
            "record {} must be a TEXTURED slice",
            r + 1
        );
        assert_eq!(
            rec.min_px,
            [COL_X[col], ROW_Y[row]],
            "record {} must be region {r} ({}) — the emitter pushes in `subs[..n]` order \
             and does NOT sort, so its push order IS D4's emission order",
            r + 1,
            REGION[r]
        );
        assert_eq!(
            rec.size_px,
            [COL_W[col], ROW_H[row]],
            "region {} ({}) destination extent",
            r,
            REGION[r]
        );
    }
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
