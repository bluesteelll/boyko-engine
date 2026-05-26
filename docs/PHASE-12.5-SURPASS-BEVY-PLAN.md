# Phase 12.5 — "Surpass Bevy" Performance Push

Intermediate phase between Phase 12 (Events as SystemParam) and Phase 13.
Goal: beat `bevy_ecs` 0.18.1 on **all four** head-to-head benchmarks, not just
two out of four.

## Status quo (real measurements)

Measured on `crates/bench_bevy_vs_boyko/` against `bevy_ecs` 0.18.1 (release,
LTO, single thread for serial benches, default Rayon pool for parallel).

| Benchmark            | boyko      | bevy       | Ratio       | Status        |
|----------------------|------------|------------|-------------|---------------|
| 50 empty systems     | 13.94 µs   | 22.99 µs   | **1.65× win** | hold          |
| Query iter 10k       | 7.88 µs    | 6.90 µs    | **0.88× loss** | close gap     |
| par_iter 10k         | 39.12 µs   | 122.07 µs  | **3.12× win** | hold          |
| Spawn 10k (Commands) | 1.044 ms   | 530 µs     | **0.51× loss** | close gap     |

Score: **2 wins / 2 losses**. Two losses to close: query iter (small,
~1 µs delta) and Commands::spawn batch (large, ~514 µs delta).

## Success criteria

1. **3 of 4 benchmarks**: `boyko ≥ 1.10× bevy` after optimisations.
   - 50 empty systems: holds (existing 1.65× win, no regression).
   - par_iter 10k: holds (existing 3.12× win, no regression).
   - Spawn 10k (Commands): `boyko ≥ 1.10× bevy` (Track A target — closing 0.51× loss).
2. **Query iter 10k bench**: `boyko ≥ Bevy parity (within 5% noise floor)` — concretely ≤ Bevy reference + 5%.
   - **Rationale (per Round 2 critic findings)**: profile (PHASE-12.5-PROFILE-QUERY.md §1) proved the inner loop is byte-identical to Bevy in compiled asm (same 5-instruction sequence). The residual ~1 µs lives in outer-loop and system-wrapper overhead. Track B's Opt-B1 (direct query API) + Opt-B2 (NEEDS_CHANGE_DETECTION const elision) close the existing 0.88× loss to parity. **Surpassing Bevy by 10% on this specific bench requires fundamental redesign** of the query inner loop or storage layout (sparse-set storage, PGO, allocator tuning, SIMD-batched fetch) — that is Phase 13+ work, explicitly out of scope for Phase 12.5.
   - **Residual filed as Phase 13**: "Query iter ≥ 1.10× Bevy via inner-loop or storage-layout redesign."
3. No regression on the two existing wins (50 empty systems, par_iter).
4. All 612 existing tests still pass on `--test-threads=1`.
5. No new `unsafe` block without `// SAFETY:` justification.
6. No new alloc on hot path that survives clippy + Miri.

Track P (Profile) decides hotspot priority; Tracks A and B implement fixes;
Track C verifies wins.

---

## Track P — Profile (must run FIRST)

Optimising without measurement contradicts CLAUDE.md principle 7 (measured
inlining) and #3 (cache-driven). Output of this track is a documented
breakdown of where each lost benchmark spends its cycles.

### P1. Profile `SpawnAtCommand::apply` × 10 000

Question: where do the **514 µs** of delta to Bevy go, per entity?

Hypothesised hotspots, in order of suspicion:

1. **CommandQueue dispatch overhead** per entity — bundle boxing + the
   `Command::apply` indirect call. 10 000 of these in a tight loop is the
   prime suspect.
2. **`EntityMaster::register_entity_with_ptr`** — atomic counter increment +
   inland write per entity. Per-entity atomic on a hot counter could cost
   ~5–10 ns.
3. **`Archetype::create_entity`** — Unit alloc, pool grow on capacity miss.
   For 10k entities into a fresh archetype the pool will grow ~13 times
   (1.5× geometric).
4. **Phase 10 per-component tick init** — two `Tick` writes (added +
   changed) per component × 3 components × 10k = 60k writes. With a
   non-vectorised loop this is significant.
5. **`Bundle::for_each_component_bytes` callback overhead** per entity —
   the closure dispatch could prevent the compiler from inlining writes.

Deliverable: `docs/PHASE-12.5-PROFILE-SPAWN.md` containing a per-stage
timing breakdown (with `Instant::now()` checkpoints in a controlled
copy of the bench), and a comparable breakdown of Bevy's `spawn_batch`
inner loop drawn from the source under `crates/bench_bevy_vs_boyko/target/`.

### P2. Profile query iter inner loop

Question: where does the **980 ns** delta on 10 000 iterations come from?

Hypothesised hotspots:

1. **`set_change_ticks` per archetype** on every `query.iter()` call —
   Phase 10 bookkeeping that is pure overhead when the query has no
   `Added`/`Changed` filters and no `Ref`/`Mut` data.
2. **Archetype matching** — linear scan over all archetypes on every
   `iter()`. Bevy caches `matched_archetype_ids` in `QueryState`.
3. **`run_closure_once` wrapper** — the bench uses a system-style
   dispatch path; Bevy's bench uses `World::query` directly.
4. **SoA iter codegen** — `#[inline]` may not be propagated through
   `QueryIter::next`, `Fetch::fetch`, archetype/pool indexing.
   Assembly inspection required.

Deliverable: `docs/PHASE-12.5-PROFILE-QUERY.md` with isolated micro-bench
timings for each suspected contributor, plus an assembly dump of the
inner loop (`cargo rustc --release -- --emit asm`).

---

## Track A — Spawn batch (close the 1.97× gap)

Depends on P1. Headline hypothesis: per-entity `CommandQueue` overhead +
per-entity tick init are the two biggest contributors; collapsing N
`SpawnAtCommand` invocations into one `SpawnBatchCommand` with a single
archetype resolve and capacity reserve should beat Bevy.

### A1. Architect — `SpawnBatchCommand<B, I>` + bulk APIs

Design surface:

- `SpawnBatchCommand<B: Bundle, I: IntoIterator<Item = B>>` — one
  deferred command instead of N. `apply` resolves the target archetype
  **once**, reserves capacity once, then runs a tight loop that writes
  N bundles.
- `Commands::spawn_batch<B, I>(iter) -> SpawnBatchIter<'_, B>` —
  Bevy-style API. Returns an iterator over reserved `Entity` values so
  the caller can `.collect()` IDs.
- `EcsMaster::spawn_batch<B, I>(iter) -> Vec<Entity>` — direct
  non-deferred path for main-thread / tests.
- `Archetype::reserve_capacity(n: usize)` — pre-grow every component
  pool + the units array by `n`. One alloc per pool instead of N.
- `EntityMaster::reserve_batch(n: usize) -> Range<EntityId>` — single
  `fetch_add(n, Relaxed)` instead of N single fetch_add(1) calls.
- `EntityMaster::register_batch(entities: &[Entity], ptrs: &[*mut u8])` —
  bulk inland write, vectorisable.
- `ComponentPool::extend_from_bundle_writer(start_row, count, writer)` —
  bulk write API with no per-row grow checks (capacity asserted upfront).
- Vectorised tick init — `Tick` row is 8 bytes (`u64`). Filling
  `current_tick` for N rows reduces to `slice::fill` → SIMD memset.

Open architecture questions:

- **Q1**: Bundle iteration safety when `I: Iterator` is **not**
  `ExactSizeIterator`. Two options:
  - (a) Require `ExactSizeIterator` — restricts API but enables single
    upfront capacity reserve.
  - (b) Accept any iterator — `reserve_capacity` grows lazily (chunked).
  Bevy chose (a) for `spawn_batch`. Recommend (a).
- **Q2**: Panic safety. If the 5 000th `B::clone` (or bundle Drop)
  panics mid-batch — partial state. Bevy's behaviour: half-spawned
  entities survive. Recommend Bevy parity (cleaner than rollback).
- **Q3**: `CommandQueue` layout — `SpawnBatchCommand<B, I>` has `I`
  inside, `sizeof::<I>()` unknowable in queue. Options:
  - (a) Box the iterator inside the command (`Box<dyn Iterator<Item=B>>`)
    — defeats the purpose if iterator drop allocates.
  - (b) Store the iterator state in the command struct directly (uses
    existing `CommandQueue::push<C: Command>` generic path).
  Recommend (b).
- **Q4**: Whether to also collapse `InsertCommand` batches (e.g.
  `commands.insert_many(entities, bundle_iter)`). Out of scope for
  12.5; record as Phase 13 followup.

Deliverable: `docs/PHASE-12.5-SPAWN-BATCH-PLAN.md` (architect-grade plan
following Phase 11/12 pattern).

### A2. Architecture-critic

Standard critic rounds until APPROVED. Specific checks:

- Capacity reserve semantics under existing `ComponentPool` grow policy.
- Drop ordering when the iterator panics mid-batch.
- Bundle's `for_each_component_bytes` interaction with bulk write —
  the same lifetime trap that bit Phase 11 InsertCommand (dangling
  slices after stack frame return).
- Whether `register_batch` needs free-list integration (no — fresh-only
  per EM2, same as `reserve_entity`).
- False-sharing on `EntityMaster::next_entity_id` when many threads
  hammer `reserve_batch` — should be fine, batch operation amortises
  cache-line bouncing.

### A3. Implementation

Sequenced waves:

1. **Wave A1**: `EntityMaster::reserve_batch` + `register_batch` +
   tests (range correctness, generation=0, no free-list interaction).
2. **Wave A2**: `Archetype::reserve_capacity` + `ComponentPool::reserve`
   + tests (capacity correctness, no spurious grow).
3. **Wave A3**: `EcsMaster::spawn_batch` (direct path) + tests
   (10/100/10 000 entities, mixed archetypes, panic safety).
4. **Wave A4**: `SpawnBatchCommand` + `Commands::spawn_batch` + tests.
5. **Wave A5**: Vectorised tick init (`Tick::write_row_n` or
   `pool.fill_tick_rows(start, count, tick)`) + assembly verification.

### A4. Bench

Add `spawn_batch_10k` group to `crates/bench_bevy_vs_boyko/`. Both
engines must use their batch API (`Commands::spawn_batch` in boyko,
`Commands::spawn_batch` in Bevy — same name, different impl).

Targets:
- `spawn_batch_10k`: boyko ≥ 1.10× bevy.
- `spawn_single`: unchanged (no regression on `Commands::spawn` 1
  entity).

---

## Track B — Query iter direct API (close the 1.14× gap)

Depends on P2. Headline hypothesis: the bench wraps iteration in
system-style dispatch, and Phase 10 bookkeeping fires unconditionally;
a direct `World::query`-equivalent API plus compile-time elision of
change-detection setup will close the gap.

### B1. Architect — `EcsMaster::query<D, F>()` + QueryState cache

Design surface:

- `EcsMaster::query<D: QueryData, F: QueryFilter>() -> QueryView<'_, D, F>`
  — direct non-system API. Does **not** go through the scheduler;
  does **not** call `set_change_ticks` unless `D::NEEDS_CHANGE_DETECTION
  || F::NEEDS_CHANGE_DETECTION`.
- **Compile-time elision**: associated `const NEEDS_CHANGE_DETECTION:
  bool` on `QueryData` and `QueryFilter`. For `&T`, `&mut T`, `()` it
  is `false`. For `Added<T>`, `Changed<T>`, `Ref<T>`, `Mut<T>` it is
  `true`. Inside `query()` the branch is a `const` if — the dead code
  is elided.
- `QueryState<D, F>` cache: `EcsMaster` keeps a `BTreeMap<TypeId,
  Box<dyn Any>>` mapping `TypeId::of::<(D, F)>()` to a pre-resolved
  state with `matched_archetype_ids: Box<[ArchetypeId]>`. On archetype
  creation an internal version counter is bumped; on `query()` the
  cached state is rebuilt lazily when its stored version is stale.
- `#[inline]` hygiene through the iter chain: `QueryView::iter`,
  `QueryIter::next`, fetch methods. Assembly verification post-impl.

Open questions:

- **Q1**: `HashMap` vs `BTreeMap` — CLAUDE.md forbids `HashMap` on hot
  path. Cache lookup happens once per `query()` call, not per entity,
  so the cost is ambient. Still — prefer `BTreeMap` (no hash work, no
  allocations on insertion past initial capacity).
- **Q2**: Whether the cache is per-`EcsMaster` or `static`. Per-master
  is safer (no cross-world bleed in tests). Recommend per-master.
- **Q3**: Invalidation granularity. Bumping a global version on every
  archetype creation forces all cached `QueryState`s to rebuild on
  next access. Bevy keeps a `last_archetype_id` per state and only
  scans the new archetypes since. Recommend Bevy approach (incremental
  rebuild).
- **Q4**: Whether the new `query()` accepts the change-tick context.
  Outside a system, `last_run = 0`, `this_run = world.current_tick()`.
  Hard-code those defaults; document.

Deliverable: `docs/PHASE-12.5-QUERY-DIRECT-PLAN.md`.

### B2. Architecture-critic

Specific checks:

- Cache lifetime vs Phase 9 parallel access — the cache stores
  `Box<dyn Any>` and is read-only during a scheduler run; writes
  happen between frames. Verify nothing in a parallel `par_iter` path
  needs cache mutation.
- `TypeId::of::<(D, F)>()` uniqueness — two queries with identical
  `D` and `F` share state; verify that is correct (yes, they always do
  in Bevy).
- Incremental rebuild correctness when archetypes are created mid-frame
  (e.g. by deferred `SpawnAtCommand`).

### B3. Implementation

1. `QueryData::NEEDS_CHANGE_DETECTION` const + `QueryFilter::NEEDS_CHANGE_DETECTION`
   const. Impl on all existing types via the 4 derive-macros + 6 leaf
   impls (78 total per Phase 10 audit).
2. `QueryState<D, F>` lazy cache in `EcsMaster` with version-stamped
   incremental rebuild.
3. `EcsMaster::query<D, F>()` API + tests (cache hit, cache rebuild on
   new archetype, change-detection-on vs change-detection-off paths).
4. Assembly verification of inner loop (must be a tight register-only
   loop without indirect calls).

### B4. Bench

Update `query_iter_10k` to call `world.query::<&Position>().iter()`
directly. Bevy bench already uses `world.query::<&Position>().iter(&world)`.

Target: boyko ≥ 1.10× bevy.

---

## Track C — Verification & accountability

### C1. Re-run scoreboard after each track

After Track A lands: full 4-bench run vs Bevy, report in chat + commit
into `docs/PHASE-12.5-RESULTS-INTERIM-A.md`.

After Track B lands: same, into `RESULTS-INTERIM-B.md`.

### C2. Final scoreboard

`docs/PHASE-12.5-RESULTS.md` — single page:

- All 4 benches: boyko vs bevy ms/µs, ratio, status (✅ ≥1.10× / ⚠️
  <1.10× / ❌ regression).
- What concrete change closed each gap (or why a gap was not closed).
- Negative results: which hypotheses were wrong (e.g. "expected
  tick-init to dominate; profile showed CommandQueue dispatch was 3×
  larger" — document so future agents don't repeat).
- Updated memory file `project-phase-12.5-complete.md`.

### C3. No sweeping under the rug

If a track does **not** close its gap to ≥1.10×: acknowledge in
results, file the residual as a Phase 13 target, do not silently
revert to "good enough".

---

## Dependency graph

```
P1 ─┬─→ A1 → A2 → A3 → A4 ─┐
    │                       │
P2 ─┴─→ B1 → B2 → B3 → B4 ─┴─→ C1 → C2 → C3
```

- **P1 and P2 run in parallel** (independent profilers).
- **A and B tracks run in parallel** once their respective profile
  report is in (different files, no implementation overlap).
- **C1 runs incrementally** after each track lands.

Realistic estimate: **3–4 days** of work if profile findings confirm
hypotheses. If the profile points to a third hotspot (e.g. allocator
pressure, atomic contention), add Track D.

---

## Explicitly out of scope for 12.5

- Sparse-set storage (Bevy's hybrid Table+SparseSet) — Phase 13+.
- Observers / hooks — Phase 13+.
- `Local<T>` SystemParam — Phase 13+ (deferred from Phase 12).
- Component relations / many-to-many — Phase 14+.
- Cryptomicro-optimisations without measured payoff.

---

## Risk register

| Risk | Probability | Mitigation |
|------|-------------|------------|
| Profile points to allocator, not the suspected hotspot | M | Add Track D (allocator), don't shoehorn into A/B |
| Spawn batch breaks existing single-spawn fast path | M | Bench gate: `spawn_single` must not regress |
| QueryState cache invalidation race under parallel scheduler | L | Loom test in Track B critic + tester |
| Bevy 0.18 → 0.19 changes baseline numbers | L | Pin `bevy_ecs = "=0.18.1"` in bench Cargo.toml |
| Compile-time `NEEDS_CHANGE_DETECTION` const breaks downstream proc-macro consumers | L | Default impl `const NEEDS_CHANGE_DETECTION: bool = false` on trait |

---

## Approval checklist

- [ ] P1 + P2 launched in parallel — profile reports land first.
- [ ] A1 architect plan reviewed by critic until APPROVED before A3.
- [ ] B1 architect plan reviewed by critic until APPROVED before B3.
- [ ] Each implementation wave passes `cargo test --workspace --lib
  --tests -- --test-threads=1` before the next wave starts.
- [ ] Final 4-bench scoreboard committed to `docs/PHASE-12.5-RESULTS.md`.
- [ ] Memory file `project-phase-12.5-complete.md` created on success.
