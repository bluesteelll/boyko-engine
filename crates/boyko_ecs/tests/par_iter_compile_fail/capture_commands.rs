//! Phase 9 PAR1 / W2 compile-fail — par_iter cannot capture mutable state.
//!
//! `Query::par_iter().for_each(closure)` declares `closure: Fn(D::Item<'_>)
//! + Send + Sync` (PAR1). The `Fn` bound (not `FnMut`) means the closure
//! cannot mutate or move out of its captures, which by extension blocks
//! `&mut Commands` capture: the borrow checker rejects the mutable
//! reborrow inside the `Fn` body before the trait-system layer is reached.
//!
//! This file documents the Round 2 W2 invariant: the canonical user
//! mistake — taking `&mut commands` inside `par_iter` to enqueue a spawn
//! per-row — must fail at compile time. The user must instead collect
//! commands sequentially after `par_iter` returns, or use a deterministic
//! per-thread buffer reduced post-hoc.
//!
//! The deeper `!Sync` story for `Commands<'_>` is enforced indirectly:
//! `Commands::add` / `Commands::spawn` take `&mut self`, and the `Fn`
//! bound forbids `&mut self` calls from inside the closure.

use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::Commands;

#[derive(Clone, Copy)]
#[repr(C)]
struct Tag(u32);

impl boyko_ecs::ecs::core::component::component::Component for Tag {
    fn component_id() -> boyko_ecs::ecs::identifiers::primitives::ComponentId {
        boyko_ecs::ecs::identifiers::primitives::ComponentId(999)
    }
}

fn must_not_compile(q: Query<&Tag>, mut cmds: Commands<'_>) {
    q.par_iter().for_each(|_tag: &Tag| {
        // Mutable reborrow inside an `Fn` closure body is rejected by the
        // borrow checker (E0596 — cannot borrow as mutable in a Fn). The
        // par_iter contract (PAR1: `Body: Fn + Send + Sync`) is the
        // upstream cause: the architectural intent is "no per-row
        // mutation of any external state", which `Fn` rather than `FnMut`
        // encodes.
        let _ = &mut cmds;
    });
}

fn main() {}
