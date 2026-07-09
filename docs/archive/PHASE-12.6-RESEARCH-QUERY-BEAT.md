> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 12.6 Research — Beating Bevy on Query::iter

## TL;DR

**Most credible technique on stated workload:** **None of H1-H9 yields 10% on
the current benchmark.** Both engines emit identical 5-instruction asm
(`lea/inc/addss/cmp/j`); the bench uses `acc += black_box(p.x)` per element,
which prevents vectorization regardless of which engine.

**Verdict on 10% feasibility:** **NOT feasible without redefining the workload
or the API.** File Phase 13 to either:
1. Add `Query::for_each_chunk(|slice: &[T]|)` (flecs-style batched API) +
   adapt bench harness to drop per-element `black_box` and use
   `core::intrinsics::fadd_algebraic` for the reduction; OR
2. Broaden the benchmark surface to multi-component / par_iter / change-filtered
   queries — areas where boyko's Phase 9/10 architecture has structural
   headroom that has not been measured against Bevy.

## Profile context

- boyko `iter`: 7.62 µs / 10k = **0.762 ns/entity**.
- Bevy `iter`: 7.58 µs / 10k = **0.758 ns/entity**.
- 10%-faster target: ≤ **0.685 ns/entity**.

Both inner loops: `lea` (row × stride) + `inc row` + `addss xmm0, [base + row*stride]`
+ `cmp row, len` + `j`. The loop is bound by `addss` throughput (1-cycle reciprocal
on Skylake/Zen) PLUS `black_box` opaqueness barrier preventing reordering.

## Hypothesis evaluations

### H1 — SIMD-batched fetch (`vpgatherdd` / `vmovups`)
- **Credibility for stated workload:** LOW (0% gain; bench prevents vectorization).
- **Credibility as new feature:** HIGH (flecs/Unity-DOTS proven path).
- **Cost:** 5-10 days (new API + alignment + tail handling).
- **CLAUDE.md compat:** OK in principle (principle 6 SIMD-friendly).
- **Bevy:** Does NOT batch. Issue [#1990 "Batched ECS Query"](https://github.com/bevyengine/bevy/issues/1990) open since 2021 with `Adopt-Me` label.
- **flecs:** Designed for this. C API: `Position *p = ecs_field(&it, Position, 0); for (i = 0; i < it.count; i++) p[i].x += v[i].x;` — user owns inner loop, compiler auto-vectorizes ([Mertens 4.1 blog](https://ajmmertens.medium.com/flecs-4-1-is-out-fab4f32e36f6)).
- **EnTT:** Per-entity closure (same scalar shape as Bevy/boyko).
- **Unity DOTS / Burst:** `IJobChunk` exposes chunk as native arrays; Burst auto-vectorizes. 27× cited speedup.
- **Verdict:** **DEFER.** Sound technique but requires API + bench redesign.

### H2 — Software prefetching
- **Credibility:** LOW for this workload.
- **Expected gain:** 0-2%.
- **Reason:** 10k × 4 B = 40 KB fits comfortably in HW prefetcher reach. flecs explicitly relies on HW prefetcher.
- **Verdict:** **DROP.**

### H3 — Custom `iter_optimized` for `Query<&T>`
- **Credibility:** LOW.
- **Expected gain:** 0%.
- **Reason:** Asm is already identical. Bevy has no separate optimized path either.
- **Verdict:** **DROP.**

### H4 — Pre-fetched column pointer per archetype (register caching)
- **Credibility:** ALREADY DONE by both engines.
- **Expected gain:** 0% (already realized).
- **boyko evidence:** `ReadFetch::base = column.ptr as *const T` at `data.rs:367-370`.
- **Bevy evidence:** `fetch.base` cached via `set_table` in `fetch.rs`.
- **flecs:** v4.1 added "column pointer per matched table" cache — 2× faster than v4.0.
- **Verdict:** **DROP — already done.**

### H5 — Sparse-set storage
- **Credibility:** LOW for this workload (single component, dense iter).
- **Expected gain:** Negative (sparse-set adds dense-array indirection on top of component load).
- **Reason:** Bevy supports both Table + SparseSet but recommends SparseSet only for add/remove-heavy components, NOT hot iteration.
- **Verdict:** **DROP.**

### H6 — Profile-Guided Optimization (PGO)
- **Credibility:** Low for THIS workload.
- **Expected gain:** 1-5% (PGO surface on a 5-instruction loop is near-zero).
- **Cost:** High tooling complexity.
- **Verdict:** **DEFER** (only useful for full scheduler workload).

### H7 — Aggressive `#[inline(always)]` on fetch hot path
- **Credibility:** Already done (effectively).
- **Expected gain:** 0%.
- **CLAUDE.md compat:** Conflict with principle 7 ("Measured inlining"; `inline(always)` needs profiler evidence).
- **Reason:** Phase 12.5 asm already verifies identical inlining to Bevy.
- **Verdict:** **DROP.**

### H8 — Storage layout: `Box<[T]>` vs `Vec<MaybeUninit<T>>`
- **Credibility:** No difference.
- **Expected gain:** 0%.
- **Reason:** boyko's pool buffer IS a single contiguous arena allocation. Bevy's `Column` IS a `BlobVec`. Equivalent.
- **Verdict:** **DROP.**

### H9 — Per-row tick read overhead (NCD elision)
- **Credibility:** Already done.
- **Expected gain:** 0%.
- **Reason:** Phase 12.5 Track B NCD6 const-folds out the meta plumbing for `&T` queries.
- **Verdict:** **DROP — already done.**

## Comparative table

| Aspect | Bevy 0.18 | flecs v4.1 | EnTT | Unity DOTS | boyko (current) |
|---|---|---|---|---|---|
| Storage | Table + SparseSet | Archetype (column) | Sparse-set | Chunk (16 KB) | Archetype (contiguous) |
| Inner loop API | `iter()` per-row | batched `for (i; i < count; i++)` | `view.each(...)` | `IJobChunk` slice | `iter()` per-row |
| Auto-vectorizes f32 sum | No | **Yes** | No | **Yes** (Burst) | No |
| Inline on fetch | `inline(always)` | `__attribute__((always_inline))` | `inline` (header-only) | Burst-managed | `#[inline]` |
| Column pointer cache | Yes (set_table) | Yes (since v4.1) | implicit | implicit (chunk) | Yes (set_table_*) |
| NCD elision const-fold | Yes | Yes (terms) | N/A | N/A | Yes (Phase 12.5 NCD6) |

## Why 10% is structurally blocked

1. **Inner loop asm is byte-identical.** Phase 12.5 verified this. There is no missing optimization for the compiler to find.
2. **`black_box(p.x)` per element prevents vectorization.** This is a measurement-fairness barrier — both engines see the same constraint. Removing it changes the bench, not the engines.
3. **`addss` throughput floor.** At 1 cycle reciprocal × 10k = 10000 cycles = 3.3 µs @ 3 GHz. Both engines are within 2× of this floor; closing more requires SIMD reduction (`vaddps` over 8 lanes = ~417 cycles = 0.14 µs).
4. **Per-row API mandates scalar shape.** A `for x in query.iter()` loop fundamentally yields one element at a time; even with perfect inlining, the compiler cannot widen this without breaking the API contract.

## Recommended path

**If goal: ship a 10% Bevy beat on the stated workload** — **NOT ACHIEVABLE.**

**If goal: ship a credible 10% beat on a similar-shape workload (multi-component / chunk-iter / par_iter)**:
1. Add `Query::for_each_chunk(|slice: &[T]|)` per-archetype chunked API (analogous to flecs's two-loop pattern). Cost: ~1 week.
2. Adapt bench harness to drop per-element `black_box` and use `core::intrinsics::fadd_algebraic` (nightly) or explicit `std::simd` reduction.
3. Expected gain on the new workload: 5-20× (per orlp.net `fadd_algebraic` and Nick Wilcox `chunks_exact` measurements).

**If goal: surpass Bevy meaningfully without redefining the workload** — file as **Phase 13**:
- (a) Multi-component queries (Transform + Velocity + Mass) — boyko's archetype layout has potential headroom.
- (b) `par_iter` with realistic chunking — boyko's Phase 9 scheduler outperforms Bevy by 3× on the existing 10k bench; broaden the surface.
- (c) Change-filtered queries — boyko's Phase 10 const-folded NCD elision is faster than Bevy's runtime flag check.

## Key sources

- [Bevy iter.rs (main)](https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/query/iter.rs)
- [Bevy fetch.rs](https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/query/fetch.rs)
- [Bevy par_iter.rs](https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/query/par_iter.rs)
- [Bevy #1990 Batched ECS Query](https://github.com/bevyengine/bevy/issues/1990)
- [Bevy #1822 SIMD discussion](https://github.com/bevyengine/bevy/discussions/1822)
- [flecs Queries.md](https://github.com/SanderMertens/flecs/blob/master/docs/Queries.md)
- [Mertens — Building an ECS #2: Archetypes and Vectorization](https://ajmmertens.medium.com/building-an-ecs-2-archetypes-and-vectorization-fe21690805f9)
- [Mertens — Flecs 4.1 is out](https://ajmmertens.medium.com/flecs-4-1-is-out-fab4f32e36f6)
- [EnTT Crash Course (wiki)](https://github.com/skypjack/entt/wiki/Crash-Course:-entity-component-system)
- [skypjack ECS BAF #2](https://skypjack.github.io/2019-03-07-ecs-baf-part-2/)
- [Unity Burst manual](https://docs.unity3d.com/Packages/com.unity.burst@latest/manual/index.html)
- [Rust PR #120718 — fadd_algebraic intrinsic](https://github.com/rust-lang/rust/pull/120718)
- [orlp.net — Taming Floating-Point Sums (2024)](https://orlp.net/blog/taming-float-sums/) — 21.6× on f32 reduction
- [Nick Wilcox — Auto-Vectorization in Rust](https://www.nickwilcox.com/blog/autovec/)
- [LLVM Auto-Vectorization](https://llvm.org/docs/Vectorizers.html)
- [Mike Acton — DOD CppCon 2014](https://neil3d.github.io/assets/img/ecs/DOD-Cpp.pdf)
- [Algorithmica — Prefetching](https://en.algorithmica.org/hpc/cpu-cache/prefetching/)
