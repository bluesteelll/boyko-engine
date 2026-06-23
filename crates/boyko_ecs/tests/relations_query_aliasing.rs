//! D (C2) — intra-system aliasing: `Query<(&mut T, Related<R, &T>)>` and the
//! reverse declaration order MUST be rejected.
//!
//! # Where the detector fires (determined empirically)
//!
//! `Related<R, &T>::init_access` declares `R`'s read THEN forwards to the inner
//! `D::init_access` (`&T`'s read) against the SAME `FilteredAccessSet`, in
//! declaration order. A sibling `&mut T` adds a write of `T`, so whichever term is
//! declared second observes a ComponentWriteVsRead / ComponentReadVsWrite conflict
//! (boyko-B0002).
//!
//! Crucially, `init_access` runs ONLY on the `Query<D, F>` **SystemParam** init path
//! (`FunctionSystem::initialize` → `SystemParam::init_access`). The
//! `EcsMaster::query::<D, F>()` fast path deliberately builds NO `FilteredAccessSet`
//! (its aliasing is gated by `&mut self` exclusivity), so it does NOT trip the
//! detector. Therefore the rejection is an **init-time panic** when the conflicting
//! query is used as a SystemParam inside a system — exercised here through
//! `run_system` (which calls `System::initialize`). Both declaration orders are
//! tested.

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::hierarchy::ChildOf;
use boyko_ecs::ecs::core::iters::query::relation::Related;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_macros::Component;

/// The aliased data component: read by the `Related` inner AND written by the
/// sibling `&mut` term.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Pos {
    x: f32,
}

/// A disjoint component for the control case.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Other {
    v: u32,
}

// ────────────────────────────────────────────────────────────────────────────
// Both orders MUST panic at system init with the B0002 conflict diagnostic.
// ────────────────────────────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "boyko-B0002")]
fn mut_then_related_same_component_conflicts() {
    let mut ecs = EcsMaster::new();
    // `&mut Pos` (write) followed by `Related<ChildOf, &Pos>` (read of Pos via the
    // join). The `Related` read forwarded by `init_access` collides with the
    // already-declared write ⇒ panic during `System::initialize`.
    ecs.run_system(|_q: Query<(&mut Pos, Related<ChildOf, &Pos>)>| {});
}

#[test]
#[should_panic(expected = "boyko-B0002")]
fn related_then_mut_same_component_conflicts() {
    let mut ecs = EcsMaster::new();
    // Reverse order: `Related<ChildOf, &Pos>` (read of Pos) followed by `&mut Pos`
    // (write). The write collides with the already-declared read ⇒ panic.
    ecs.run_system(|_q: Query<(Related<ChildOf, &Pos>, &mut Pos)>| {});
}

// ────────────────────────────────────────────────────────────────────────────
// Control: `Related<R, &T>` joined with a `&mut U` on a DISJOINT component is
// FINE (no conflict) — proves the rejection is component-specific, not a blanket
// "no &mut alongside Related".
// ────────────────────────────────────────────────────────────────────────────

#[test]
fn mut_disjoint_alongside_related_is_allowed() {
    let mut ecs = EcsMaster::new();
    // `&mut Other` writes a DIFFERENT component than the `Related` reads (Pos), so
    // there is no aliasing conflict — the system initializes without panicking.
    ecs.run_system(|_q: Query<(&mut Other, Related<ChildOf, &Pos>)>| {});
    // Reaching here (no panic during init) is the assertion.
}
