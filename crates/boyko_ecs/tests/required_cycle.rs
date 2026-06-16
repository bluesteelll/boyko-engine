//! Feature 1 (required components) — the W2 cycle break.
//!
//! Spec `docs/REQUIRED-COMPONENTS-PLAN.md` FIX W2: a `#[require]` cycle
//! (`A → B → A`) must be detected and FAIL LOUD with a named diagnostic
//! (`RequiredError::Cycle`) at registration / first-expansion — a REAL runtime
//! check present in release, not a vanishing `debug_assert`. Test:
//! `require_cycle_panics`.
//!
//! ──────────────────────────────────────────────────────────────────────────
//! BUG-REQ-CYCLE-1 — FIXED (re-verified): the cycle path now PANICS with a
//! "cycle" diagnostic (`RequiredError::Cycle`) instead of deadlocking. These
//! tests are no longer `#[ignore]`d.
//! ──────────────────────────────────────────────────────────────────────────
//!
//! Original defect (deferred-resolution fix): the derive used to call the
//! required type's `B::component_id()` from INSIDE `A::component_id()`'s
//! `OnceLock::get_or_init` initializer (via `install_required` →
//! `register_required` → `builder.require(B::component_id(), …)`). With a cycle
//! (B requires A), that re-entered A's still-running `get_or_init` and `std`'s
//! `OnceLock` blocked the thread forever — the deadlock happened BEFORE the W2
//! `BuildingGuard` stack in `build_required_plan` was ever reached.
//!
//! Fix: `REQUIRES_DIRECT` now stores an UNCALLED `RequiredIdFn` resolver, and
//! `build_required_plan` invokes `(direct.id_fn)()` LAZILY at first-expansion —
//! well outside any mid-init `OnceLock`. A genuine cycle re-enters
//! `build_required_plan` with the id already on the `BUILDING` stack, so the
//! `BuildingGuard` fail-loud panic (`required_cycle_panic`, `#[cold]`,
//! release-active) fires. Both debug and release must PANIC, never hang.
//!
//! These tests assert the cycle PANICS with a "cycle" diagnostic. They are run
//! with a wall-clock guard (own `required_cycle` invocation, bounded
//! `--test-threads`) so a regression to a hang is detected rather than blocking
//! the suite indefinitely.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

// A ↔ B direct two-cycle. Both compile: each `#[require]` names the other's TYPE
// path. `Default` is required by the bare `#[require]` form (the ctor lowers to
// `T::default()`), though the cycle is expected to panic before any ctor runs.
#[derive(Component, Default)]
#[require(CycB)]
#[repr(C)]
struct CycA(u32);

#[derive(Component, Default)]
#[require(CycA)]
#[repr(C)]
struct CycB(u32);

#[derive(Bundle)]
struct CycABundle {
    a: CycA,
}

#[test]
#[should_panic(expected = "cycle")]
fn require_cycle_panics() {
    let mut ecs = EcsMaster::new();
    // Minting the ids no longer deadlocks: with the deferred-resolution fix the
    // derive stores an UNCALLED id resolver, so `CycA::component_id()` fully
    // initializes without re-entering its own `get_or_init`. The "cycle" panic
    // now fires at the spawn below, when `build_required_plan` resolves the
    // CycA → CycB → CycA edge and the `BuildingGuard` catches the re-entry.
    let _ = CycA::component_id();
    let _ = CycB::component_id();

    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(CycABundle { a: CycA(1) });
    });
}

// A longer 3-cycle (A → B → C → A) to prove the detection is not limited to the
// degenerate 2-cycle (it must catch any re-entry of an id already on the stack).
#[derive(Component, Default)]
#[require(Cyc3B)]
#[repr(C)]
struct Cyc3A(u32);

#[derive(Component, Default)]
#[require(Cyc3C)]
#[repr(C)]
struct Cyc3B(u32);

#[derive(Component, Default)]
#[require(Cyc3A)]
#[repr(C)]
struct Cyc3C(u32);

#[derive(Bundle)]
struct Cyc3ABundle {
    a: Cyc3A,
}

#[test]
#[should_panic(expected = "cycle")]
fn require_three_cycle_panics() {
    let mut ecs = EcsMaster::new();
    let _ = Cyc3A::component_id();
    let _ = Cyc3B::component_id();
    let _ = Cyc3C::component_id();

    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(Cyc3ABundle { a: Cyc3A(1) });
    });
}
