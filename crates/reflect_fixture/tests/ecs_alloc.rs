//! **ECS EG1 gate 3** — `components_of_into` allocates **zero**, measured under a
//! counting `#[global_allocator]` rather than claimed.
//!
//! `docs/REFLECTION-PLAN-ECS.md` §7's allocation table opens *"Every zero below is an
//! **asserted equality**, not a claim"*, and names this file as the instrument. The row it
//! discharges is `components_of_into` (all three sources — EG1 owns two of them, and EG3
//! re-runs this binary when source 3 lands).
//!
//! # Why this is a SEPARATE binary from EG1's gates, and from gate 6
//!
//! A `#[global_allocator]` is **one per binary**, and this one cannot be present in either
//! of the other two:
//!
//! * `eg1_miri_tb.rs` (gate 6) must be reached by CI's Miri sweep. A `System`-forwarding
//!   `#[global_allocator]` is not transparent under Miri + Tree Borrows on
//!   `x86_64-pc-windows-gnu` — it aborts in libtest's own shutdown with `running 0 tests`.
//!   That was MEASURED in this package two rungs ago and is recorded at
//!   `crates/reflect_fixture/tests/c7_alloc_delta.rs:26~`, which carries `#![cfg(not(miri))]`
//!   for exactly this reason. Fold the two and gate 6 becomes a vacuous pass; leave the
//!   guard off and CI's Miri row reds for a reason that has nothing to do with reflection.
//! * `eg1_components_of.rs` (gates 1/1b/2/2b/4/5) is Miri-visible for the same reason, and
//!   an allocator here would drag `#![cfg(not(miri))]` onto all of it.
//!
//! # The instrument
//!
//! Verbatim in shape from `c7_alloc_delta.rs`, including the two facts that file MEASURED
//! rather than preferred: the counter is **thread-local** (a process-global one counts
//! libtest's own allocations on other threads and produced a `delta = -1`, which is the
//! diagnostic that a measured path cannot allocate less than nothing), and the binary is
//! excluded from Miri.
//!
//! # The invocation is part of the gate
//!
//! ```text
//! cargo test -p reflect-fixture --features reflect-fixture/reflect --test ecs_alloc
//! ```
//!
//! The output must read `running [1-9]`; a plain `cargo test -p reflect-fixture` compiles
//! this file to nothing and exits 0.
#![cfg(feature = "reflect")]
#![cfg(not(miri))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;

use boyko_ecs::ecs::core::component::component::Component as ComponentTrait;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::{EcsMaster, Entity};
use boyko_macros::Component;
use boyko_reflect::ecs::{IdEntry, IdKind, components_of_into, display_name};

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
/// noise, and the same figure `c7_alloc_delta.rs` and `c4_prim_zero_alloc.rs` use.
const REPS: usize = 1000;

// ───────────────────────────────── the subjects ─────────────────────────────

/// The signature-storage citizen.
#[derive(Component, Default)]
#[component(reflect)]
#[repr(C)]
struct AllocTable {
    x: f32,
    y: f32,
}

/// The dense citizen — source 2's subject, so the measured call walks BOTH sources.
#[derive(Component, Default)]
#[component(reflect, storage = "dense")]
#[repr(C)]
struct AllocDense {
    lane: [f32; 4],
}

/// Builds the world OUTSIDE any armed window. Spawning allocates — arenas, slabs, the
/// dense store — and every one of those allocations belongs to the fixture, not to the
/// measured call.
fn world() -> (EcsMaster, Entity) {
    let mut ecs = EcsMaster::new();
    let archetype = ecs.get_or_create_archetype(&[
        <AllocTable as ComponentTrait>::component_id(),
        <AllocDense as ComponentTrait>::component_id(),
    ]);
    let entity = ecs
        .spawn_two(archetype, AllocTable::default(), AllocDense::default())
        .expect("invariant: a fresh archetype accepts its own two-component push");
    (ecs, entity)
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

// ────────────────────────────────── gate 3 ──────────────────────────────────

/// **EG1 gate 3 / §7's `components_of_into` row** — the call allocates nothing, over a
/// subject that exercises source 1 (the filtered archetype id list) and source 2 (the
/// dense registry probe) in the same window.
#[test]
fn components_of_into_allocates_nothing() {
    let (ecs, entity) = world();
    let mut buf = [IdEntry { id: ComponentId(usize::MAX), kind: IdKind::Bitset }; 32];

    // The subject is verified BEFORE the window opens: a call that refused early would
    // also allocate nothing, and this gate must not be satisfiable by a broken read.
    assert_eq!(
        components_of_into(&ecs, entity, &mut buf),
        Ok(2),
        "the measured call must actually enumerate the two components, or a zero here is \
         the zero of a refusal rather than of a zero-allocation walk"
    );

    let observed = count_allocs(|| {
        for _ in 0..REPS {
            let n = components_of_into(black_box(&ecs), black_box(entity), black_box(&mut buf))
                .expect("live entity");
            black_box(n);
        }
    });

    println!("components_of_into: allocations over {REPS} calls = {observed}");
    assert_eq!(
        observed, 0,
        "`components_of_into` allocated {observed} time(s) over {REPS} calls. §7's table \
         states this row as an asserted equality, and the buffer-filling design exists \
         precisely so no `Vec` is minted per entity per inspector frame"
    );
}

/// **The `display_name` half of the same row.** `ComponentLayout::type_name` is a
/// `&'static str` lookup; a `String` anywhere on that path would allocate once per row per
/// inspector frame.
#[test]
fn display_name_allocates_nothing() {
    let table = <AllocTable as ComponentTrait>::component_id();
    assert!(
        !display_name(table).is_empty(),
        "the subject must resolve to a real name, or the measured path is the \
         unregistered early-out and proves nothing"
    );

    let observed = count_allocs(|| {
        for _ in 0..REPS {
            black_box(display_name(black_box(table)));
        }
    });

    println!("display_name: allocations over {REPS} calls = {observed}");
    assert_eq!(observed, 0, "`display_name` allocated {observed} time(s) over {REPS} calls");
}
