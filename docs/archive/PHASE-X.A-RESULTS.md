> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase X.A — Results

**Status:** ✅ LANDED. 9 waves, 9 commits (architect → critic ↔ developer
↔ code-reviewer ↔ tester pipeline). Verdict: implementation-ready and
ready for downstream consumption.

## Goal (from plan §1.1)

Add two new methods to the `Query<'w, 's, D, F>` SystemParam (and to the
direct-API `QueryView<'w, D, F>`):

- `for_each_chunk<Func>(&mut self, f: Func)` — sequential, per-archetype
  slice closure.
- `par_for_each_chunk<Func>(&mut self, f: Func, batching)` — parallel
  variant fanning archetype subranges across `boyko_threadpool` workers.

The closure receives **one contiguous columnar slice per matched
archetype** (a per-element slice for tuple `D`), giving the user direct
control of the inner loop. This is the flecs `ecs_query_next + ecs_field`
shape — the only shape that production-ECS evidence (flecs + Unity DOTS)
shows can sustain LLVM auto-vectorization of multi-row reductions in
stock-language code.

## Architectural plan

`docs/PHASE-X.A-PLAN.md` (Round 3, ~1370 lines including the
"Changes from Rounds 1+2" sub-tables at the top).

## Critic rounds

- **Round 1** (`docs/PHASE-X.A-CRITIC-ROUND-1.md`, 308 lines): APPROVED
  WITH MINOR CHANGES — 5 W-tier + 4 N-tier remarks. No critical
  blockers.
- **Round 2** (`docs/PHASE-X.A-CRITIC-ROUND-2.md`, 219 lines): APPROVED
  WITH FOLLOW-UP — 2 W-tier + 3 N-tier. Wave 1 cleared to start in
  parallel; W2.1 + W2.2 deferred to before Wave 8.
- **Round 3 architect**: all 9 Round 1 + 5 Round 2 patches applied
  inline; closing line updated to "ready for developer". Round 3
  critic deemed not warranted (mechanical scope clarifications only).

## Wave-by-wave delivery

| Wave | Commit | Scope | Code-review verdict |
|------|--------|-------|---------------------|
| 1A | `b14e9f9` | `SIMD_BUFFER_ALIGN` + `ComponentPool` alignment lift | (rolled into Wave 1 review) |
| 1B | `bd9f7bb` | `ArchetypalQueryFilter` marker trait + leaf/Or/tuple impls | (rolled into Wave 1 review) |
| 1C | `a6d4581` | `ChunkedQueryData` trait skeleton (no impls) | APPROVED (Wave 1) |
| Plan | `d7aade6` | docs: critic Rounds 1+2 + Round 3 architect patches | n/a |
| 2 | `d60b121` | Leaf `ChunkedQueryData` impls for `&T` / `&mut T` / `()` | APPROVED WITH NITS |
| 3 | `f231a27` | Tuple variadic propagation 1..=12 + §5.4 too-large justification | APPROVED |
| 4 | `99a3fed` | Sequential `chunk_iter::for_each_chunk_impl` + `Query::for_each_chunk` | APPROVED WITH NITS |
| 5 | `8fe7a1c` | `QueryView::for_each_chunk` direct-API mirror + N2/N3 fixes | (skipped — simple Wave 4 mirror) |
| 6 | `e13de1f` | Parallel `par_chunk::par_for_each_chunk_impl` + `Query`/`QueryView::par_for_each_chunk` | APPROVED WITH NITS |
| 7 | (post 6) | Unit + property + trybuild compile-fail + Miri sequential | tester verified |
| 8 | `04de06d` | Bench harness `g6_for_each_chunk` + per-package nightly toolchain pin | tester verified |
| 9 | (this commit) | Internal doc updates + this RESULTS file | n/a |

## What the chunked API does NOT do (deliberate, deferred to future phases)

- No alignment generic (`::<N>`, `Align16`, `Align32`). Bevy PR #6161
  sank on this complexity budget.
- No engine-side pre-padding to lane width.
- No per-row SIMD-alignment guarantee for arbitrary `T` — column-start
  alignment only (Phase X.A § 6).
- No `iter_chunks() -> impl Iterator<Item = &[T]>` — the
  streaming-iter lifetime puzzle is not worth solving when
  `for_each_chunk(FnMut)` covers the same use cases.
- No `for_each_chunk_with_mask` — filed Phase 13.X as opt-in.
- No support for `Changed<T>` / `Added<T>` / `Ref<T>` / `Mut<T>` filter
  + data inside `for_each_chunk` — gated out at compile time via
  `ArchetypalQueryFilter` + `ChunkedQueryData` bounds.
- No `par_fold_chunks` parallel reducing variant — filed Phase 13.X.

## Measured results

### Test coverage

Total tests added across Wave 4-7: **22**:

- 7 sequential unit tests in `chunk_iter::tests` (single, multi,
  empty, stale-id, mut-doubles, tuple-3, empty-tuple-D).
- 5 parallel tests in `par_chunk::tests` (PAR7 fallback, PAR9 inline,
  disjoint-coverage, multi-archetype, mut-doubles).
- 4 compile-time tuple sanity tests in `chunked_data::chunked_tuple_tests`
  (arity 0/1/3-mixed/12).
- 3 marker tests in `filter::archetypal_marker_tests` (Wave 1B).
- 3 proptest properties in `tests/chunk_proptest.rs` (256 cases each,
  multi-archetype total rows / parallel no-overlap / parallel
  sum-matches-sequential).
- 5 trybuild compile-fail tests in `tests/compile_fail_chunk/`
  (Changed / Added / Ref / Mut / Or-with-Changed).
- 1 runtime aliasing `#[should_panic]` test
  (`Query<(&mut T, &mut T)>` via SystemParam path → `boyko-B0002`).

Lib + integration baseline post Wave 7: **440 lib tests** + 3 proptest
binaries + 2 compile_fail binaries pass clean.

### Miri (Phase X.A § 11.5)

- `cargo +nightly miri test --lib chunk_iter::tests`: **7/7 PASS**
  under `-Zmiri-tree-borrows` (~32 min including build).
- `cargo +nightly miri test --lib par_chunk::tests::par_for_each_chunk_no_pool_fallback`:
  **1/1 PASS** (sequential PAR7 fallback path).
- Parallel `par_chunk` tests **deferred to Phase 9.1** per Phase 9
  closeout memory: `boyko_threadpool::Scope::spawn` triggers a Tree
  Borrows protected-tag conflict (sound by design; the
  `ScopeShared::pending.fetch_sub` worker-side foreign-writes a tag
  the dispatcher still holds as Reserved-conflicted). Documented
  inline in `par_chunk.rs::tests` Miri-skip block.

### Benchmarks (Phase X.A § 8.2 target)

**Target:** boyko `Query::for_each_chunk` ≥ **5×** Bevy
`Query::iter().fold(_, f32::algebraic_add)` median over 60 criterion
samples on 10k single-archetype f32-sum reduction.

**Measured** (`g6_for_each_chunk_algebraic_sum_10k`, Windows x86_64,
criterion 60 samples):

| RUSTFLAGS | boyko median | Bevy median | Ratio |
|-----------|--------------|-------------|-------|
| default | 890.82 ns | 851.70 ns | 1.05× (boyko ~5 % slower) |
| `-Ctarget-cpu=native` | 338.63 ns | 304.35 ns | 1.11× (boyko ~11 % slower) |

**Result:** the 5× target was **NOT met** on this canonical
single-component f32 reduction. The outcome matches plan § 13 Risk 5
case 3 exactly: *"Bevy actually autovectorizes too when the fold body
is simple enough → the speedup compresses."* Phase 12.6 memory note
pre-anticipated this: *"Query iter inner loop byte-identical Bevy в
asm (5 инструкций); `black_box(p.x)` per-element блокирует SIMD
независимо от engine."*

#### Phase X.A.1 — multi-component (3-tuple) reduction (`g6b_*`)

The single-component bench's parity outcome left the plan's § 13 Risk 5
hypothesis untested: the win should **widen on multi-component
reductions** because Bevy pays a per-row *tuple-fetch* state-machine
cost (one `Iterator::next` advancing three column cursors and
materialising a `(&P, &V, &A)` tuple per row) that boyko's batched path
elides (the closure receives three contiguous column slices, one per
component, and runs a single index loop).

The `g6b_*` group reduces `pos[i] + vel[i] + acc[i]` over a 10k-entity
`(PosF32, VelF32, AccF32)` archetype — identical harness shape to `g6`
(single archetype, deterministic sequential spawn, `algebraic_add`,
`black_box` only at the sink, 60 samples). Two boyko inner-loop shapes
were benched (the inner-loop shape is the user's responsibility, not the
dispatcher's):

* **`idx`** — `for i in 0..len` over the three slices with nested
  `algebraic_add`. The bounds check hoists once; the three
  `SIMD_BUFFER_ALIGN`-aligned column loads fuse into a packed reduction.
* **`zip`** — `iter().zip().zip().fold(_, algebraic_add)`. The nested
  `Zip` adaptors defeated LLVM's column-fusion and ran **slower than
  Bevy** in both RUSTFLAGS settings (~0.90× native), so the `idx` shape
  is the one to use. Reported for completeness.

**Measured** (`g6b_*_10k`, Windows x86_64, criterion 60 samples; the
multi-component rows are the load-bearing ones):

| RUSTFLAGS | shape | boyko median | Bevy median | Ratio (boyko-relative) |
|-----------|-------|--------------|-------------|------------------------|
| default | single | 947.11 ns | 1025.1 ns | 0.92× (boyko ~8 % faster) |
| default | triple `idx` | 2.170 µs | 2.096 µs | 1.04× (boyko ~4 % slower) |
| default | triple `zip` | 2.194 µs | 2.096 µs | 1.05× (boyko ~5 % slower) |
| `-Ctarget-cpu=native` | single | 377.53 ns | 363.07 ns | 1.04× (boyko ~4 % slower) |
| `-Ctarget-cpu=native` | triple `idx` | **1.032 µs** | **1.385 µs** | **0.745× → boyko 1.34× FASTER** |
| `-Ctarget-cpu=native` | triple `zip` | 1.522 µs | 1.385 µs | 1.10× (boyko ~10 % slower) |

The native `idx` win was confirmed across two back-to-back runs
(1.032 µs / 1.070 µs boyko vs 1.385 µs / 1.365 µs Bevy → **1.34× / 1.28×
faster**, both with criterion reporting *"No change in performance
detected"* between runs). The single-component absolute medians drifted
up versus the Wave 8 documented 890/851 ns values (different machine
state, wider criterion CIs + 10–16 % high-outlier counts); the
**within-run ratio** is the comparable quantity, not the absolute.

**Verdict — Phase X.A.1: CREDIBLE WIN (1.10–5× band), native-gated.**
The multi-component ratio **is wider** than the single-component case,
but only with `-Ctarget-cpu=native`:

* Native `idx`: **boyko 1.28–1.34× faster** — crosses the plan's
  ≥ 1.10× "credible win" threshold (Risk 5 last bullet), confirming the
  batched API is worthwhile where wide SIMD is enabled. The headline
  **5× target is still NOT met** even on this wider workload.
* Default RUSTFLAGS `idx`: ~parity (boyko ~4 % slower). The win is
  SIMD-width-gated: at SSE2 baseline both engines bottleneck on the same
  scalar/128-bit throughput envelope; the per-row tuple-fetch overhead
  only dominates once the aligned-column AVX path widens boyko's
  reduction faster than Bevy's cursor walk can feed it.

This is exactly the Risk 5 last-bullet outcome the plan budgeted for:
*"If the 5× bar fails on the single-component bench but passes on the
multi-component bench, that's still a credible win."* The 5× headline
remains a wider-workload aspiration (more component columns, or a
genuinely SIMD-heavy body such as a vec3 normalise) — filed below.

With `algebraic_add` (not `black_box` per-element), both engines now
autovectorize the inner loop. The per-row state-machine cost on the
Bevy side amortises into the same throughput envelope on a single
scalar component. The architectural shape of the `for_each_chunk`
dispatcher (the load-bearing win — a flecs-style batched API not
available in Bevy 0.18) holds regardless of the single-component
perf ratio.

**Filed:** Phase X.A.1 — single-component re-tune. Status of the
investigation paths:

- **Multi-component variant (`g6b_*`): DONE — credible win confirmed.**
  boyko `idx` runs **1.28–1.34× faster** than Bevy on the native-SIMD
  3-component reduction (≥ 1.10× threshold met; 5× not met). ~parity at
  default RUSTFLAGS. See the "Phase X.A.1 — multi-component" subsection
  above for the full table + verdict.
- `cargo asm` inspection (deferred: tool not installed in dev
  environment) to identify any boyko-specific dispatch dead-weight.
- Wider SIMD lane harness (AVX-512 via `cfg(target_feature)`) and a
  genuinely SIMD-heavy body (vec3 normalise) to chase the 5× headline.

The Risk 5 last-bullet mitigation from the plan is in effect:
*"If the 5× bar fails on the single-component bench but passes on the
multi-component bench, that's still a credible win — file Phase X.A.1
(single-comp re-tune) as follow-up rather than blocking the phase."*
The multi-component bench delivered that credible win (native-gated).
Phase X.A lands.

## Code footprint

| File | Lines added | Status |
|------|-------------|--------|
| `crates/boyko_ecs/src/ecs/constants.rs` | +10 | SIMD_BUFFER_ALIGN constant |
| `crates/boyko_ecs/src/ecs/memory/component_pool.rs` | +131 / -3 | Alignment lift + debug_assert + test + buffer_ptr doc |
| `crates/boyko_ecs/src/ecs/core/iters/query/filter.rs` | +158 | ArchetypalQueryFilter trait + impls + tests |
| `crates/boyko_ecs/src/ecs/core/iters/query/chunked_data.rs` | +646 (new) | ChunkedQueryData trait + leaf + tuple impls |
| `crates/boyko_ecs/src/ecs/core/iters/query/chunk_iter.rs` | +393 (new) | Sequential driver + 7 tests |
| `crates/boyko_ecs/src/ecs/core/iters/query/par_chunk.rs` | +~570 (new) | Parallel driver + 5 tests |
| `crates/boyko_ecs/src/ecs/core/iters/query/mod.rs` | +5 | Module wiring + ArchetypalQueryFilter re-export |
| `crates/boyko_ecs/src/ecs/core/iters/query/query.rs` | +~100 | Query method bodies + doc updates |
| `crates/boyko_ecs/src/ecs/core/iters/query/query_view.rs` | +~150 | QueryView mirrors |
| `crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs` | 1 token | `BatchingStrategy::chunk_size` pub(crate) |
| `crates/boyko_ecs/tests/chunk_proptest.rs` | new | 3 proptest properties |
| `crates/boyko_ecs/tests/compile_fail_chunk.rs` | new | Trybuild driver + runtime aliasing |
| `crates/boyko_ecs/tests/compile_fail_chunk/*.rs` | 5 new files | Trybuild compile-fail fixtures |
| `crates/bench_bevy_vs_boyko/rust-toolchain.toml` | new | Per-package nightly pin |
| `crates/bench_bevy_vs_boyko/benches/g6_for_each_chunk.rs` | new (173) | SIMD-amenable f32 reduction bench |

Estimated **~2 300 lines** of new code across the engine, ~700 lines
across tests, ~180 lines bench harness.

## Public API surface

### SystemParam (used inside system bodies)

```rust
impl<'w, 's, D: QueryData, F: QueryFilter> Query<'w, 's, D, F> {
    #[inline]
    pub fn for_each_chunk<Func>(&mut self, f: Func)
    where
        D: ChunkedQueryData,
        F: ArchetypalQueryFilter,
        Func: for<'c> FnMut(D::ChunkItem<'c>);

    #[inline]
    pub fn par_for_each_chunk<Func>(&mut self, f: Func, batching: BatchingStrategy)
    where
        D: ChunkedQueryData,
        F: ArchetypalQueryFilter,
        Func: for<'c> Fn(D::ChunkItem<'c>) + Send + Sync;
}
```

### Direct API (used via `EcsMaster::query<D, F>()`)

```rust
impl<'w, D: QueryData, F: QueryFilter> QueryView<'w, D, F> {
    #[inline]
    pub fn for_each_chunk<Func>(&mut self, f: Func) where /* same bounds */;

    #[inline]
    pub fn par_for_each_chunk<Func>(&mut self, f: Func, batching: BatchingStrategy)
    where /* same bounds */;
}
```

### Trait surface

- `pub unsafe trait ArchetypalQueryFilter: QueryFilter {}` — marker
  for archetype-level filters (`With`, `Without`, `Or<F>` where `F`
  propagates, tuples 1..=12).
- `pub unsafe trait ChunkedQueryData: QueryData { type ChunkFetch<'c>;
  type ChunkItem<'c>; fn init_chunk_fetch; unsafe fn set_chunk_readonly;
  unsafe fn set_chunk_mut; unsafe fn fetch_chunk; }` — sibling to
  `QueryData` with per-archetype-slice fetch.
- Leaf impls: `&T`, `&mut T`, `()` and tuples 1..=12.

## Cross-phase residuals + follow-ups

| Item | Status | Filed as |
|------|--------|----------|
| Multi-thread Miri for parallel `par_for_each_chunk` | DEFERRED | Phase 9.1 (Scope::spawn protected-tag) |
| Single-component bench 5× target | NOT MET (~parity, both engines autovec) | Phase X.A.1 re-tune |
| Multi-component bench (`g6b_*`) 5× target | NOT MET — but **CREDIBLE WIN** (boyko 1.28–1.34× faster, native-SIMD only; ~parity default RUSTFLAGS) | Phase X.A.1 DONE |
| 5× headline on a wider/SIMD-heavy workload (≥4 columns or vec3 normalise) + AVX-512 harness | OPEN | Phase X.A.2 |
| `for_each_chunk_with_mask` | OUT OF SCOPE | Phase 13.X opt-in |
| `par_fold_chunks` reducing variant | OUT OF SCOPE | Phase 13.X |
| `Changed<T>` / `Added<T>` / `Ref<T>` / `Mut<T>` chunk support | DELIBERATE EXCLUSION | Phase 13.X via `ChunkedTickedQueryData` |
| `cargo asm` autovec + L1i qualitative check | SKIPPED (tool unavailable) | Run when `cargo-show-asm` available |
| `arch_ptr` mint helper extraction (4 duplicated sites) | NIT (Wave 4 code-review N1) | Wave 4 polish backlog |
| `run_chunk_seq` PAR9 inline-path extraction | NIT (Wave 6 code-review N2) | Polish backlog |
| `tuple_12_max_arity_compiles_and_iterates` unit test | SKIPPED (variadic macro covered by Wave 3 compile-only tests) | Optional Wave 7 expansion |

## Notes for downstream consumers

### When to use chunked vs per-row iter

Use `for_each_chunk` (or `par_for_each_chunk`) when:

- The inner body is a **reduction** over a contiguous slice (sum, dot
  product, max, etc.) where LLVM auto-vectorization helps.
- You want to apply nightly `f32::algebraic_add` / `std::simd` /
  hand-rolled SIMD to a per-archetype window.
- You need **batched** access (e.g. for a SIMD vec3 transform pass over
  every entity in an archetype) without paying the per-row
  `Iterator::next` state-machine cost.

Use `iter()` / `iter_mut()` when:

- You need per-row filtering via `Changed<T>` / `Added<T>` / `Ref<T>` /
  `Mut<T>` (those types are deliberately excluded from the chunked
  API — `iter()` handles them via the existing tick comparison).
- The body is **branchy** per row (early-exit, `if`-heavy logic) —
  vectorization is unlikely to help anyway.
- You need the elegance of `for (a, b) in &mut query` with
  `IntoIterator`.

### Parallel-variant accumulator sizing

`par_for_each_chunk` invokes the closure once per **archetype
sub-range**, not once per archetype. The exact count is derived from
`BatchingStrategy::chunk_size`. Two regimes (see plan § 1.2 + § 2.4):

- **Medium-large archetypes** (`entity_count / worker_count ≥
  MIN_ARCHETYPE_FOR_PARALLEL = 1024`): invocations ≈ `worker_count ×
  batches_per_thread`. Example: 100k-row archetype on 8 workers,
  `batches_per_thread = 1` → 8 closure invocations.
- **Small archetypes** (below the floor): invocations ≈ `entity_count
  / 1024`.

For reductions: size your accumulator (`[AtomicF32; N]` or sharded
TLS) to `worker_count`, **not** to invocation count. A future
`par_fold_chunks` (Phase 13.X) will absorb this discipline into the
API.

## References

- Plan: `docs/PHASE-X.A-PLAN.md`.
- Research: `docs/PHASE-X.A-RESEARCH.md`.
- Critic rounds: `docs/PHASE-X.A-CRITIC-ROUND-1.md`,
  `docs/PHASE-X.A-CRITIC-ROUND-2.md`.
- Roadmap context: `docs/PHASE-13-ROADMAP.md` § Phase X.A.
- Phase 12.5/12.6 memory notes on `algebraic_add` bench shape.
- Round 1+2+3 patches consolidated at the top of the plan
  (`Changes from Rounds 1+2` sub-table).
