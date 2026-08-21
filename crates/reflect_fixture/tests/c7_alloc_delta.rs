//! **CORE C7 gate 5 / §3.3's `default_in_place` row** — the derive-generated
//! `default_in_place` allocates **0 bespoke**, measured as a delta against an
//! identically-shaped no-op.
//!
//! # Why this is a SECOND instrument and not an arm on `c4_prim_zero_alloc.rs` (D23)
//!
//! `boyko_reflect/tests/c4_prim_zero_alloc.rs:7-11` argues in advance against a second
//! copy of the counting allocator — *"three things that would then be free to drift
//! apart, for no gain"* — and that argument was written before a subject existed that
//! its binary cannot reach. `boyko_reflect` can neither invoke the derive nor compile
//! its output (no `boyko-macros` edge, and no `[features]` table "now or ever"), so an
//! arm added there would measure a **hand-written** `default_in_place` wearing the
//! derived one's verdict — the weaker-subject substitution CORE C11 forbids by name.
//!
//! The gain is twofold. A `#[global_allocator]` is one per binary and c4's is
//! `#![cfg(not(miri))]`, so folding this arm into `tests/c7_derive_bake.rs` would carry
//! that `cfg` onto **all** of C7's gates and delete C7's derived descriptors from §7.2's
//! Miri row — the only row that reaches derive-generated `unsafe`. The allocator stays
//! here; `c7_derive_bake.rs` stays Miri-visible.
//!
//! # The instrument
//!
//! Verbatim in shape from `c4_prim_zero_alloc.rs`, including the two facts that file
//! MEASURED rather than preferred: the counter is **thread-local** (a process-global one
//! counts libtest's own allocations on other threads and produced a `delta = -1`, which
//! is the diagnostic that a measured path cannot allocate less than nothing), and the
//! binary is **excluded from Miri** (a `#[global_allocator]` forwarding to `System` is
//! not transparent under Miri + Tree Borrows on `x86_64-pc-windows-gnu` — it aborts in
//! libtest's own shutdown with `running 0 tests`).
//!
//! # The invocation is part of the gate (D23)
//!
//! ```text
//! cargo test -p reflect-fixture --features reflect-fixture/reflect --test c7_alloc_delta
//! ```
//!
//! The output must read `running [1-9]`; a plain `cargo test -p reflect-fixture`
//! compiles this file to nothing and exits 0.
#![cfg(feature = "reflect")]
#![cfg(not(miri))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::mem::MaybeUninit;

use boyko_macros::Component;
use boyko_reflect::Reflect;

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
        // SAFETY: `ptr`/`layout` are forwarded unchanged from the caller, who satisfies
        // `GlobalAlloc::dealloc`'s contract; this allocator only ever hands out
        // `System`'s blocks.
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

/// Reps per window — large enough that a single per-call allocation cannot hide in the
/// noise, and the same figure `c4_prim_zero_alloc.rs` uses.
const REPS: usize = 1000;

// ───────────────────────────────── the subjects ─────────────────────────────

/// A flat POD subject.
#[derive(Component, Default)]
#[component(reflect)]
struct AllocPod {
    value: f32,
}

/// One nesting level.
#[derive(Component, Default)]
#[component(reflect)]
struct AllocNested {
    inner: AllocPod,
}

/// The `[f32; 4]` wrapper (the derive applies to an item, not to an array type).
#[derive(Component, Default)]
#[component(reflect)]
struct AllocArrPack {
    data: [f32; 4],
}

/// A subject that owns drop glue, so the measured path is not only the trivial one.
#[derive(Component, Default)]
#[component(reflect)]
struct AllocOwned {
    tag: u32,
    flag: bool,
}

impl Drop for AllocOwned {
    fn drop(&mut self) {
        black_box(self.tag);
    }
}

/// The baseline: the same `unsafe fn(*mut u8)` signature, driven through the same
/// fn-pointer indirection, doing nothing.
///
/// # Safety
///
/// None required — the pointer is ignored. The `unsafe` is here so the *shape* matches
/// the slot it stands in for.
unsafe fn noop_default_in_place(_p: *mut u8) {}

/// Measures one type's `default_in_place` over [`REPS`] writes into a destination this
/// frame owns, and returns `(baseline, measured)`.
///
/// The destination is written repeatedly without an intervening drop. That is deliberate
/// and it is sound for every subject here: none owns heap memory, so a skipped drop
/// leaks nothing the allocator could see — and running the drop slot inside the window
/// would fold `drop_in_place`'s cost into a row §3.3 assigns to `default_in_place`.
/// The final value's drop glue is run once, after the window closes.
fn measure<T: Reflect>(label: &str) -> (usize, usize) {
    let ti = <T as Reflect>::TYPE_INFO;
    let default_in_place = ti.default_in_place.expect("every subject here derives Default");

    let mut slot = MaybeUninit::<T>::uninit();
    let p = slot.as_mut_ptr().cast::<u8>();

    let baseline = count_allocs(|| {
        for _ in 0..REPS {
            // SAFETY: `noop_default_in_place` ignores the pointer; the call exists to
            // match the measured path's shape (one indirect call through the same
            // signature).
            unsafe { noop_default_in_place(black_box(p)) };
        }
    });
    let measured = count_allocs(|| {
        for _ in 0..REPS {
            // SAFETY: `p` addresses a `MaybeUninit<T>` this frame owns, writable for
            // `size_of::<T>()` and aligned to `align_of::<T>()`. No subject here owns
            // heap memory, so overwriting a previously written value leaks nothing; the
            // last one's drop glue is run below.
            unsafe { default_in_place(black_box(p)) };
        }
    });

    if let Some(drop_in_place) = ti.drop_in_place {
        // SAFETY: `p` holds the live `T` the last iteration wrote; this frame owns it and
        // never reads it again.
        unsafe { drop_in_place(p) };
    }

    println!(
        "{label}: baseline={baseline} measured={measured} delta={}",
        measured as isize - baseline as isize
    );
    (baseline, measured)
}

// ───────────────────────────── the positive control ─────────────────────────

/// **A zero-allocation harness whose red nobody has seen is not a harness.** This
/// permanent positive control keeps the instrument's liveness in the binary, so a green
/// gate below can never mean "the counter was never armed".
#[test]
fn the_counter_sees_a_deliberate_allocation() {
    let observed = count_allocs(|| {
        let v = Vec::<u8>::with_capacity(64);
        black_box(&v);
    });
    println!("positive control: deliberate allocations observed = {observed}");
    assert!(observed > 0, "the counting allocator saw NOTHING -- the instrument is dead");
}

// ────────────────────────────────── gate 5 ──────────────────────────────────

/// **CORE C7 gate 5 / §3.3's `default_in_place` row** — the DERIVE-GENERATED
/// `default_in_place` allocates 0 bespoke, over a flat POD, a nest, an array pack and a
/// drop-owning type.
#[test]
fn derived_default_in_place_allocates_nothing() {
    for (label, (baseline, measured)) in [
        ("default_in_place (flat POD)", measure::<AllocPod>("default_in_place (flat POD)")),
        ("default_in_place (nested)", measure::<AllocNested>("default_in_place (nested)")),
        ("default_in_place (array pack)", measure::<AllocArrPack>("default_in_place (array pack)")),
        ("default_in_place (owns drop glue)", measure::<AllocOwned>("default_in_place (owns drop glue)")),
    ] {
        assert_eq!(
            measured, baseline,
            "{label}: the derived `default_in_place` allocated {} time(s) over {REPS} \
             writes that an identically-shaped no-op did not",
            measured as isize - baseline as isize
        );
    }
}
