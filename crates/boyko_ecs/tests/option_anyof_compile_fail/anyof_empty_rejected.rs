// Task #9 (compile-fail #13): empty `AnyOf<()>` has no `QueryData` impl
// (Decision 7) — the variadic `impl_any_of!` macro is invoked only for arity
// `1..=12`. So `AnyOf<()>` is not a `QueryData` and cannot be a `Query` data
// parameter.
//
// Expected diagnostic: `AnyOf<()>: QueryData` not satisfied (the `Query<D, F>`
// `D: QueryData` bound fails).

use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::data::AnyOf;

fn must_not_compile(q: Query<AnyOf<()>>) {
    for _ in q.iter() {}
}

fn main() {}
