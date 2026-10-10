//! Phase 8.5 Step 8 — `Bundle::for_each_component_bytes` panic-safety
//! acceptance tests.
//!
//! Locks down invariant **B4** (plan §6.3 SAFETY clause iv): if the user's
//! callback panics mid-iteration, the Components emitted so far have
//! already been transferred (logical ownership moved into the archetype
//! slot or, in these tests, observed by the callback); the components NOT
//! yet emitted leak unconditionally because `#[derive(Bundle)]` wraps
//! every destructured field in `ManuallyDrop<T>` BEFORE any callback runs.
//!
//! # Test matrix
//!
//! 1. `bundle_for_each_panics_no_double_drop` — 3-arity bundle, callback
//!    panics on the 2nd invocation. Verify total Drop count == 0 (no
//!    double-drop with the archetype-side "owns the bytes" semantic).
//!
//! 2. `bundle_for_each_panics_leak_unfinished_components` — same shape;
//!    verify the Drop count is EXACTLY 0 (the un-yielded components leak
//!    by design — leak < double-drop). The two assertions deliberately
//!    overlap so a regression that fired only one Drop (instead of zero
//!    or two) would surface against both tests.
//!
//! # Test isolation
//!
//! Each test uses ONE process-global `AtomicUsize` Drop counter
//! (`DROP_COUNT`) shared across all `PanicComp*` types in this file. A
//! `TEST_MUTEX` serialises the two test bodies so their counter resets
//! cannot interleave under parallel test execution. Pattern mirrored from
//! `tests/command_queue_panic_recovery.rs`.
//!
//! # Component-slot range
//!
//! 310..=320 per the Step 8 panic-safety spec. Three distinct
//! `Component` types are needed (the derive sorts by `ComponentId.0` and
//! rejects duplicate ids in the same Bundle — see Phase 8d Step 12 / Test
//! 6's `PanicTrackerA` + `PanicTrackerB` workaround).

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

use boyko_ecs::ecs::core::bundle::Bundle;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle;

// ── Test serialisation ──────────────────────────────────────────────────────

static TEST_MUTEX: Mutex<()> = Mutex::new(());

fn acquire_test_lock() -> MutexGuard<'static, ()> {
    match TEST_MUTEX.lock() {
        Ok(g) => g,
        // Panic-driven tests poison the mutex; recover the guard so
        // subsequent tests keep running.
        Err(poisoned) => poisoned.into_inner(),
    }
}

// ── Shared Drop counter ─────────────────────────────────────────────────────
//
// All three `PanicComp*` types funnel their Drop callback into the same
// counter so the test bodies need exactly one observe-and-assert. Each
// test resets the counter under the mutex before exercising the bundle.

static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

// ── Drop-counting components ────────────────────────────────────────────────
//
// Three distinct types (not three values of one type) because the derive
// sorts the bundle's component-id slice — a bundle with two fields of the
// same `Component` would emit a duplicate id, which the archetype layer
// rejects. Three types give the bundle three distinct slots.
//
// Each carries a `u32` payload purely so the struct has SOMETHING to
// observe in the bytes the callback receives.

const SLOT_PANIC_A: ComponentId = ComponentId(310);
const SLOT_PANIC_B: ComponentId = ComponentId(311);
const SLOT_PANIC_C: ComponentId = ComponentId(312);

#[repr(C)]
struct PanicCompA {
    _marker: u32,
}

#[repr(C)]
struct PanicCompB {
    _marker: u32,
}

#[repr(C)]
struct PanicCompC {
    _marker: u32,
}

impl Drop for PanicCompA {
    fn drop(&mut self) {
        DROP_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for PanicCompB {
    fn drop(&mut self) {
        DROP_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for PanicCompC {
    fn drop(&mut self) {
        DROP_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

impl Component for PanicCompA {
    fn component_id() -> ComponentId {
        SLOT_PANIC_A
    }
}

impl Component for PanicCompB {
    fn component_id() -> ComponentId {
        SLOT_PANIC_B
    }
}

impl Component for PanicCompC {
    fn component_id() -> ComponentId {
        SLOT_PANIC_C
    }
}

/// Three-arity bundle wrapping the three drop-counting components. Fields
/// declared in canonical order (A < B < C) so the derive's sort is a
/// no-op for this type — the panic-safety contract is independent of the
/// sort step.
#[derive(Bundle)]
struct PanicBundle {
    a: PanicCompA,
    b: PanicCompB,
    c: PanicCompC,
}

fn register_panic_comps() {
    register_layout::<PanicCompA>(SLOT_PANIC_A.0);
    register_layout::<PanicCompB>(SLOT_PANIC_B.0);
    register_layout::<PanicCompC>(SLOT_PANIC_C.0);
}

// ── Test 1 — B4: panic on 2nd callback → no double-drop ─────────────────────

/// Push a 3-arity `PanicBundle` through `for_each_component_bytes`. The
/// callback panics on the SECOND invocation. The bundle's `ManuallyDrop`
/// upfront-wrap suppresses Drop on ALL three components unconditionally;
/// the un-yielded ones (the 3rd, and the 2nd whose callback panicked)
/// leak by design, the yielded one (the 1st) was logically transferred
/// to the callback's side (which in this isolation test does NOT do
/// anything with the bytes — so leaking is the only outcome anyway).
///
/// Assertion: `DROP_COUNT == 0` after the panic unwinds.
///
/// If a regression re-enabled Drop on the ManuallyDrop wrappers, this
/// test would fire 3 Drops; if it re-enabled Drop only on the survivors,
/// it would fire 2 Drops; either way the assertion explodes.
#[test]
fn bundle_for_each_panics_no_double_drop() {
    let _serial = acquire_test_lock();
    register_panic_comps();
    DROP_COUNT.store(0, Ordering::Relaxed);

    let bundle = PanicBundle {
        a: PanicCompA { _marker: 1 },
        b: PanicCompB { _marker: 2 },
        c: PanicCompC { _marker: 3 },
    };

    let mut call_count = 0usize;
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        bundle.for_each_component_bytes(|_id, _bytes| {
            call_count += 1;
            if call_count == 2 {
                panic!("Phase 8.5 Step 8 deliberate panic on 2nd callback (B4)");
            }
        });
    }));

    assert!(result.is_err(), "callback panic must propagate up the stack");
    assert_eq!(
        DROP_COUNT.load(Ordering::Relaxed),
        0,
        "B4: every field is ManuallyDrop-wrapped upfront ⇒ no Drop runs after panic, \
         no double-drop with archetype ownership"
    );
}

// ── Test 2 — B4 quantitative: Drop count exactly matches what's been ─────────
//             processed BEFORE panic (which is 0 in our model)

/// Same setup as Test 1, but the assertion is phrased to lock down the
/// quantitative B4 guarantee: the Drop count equals the count of
/// components the callback successfully consumed BEFORE the panic.
///
/// In this isolated test the callback does not transfer ownership
/// anywhere — it just observes the bytes — so the "successfully consumed"
/// count is conceptually 0 for the Drop accounting. The bundle's
/// ManuallyDrop wrappers ensure ZERO Drops regardless of how many
/// callbacks ran before the panic. A failing variant would either:
///
/// * Fire 1 Drop  (the 1st field, which the callback "saw" — a regression
///   that re-enabled Drop on consumed components in the isolation test).
/// * Fire 2 Drops (consumed + the panicker — a regression that re-enabled
///   Drop on the panic-site field).
/// * Fire 3 Drops (all fields — a regression that removed the
///   ManuallyDrop wrap entirely).
#[test]
fn bundle_for_each_panics_leak_unfinished_components() {
    let _serial = acquire_test_lock();
    register_panic_comps();
    DROP_COUNT.store(0, Ordering::Relaxed);

    let bundle = PanicBundle {
        a: PanicCompA { _marker: 10 },
        b: PanicCompB { _marker: 20 },
        c: PanicCompC { _marker: 30 },
    };

    let mut call_count = 0usize;
    let mut bytes_seen_pre_panic = 0usize;
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        bundle.for_each_component_bytes(|_id, bytes| {
            call_count += 1;
            if call_count == 2 {
                panic!("Phase 8.5 Step 8 deliberate panic on 2nd callback (B4)");
            }
            // Touch the bytes so the compiler cannot elide the parameter
            // entirely (which would let a misbehaving derive emit zero
            // callbacks before the panic and still "pass").
            bytes_seen_pre_panic = bytes.len();
        });
    }));

    assert!(result.is_err(), "callback panic must propagate");
    assert_eq!(
        call_count, 2,
        "first callback succeeded, second panicked ⇒ exactly 2 callback entries"
    );
    assert!(
        bytes_seen_pre_panic > 0,
        "pre-panic callback observed a non-empty byte slice (sanity)"
    );
    assert_eq!(
        DROP_COUNT.load(Ordering::Relaxed),
        0,
        "B4: ManuallyDrop suppresses every field's Drop unconditionally ⇒ \
         the un-yielded components leak (leak < double-drop)"
    );
}
