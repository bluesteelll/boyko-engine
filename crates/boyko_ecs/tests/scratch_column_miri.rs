//! `ScratchColumn` — Miri Tree-Borrows + data-race validation (mirrors
//! `dense_d1_miri`).
//!
//! Two groups:
//!  (a) single-threaded build + read under Miri-TB — UB-clean coverage of the
//!      `push` byte copies, `clear` (no-drop empty), the whole-buffer
//!      `as_mut_slice` refill, and the typed `row_ptr` reads.
//!  (b) the PARALLEL distinct-index test — N `std::thread::scope` workers each
//!      writing DISTINCT indices through a SHARED `ScratchSolveView::row_ptr`.
//!      This is the load-bearing P1 enabler: it proves the rigid gather-scratch
//!      race fix works — a `Copy + Send + Sync` view that yields per-element
//!      `*mut T` only, with the coloring distinct-index invariant making the
//!      concurrent writes non-aliasing. Miri-TB must report it Tree-Borrows +
//!      data-race clean.
//!
//!  `std::thread::scope` is used (NOT the threadpool) to avoid the
//!  crossbeam-deque Miri over-approximation (per the toolchain note).
//!
//! Run:
//! ```text
//! RUSTUP_TOOLCHAIN=nightly-x86_64-pc-windows-gnu \
//!   MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
//!   cargo miri test -p boyko-ecs --test scratch_column_miri
//! ```
//!
//! Component-id allocation: 112 (`Body16`) — free band below MAX_COMPONENTS.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::core::component::scratch::ScratchColumn;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Body16 {
    px: f32,
    py: f32,
    vx: f32,
    vy: f32,
}

const BODY_ID: ComponentId = ComponentId(112);

impl Component for Body16 {
    fn component_id() -> ComponentId {
        BODY_ID
    }
}

fn register() {
    component_registry::register_layout::<Body16>(BODY_ID.0);
}

fn body_column(reserve_rows: usize) -> ScratchColumn<Body16> {
    register();
    ScratchColumn::new(BODY_ID, reserve_rows)
}

// ── (a) single-threaded build + read under Miri-TB ──────────────────────────

#[test]
fn miri_build_and_read_ub_clean() {
    let mut col = body_column(128);

    {
        let mut build = col.build_view();
        for i in 0..32u32 {
            build.push(Body16 { px: i as f32, py: 0.0, vx: 0.0, vy: 0.0 });
        }
        // Whole-buffer refill mutation (the single-threaded surface).
        for (i, v) in build.as_mut_slice().iter_mut().enumerate() {
            v.vx = i as f32;
        }
        // Read-back through the whole-buffer slice.
        for (i, v) in build.as_slice().iter().enumerate() {
            assert_eq!(v.px, i as f32);
            assert_eq!(v.vx, i as f32);
        }
    }

    // clear() then refill — proves the no-drop empty + re-commit-free reuse is
    // UB-clean (no stale read, no drop glue on a Copy type).
    {
        let mut build = col.build_view();
        build.clear();
        assert_eq!(build.len(), 0);
        build.extend_from_slice(&[
            Body16 { px: 100.0, py: 0.0, vx: 0.0, vy: 0.0 },
            Body16 { px: 200.0, py: 0.0, vx: 0.0, vy: 0.0 },
        ]);
        assert_eq!(build.len(), 2);
    }

    // Typed row_ptr reads through the solve view (TB read provenance).
    let view = col.solve_view();
    for i in 0..view.len() {
        // SAFETY: `i < len`; `row_ptr`'s contract holds; the pointer is a valid
        // `*mut Body16` and we only read.
        let val = unsafe { *view.row_ptr(i) };
        let _ = val;
    }
}

// ── (b) parallel distinct-index writes through a shared ScratchSolveView ─────

#[test]
fn miri_parallel_distinct_index_writes_tb_clean() {
    let mut col = body_column(256);

    // Build a fully-live contiguous column — mirrors the solver's gather scratch
    // refilled at the top of a step.
    const N: usize = 64;
    {
        let mut build = col.build_view();
        for _ in 0..N {
            build.push(Body16 { px: 0.0, py: 0.0, vx: 0.0, vy: 0.0 });
        }
        assert_eq!(build.len(), N);
    }

    // The Copy + Send + Sync solve view — the SP4-fix primitive.
    let view = col.solve_view();

    // 4 workers, index-striped coloring: worker w owns indices {w, w+4, ...}.
    // Each owned set is DISJOINT, so the concurrent `row_ptr`-derived writes
    // never alias — the coloring distinct-index invariant (the P1 gather-scratch
    // access pattern).
    const WORKERS: usize = 4;
    std::thread::scope(|scope| {
        for w in 0..WORKERS {
            // `view` is `Copy`: the `move` closure copies it, so each worker gets
            // its own independent copy of the solve view.
            scope.spawn(move || {
                let mut i = w;
                while i < N {
                    // SAFETY: `i < N == len` and the coloring guarantees this
                    // worker is the SOLE writer of index `i` (disjoint stripes),
                    // so the `&mut`-equivalent write through the typed raw pointer
                    // does not alias any other worker's write — Tree-Borrows clean.
                    unsafe {
                        let p = view.row_ptr(i);
                        (*p).px = i as f32;
                        (*p).vx = (w as f32) + 1.0;
                    }
                    i += WORKERS;
                }
            });
        }
    });

    // Every index was written by exactly its owning worker.
    for i in 0..N {
        // SAFETY: `i < N` and live; single-threaded read here.
        let b = unsafe { *view.row_ptr(i) };
        assert_eq!(b.px, i as f32, "index {i} px written by its worker");
        assert_eq!(
            b.vx,
            (i % WORKERS) as f32 + 1.0,
            "index {i} owned by worker {}",
            i % WORKERS
        );
    }
}
