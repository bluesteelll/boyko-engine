// C / D(par) — a `par_iter().for_each(..)` over a query containing
// `Related<R, &T>` must NOT compile: the parallel chunk runner has no world cell
// to resolve the FK target's archetype per row, so the join is sequential-only.
// The rejection is a `const { assert!(!D::HAS_RELATED) }` in the parallel
// for-each path (par_iter.rs).
//
// Expected diagnostic: the `Related<R, D>` relation join is not supported on
// `par_iter` (sequential-only).

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::hierarchy::ChildOf;
use boyko_ecs::ecs::core::iters::query::relation::Related;
use boyko_macros::Component;

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Pos {
    x: f32,
}

fn main() {
    let mut world = EcsMaster::new();
    // `par_iter().for_each(..)` monomorphises the parallel for-each, whose const
    // block asserts `!D::HAS_RELATED`. `Related` sets `HAS_RELATED = true`, so the
    // assertion fails at codegen.
    world
        .query::<Related<ChildOf, &Pos>, ()>()
        .par_iter()
        .for_each(|_p: Option<&Pos>| {});
}
