// H — Criterion bench for the relation JOIN query DSL.
//
// Two measurements:
//   1. `related_join_vs_manual` — `Related<ChildOf, &Pos>` iteration vs a manual
//      `targets::<ChildOf>(child)` + `get_component::<Pos>(parent)` loop over the
//      same source set. The join should be within noise of (or faster than) the
//      manual two-lookup loop, since both pay the same data-dependent random loads.
//   2. `plain_query_0pct_gate` — a plain `Query<&Pos>` iter (NO relation term) to
//      confirm the relation seam is a 0%-gate: a relation-free query's per-row cost
//      is unaffected by the relation DSL existing in the crate. (Compared against
//      the historical query_dsl bench baseline.)
//
// Both go through `EcsMaster::query` (the direct `&mut self` path). `Related` is
// ALSO usable as a `Query` SystemParam inside a system body (FINDING-1 fixed —
// the `D: 'static` bound was dropped); this bench just exercises the direct path.

#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::ChildOf;
use boyko_ecs::ecs::core::iters::query::filter::With;
use boyko_ecs::ecs::core::iters::query::relation::Related;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::Component;
use criterion::{Criterion, black_box, criterion_group, criterion_main};

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Pos {
    x: i64,
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Tag(u32);

/// Builds a world with `n` parents (each `Pos`) and `n` children (each `Tag` +
/// `ChildOf` pointing at a distinct parent). Returns the world + the child handles.
fn build(n: usize) -> (EcsMaster, Vec<Entity>) {
    let mut ecs = EcsMaster::new();

    // Parents.
    let psink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::with_capacity(n)));
    let pp = Arc::clone(&psink);
    ecs.run_system(move |mut cmds: Commands| {
        let mut v = pp.lock().unwrap();
        for i in 0..n {
            v.push(cmds.spawn(Pos { x: i as i64 }).id());
        }
    });
    let parents = psink.lock().unwrap().clone();

    // Children, each ChildOf parents[i].
    let csink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::with_capacity(n)));
    let cc = Arc::clone(&csink);
    let par = parents.clone();
    ecs.run_system(move |mut cmds: Commands| {
        let mut v = cc.lock().unwrap();
        for (i, &p) in par.iter().enumerate() {
            v.push(cmds.spawn(Tag(i as u32)).insert(ChildOf(p)).id());
        }
    });
    let children = csink.lock().unwrap().clone();
    (ecs, children)
}

fn bench_related_join_vs_manual(c: &mut Criterion) {
    let n = 10_000usize;
    let (mut ecs, children) = build(n);

    let mut group = c.benchmark_group("relation_join");

    // The DSL join: Related<ChildOf, &Pos> over the child set.
    group.bench_function("Related<ChildOf,&Pos>::iter (10k)", |b| {
        b.iter(|| {
            let mut sum = 0i64;
            for p in ecs
                .query::<Related<ChildOf, &Pos>, With<ChildOf>>()
                .iter()
                .flatten()
            {
                sum += p.x;
            }
            black_box(sum)
        });
    });

    // The manual two-lookup loop over the same source set.
    group.bench_function("manual targets+get_component (10k)", |b| {
        b.iter(|| {
            let mut sum = 0i64;
            for &child in &children {
                if let Some(parent) = ecs.targets::<ChildOf>(child).next()
                    && let Some(p) = ecs.get_component::<Pos>(parent)
                {
                    sum += p.x;
                }
            }
            black_box(sum)
        });
    });

    group.finish();
}

fn bench_plain_query_0pct_gate(c: &mut Criterion) {
    // A relation-FREE query over a Pos-only set: the per-row cost must be the same
    // whether or not the relation DSL exists (0%-gate). Compare against the
    // query_dsl `bench_query_ref_iter` baseline.
    let n = 10_000usize;
    let mut ecs = EcsMaster::new();
    let sink: Arc<Mutex<()>> = Arc::new(Mutex::new(()));
    let _ = &sink;
    ecs.run_system(move |mut cmds: Commands| {
        for i in 0..n {
            cmds.spawn(Pos { x: i as i64 });
        }
    });

    c.bench_function("plain Query<&Pos>::iter (10k, no relation term)", |b| {
        b.iter(|| {
            let mut sum = 0i64;
            for p in ecs.query::<&Pos, ()>().iter() {
                sum += p.x;
            }
            black_box(sum)
        });
    });
}

criterion_group!(benches, bench_related_join_vs_manual, bench_plain_query_0pct_gate);
criterion_main!(benches);
