//! CORE C4 gate 5 / §3.3 — the allocation-delta harness, and its first arm:
//! **`Prim` get / set allocate 0**, per [`ScalarKind`].
//!
//! # Why the instrument lands HERE and not at C5
//!
//! §3.3 says *"Instrument (rung C5)"* and its table assigns the `Prim get / set` row to
//! C5 as well — but **C4 gate 5 is** *"Alloc-delta harness arm: `Prim` get/set = 0
//! (§3.3)"*, one rung earlier. A gate whose instrument is specified to land after it is
//! a gate that cannot run, which is this campaign's most-repeated defect class (twelve
//! benches in a gate table, none of which existed). The contradiction is resolved in the
//! direction that keeps every gate runnable: the counting allocator and its baseline
//! subtraction land at **C4**, with the `Prim` arm; C5 adds the `enumerate` and
//! `array read` arms on top. §3.3 and C5's Lands are amended to say so.
//!
//! # The instrument
//!
//! A counting global allocator with **baseline subtraction**, modelled on
//! `crates/boyko_ui/tests/p4_bind_zero_alloc.rs` (CORE F20 — the tree's established
//! zero-allocation instrument): the number reported is the delta between the measured
//! path and an *identically shaped* no-op driven through the same fn-pointer
//! indirection, so the harness's own machinery cancels instead of being argued about.
//!
//! **A zero-allocation harness whose red nobody has seen is not a harness.** The rung's
//! record carries the output of an allocation deliberately inserted into
//! `prim::get_f32`, and [`the_counter_sees_a_deliberate_allocation`] keeps a permanent
//! positive control in the binary so a green can never mean "the counter was never
//! armed".
//!
//! # One deviation from the F20 precedent, and it is MEASURED, not preferred
//!
//! `p4_bind_zero_alloc.rs` arms a **process-global** `AtomicUsize` and serializes its
//! armed windows with a file-static `Mutex`. Built that way first, this harness was
//! **not exact** (run 2026-08-21, this worktree, first execution of gate 5):
//!
//! ```text
//! get  Bool: baseline=1 measured=0 delta=-1      <- a NEGATIVE delta
//! set  Bool: baseline=0 measured=1 delta=+1
//! refused set Bool: baseline=0 measured=2
//! ```
//!
//! A global counter counts **every thread's** allocations, and libtest's own machinery
//! allocates on other threads *while* an armed window is open — the mutex serializes
//! this file's windows against each other, and cannot serialize them against the
//! harness. `delta = -1` is the diagnostic that settles it: the measured path cannot
//! allocate *less* than nothing, so the number is noise, not signal. Noise of ±2 would
//! not hide a per-call allocation (that is `REPS` = 1000), but it destroys the exact
//! `assert_eq!` this gate wants, and the alternative — a tolerance — is a gate that
//! cannot fail for small leaks.
//!
//! So the counter is **thread-local**, `const`-initialized and `Drop`-free, which
//! compiles to a plain TLS read with no lazy init and no destructor registration —
//! therefore no allocation inside the allocator, and no reentrancy. Each armed window
//! then sees exactly its own thread, the deltas are exactly 0, and no `Mutex` (and no
//! `disallowed_types` exception) is needed at all.
//!
//! # Why this binary is EXCLUDED from Miri, and how that was established
//!
//! A `#[global_allocator]` that forwards to `System` is **not transparent under Miri +
//! Tree Borrows on `x86_64-pc-windows-gnu`**: `System::dealloc` reaches the real
//! `HeapFree`, and freeing through the raw pointer `System.alloc` returned is a foreign
//! write to whatever protected tag the caller's `&mut` holds —
//!
//! ```text
//! error: Undefined Behavior: deallocation through <166118> (root of the allocation) is forbidden
//!   help: the accessed tag <166118> was created here: unsafe { System.alloc(layout) }
//!   help: protected tags must never be Disabled
//!   5: <Box<mpmc::counter::Counter<Channel<test::event::CompletedTest>>> as Drop>::drop
//! ```
//!
//! **The UB is in libtest, not here, and that was MEASURED rather than argued**: run
//! with a filter selecting *nothing* (`-- zzz_no_such_test`), the binary prints
//! `running 0 tests` and **still aborts** with the same diagnostic, in
//! `mpmc::Sender::drop` during harness shutdown. No test body is involved; merely
//! *installing* a forwarding allocator in a libtest binary is enough. Miri's normal
//! path models `__rust_alloc`/`__rust_dealloc` directly and never reaches `HeapFree`,
//! which is why nothing else in the tree has met this — `boyko_ui`'s F20 precedent is
//! not on the Miri allowlist, so this harness is the first counting allocator in the
//! repo to run under Miri at all.
//!
//! Nothing is lost: **C4 gate 4's subject is the `prim::` module**, and `c4_prim.rs`
//! exercises every one of the twenty-four accessors — round-trip, 12×12 mismatch
//! matrix, non-canonical payloads — under Miri, green. What is excluded is the
//! *counting* of allocations, which Miri has no business re-measuring.
#![cfg(not(miri))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;

use boyko_ecs::ecs::identifiers::primitives::EntityId;

use boyko_reflect::prim;
use boyko_reflect::scalar::{Scalar, ScalarKind};

// ───────────────────────────── counting allocator ──────────────────────────

thread_local! {
    /// Whether this thread's armed window is open. `const`-init + no `Drop` ⇒ a plain
    /// TLS read, so reading it from inside the allocator cannot allocate.
    static ARMED: Cell<bool> = const { Cell::new(false) };
    /// This thread's allocation count while armed.
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
}

struct Counting;

// SAFETY: every call is forwarded verbatim to the system allocator; the only added
// behavior is a thread-local increment on alloc/realloc while this thread is armed,
// which changes no allocation semantics. The counter itself cannot allocate (a
// `const`-initialized, `Drop`-free `Cell` is a direct TLS read), so there is no
// reentrancy into this allocator; `try_with` additionally degrades to a no-op rather
// than panicking if it is ever reached during TLS teardown.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_allocation();
        // SAFETY: `layout` is forwarded unchanged from the caller, who satisfies
        // `GlobalAlloc::alloc`'s contract.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr`/`layout` are forwarded unchanged from the caller, who
        // satisfies `GlobalAlloc::dealloc`'s contract; this allocator only ever hands
        // out `System`'s blocks.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note_allocation();
        // SAFETY: as `dealloc`, forwarded unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// Increments this thread's counter if its window is open.
fn note_allocation() {
    if ARMED.try_with(Cell::get).unwrap_or(false) {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Runs `f` with THIS THREAD's counter armed and returns the allocations observed.
fn count_allocs(f: impl FnOnce()) -> usize {
    ALLOCS.with(|c| c.set(0));
    ARMED.with(|c| c.set(true));
    f();
    ARMED.with(|c| c.set(false));
    ALLOCS.with(Cell::get)
}

// ───────────────────────────────── the subject ──────────────────────────────

/// The same no-padding fixture C4's gates 1–3 use, minus the fields no arm touches.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct AllPrims {
    e: EntityId,
    u64_: u64,
    i64_: i64,
    f64_: f64,
    u32_: u32,
    i32_: i32,
    f32_: f32,
    u16_: u16,
    i16_: i16,
    u8_: u8,
    i8_: i8,
    b: bool,
    _pad: [u8; 5],
}

fn sample_struct() -> AllPrims {
    AllPrims {
        e: EntityId(1),
        u64_: 2,
        i64_: -3,
        f64_: 4.5,
        u32_: 5,
        i32_: -6,
        f32_: 7.5,
        u16_: 8,
        i16_: -9,
        u8_: 10,
        i8_: -11,
        b: true,
        _pad: [0; 5],
    }
}

/// The baseline's read half: the same `unsafe fn(*const u8) -> Scalar` signature,
/// driven through the same fn-pointer indirection, doing nothing.
///
/// # Safety
///
/// None required — the pointer is ignored. The `unsafe` is here so the *shape* matches
/// the accessor it stands in for.
unsafe fn noop_get(_p: *const u8) -> Scalar {
    Scalar::from(0u8)
}

/// The baseline's write half. See [`noop_get`].
///
/// # Safety
///
/// None required — the pointer is ignored.
unsafe fn noop_set(_p: *mut u8, _v: Scalar) -> bool {
    true
}

struct Arm {
    kind: ScalarKind,
    offset: usize,
    get: unsafe fn(*const u8) -> Scalar,
    set: unsafe fn(*mut u8, Scalar) -> bool,
    sample: Scalar,
}

fn arms() -> [Arm; 12] {
    use std::mem::offset_of;
    [
        Arm {
            kind: ScalarKind::Bool,
            offset: offset_of!(AllPrims, b),
            get: prim::get_bool,
            set: prim::set_bool,
            sample: Scalar::from(false),
        },
        Arm {
            kind: ScalarKind::U8,
            offset: offset_of!(AllPrims, u8_),
            get: prim::get_u8,
            set: prim::set_u8,
            sample: Scalar::from(21u8),
        },
        Arm {
            kind: ScalarKind::U16,
            offset: offset_of!(AllPrims, u16_),
            get: prim::get_u16,
            set: prim::set_u16,
            sample: Scalar::from(22u16),
        },
        Arm {
            kind: ScalarKind::U32,
            offset: offset_of!(AllPrims, u32_),
            get: prim::get_u32,
            set: prim::set_u32,
            sample: Scalar::from(23u32),
        },
        Arm {
            kind: ScalarKind::U64,
            offset: offset_of!(AllPrims, u64_),
            get: prim::get_u64,
            set: prim::set_u64,
            sample: Scalar::from(24u64),
        },
        Arm {
            kind: ScalarKind::I8,
            offset: offset_of!(AllPrims, i8_),
            get: prim::get_i8,
            set: prim::set_i8,
            sample: Scalar::from(-25i8),
        },
        Arm {
            kind: ScalarKind::I16,
            offset: offset_of!(AllPrims, i16_),
            get: prim::get_i16,
            set: prim::set_i16,
            sample: Scalar::from(-26i16),
        },
        Arm {
            kind: ScalarKind::I32,
            offset: offset_of!(AllPrims, i32_),
            get: prim::get_i32,
            set: prim::set_i32,
            sample: Scalar::from(-27i32),
        },
        Arm {
            kind: ScalarKind::I64,
            offset: offset_of!(AllPrims, i64_),
            get: prim::get_i64,
            set: prim::set_i64,
            sample: Scalar::from(-28i64),
        },
        Arm {
            kind: ScalarKind::F32,
            offset: offset_of!(AllPrims, f32_),
            get: prim::get_f32,
            set: prim::set_f32,
            sample: Scalar::from(29.5f32),
        },
        Arm {
            kind: ScalarKind::F64,
            offset: offset_of!(AllPrims, f64_),
            get: prim::get_f64,
            set: prim::set_f64,
            sample: Scalar::from(30.5f64),
        },
        Arm {
            kind: ScalarKind::EntityId,
            offset: offset_of!(AllPrims, e),
            get: prim::get_entity_id,
            set: prim::set_entity_id,
            sample: Scalar::from(EntityId(31)),
        },
    ]
}

/// Iterations per armed window — enough that a per-call allocation cannot hide inside
/// a rounding, few enough that the window stays short.
const REPS: usize = 1_000;

// ───────────────────────────── the positive control ─────────────────────────

/// The instrument's own non-vacuity: a deliberate allocation inside an armed window is
/// SEEN. Without this, "delta 0" is indistinguishable from "the counter never armed" —
/// the vacuous-green class this campaign has paid for repeatedly.
#[test]
fn the_counter_sees_a_deliberate_allocation() {
    let observed = count_allocs(|| {
        let v = Vec::<u8>::with_capacity(64);
        black_box(&v);
    });
    println!("positive control: deliberate allocations observed = {observed}");
    assert!(observed > 0, "the counting allocator saw NOTHING -- the instrument is dead");
}

// ───────────────────────────────── gate 5 ───────────────────────────────────

/// CORE C4 gate 5 / §3.3 row 2 — **`Prim` get = 0 allocations**, per `ScalarKind`,
/// measured as a delta against an identically-shaped no-op.
#[test]
fn prim_get_allocates_nothing_per_scalar_kind() {
    let value = sample_struct();
    let base = (&raw const value).cast::<u8>();

    for arm in arms() {
        let baseline = count_allocs(|| {
            for _ in 0..REPS {
                // SAFETY: `noop_get` ignores the pointer; the call exists to match the
                // measured path's shape (one indirect call returning a `Scalar`).
                let s = unsafe { (noop_get)(black_box(base)) };
                black_box(s);
            }
        });
        let measured = count_allocs(|| {
            for _ in 0..REPS {
                // SAFETY: `base` is a live, initialized, correctly aligned `AllPrims`
                // owned by this frame and not concurrently written; `arm.offset` is
                // that type's own `offset_of!`, so the derived pointer is in bounds,
                // field-aligned and carries the base's provenance.
                let s = unsafe { (arm.get)(black_box(base).add(arm.offset)) };
                black_box(s);
            }
        });
        println!(
            "get {:?}: baseline={baseline} measured={measured} delta={}",
            arm.kind,
            measured as i64 - baseline as i64
        );
        assert_eq!(
            measured, baseline,
            "prim::get for {:?} allocated {} time(s) over the no-op baseline in {REPS} \
             calls -- §3.3 claims 0",
            arm.kind,
            measured as i64 - baseline as i64
        );
    }
}

/// CORE C4 gate 5 / §3.3 row 2 — **`Prim` set = 0 allocations**, per `ScalarKind`.
#[test]
fn prim_set_allocates_nothing_per_scalar_kind() {
    let mut value = sample_struct();
    let base = (&raw mut value).cast::<u8>();

    for arm in arms() {
        let baseline = count_allocs(|| {
            for _ in 0..REPS {
                // SAFETY: `noop_set` ignores the pointer; shape-matching call.
                let ok = unsafe { (noop_set)(black_box(base), arm.sample) };
                black_box(ok);
            }
        });
        let measured = count_allocs(|| {
            for _ in 0..REPS {
                // SAFETY: as the get arm, with write permission -- this frame owns the
                // `AllPrims` exclusively and holds no reference into it across the call.
                let ok = unsafe { (arm.set)(black_box(base).add(arm.offset), arm.sample) };
                black_box(ok);
            }
        });
        println!(
            "set {:?}: baseline={baseline} measured={measured} delta={}",
            arm.kind,
            measured as i64 - baseline as i64
        );
        assert_eq!(
            measured, baseline,
            "prim::set for {:?} allocated {} time(s) over the no-op baseline in {REPS} \
             calls -- §3.3 claims 0",
            arm.kind,
            measured as i64 - baseline as i64
        );
    }
}

/// A **refused** set allocates nothing either. The refusal path is the one an editor
/// hits with a stale triple, and a refusal that formats a diagnostic string on the way
/// out would be a per-frame allocation nobody asked for.
#[test]
fn a_refused_set_allocates_nothing() {
    let mut value = sample_struct();
    let base = (&raw mut value).cast::<u8>();
    let wrong = Scalar::from(0xDEAD_BEEFu32);

    for arm in arms() {
        if arm.kind == ScalarKind::U32 {
            continue;
        }
        let baseline = count_allocs(|| {
            for _ in 0..REPS {
                // SAFETY: `noop_set` ignores the pointer; shape-matching call.
                let ok = unsafe { (noop_set)(black_box(base), wrong) };
                black_box(ok);
            }
        });
        let measured = count_allocs(|| {
            for _ in 0..REPS {
                // SAFETY: as the set arm; the accessor refuses before storing, which is
                // exactly the path under measurement.
                let ok = unsafe { (arm.set)(black_box(base).add(arm.offset), wrong) };
                debug_assert!(!ok, "the mismatch must be refused");
                black_box(ok);
            }
        });
        println!("refused set {:?}: baseline={baseline} measured={measured}", arm.kind);
        assert_eq!(measured, baseline, "the refusal path for {:?} allocated", arm.kind);
    }
}
