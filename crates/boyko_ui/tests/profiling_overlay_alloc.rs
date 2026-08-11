//! **`G19` — the overlay read path is allocation-free**, with the positive control that makes the
//! zero mean something.
//!
//! # The clause, and why the control is not optional
//!
//! The corpus states it in one line: *"the reference overlay runs 600 frames under the
//! counting-allocator gate ⇒ 0 allocations, **and** a control system that formats a `String` in the
//! same test ⇒ > 0. Remove the control ⇒ the gate cannot distinguish 'no allocations' from 'the
//! hook is not installed'."*
//!
//! That last sentence is the whole design. A `#[global_allocator]` in a test binary is easy to get
//! wrong in a way that reads as success: declare it in a module that is never linked, count in
//! `dealloc` instead of `alloc`, or measure a stretch where nothing runs at all. Every one of those
//! reports **zero acquisitions** and every one is a gate that cannot fail. The control runs the same
//! counter over a `format!` in the same process and must report a positive number, so a zero from
//! the overlay is a zero the instrument was capable of exceeding.
//!
//! # What this gate covers, and what it deliberately does not
//!
//! It drives [`write_row`] — the formatting path — 600 frames × 8 rows, plus the set-if-changed
//! comparison the system performs around it. It does **not** drive the ECS system through a
//! scheduler. Query iteration and the `Mut` deref are kernel code with their own zero-allocation
//! gates, and folding them in here would make a red ambiguous between "the overlay allocated" and
//! "iterating a query allocated". What remains is exactly the surface an overlay author can get
//! wrong: a `String`, a `format!`, a `Vec` of rows, a `to_string()` on a zone name.
//!
//! It cannot claim a **game's** overlay is allocation-free — only the reference one. A title that
//! copies this file and adds a `format!` gets no warning from here, which is stated in the corpus's
//! own "cannot claim" column and repeated because it is the limit a reader is most likely to forget.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use boyko_diag::profiling_abi::{ZoneTier, arm_scope, zone_id};
use boyko_ecs::ecs::core::profiling::{Profiler, ProfilerConfig, ROOT_SCOPE, fold};
use boyko_ui::binding::UiTextBuffer;
use boyko_ui::profiling_overlay::{complete_row, write_row};

/// Counts every heap ACQUISITION (alloc / alloc_zeroed / realloc) **made by the calling thread**.
/// Frees are not counted: a free is not a per-frame allocation, and counting it would let a steady
/// acquire/release pair cancel out.
struct CountingAlloc;

thread_local! {
    /// # Why per-THREAD and not a process-global `AtomicUsize`
    ///
    /// MEASURED, and it is the reason this file exists in its second form. The first draft used a
    /// process-global counter — the shape `crates/boyko_app/tests/zero_alloc.rs` uses — and the
    /// overlay leg reported **6 acquisitions** in 4 800 row writes. None of them were the overlay's:
    /// `cargo test` runs a binary's tests on several threads by default, the sibling test in this
    /// same file arms a profiler, and its allocations landed inside the measured window.
    ///
    /// Running the file with `--test-threads=1` gives 0, and that is exactly why it is **not** the
    /// fix: the workspace sweep runs tests in parallel, so the gate would be intermittently red for
    /// a reason having nothing to do with the code it gates — and an intermittently red gate gets
    /// disabled, which is worse than not having it. A process-global counter in a multi-test binary
    /// measures the PROCESS, not the code under test.
    ///
    /// `const`-initialised and holding a `Cell<usize>`, which has no destructor: the thread-local
    /// needs no lazy initialisation and registers no TLS destructor, so touching it from inside the
    /// allocator cannot recurse into the allocator.
    static ACQUISITIONS: Cell<usize> = const { Cell::new(0) };
}

/// This thread's acquisition count. `try_with` because a thread tearing down can no longer reach
/// its TLS, and a gate must not panic inside the allocator.
fn acquisitions() -> usize {
    ACQUISITIONS.try_with(Cell::get).unwrap_or(0)
}

#[inline]
fn bump() {
    let _ = ACQUISITIONS.try_with(|c| c.set(c.get() + 1));
}

// SAFETY: pure delegation to `System` with a thread-local counter side-effect; every layout and
// pointer contract is forwarded unchanged, so the allocator's own invariants are the system
// allocator's. The counter touches a `const`-initialised `Cell` with no destructor, so it performs
// no allocation of its own and cannot recurse.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump();
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        bump();
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump();
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

/// Frames the corpus asks for.
const FRAMES: usize = 600;

/// Rows on screen.
const ROWS: usize = 8;

// The fixture declares its own zones, in the ENGINE partition, and this is load-bearing rather than
// convenient. A `User`-partition zone mints an id at or above `ENGINE_ZONE_SLOTS`, and the store's
// column stride is `ENGINE_ZONE_SLOTS + user_zone_budget` with the budget defaulting to **0** — so
// a user row would resolve to `out of window` and the gate would measure that early return instead
// of the cell path. Engine zones are also what a developer's overlay actually watches.
boyko_diag::profiling_partition!(Engine);

boyko_diag::declare_zone!(
    OVERLAY_A,
    name = "overlay.fixture.a",
    scope = ROOT_SCOPE,
    tier = ZoneTier::Always,
);
boyko_diag::declare_zone!(
    OVERLAY_B,
    name = "overlay.fixture.b",
    scope = ROOT_SCOPE,
    tier = ZoneTier::Always,
);

/// The lane this fixture's thread claims. Any valid index; 3 is arbitrary and only has to be one
/// the fold walks, which is all of them.
const FIXTURE_LANE: u16 = 3;

/// A zone id no process can have minted: the whole id space is `ENGINE_ZONE_SLOTS +
/// MAX_USER_BUDGET`, far below this.
const NEVER_MINTED: u16 = u16::MAX - 1;

/// **THE FIXTURE'S OWN CORRECTION, and the reason this file has a second form.**
///
/// The first draft filled the rows with the literal ids `0..8` and armed a profiler without folding
/// anything. MEASURED with a throwaway probe after a RED failed to fire: `complete_row` was `None`
/// and **every one of the eight rows rendered `"zone N not registered"`** — so the gate drove one
/// early return 4 800 times and never reached the cell path, the observed-kind lookup or the
/// formatting at all. It reported zero allocations, truthfully, about almost nothing.
///
/// So the setup below MINTS the ids by running the zones, and folds until a complete row exists.
/// The row set then covers three genuinely different paths on the measured window: two live zones
/// with cells, and one id that can never be registered — because a branch that only ever runs
/// outside the gate is a branch the gate says nothing about.
fn arm_and_populate() -> (Profiler, [u16; ROWS]) {
    // MEASURED, and the third vacuity this fixture had to be walked out of. A thread that has not
    // claimed a lane produces samples the fold NEVER SEES -- and it lies to you while doing it: the
    // zone's own `calls()` accumulator increments on every open/close, so the zone looks alive,
    // while every cell in the store stays `Empty`. Without this line the rows below render
    // `     0 x` -- the "ran zero times" branch -- and `G19` would measure THAT 4 800 times.
    // `boyko_ecs`'s own store tests call `set_lane` for the same reason; the obligation is not
    // discoverable from the profiler's public API, which is why it is written down here.
    boyko_diag::lane::set_lane(FIXTURE_LANE);

    let mut p = Profiler::new();
    let _ = p.arm(ProfilerConfig::default());
    arm_scope(ROOT_SCOPE);

    // Three folds, not one: `complete_row` is `live_frame - cpu_frames_behind` and
    // `cpu_frames_behind` is 1, so a single fold leaves no complete frame and the rows would render
    // `no complete frame` — the second vacuous state this fixture had to be walked out of.
    for _ in 0..3 {
        {
            let _a = boyko_diag::zone!(OVERLAY_A);
        }
        {
            let _b = boyko_diag::zone!(OVERLAY_B);
        }
        fold(&mut p);
    }

    let a = zone_id(&OVERLAY_A);
    let b = zone_id(&OVERLAY_B);
    let mut ids = [NEVER_MINTED; ROWS];
    for (i, slot) in ids.iter_mut().enumerate() {
        *slot = match i % 3 {
            0 => a,
            1 => b,
            _ => NEVER_MINTED,
        };
    }
    (p, ids)
}

/// **G19** — 600 frames of the reference overlay acquire nothing, and the control proves the
/// counter was live while they did not.
///
/// RED 1: put a `format!` in `write_row` ⇒ the overlay leg goes positive.
/// RED 2: delete the control ⇒ the test still passes with the allocator hook removed entirely,
/// which is the state the control exists to make impossible. That RED is not a code change to the
/// overlay at all, which is why it is the one worth naming.
#[test]
fn g19_the_overlay_read_path_acquires_nothing_and_the_counter_could_have_seen_it() {
    let (profiler, ids) = arm_and_populate();
    let row = complete_row(&profiler);

    // ── PHASE 1: the subject is present ──────────────────────────────────────────────────────────
    //
    // THE CLAUSE THAT WOULD HAVE CAUGHT THE FIRST DRAFT, and it lives INSIDE this test rather than
    // beside it. A sibling `#[test]` cannot do this job here: `Profiler` folds process-global lane
    // rings, so two tests arming two profilers each take half the samples and the rows come back
    // with `count == 0` — measured, and the second vacuity this file had to be walked out of.
    //
    // Phase 2 below measures whatever branch its rows happen to take. Without these assertions it
    // cannot tell a measured zero from a zero measured about an early return.
    assert!(row.is_some(), "the fixture folded but produced no complete frame to read");
    let mut probe = UiTextBuffer::default();
    write_row(&mut probe, &profiler, ids[0], row, true);
    let text = probe.as_str();
    assert!(text.contains("overlay.fixture"), "the live row lost its zone name: {text:?}");
    assert!(
        text.contains("tk") || text.contains("ct") || text.contains("lv"),
        "the live row printed no observed-kind unit, so it never reached the cell path -- G19's \
         window would be measuring an early return: {text:?}"
    );

    // The absence states, each distinct, each also on the measured path below. A reader acts
    // differently on "the profiler is off" than on "this id is not a zone", so they are different
    // strings rather than one blank.
    write_row(&mut probe, &profiler, NEVER_MINTED, row, true);
    assert!(probe.as_str().contains("not registered"), "got {:?}", probe.as_str());
    write_row(&mut probe, &profiler, ids[0], row, false);
    assert!(probe.as_str().contains("disarmed"), "got {:?}", probe.as_str());
    write_row(&mut probe, &profiler, ids[0], None, true);
    assert!(probe.as_str().contains("no complete frame"), "got {:?}", probe.as_str());

    // ── PHASE 2: and it costs nothing ────────────────────────────────────────────────────────────
    //
    // Warm-up OUTSIDE the measured window: the first call through any code path may fault in a
    // lazily-initialised static, and a one-off is not a per-frame allocation. Measuring it would
    // make the gate red for a reason it does not claim to be about.
    let mut warm = UiTextBuffer::default();
    for id in ids {
        write_row(&mut warm, &profiler, id, row, true);
    }

    let mut buffers = [UiTextBuffer::default(); ROWS];

    let before = acquisitions();
    for _ in 0..FRAMES {
        let row = complete_row(&profiler);
        for (i, id) in ids.iter().enumerate() {
            let mut scratch = UiTextBuffer::default();
            write_row(&mut scratch, &profiler, *id, row, true);
            // The set-if-changed comparison the system performs. Included because it is on the
            // per-frame path and because a future `PartialEq` that allocated would be invisible
            // otherwise.
            if buffers[i] != scratch {
                buffers[i] = scratch;
            }
        }
    }
    let overlay_acquisitions = acquisitions() - before;

    // THE CONTROL. Same counter, same process, same measured window shape — one `format!` per
    // frame, which is precisely what an overlay author reaches for and precisely what this design
    // refuses. If this is not positive, the instrument measured nothing above.
    let control_before = acquisitions();
    let mut sink = 0usize;
    for f in 0..FRAMES {
        let s = format!("frame {f}");
        sink = sink.wrapping_add(s.len());
    }
    let control_acquisitions = acquisitions() - control_before;
    assert!(sink > 0, "the control's work must not be optimised away");

    assert!(
        control_acquisitions > 0,
        "the control formatted {FRAMES} `String`s and the counter saw {control_acquisitions} \
         acquisitions. The allocator hook is not measuring this process, so the overlay's \
         {overlay_acquisitions} is NOT RESOLVED (instrument inert) -- never a pass."
    );
    assert_eq!(
        overlay_acquisitions, 0,
        "{FRAMES} frames x {ROWS} rows of the reference overlay acquired {overlay_acquisitions} \
         times. The control saw {control_acquisitions}, so the counter was live. Look for a \
         `format!`, a `String`, a `to_string()` or a growable buffer on the row path."
    );
}
