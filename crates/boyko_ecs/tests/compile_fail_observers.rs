//! Phase 14b — `compile_fail` acceptance tests proving the read-only
//! `DeferredEcsMaster` runner view does NOT expose the world-mutating observer
//! APIs (architect plan §11 case 15 / O1).
//!
//! A runner receives a `DeferredEcsMaster<'_>` (a read-only world view + a
//! deferred command queue). It must NOT be able to:
//!   * mutate the observer registry (`add_observer` / `remove_observer` are
//!     `&mut EcsMaster`, never on the view), nor
//!   * obtain a `&mut`-into-pool (`get_component_mut` is `&mut EcsMaster`).
//!
//! These are the O1 invariants that keep the fire walk sound (no registry
//! mutation or aliasing `&mut` reachable from inside a fire).
//!
//! Each `.rs` file under `tests/compile_fail_observers/` is compiled in
//! isolation; the matching `.stderr` baseline records the expected diagnostic.
//! Regenerate after a rustc point release that shifts the "no method named ..."
//! wording via:
//!
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo test -p boyko-ecs --test compile_fail_observers
//! ```
//!
//! Gated behind `#[cfg(not(miri))]` — trybuild is not wired under Miri (mirrors
//! `compile_fail_hooks.rs`).

#![cfg(not(miri))]

#[test]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail_observers/*.rs");
}
