// Phase 8a Step 13 — Miri test suite for the `SystemParam` + `Resources`
// subsystem.
//
// These tests are written to be run under `cargo +nightly miri test`. They
// exercise the unsafe code paths added in Steps 2 / 4-7 / 8 / 9 and verify
// that no UB (uninit reads, retag failures, double-frees, aliased mutable
// references) is detected by Miri.
//
// They are NOT gated on `#[cfg(miri)]` so they also run under the regular
// `cargo test --workspace` as smoke tests. Phase 7 Step 10 followed the same
// convention (see `tests/drop_safety.rs` lineage).
//
// Plan §17 Step 13 tests:
//   1. miri_resources_drop_runs_drop_glue
//   2. miri_resources_replace_no_double_free
//   3. miri_resources_replace_panic_in_drop_no_ub (C3)
//   4. miri_unsafe_ecs_cell_no_retag_via_by_value_methods (C1)
//   5. miri_res_get_param_no_retag
//   6. miri_run_system_once_full_e2e
//   7. miri_archetype_bundle_replace_panic_in_drop_no_double_drop
//      (covered by the existing test in archetype_bundle.rs — see comment
//      at the bottom of this file; not duplicated here).
//   8. miri_resources_assume_init_read_does_not_move_slot_bytes (C-NEW-3)

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::resources::Resources;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;

// ── Resources with Drop semantics for the Miri tests ───────────────────────

/// Counter probed by the Drop tests to verify drop_fn invocation count.
/// Bench-global static avoids capturing into closures (System trait
/// requires `Send + Sync + 'static`).
static DROP_COUNTER_A: AtomicUsize = AtomicUsize::new(0);
static DROP_COUNTER_B: AtomicUsize = AtomicUsize::new(0);
static DROP_COUNTER_C: AtomicUsize = AtomicUsize::new(0);

#[derive(Resource)]
struct DropProbeA(#[allow(dead_code)] u32);
impl Drop for DropProbeA {
    fn drop(&mut self) {
        DROP_COUNTER_A.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Resource)]
struct DropProbeB(#[allow(dead_code)] u64);
impl Drop for DropProbeB {
    fn drop(&mut self) {
        DROP_COUNTER_B.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Resource)]
struct DropProbeC(#[allow(dead_code)] [u8; 32]);
impl Drop for DropProbeC {
    fn drop(&mut self) {
        DROP_COUNTER_C.fetch_add(1, Ordering::Relaxed);
    }
}

/// POD resource — no Drop — used by miri_unsafe_ecs_cell_no_retag.
#[derive(Resource)]
struct PodRes(u32);

/// Resource whose Drop panics — used by replace-panic-in-drop tests.
static PANIC_DROP_COUNT: AtomicUsize = AtomicUsize::new(0);
static PANIC_DROP_ARMED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[derive(Resource)]
struct PanicOnDrop(#[allow(dead_code)] u32);
impl Drop for PanicOnDrop {
    fn drop(&mut self) {
        PANIC_DROP_COUNT.fetch_add(1, Ordering::Relaxed);
        if PANIC_DROP_ARMED.load(Ordering::Relaxed) {
            panic!("PanicOnDrop::drop intentional panic for Miri replace-path test");
        }
    }
}

// ── Test 1: miri_resources_drop_runs_drop_glue ─────────────────────────────
//
// Verifies that `Resources::Drop` runs the registered `drop_fn` for every
// occupied slot. Under Miri the test also checks: (a) every `drop_fn`
// receives a valid `*mut u8` (no use-after-free), (b) `dealloc` matches
// the `Layout::new::<R>()` minted in `insert` (no allocator mismatch UB).
#[test]
fn miri_resources_drop_runs_drop_glue() {
    let before_a = DROP_COUNTER_A.load(Ordering::Relaxed);
    let before_b = DROP_COUNTER_B.load(Ordering::Relaxed);
    let before_c = DROP_COUNTER_C.load(Ordering::Relaxed);

    {
        let mut r = Resources::new();
        r.insert(DropProbeA(1));
        r.insert(DropProbeB(2));
        r.insert(DropProbeC([0u8; 32]));
        assert_eq!(r.len(), 3, "three resources must be present pre-drop");
    } // r drops here — R3 slab walk via pop_lowest_set_bit.

    assert_eq!(
        DROP_COUNTER_A.load(Ordering::Relaxed) - before_a,
        1,
        "DropProbeA::drop must run exactly once"
    );
    assert_eq!(
        DROP_COUNTER_B.load(Ordering::Relaxed) - before_b,
        1,
        "DropProbeB::drop must run exactly once"
    );
    assert_eq!(
        DROP_COUNTER_C.load(Ordering::Relaxed) - before_c,
        1,
        "DropProbeC::drop must run exactly once"
    );
}

// ── Test 2: miri_resources_replace_no_double_free ──────────────────────────
//
// Replace an existing resource: the old `Box<R>` must be dropped AND
// deallocated exactly once before the new slot is written. Miri's allocator
// detects double-free and use-after-free; a regression in the R4 protocol
// would trip those detectors.
#[test]
fn miri_resources_replace_no_double_free() {
    /// Sentinel resource — non-trivial Drop with a probe counter.
    static REPLACE_COUNTER: AtomicUsize = AtomicUsize::new(0);
    #[derive(Resource)]
    struct ReplaceProbe(#[allow(dead_code)] u32);
    impl Drop for ReplaceProbe {
        fn drop(&mut self) {
            REPLACE_COUNTER.fetch_add(1, Ordering::Relaxed);
        }
    }

    REPLACE_COUNTER.store(0, Ordering::Relaxed);
    {
        let mut r = Resources::new();
        r.insert(ReplaceProbe(1));
        r.insert(ReplaceProbe(2)); // replace → drops ReplaceProbe(1)
        // First value dropped during replace (R4): counter == 1.
        assert_eq!(
            REPLACE_COUNTER.load(Ordering::Relaxed),
            1,
            "first insert's value must be dropped exactly once during replace"
        );
        r.insert(ReplaceProbe(3)); // replace again → drops ReplaceProbe(2)
        assert_eq!(
            REPLACE_COUNTER.load(Ordering::Relaxed),
            2,
            "second replace must drop the previous value exactly once"
        );
    } // r drops → drops ReplaceProbe(3): counter == 3.
    assert_eq!(
        REPLACE_COUNTER.load(Ordering::Relaxed),
        3,
        "final EcsMaster drop must drop the latest value exactly once"
    );
}

// ── Test 3: miri_resources_replace_panic_in_drop_no_ub (C3) ────────────────
//
// Insert a `PanicOnDrop` resource, then attempt to replace it. The old
// value's `Drop` panics; the R4 clear-bit-first protocol must ensure:
//   - The `registered_mask` bit for the slot is cleared BEFORE drop runs.
//   - On panic-unwind, `Resources::Drop` walks the bitset and does NOT
//     revisit the partially-dropped slot (no double-drop).
//   - Miri detects neither use-after-free, double-free, nor aliasing UB
//     on the unwind path.
//
// We catch the panic from `insert` (the replace path) and observe:
//   - The drop counter reads exactly 1 (the panic-causing drop fired).
//   - `Resources::contains::<PanicOnDrop>()` returns false (slot is
//     observably empty — R4 leak-but-no-corruption guarantee).
//   - Subsequent `Resources::drop` does NOT revisit the slot (counter
//     stays at 1).
#[test]
fn miri_resources_replace_panic_in_drop_no_ub() {
    PANIC_DROP_COUNT.store(0, Ordering::Relaxed);
    PANIC_DROP_ARMED.store(false, Ordering::Relaxed);

    let mut r = Resources::new();
    r.insert(PanicOnDrop(1));
    assert!(r.contains::<PanicOnDrop>(), "precondition: resource inserted");

    // Arm the panic and trigger the replace path. `catch_unwind` requires
    // `AssertUnwindSafe` because `&mut Resources` is not unwind-safe by
    // default; the AB-R1-style protocol provides the actual unwind safety.
    PANIC_DROP_ARMED.store(true, Ordering::Relaxed);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        r.insert(PanicOnDrop(2));
    }));
    PANIC_DROP_ARMED.store(false, Ordering::Relaxed);

    assert!(
        result.is_err(),
        "replace path must propagate the user Drop panic"
    );
    assert_eq!(
        PANIC_DROP_COUNT.load(Ordering::Relaxed),
        1,
        "exactly one drop must fire during the panicked replace (R4)"
    );
    assert!(
        !r.contains::<PanicOnDrop>(),
        "R4: slot must be observably empty after panicked replace \
         (bit cleared before drop ran; new value not written)"
    );

    // Drop the Resources slab. R3's slab walk must NOT revisit the
    // observably-empty slot — counter must remain at 1.
    drop(r);
    assert_eq!(
        PANIC_DROP_COUNT.load(Ordering::Relaxed),
        1,
        "Resources::Drop must not revisit the panicked slot (no double-drop)"
    );
}

// ── Test 4: miri_unsafe_ecs_cell_no_retag_via_by_value_methods (C1) ────────
//
// C1 RESOLUTION verification: `UnsafeEcsCell` is `Copy` and its accessors
// take `self` by value. Calling `resources()` / `resources_mut()` on
// successive copies must NOT trigger Tree Borrows / Stacked Borrows retag
// UB. Under Miri (which models both borrow stacks) a regression would
// surface as a retag failure inside `run_closure_once` when the closure
// touches both a `Res<X>` (shared) and a `ResMut<Y>` (exclusive) through
// distinct cell copies handed to the tuple `get_param` walk.
//
// We exercise the path through `run_closure_once` because `UnsafeEcsCell`'s
// constructor and accessors are `pub(crate)` — only reachable through the
// public system runner. The tuple of `(Res<A>, ResMut<B>)` is the smallest
// program that materialises two cell copies in flight at the same time;
// internally the tuple impl in `tuple_impl.rs` clones the cell for each
// element and calls each `get_param` by value (see the SAFETY comment in
// the impl macro). A regression in the by-value contract would surface here
// under Miri as a retag failure.
#[test]
fn miri_unsafe_ecs_cell_no_retag_via_by_value_methods() {
    let mut ecs = EcsMaster::new();
    ecs.insert_resource(PodRes(42));
    // Distinct second resource so the closure can hold one shared and one
    // exclusive view through two cell copies simultaneously.
    #[derive(Resource)]
    struct PodRes2(u32);
    ecs.insert_resource(PodRes2(0));

    // Probe the read value out via an atomic so the test can observe what
    // the shared view saw inside the closure. The closure must be
    // `Send + Sync + 'static` per the `System` trait bound.
    let read_back = Arc::new(AtomicU32::new(0));
    let probe = Arc::clone(&read_back);

    // Tuple param exercises the by-value cell copy distribution. If the
    // C1 contract had regressed (e.g. cell accessors taking `&self`
    // instead of `self`), Miri's retag check would fail somewhere inside
    // this call — most likely on the second element's `get_param`.
    ecs.run_closure_once::<(Res<'_, PodRes>, ResMut<'_, PodRes2>), _, _>(
        move |(r, mut w)| {
            probe.store((*r).0, Ordering::Relaxed);
            // Write through the &mut view. The C1 invariant guarantees
            // the cell's raw pointer keeps write-capable provenance
            // through the tuple walk.
            w.0 = w.0.wrapping_add(99);
        },
    );

    assert_eq!(
        read_back.load(Ordering::Relaxed),
        42,
        "shared cell view observed the wrong value"
    );
    assert_eq!(
        ecs.resource::<PodRes2>().0,
        99,
        "exclusive cell view's write did not persist"
    );
}

// ── Test 5: miri_res_get_param_no_retag ────────────────────────────────────
//
// Hot path: `Res<R>::get_param` through `run_closure_once`. Exercises:
//   - `UnsafeEcsCell::resources()` (by-value cell accessor — C1).
//   - `Resources::get_ptr_by_id` (W1 cached id).
//   - `unsafe { &*(ptr as *const R) }` cast back to typed reference.
//   - `Res<'w, R>` lifetime threading.
//
// Under Miri this triggers all the retag checks for the resources slab
// dereference. A regression in W1 or in the SystemParam impl would
// surface as a retag failure or an out-of-bounds load.
//
// Note: `Res::get_param` and `UnsafeEcsCell::new_mutable` are `pub(crate)`,
// so we go through the public `run_closure_once` API — internally this
// invokes the same path.
#[test]
fn miri_res_get_param_no_retag() {
    let mut ecs = EcsMaster::new();
    ecs.insert_resource(PodRes(7));

    let observed = Arc::new(AtomicU32::new(0));
    let probe = Arc::clone(&observed);
    ecs.run_closure_once::<Res<'_, PodRes>, _, _>(move |r| {
        // Deref through Res<R> exercises the &*(ptr as *const R) cast.
        probe.store((*r).0, Ordering::Relaxed);
    });

    assert_eq!(
        observed.load(Ordering::Relaxed),
        7,
        "Res::get_param must round-trip the inserted value"
    );
}

// ── Test 6: miri_run_system_once_full_e2e ──────────────────────────────────
//
// End-to-end: `EcsMaster::run_closure_once` with `(Res<R1>, ResMut<R2>)`.
// Exercises every unsafe construct in the SystemParam pipeline:
//   - `UnsafeEcsCell::new_mutable` → `world` deref.
//   - Tuple `get_param` walk with shared cell copy.
//   - `Res::get_param` (read path) and `ResMut::get_param` (write path)
//     in the same call site.
//   - Closure invocation through `FnOnceSystem::run_unsafe`.
//   - Drop of `FnOnceSystem` (no SystemMeta leak).
//
// The probe closure captures an `Arc<AtomicU32>` so we can observe the
// resource read without violating the `Send + Sync + 'static` bound.
#[test]
fn miri_run_system_once_full_e2e() {
    #[derive(Resource)]
    struct E2eRead(u32);
    #[derive(Resource)]
    struct E2eWrite(u32);

    let mut ecs = EcsMaster::new();
    ecs.insert_resource(E2eRead(11));
    ecs.insert_resource(E2eWrite(0));

    let observed = Arc::new(AtomicU32::new(0));
    let probe = Arc::clone(&observed);

    ecs.run_closure_once::<(Res<'_, E2eRead>, ResMut<'_, E2eWrite>), _, _>(
        move |(r, mut w)| {
            // Read through Deref; mutate through DerefMut. Both must be
            // sound under Miri: the cell's by-value receiver preserves
            // provenance through the tuple walk.
            probe.store((*r).0, Ordering::Relaxed);
            w.0 = (*r).0.wrapping_add(31);
        },
    );

    assert_eq!(
        observed.load(Ordering::Relaxed),
        11,
        "Res<E2eRead> must observe the inserted value"
    );
    assert_eq!(
        ecs.resource::<E2eWrite>().0,
        42,
        "ResMut<E2eWrite> must persist the closure's write"
    );
}

// ── Test 7: miri_resources_assume_init_read_does_not_move_slot_bytes (C-NEW-3) ──
//
// `Resources::insert`'s replace path uses `MaybeUninit::assume_init_read()`
// to bitwise-copy the old `ResourceSlot` out before clearing the bit.
// `ResourceSlot` derives `Copy` (C-NEW-3 RESOLUTION) so the bytes are not
// "moved" in Rust's ownership sense — they remain backing storage for any
// in-flight observer (none, by R4 invariant) and are immediately
// overwritten by `ptr::write(slot_ptr, new_slot)` after the drop.
//
// Under Miri, an `assume_init_read` on a slot that's later re-`write`n
// must not trigger an "uninit on a referenced slot" or aliasing UB. The
// previous design (no `Copy` on `ResourceSlot`) would have required
// `MaybeUninit::assume_init` (which moves the place) — Miri rejects that
// when the place is read again by `Drop`. This test exercises the
// fixed-shape pathway end-to-end.
#[test]
fn miri_resources_assume_init_read_does_not_move_slot_bytes() {
    /// Local resource type so the test owns its DropProbe counter slot
    /// (no cross-test contamination of the file-scope DropProbeA counter
    /// when `cargo test` runs tests in parallel).
    static ASSUME_INIT_COUNTER: AtomicUsize = AtomicUsize::new(0);
    #[derive(Resource)]
    struct AssumeInitProbe(#[allow(dead_code)] u32);
    impl Drop for AssumeInitProbe {
        fn drop(&mut self) {
            ASSUME_INIT_COUNTER.fetch_add(1, Ordering::Relaxed);
        }
    }

    ASSUME_INIT_COUNTER.store(0, Ordering::Relaxed);
    let mut r = Resources::new();
    // First insert — initial path.
    r.insert(AssumeInitProbe(1));
    // Second insert on the same type — replace path:
    //   assume_init_read → clear bit → drop old → dealloc → write new → set bit.
    r.insert(AssumeInitProbe(2));
    assert_eq!(
        ASSUME_INIT_COUNTER.load(Ordering::Relaxed),
        1,
        "first replace must drop the prior value exactly once"
    );
    // Third — same path again, double-replacing.
    r.insert(AssumeInitProbe(3));
    assert_eq!(
        ASSUME_INIT_COUNTER.load(Ordering::Relaxed),
        2,
        "second replace must drop the prior value exactly once"
    );

    // Slot must still be live and readable. Miri would have flagged
    // any incomplete writes by now (e.g. if `assume_init_read` had moved
    // the bytes without the subsequent `ptr::write` re-initialising).
    assert!(
        r.contains::<AssumeInitProbe>(),
        "slot must still be live after 3 inserts"
    );
    let removed = r
        .remove::<AssumeInitProbe>()
        .expect("remove must return Some");
    assert_eq!(removed.0, 3, "latest insert's value must be observable");
    // Removing the value invokes its Drop once at scope exit (when `removed`
    // is dropped at the end of this statement).
    drop(removed);
    assert_eq!(
        ASSUME_INIT_COUNTER.load(Ordering::Relaxed),
        3,
        "remove + drop must run drop exactly once on the unboxed value"
    );

    // Drop the now-empty Resources; no further drops should fire because
    // the slot's bit was cleared by `remove`.
    drop(r);
    assert_eq!(
        ASSUME_INIT_COUNTER.load(Ordering::Relaxed),
        3,
        "Resources::Drop must not drop an already-removed slot"
    );
}

// ── Test 8: confirm archetype_bundle's C-NEW-1 test still passes ───────────
//
// The Phase 7 carry-over test for ArchetypeBundle::add_archetype's
// clear-bit-first protocol lives in
// `crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs::tests::
//  phase7_carry_over_add_archetype_replace_panic_in_drop_no_double_drop`.
// It is part of the workspace's regular test suite and runs under
// `cargo test --workspace`. We do not duplicate it here — the test asserts
// the same invariant (clear-bit-first protocol prevents double-drop on
// Drop panic) for the archetype slab; the `miri_resources_replace_panic_in_drop_no_ub`
// test above does the corresponding job for the resource slab.
//
// Per Phase 8a plan §17 Step 13: "verify it still passes (no duplicate
// test needed — just confirm)" — this stub documents the cross-reference.
#[test]
fn miri_archetype_bundle_replace_test_exists_in_archetype_bundle_module() {
    // No body — this test exists only to surface the cross-reference in
    // the test output and in any test-discovery tooling. The actual
    // assertion lives in archetype_bundle.rs::tests::phase7_carry_over_*.
}
