//! Dense plan D1 — Miri Tree-Borrows + data-race validation for `DenseStore`.
//!
//! Two groups:
//!  (a) single-threaded structural ops (insert / remove / compact / drop) under
//!      Miri-TB — UB-clean coverage of the byte copies, `drop_at`, and the
//!      `pop_entity_no_drop` neutralisation in the store's `Drop`.
//!  (b) the PARALLEL distinct-slot test — N `std::thread::scope` workers each
//!      writing DISTINCT slots through a SHARED `DenseSolveView::row_ptr`. This
//!      validates the SP4-fix primitive for Stage P: a `Copy + Send + Sync`
//!      view that yields per-element `*mut u8` only, with the coloring
//!      distinct-slot invariant making the concurrent writes non-aliasing.
//!      Miri-TB must report it Tree-Borrows + data-race clean.
//!
//! Run (per the toolchain note):
//! ```text
//! RUSTUP_TOOLCHAIN=nightly-x86_64-pc-windows-gnu \
//!   MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
//!   cargo miri test -p boyko-ecs --test dense_d1_miri
//! ```
//!
//! Component-id allocation: 105 (`Body`) — free band below MAX_COMPONENTS.

use boyko_ecs::ecs::core::change_detection::Tick;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::core::component::dense::DenseStore;
use boyko_ecs::ecs::identifiers::primitives::{ComponentId, EntityId};

/// Fixed change-detection tick the D1 Miri ops stamp on insert (D4 `current_tick`
/// arg — the structural-op tests use any nonzero tick).
const D1_TICK: Tick = Tick::new(1);

#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Body {
    px: f32,
    py: f32,
    vx: f32,
    vy: f32,
}

const BODY_ID: ComponentId = ComponentId(105);

impl Component for Body {
    fn component_id() -> ComponentId {
        BODY_ID
    }
}

fn register() {
    component_registry::register_layout::<Body>(BODY_ID.0);
}

fn body_bytes(b: &Body) -> &[u8] {
    // SAFETY: `Body` is `#[repr(C)]` POD; its byte span is a valid representation.
    unsafe { std::slice::from_raw_parts((b as *const Body).cast::<u8>(), std::mem::size_of::<Body>()) }
}

fn e(i: usize) -> EntityId {
    EntityId(i)
}

// ── (a) single-threaded structural ops under Miri-TB ────────────────────────

#[test]
fn miri_structural_ops_ub_clean() {
    register();
    let mut store = DenseStore::new(BODY_ID, 128);

    for i in 0..32usize {
        store.insert(e(i), body_bytes(&Body { px: i as f32, py: 0.0, vx: 0.0, vy: 0.0 }), D1_TICK);
    }
    // Scatter removes (tombstone + free-list, drop_at on each).
    for i in (0..32usize).step_by(3) {
        store.remove(e(i));
    }
    // Reuse some freed slots.
    for i in 100..108usize {
        store.insert(e(i), body_bytes(&Body { px: i as f32, py: 1.0, vx: 0.0, vy: 0.0 }), D1_TICK);
    }
    assert!(store.check_invariant());

    // Read back every live slot through the solve view (TB read provenance).
    store.for_each_live(|slot, _entity| {
        let view = store.solve_view();
        // SAFETY: `slot` is live (from `for_each_live`); `row_ptr`'s contract
        // holds; the pointer is valid for one `Body`.
        let val = unsafe { *view.row_ptr(slot as usize).cast::<Body>() };
        let _ = val;
    });

    store.compact();
    assert!(store.check_invariant());
    // store drops here — neutralised column Drop, exactly-once on survivors.
}

// ── W3 liveness guard: row_ptr on a tombstoned slot trips the assert ────────

#[test]
#[cfg(debug_assertions)]
fn row_ptr_on_tombstoned_slot_trips_liveness_assert() {
    // Dense plan proof obligation #5 (W3): the liveness-checked `row_ptr`
    // debug_assert fires on a tombstoned slot — Miri-TB confirms it traps
    // before any out-of-contract pointer is produced.
    register();
    let mut store = DenseStore::new(BODY_ID, 16);
    let s = store.insert(e(7), body_bytes(&Body { px: 0.0, py: 0.0, vx: 0.0, vy: 0.0 }), D1_TICK);
    assert!(store.remove(e(7)), "slot {s} tombstoned");

    let view = store.solve_view();
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: this call is EXPECTED to panic on the W3 liveness
        // debug_assert before dereferencing; we only want to observe the trap.
        let _ = unsafe { view.row_ptr(s as usize) };
    }));
    assert!(caught.is_err(), "row_ptr on a tombstoned slot must trip the W3 liveness assert");
}

// ── (b) parallel distinct-slot writes through a shared DenseSolveView ────────

#[test]
fn miri_parallel_distinct_slot_writes_tb_clean() {
    register();
    let mut store = DenseStore::new(BODY_ID, 256);

    // Seed a contiguous, fully-live column (no tombstones) so every slot
    // 0..N is live — mirrors the post-compact solver column.
    const N: usize = 64;
    for i in 0..N {
        store.insert(e(i), body_bytes(&Body { px: 0.0, py: 0.0, vx: 0.0, vy: 0.0 }), D1_TICK);
    }
    assert_eq!(store.live_count(), N);

    // The Copy + Send + Sync solve view — the SP4-fix primitive.
    let view = store.solve_view();

    // 4 workers, slot-striped coloring: worker w owns slots {w, w+4, w+8, ...}.
    // Each owned set is DISJOINT, so the concurrent `row_ptr`-derived writes
    // never alias — the coloring distinct-slot invariant.
    const WORKERS: usize = 4;
    std::thread::scope(|scope| {
        for w in 0..WORKERS {
            // `view` is `Copy`: the `move` closure copies it, so each worker
            // gets its own independent copy of the solve view.
            scope.spawn(move || {
                let mut slot = w;
                while slot < N {
                    // SAFETY: `slot < N == len` and live (seeded above), so
                    // `row_ptr`'s contract holds. The coloring guarantees this
                    // worker is the SOLE writer of `slot` (disjoint stripes), so
                    // the `&mut`-equivalent write through the raw pointer does
                    // not alias any other worker's write — Tree-Borrows clean.
                    unsafe {
                        let p = view.row_ptr(slot).cast::<Body>();
                        (*p).px = slot as f32;
                        (*p).vx = (w as f32) + 1.0;
                    }
                    slot += WORKERS;
                }
            });
        }
    });

    // Every slot was written by exactly its owning worker.
    for slot in 0..N {
        // SAFETY: slot < N and live; single-threaded read here.
        let b = unsafe { *view.row_ptr(slot).cast::<Body>() };
        assert_eq!(b.px, slot as f32, "slot {slot} px written by its worker");
        assert_eq!(b.vx, (slot % WORKERS) as f32 + 1.0, "slot {slot} owned by worker {}", slot % WORKERS);
    }
    assert!(store.check_invariant());
}
