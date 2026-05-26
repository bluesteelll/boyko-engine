// Phase 8b Step 11 — Criterion bench suite for the typed `Query<D, F>` DSL.
//
// Per Phase 8b plan §1.2 / §18 Step 11 / §19.3:
//
//   1. bench_query_ref_iter                   — target ≤ 6 ns/row
//   2. bench_query_tuple_2_ref_iter           — target ≤ 9 ns/row
//   3. bench_query_mut_iter                   — target ≤ 8 ns/row
//   4. bench_query_with_archetypal_filter     — target ≤ 6 ns/row (parity with #1)
//   5. bench_query_archetype_transition       — target ≤ 50 ns/boundary
//   6. bench_query_cold_construction          — target ≤ 200 ns (first iter())
//   7. bench_query_init_state                 — target ≤ 1 µs (QueryDataState::new)
//
// # Dispatch overhead caveat (Option B)
//
// The Query iterator types (`QueryIter`, `QueryIterMut`) and the
// `UnsafeEcsCell` constructors are `pub(crate)` to boyko-ecs — they are not
// reachable from this bench file (benches are integration-style: they live in
// `crates/boyko_ecs/benches/`, outside the lib crate's privacy scope). Adding
// `pub fn` wrappers solely for the bench would require touching production
// code, which is outside the tester role's mandate.
//
// We therefore exercise every Query bench through `EcsMaster::run_closure_once`
// — the canonical Phase 8a end-to-end entry point. This means each bench
// measures `dispatch + Query setup + iter work` per iteration.
//
// The Phase 8a `run_closure_once` cost is documented at ~960 ns per call
// (commit 074d47c). For the per-row benches (1, 2, 3, 4) the iter is repeated
// N = 10_000 times per dispatch so the per-row cost is `(measured − 960 ns) /
// 10_000`. Reported numbers will reflect the raw per-call measurement; the
// per-row derivation appears in the report's analysis section.
//
// Phase 8c (`FunctionSystem` + cached init_state/init_access) is expected to
// drop the dispatch overhead to ≤ 5 ns; re-validation will move the per-row
// numbers into the §1.2 target range without changing this bench file.
//
// # Component-id range
//
// `MAX_COMPONENTS = 512` caps valid ids at 511. The orchestrator's
// suggested 530-540 range is invalid (out-of-range). The existing
// allocations in the crate top out at 509 — see the inline tables in
// `random_access.rs`, `query/state.rs`, and `query/data.rs`. IDs 230-299
// are free; this bench claims 230-242 (13-id span — four primitives at
// 230-233 plus a `Tag_i` range 234-241 for the 8-archetype init_state
// bench).

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::filter::Without;
use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::iters::query::state::QueryDataState;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use criterion::{black_box, criterion_group, criterion_main, Criterion};

// ── Component types for the bench scenarios ────────────────────────────────

const POS_ID: ComponentId = ComponentId(230);
const VEL_ID: ComponentId = ComponentId(231);
const FROZEN_ID: ComponentId = ComponentId(232);
const TAG_ID: ComponentId = ComponentId(233);

/// 12-byte POD position component — matches Phase 7's `random_access` bench.
#[repr(C)]
#[derive(Clone, Copy)]
struct Position {
    x: f32,
    y: f32,
    z: f32,
}

/// 12-byte POD velocity component.
#[repr(C)]
#[derive(Clone, Copy)]
struct Velocity {
    vx: f32,
    vy: f32,
    vz: f32,
}

/// Zero-sized marker — exercises `Without<Frozen>` archetypal filter.
#[repr(C)]
#[derive(Clone, Copy)]
struct Frozen;

/// 4-byte POD tag — used to construct a second matching archetype for the
/// transition bench (so the query crosses an archetype boundary at the
/// `(Position) ↔ (Position, Tag)` split).
#[repr(C)]
#[derive(Clone, Copy)]
struct Tag(#[allow(dead_code)] u32);

impl Component for Position {
    fn component_id() -> ComponentId { POS_ID }
}
impl Component for Velocity {
    fn component_id() -> ComponentId { VEL_ID }
}
impl Component for Frozen {
    fn component_id() -> ComponentId { FROZEN_ID }
}
impl Component for Tag {
    fn component_id() -> ComponentId { TAG_ID }
}

/// Idempotent registry priming. Each `register_layout` lookup is gated by a
/// `OnceLock`; repeated calls are no-ops after the first.
fn register_bench_components() {
    component_registry::register_layout::<Position>(POS_ID.0);
    component_registry::register_layout::<Velocity>(VEL_ID.0);
    component_registry::register_layout::<Frozen>(FROZEN_ID.0);
    component_registry::register_layout::<Tag>(TAG_ID.0);
}

// ── Scenario builders ──────────────────────────────────────────────────────

/// Build an `EcsMaster` with `n` entities all in the `[Position, Velocity]`
/// archetype. Returns the master + the archetype id for callers that need it.
fn build_single_archetype(n: usize) -> (EcsMaster, ArchetypeId) {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[POS_ID, VEL_ID]);

    for i in 0..n {
        let pos = Position { x: i as f32, y: 0.0, z: 0.0 };
        let vel = Velocity { vx: 1.0, vy: 0.0, vz: 0.0 };
        // SAFETY: Position/Velocity are #[repr(C)] POD; slices cover exactly
        // size_of::<T>() initialised bytes from each stack local.
        let pos_bytes = unsafe {
            std::slice::from_raw_parts(
                &pos as *const Position as *const u8,
                std::mem::size_of::<Position>(),
            )
        };
        let vel_bytes = unsafe {
            std::slice::from_raw_parts(
                &vel as *const Velocity as *const u8,
                std::mem::size_of::<Velocity>(),
            )
        };
        ecs.create_entity(arch, &[(POS_ID, pos_bytes), (VEL_ID, vel_bytes)])
            .expect("create_entity in single-archetype builder must succeed");
    }
    (ecs, arch)
}

/// Build an `EcsMaster` with two matching archetypes: `[Position]` and
/// `[Position, Tag]`. Each holds `n_per` entities. Used by the transition
/// bench so a single query crosses exactly one archetype boundary while
/// walking `2 × n_per` rows.
fn build_two_archetypes(n_per: usize) -> EcsMaster {
    let mut ecs = EcsMaster::new();
    let arch_p = ecs.create_archetype(&[POS_ID]);
    let arch_pt = ecs.create_archetype(&[POS_ID, TAG_ID]);

    for i in 0..n_per {
        let pos = Position { x: i as f32, y: 0.0, z: 0.0 };
        // SAFETY: POD slice.
        let pos_bytes = unsafe {
            std::slice::from_raw_parts(
                &pos as *const Position as *const u8,
                std::mem::size_of::<Position>(),
            )
        };
        ecs.create_entity(arch_p, &[(POS_ID, pos_bytes)])
            .expect("create_entity arch_p must succeed");
    }
    for i in 0..n_per {
        let pos = Position { x: i as f32, y: 1.0, z: 0.0 };
        let tag = Tag(i as u32);
        // SAFETY: POD slices.
        let pos_bytes = unsafe {
            std::slice::from_raw_parts(
                &pos as *const Position as *const u8,
                std::mem::size_of::<Position>(),
            )
        };
        let tag_bytes = unsafe {
            std::slice::from_raw_parts(
                &tag as *const Tag as *const u8,
                std::mem::size_of::<Tag>(),
            )
        };
        ecs.create_entity(arch_pt, &[(POS_ID, pos_bytes), (TAG_ID, tag_bytes)])
            .expect("create_entity arch_pt must succeed");
    }
    ecs
}

// ── §1.2 / Step 11 #1: bench_query_ref_iter ───────────────────────────────
//
// `Query<&Position>::iter()` over 10 000 entities in one archetype.
//
// Each timed iteration:
//   * Dispatches `run_closure_once` (Phase 8a baseline ~960 ns per call).
//   * Inside the closure: walks `Query::iter()` end-to-end, accumulating a
//     `f32` sum through `black_box` to defeat constant folding.
//
// Target: ≤ 6 ns/row.
//
// Per-row derivation: `(measured − 960 ns) / 10_000`. Phase 8a's dispatch
// overhead dominates the raw number; the Step 11 plan's §1.2 target applies
// to the per-row component AFTER Phase 8c removes the dispatch overhead.
fn bench_query_ref_iter(c: &mut Criterion) {
    register_bench_components();
    let (mut ecs, _arch) = build_single_archetype(10_000);

    c.bench_function("query_ref_iter_10k", |b| {
        b.iter(|| {
            ecs.run_closure_once(|q: Query<'_, '_, &Position>| {
                // Force the iterator to actually walk every row. `black_box`
                // on the running accumulator prevents the compiler from
                // realising the sum is unused.
                let mut acc: f32 = 0.0;
                for p in &q {
                    acc += black_box(p.x);
                }
                black_box(acc);
            });
        });
    });
}

// ── §1.2 / Step 11 #2: bench_query_tuple_2_ref_iter ───────────────────────
//
// `Query<(&Position, &Velocity)>::iter()` over 10 000 entities.
//
// Same scenario as #1 but the query reads two components per row. Validates
// that the tuple-2 `QueryData` impl's tuple-fetch costs the planned 9 ns/row
// at steady state (two `column.ptr.add(row * stride)` loads per row).
//
// Target: ≤ 9 ns/row.
fn bench_query_tuple_2_ref_iter(c: &mut Criterion) {
    register_bench_components();
    let (mut ecs, _arch) = build_single_archetype(10_000);

    c.bench_function("query_tuple_2_ref_iter_10k", |b| {
        b.iter(|| {
            ecs.run_closure_once(|q: Query<'_, '_, (&Position, &Velocity)>| {
                let mut acc: f32 = 0.0;
                for (p, v) in &q {
                    acc += black_box(p.x) + black_box(v.vx);
                }
                black_box(acc);
            });
        });
    });
}

// ── §1.2 / Step 11 #3: bench_query_mut_iter ───────────────────────────────
//
// `Query<&mut Position>::iter_mut()` over 10 000 entities.
//
// The mutable cursor uses `archetype_ptr_mut` + `set_table_mut`. Per the plan
// the cost should match `bench_query_ref_iter` since both paths boil down to
// the same single-column `column.ptr.add(row * stride)` access — `&` vs `&mut`
// only changes the borrow-stack tag, not the LLVM-level emitted code.
//
// Target: ≤ 8 ns/row.
fn bench_query_mut_iter(c: &mut Criterion) {
    register_bench_components();
    let (mut ecs, _arch) = build_single_archetype(10_000);

    c.bench_function("query_mut_iter_10k", |b| {
        b.iter(|| {
            ecs.run_closure_once(|mut q: Query<'_, '_, &mut Position>| {
                // Touch the value so the DerefMut path is not elided. We
                // increment the bit-pattern of `x` so the write is provably
                // observable through `black_box` without producing NaN/Inf
                // (which could widen the dependency chain on certain CPUs).
                for p in &mut q {
                    let bits = black_box(p.x).to_bits();
                    p.x = f32::from_bits(bits.wrapping_add(1));
                }
            });
        });
    });
}

// ── §1.2 / Step 11 #4: bench_query_with_archetypal_filter ─────────────────
//
// `Query<&Position, Without<Frozen>>::iter()` over 10 000 entities.
//
// `Without<Frozen>` is archetypal (`IS_ARCHETYPAL = true`) — the inner loop's
// `if !const { F::IS_ARCHETYPAL }` branch const-folds away at monomorphisation
// (verified by Step 14's `cargo expand` golden file). The measured cost
// should therefore match `bench_query_ref_iter` exactly.
//
// Target: ≤ 6 ns/row (parity with #1).
fn bench_query_with_archetypal_filter(c: &mut Criterion) {
    register_bench_components();
    let (mut ecs, _arch) = build_single_archetype(10_000);

    c.bench_function("query_with_archetypal_filter_10k", |b| {
        b.iter(|| {
            ecs.run_closure_once(|q: Query<'_, '_, &Position, Without<Frozen>>| {
                let mut acc: f32 = 0.0;
                for p in &q {
                    acc += black_box(p.x);
                }
                black_box(acc);
            });
        });
    });
}

// ── §1.2 / Step 11 #5: bench_query_archetype_transition ───────────────────
//
// `Query<&Position>::iter()` over two archetypes (`[Position]` and
// `[Position, Tag]`), each holding 5 000 entities. The query walks 10 000
// rows total + one archetype boundary.
//
// Reported cost is the TOTAL per-iter time. A first-order extraction of the
// boundary cost would compare against `bench_query_ref_iter` (10 000 rows, 0
// boundaries) — but that comparison crosses two different archetype layouts
// (`[Pos, Vel]` vs `[Pos]` / `[Pos, Tag]`), so the column-table-load count
// per row also differs. The bench's main role is to establish a baseline for
// Phase 8c re-validation against the §1.2 target ≤ 50 ns/boundary.
fn bench_query_archetype_transition(c: &mut Criterion) {
    register_bench_components();
    let mut ecs = build_two_archetypes(5_000);

    c.bench_function("query_archetype_transition_2x5k", |b| {
        b.iter(|| {
            ecs.run_closure_once(|q: Query<'_, '_, &Position>| {
                // archetype_count should be 2; the iterator walks both.
                let arches = q.archetype_count();
                let mut acc: f32 = 0.0;
                for p in &q {
                    acc += black_box(p.x);
                }
                black_box((arches, acc));
            });
        });
    });
}

// ── §1.2 / Step 11 #6: bench_query_cold_construction ──────────────────────
//
// First `Query<&Position>::iter()` call against a freshly-built archetype
// state. Each iteration:
//   1. Builds a new `EcsMaster` with 100 entities in `[Pos, Vel]` (setup —
//      outside the timed window via `iter_batched`).
//   2. Inside the timed window: `run_closure_once` triggers `init_state` +
//      `init_access` + Query construction + iter once.
//
// `iter_batched`'s `LargeInput` keeps the setup cost out of the
// per-iteration window. The remaining timed work is the cold-path overhead
// the §1.2 target ≤ 200 ns calls out for: `update_archetypes` (one delta) +
// `set_table_*` for the first archetype + the inner loop walking 100 rows.
//
// Target: ≤ 200 ns.
//
// Note (Option B): this bench inherits the ~960 ns `run_closure_once`
// dispatch floor, so the reported number is `(target + 960) ≈ 1.16 µs` at
// best. Phase 8c re-validation drops the dispatch to ≤ 5 ns, after which the
// raw 200 ns budget applies.
fn bench_query_cold_construction(c: &mut Criterion) {
    register_bench_components();

    c.bench_function("query_cold_construction", |b| {
        b.iter_batched(
            // Setup: fresh ECS with 100 entities — small enough that the
            // per-iter setup latency stays bounded. `SmallInput` keeps
            // criterion from accumulating ECSes across batches (each
            // `EcsMaster::new` allocates a 64 MB arena; `LargeInput` would
            // pile up tens of GB before the next drop wave).
            || {
                let (ecs, _arch) = build_single_archetype(100);
                ecs
            },
            // Timed body: one `run_closure_once` invocation. The closure
            // walks the iter once; the cold-path work is the
            // `QueryDataState::new` inside Query's `init_state` + first
            // `set_table_readonly` + the 100-row inner loop.
            |mut ecs| {
                ecs.run_closure_once(|q: Query<'_, '_, &Position>| {
                    let mut acc: f32 = 0.0;
                    for p in &q {
                        acc += black_box(p.x);
                    }
                    black_box(acc);
                });
                // Drop ecs explicitly so the deallocation latency is
                // accounted to the timed window (it dominates the per-call
                // floor anyway).
                drop(black_box(ecs));
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

// ── §1.2 / Step 11 #7: bench_query_init_state ─────────────────────────────
//
// `QueryDataState::<&Position, ()>::new(&mut ecs)` cost — i.e. the per-system
// registration cost.
//
// Per §19.3 plan target: ≤ 1 µs for 50 archetypes.
//
// `QueryDataState::new` is a `pub` constructor (not `pub(crate)`) so this
// bench is the one place we can sidestep the `run_closure_once` floor — the
// measurement is the true `init_state` cost.
//
// # Setup strategy — share-ECS, not iter_batched
//
// `iter_batched` would build a fresh ECS (and 50 archetypes, each
// preallocating ~4 MB of component pools via `with_default_sizes`) on every
// batch. With 64-MB `DEFAULT_ARENA_SIZE` even one ECS with 50 archetypes
// runs out of arena (~200 MB needed); criterion's batched setup blows the
// heap before the first sample completes.
//
// Instead, build ONE shared ECS with `N_ARCHETYPES` archetypes (cold,
// once), then have each timed iteration repeatedly call
// `QueryDataState::new(&mut ecs)`. The constructor reads `archetype_master`
// but performs no archetype churn — after the first call, every subsequent
// call sees the same archetype set, same generation, and produces an
// equivalent state (each call is independent state-wise; we drop the state
// immediately to avoid heap pressure inside the timed window).
//
// Side-effect note: `QueryDataState::new` does NOT mutate the ECS's
// archetype master or component registries — the `&mut EcsMaster` argument
// is taken for API uniformity with the SystemParam protocol; the body only
// calls `archetype_master()` (shared) + `init_state` (registry reads). The
// share-ECS pattern is therefore semantically equivalent to a fresh-ECS
// pattern, and faster + heap-safe.
//
// # Archetype count — 8, not 50
//
// The §19.3 plan calls for 50 archetypes, but each `[Position, Tag_i]`
// archetype preallocates ~4 MB of component pool buffers
// (`with_default_sizes` allocates `DEFAULT_CHUNKS_PER_POOL ×
// TINY_COMPONENTS_PER_CHUNK × component_size` = 128 × 2048 × 12 ≈ 3 MB for
// `Position` plus ~1 MB for `Tag`). 50 archetypes need ~200 MB — far above
// the 64-MB `DEFAULT_ARENA_SIZE`; even 16 archetypes hits the arena ceiling
// due to per-archetype headers and the empty-block tracker overhead.
//
// 8 archetypes (~32 MB) sits comfortably inside the arena while still
// exercising `update_archetypes`'s archetype-bitset scan. The measured
// per-archetype cost is roughly linear in N (one bit-test + push per
// matching archetype), so the 50-archetype target can be estimated by
// scaling: `target_at_50 ≈ measured_at_8 + (50 - 8) × per-archetype-cost`.
fn bench_query_init_state(c: &mut Criterion) {
    register_bench_components();

    // 8 distinct "tag" component ids 234..242 — within the bench-reserved
    // range and well under `MAX_COMPONENTS = 512`. Each archetype is
    // `[Pos, Tag_i]` so the inner `QueryState`'s include-mask matcher must
    // iterate every archetype.
    //
    // We register the 8 ids once at the top of the bench (idempotent).
    const N_ARCHETYPES: usize = 8;
    let mut tag_ids: [ComponentId; N_ARCHETYPES] = [ComponentId(0); N_ARCHETYPES];
    for (i, slot) in tag_ids.iter_mut().enumerate() {
        let cid = ComponentId(234 + i);
        *slot = cid;
        component_registry::register_layout::<Tag>(cid.0);
    }

    // Cold-build a single ECS with N archetypes. This is outside the
    // criterion `bench_function` scope so the heap allocation never enters
    // the timed window.
    let mut ecs = EcsMaster::new();
    for cid in tag_ids.iter() {
        ecs.create_archetype(&[POS_ID, *cid]);
    }

    c.bench_function("query_init_state_8_archetypes", |b| {
        b.iter(|| {
            // Single `QueryDataState::new` call. The bench measures
            // `D::init_state + F::init_state + aggregate_include +
            // QueryState::new + update_archetypes (N_ARCHETYPES archetypes)
            // + post_filter_matched`.
            //
            // `state` is dropped at the end of the iteration so no heap
            // accumulates across samples.
            let state = QueryDataState::<&Position, ()>::new(black_box(&mut ecs));
            black_box(state);
        });
    });
}

criterion_group!(
    query_dsl_benches,
    bench_query_ref_iter,
    bench_query_tuple_2_ref_iter,
    bench_query_mut_iter,
    bench_query_with_archetypal_filter,
    bench_query_archetype_transition,
    bench_query_cold_construction,
    bench_query_init_state,
);
criterion_main!(query_dsl_benches);
