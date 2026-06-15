//! Decision 4 (D4) — Miri Tree-Borrows single-provenance proof for the typed
//! `write_row_typed` spawn_batch path (W2 acceptance gate).
//!
//! The load-bearing case is a **≥ 2-DATA-COLUMN** bundle: `resolve_column_ptrs`
//! must build every column base under ONE `&mut`-borrow of the pool bundle that
//! is ENDED before the row loop, and the row loop must write through the raw
//! `*mut u8` bases ONLY — never re-borrowing `component_pools_mut()` (the
//! 14a-F2 / 9.3c cached-pointer-then-reborrow UB class). A 1-pool test does NOT
//! exercise the cross-column provenance question and does not count.
//!
//! Miri-TB observes: (1) every `ptr::write::<Tk>` store targets a disjoint,
//! in-bounds, correctly-tagged slot derived from the single resolve pass;
//! (2) no aliasing/data-race violation across the per-row relocations; (3) the
//! `ManuallyDrop::take` relocations suppress source Drops (no double-drop at
//! teardown).
//!
//! # Run
//!
//! ```powershell
//! $env:MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! cargo +nightly miri test -p boyko-ecs --test miri_d4_typed_write
//! ```
//!
//! `-Zmiri-ignore-leaks` is set because the B4 partial-panic arm leaks the
//! uncommitted relocated bundles by design (documented "leak on panic"), and
//! the arena's Miri fallback allocation is process-lifetime.
//!
//! # File gate
//!
//! `#![cfg(miri)]` — only compiles under Miri; the native behavioural coverage
//! of the same path lives in `tests/d4_typed_write.rs`.

#![cfg(miri)]

use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle;

const SEQ: Ordering = Ordering::SeqCst;

// Distinct component-id range from the native suite (440-466) and other phases.
const SLOT_A: ComponentId = ComponentId(470);
const SLOT_B: ComponentId = ComponentId(471);
const SLOT_C: ComponentId = ComponentId(472);
const SLOT_DROP: ComponentId = ComponentId(473);

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct A(u64);

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct B(u32);

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct C(u16);

impl Component for A {
    fn component_id() -> ComponentId {
        SLOT_A
    }
}
impl Component for B {
    fn component_id() -> ComponentId {
        SLOT_B
    }
}
impl Component for C {
    fn component_id() -> ComponentId {
        SLOT_C
    }
}

#[derive(Bundle)]
struct Abc {
    a: A,
    b: B,
    c: C,
}

// ════════════════════════════════════════════════════════════════════════════
// W2 — three DATA columns, small N. Drives resolve_column_ptrs (single
// provenance) + the typed row loop under Miri-TB.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_typed_write_multi_column_single_provenance() {
    register_layout::<A>(SLOT_A.0);
    register_layout::<B>(SLOT_B.0);
    register_layout::<C>(SLOT_C.0);

    let mut ecs = EcsMaster::new();
    let n = 6usize;
    let spawned = ecs
        .spawn_batch((0..n as u32).map(|i| Abc {
            a: A(0xA000 + i as u64),
            b: B(0xB000 + i),
            c: C((0xC0 + i) as u16),
        }))
        .expect("typed spawn_batch (3 columns)");
    assert_eq!(spawned.len(), n);

    for (i, &e) in spawned.iter().enumerate() {
        let i = i as u32;
        // SAFETY: live initialised columns at this entity's row.
        let a = unsafe {
            *(ecs.get_component_raw(e, SLOT_A).expect("A") as *const A)
        };
        let b = unsafe {
            *(ecs.get_component_raw(e, SLOT_B).expect("B") as *const B)
        };
        let c = unsafe {
            *(ecs.get_component_raw(e, SLOT_C).expect("C") as *const C)
        };
        assert_eq!(a, A(0xA000 + i as u64), "row {i} A");
        assert_eq!(b, B(0xB000 + i), "row {i} B");
        assert_eq!(c, C((0xC0 + i) as u16), "row {i} C");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Drop-suppression under Miri-TB: a non-Copy drop-counting column. Each
// committed row is dropped exactly once at world teardown (no double-drop from
// the typed relocation).
// ════════════════════════════════════════════════════════════════════════════

static DROPS: AtomicUsize = AtomicUsize::new(0);

#[repr(C)]
struct Tracked(u32);

impl Drop for Tracked {
    fn drop(&mut self) {
        DROPS.fetch_add(1, SEQ);
    }
}

impl Component for Tracked {
    fn component_id() -> ComponentId {
        SLOT_DROP
    }
}

#[derive(Bundle)]
struct TwoCol {
    t: Tracked,
    a: A,
}

#[test]
fn miri_typed_write_two_col_drop_exact() {
    register_layout::<A>(SLOT_A.0);
    register_layout::<Tracked>(SLOT_DROP.0);
    DROPS.store(0, SEQ);

    let n = 4usize;
    {
        let mut ecs = EcsMaster::new();
        let spawned = ecs
            .spawn_batch((0..n as u32).map(|i| TwoCol {
                t: Tracked(i),
                a: A(i as u64),
            }))
            .expect("typed spawn_batch (Tracked + A)");
        assert_eq!(spawned.len(), n);
        // No Drop ran during relocation (ManuallyDrop::take).
        assert_eq!(DROPS.load(SEQ), 0, "no Drop during typed relocation");
    } // world drop runs the pool drop_fn exactly once per committed row

    assert_eq!(
        DROPS.load(SEQ),
        n,
        "exactly one Drop per committed row at teardown (no double-drop)"
    );
}
