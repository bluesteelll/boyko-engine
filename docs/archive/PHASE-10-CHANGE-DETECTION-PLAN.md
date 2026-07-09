> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 10 — Change Detection — Architecture Plan

**Branch:** `ecs`
**Status:** DRAFT v3 (architect output, Round 3; awaits architecture-critic Round 3 review)
**Saved to (intended):** `D:\claude\BoykoEngine\docs\PHASE-10-CHANGE-DETECTION-PLAN.md`
**Predecessor:** Phase 9 (Parallel Scheduler, APPROVED)
**Successor:** Phase 11 (TBD — likely Resource change detection extension OR removal detection)

---

## §0 — Changelog

### Round 3 changelog (vs Round 2)

Round 2 critic returned 1 critical (C-NEW-1) + 1 warning (W-NEW-1) — all targeted cleanup. Round 3 resolves both without introducing new architectural decisions.

- **C-NEW-1 fixed**: `Tick::is_newer_than` formula corrected to Bevy's standard signed-wraparound pattern. The Round 2 implementation `(this_run - last_run) > (self - last_run)` collapsed `age_self == age_this` for `self == this_run`, breaking the inclusive upper-bound semantic (`self ∈ (last_run, this_run]` requires `self == this_run` → true). New formula: `(this_run - last_run) > (this_run - self)`, equivalent to `ticks_since_system > ticks_since_insert`. §4.2 body rewritten + inline comment added; §6.2-bis proof re-derived against the new formula; §10.3 x86 lowering updated; §13.1 added unit test `tick_is_newer_than_self_equal_this_run` (asserts the inclusive upper bound).
- **W-NEW-1 fixed**: `pool.units_len()` (does not exist) replaced with `pool.count()` (the actual `ComponentPool::count(&self) -> usize` accessor at `component_pool.rs:620`). Replacement applied across §1.2, §2.7 WRAP3, §3 Q3.4, §4.6, §9.6, §10.6, §13.5, §14 Step 12, §21 summary — every site that previously read `units_len()`.
- **W-NEW-1b fixed**: §9.6 `world.archetype_master.archetype_ids()` and §4.6 `world.archetype_master.archetypes_iter_mut()` both reference APIs that do NOT exist on `ArchetypeMaster`. The existing API is `pub fn iter_archetypes(&self) -> impl Iterator<Item = &Archetype>` (`archetype_master.rs:590`); for mutation, `ArchetypeBundle::iter_mut()` is available but the bundle field is private. Resolution: §15.1 + §20 add a new one-liner accessor `pub fn iter_archetypes_mut(&mut self) -> ArchetypeBundleIterMut<'_>` on `ArchetypeMaster` mirroring `iter_archetypes()`; §4.6 + §9.6 use `world.archetype_master.iter_archetypes_mut()`. The accessor is trivial (one line, delegates to `self.archetypes.iter_mut()`) and does not constitute a new architectural decision.

### Round 2 (after architecture-critic Round 1)

Round 1 plan ([`PHASE-10-CRITIC-ROUND-1.md`](PHASE-10-CRITIC-ROUND-1.md)) returned 4 criticals + 8 warnings + 5 optionals. All are resolved below.

| Critic note | Fix location in this revision | Disposition |
|-------------|--------------------------------|-------------|
| **C1** — dispatcher cannot write `SystemMeta::this_run_tick` through `Box<dyn System>` (no `meta_mut()` on trait) | §2.6 SCT4 rewritten; §2.8 PHASE9.2 rewritten; §4.5 added `System::set_change_ticks`; §5.4-bis (new) trait-method footprint; §8.2 step 3 rewritten; §14 Step 11 rewritten; §15.1 row "system.rs" added; §20 row 4 rewritten | **Accept option 1** — add `fn set_change_ticks(&mut self, last_run: Tick, this_run: Tick)` accessor on `System` trait. Every impl declares it (no default body). Default-impl-free is intentional: forces every future `System` impl to opt into the contract. |
| **C2** — `QueryIter` / `QueryIterMut` do NOT hold `&SystemMeta` (verified `iter.rs:82-92, 258-268`) | §5.3 expanded with explicit field-extension; §8.2 rewritten; §14 Step 8 rewritten with full impact list; §15.2 added "broken tests" subsection; §20 added rows for `iter.rs`/`query.rs` | **Accept** — add `meta: &'s SystemMeta` field to `QueryIter` / `QueryIterMut`. Update constructors and all callers; document the breakage of existing test fixtures. |
| **C3** — `Mut<T>` deref under `par_iter` writes adjacent `UnsafeCell<u32>` slots on same cache line — data-race-free? | §11.5 expanded with formal citation; §13.3 added new Miri test `miri_par_iter_chunks_write_adjacent_ticks_disjoint_no_ub`; §8.3 footnote added | **Accept** — sound per Rust abstract machine (each `UnsafeCell<u32>` is a distinct memory location; concurrent unsynchronised writes to disjoint memory locations are race-free). Cite [Rustonomicon §"Data Races and Race Conditions"](https://doc.rust-lang.org/nomicon/races.html). The cache-line sharing is a MESI cost (false sharing), not UB. New Miri test verifies. |
| **C4** — `Or<(With<A>, Changed<B>)>` null-base cost missing from §10 perf table; conservative access declaration not documented | §3 Q7.3 expanded; §2.3 FLT2 expanded; §10.3 / §10.4 expanded with the null-base branch row; §17 OQ-12 added (future: per-archetype access narrowing) | **Accept** — null-base check costs ~0.5 ns (predicted-not-taken branch) per row; documented in §10 per-row table. Mirror Bevy: conservative access declaration — `Or<(_, Changed<C>)>` declares read of C globally; scheduler serialises against C writers even on C-absent archetypes. Documented as intentional. |
| **W1** — 40 MB memory bound ignores per-archetype duplication | §1.2 row "Storage overhead" rewritten; §2.2 STORE2 rewritten; §7.2 added per-archetype expansion; §10.7 rewritten with archetype-count axis; §11.6 rewritten; §17 OQ-1 expanded | **Accept** — recompute upper bound. At 1024 archetypes × 10 components/archetype × 1024 max_components × 8 B = 80 MB. At 100 archetypes × 50 components × 1024 × 8 B = 40 MB. At 1000 archetypes × 50 components × 4096 × 8 B = 1.6 GB (stress). Accepted tradeoff; document the per-archetype dimension. |
| **W2** — `MAX_CHANGE_AGE` derivation hand-waved ("Bevy works") | §9.3 rewritten with first-principles derivation; §9.7 reproof | **Accept** — derive from first principles for boyko's per-frame bump. Inequality: `MAX_CHANGE_AGE + CHECK_TICK_INTERVAL < 2^31`. With `CHECK_TICK_THRESHOLD = 518_400_000` and the Bevy formula, the inequality holds. Show the algebra. |
| **W3** — `check_ticks` scan walks `pool.added_ticks.iter()` (full buffer) instead of live rows | §4.6 rewritten with `count()` bound (Round 3 W-NEW-1: was `units_len()`); §9.6 rewritten; §10.6 cost recomputed with the live-rows scan; §13.5 bench updated | **Accept** — scan only the first `count()` slots. Unused slots remain `Tick::ZERO`; clamping them is harmless but pointless. Correct cost: 100 k × 50 × 2 = 10 M ops (live), not buffer-size × archetype-count = potentially 100 M ops. |
| **W4** — `Commands::spawn` tick threading contradiction (INIT2 vs INIT3) | §2.4 INIT2 dropped; INIT3 rewritten to be the canonical path; §15.1 row "Commands::spawn" rewritten | **Accept** — `EcsMaster::create_entity` reads `self.change_tick.load(Relaxed)` internally. Drop INIT2's "dispatcher passes down". Single source of truth: the world owns the tick. |
| **W5** — `last_run` init bypass when bypassing `FunctionSystem::initialize` (NoopSystem etc.) | §4.2-bis (new) added; §14 Step 3 rewritten; §15.1 rows added; §15.2 expanded; §13.6 added new debug_assert | **Accept** — change `SystemMeta::new(name, current_tick: Tick)` to take the tick. Every callsite (FunctionSystem, ExclusiveFunctionSystem, NoopSystem test stub, any future System impl) must pass it. No special-case `initialize()` path needed. |
| **W6** — `ParQuery` / `ParQueryMut` field plumbing missing inline + fallback paths | §8.4 rewritten with explicit `run_chunk_inline` / `run_chunk_owned` / PAR7-fallback enumeration; §13.2 added `par_iter_inline_path_reports_changed` integration test; §14 Step 8 expanded | **Accept** — enumerate every chunk dispatch path. `run_chunk_inline`, `run_chunk_owned`, PAR7 fallback all receive `meta`. New test for inline path. |
| **W7** — `set_table_*` `meta` lifetime annotation unspecified | §5.3 trait signature rewritten with `meta: &'_ SystemMeta`; §5.3 added note "meta is read-only input; ticks copied into Fetch by value" | **Accept** — annotate `meta: &'_ SystemMeta`. Document: ticks are `Copy`-extracted into `Fetch<'w>` at `set_table_*` time; `meta`'s lifetime is purely input-not-stored. |
| **W8** — `Or<(Added, Changed)>` archetype scan dominance | §10.5-bis (new) added; §13.5 added `bench_or_added_changed_archetype_count_dominated`; §17 OQ-12 added | **Accept** — document that `Or<F>::aggregate_include` is no-op per Phase 8b M8, so `Or<(Added, Changed)>` queries walk every archetype. Add bench. |
| **O1** — `is_changed` for same-system writer-reader semantics | §6.2 MUT6 / REF3 rewritten with `>=` semantic; §13.1 added unit test `mut_is_changed_after_self_write_observes_change` | **Accept** — Bevy's semantic: `is_changed` uses `>=` not `>` so that `self == this_run` reports true. Specifically: `age_self >= age_this` for same-system observation? Actually the right form is to check `tick.is_newer_than(meta.last_run_tick.wrapping_sub(1), meta.this_run_tick)` — equivalent to "tick > last_run - 1" i.e. inclusive. Detailed proof in §6.2-bis. |
| **O2** — `CachePadded` on `change_tick` over-engineered | §4.4 rewritten — `CachePadded` removed; `change_tick: AtomicU32` directly | **Accept** — no false sharing risk per §11.5 (touched once per frame). Saves 60 B of padding. |
| **O3** — inlining strategy unstated for hot path | §13.5-bis (new) added; §14 Step 16 amended | **Accept** — add note: `bench_changed_filter_1024_rows` is verified against `cargo asm` to confirm `is_newer_than` and `filter_fetch` inlined. PGO measurement deferred to follow-up. |
| **O4** — `Tick: !Default` | §4.2 added `impl Default for Tick` returning `Tick::ZERO` | **Accept** — trivial; `Tick::ZERO` is the canonical default. |
| **O5** — naming `last_run_tick` vs `last_run` divergence | §2.6 SCT1 rewritten; §6.1 / §6.2 rewritten with consistent field naming; §11.1 / §11.2 updated; all `last_run`/`this_run` site-naming harmonised | **Accept** — harmonise to `last_run` and `this_run` everywhere. `SystemMeta` fields become `last_run: Tick` and `this_run: Tick` (drop the `_tick` suffix). Mirrors Bevy and `SystemChangeTick { last_run, this_run }`. |

### Round 1 (initial)

Initial design after consuming `docs/PHASE-10-RESEARCH.md` and consulting the live `ecs` branch sources. Twelve architectural questions Q1..Q12 each resolved with measured justification; all decision biases from the brief honoured:
- **Q1 storage option = A** (per-row 8 B; Bevy pattern) — chosen with concrete justification §3 Q1.
- **Q2 tick bump policy = per-`Schedule::run` frame bump + per-system snapshot** (Bevy G1 pattern) — §3 Q2.
- **Q3 wraparound = `MAX_CHANGE_AGE` clamp** at `u32::MAX - (2*CHECK_TICK_THRESHOLD - 1)` — §3 Q3.
- **Q4 `Mut<T>` deref guard = full guard struct** with `DerefMut` bumping the changed tick — §3 Q4.
- **Q5 atomic tick writes = per-system snapshot, non-atomic per-row writes** (Bevy pattern; Phase 9 conflict graph supplies happens-before) — §3 Q5.
- **Q6 `Changed<T>` semantics = Bevy deref-bump** + opt-in `set_if_neq` — §3 Q6.
- **Q7 integration with `IS_ARCHETYPAL` = retain const-fold**, `Added`/`Changed` are non-archetypal — §3 Q7.
- **Q8 storage layout = parallel `Box<[UnsafeCell<Tick>]>` columns per `Column`** (Bevy split, post-PR #6547) — §3 Q8.
- **Q9 per-archetype change version = NO** (option B rejected; per-row gives required granularity) — §3 Q9.
- **Q10 Send/Sync = plain `u32` is `Send + Sync` trivially**; `UnsafeCell<Tick>` is `!Sync` but column wraps it under aliasing-discipline contract — §3 Q10.
- **Q11 `SystemMeta::last_run` update = at apply-window-end** (single mutator: dispatcher) — §3 Q11.
- **Q12 single `Tick` + `Mut<T>` writes only `changed` tick** (Bevy split `added`/`changed`) — §3 Q12.

This plan deliberately mirrors Bevy's `Tick` design because it is the only mainstream ECS pattern that:
- Composes with the existing `Query<D, F>` DSL via `Or<F>` (already shipped Phase 8b).
- Provides per-entity granularity (Unity DOTS chunk-level is too coarse for boyko archetypes-of-millions).
- Is sound under the Phase 9 conflict-graph model (writes through `UnsafeCell<Tick>` riding the same `(archetype, component)` exclusivity guarantee).

---

## §1 — Summary

### 1.1 Goal

Phase 10 ships a **per-row tick-based change detection system** modelled after Bevy ECS (post-PR #6547). After Phase 10:

1. Every `(entity, component)` pair carries two `u32` ticks: `added` and `changed`.
2. A new `Tick(u32)` newtype with wrapping-safe comparison.
3. An `EcsMaster::change_tick: AtomicU32` global counter, bumped once per `Schedule::run` frame.
4. Per-system `last_run: Tick` + `this_run: Tick` snapshot — stored on `SystemMeta`.
5. `Added<T>` / `Changed<T>` filters (non-archetypal) that compose with Phase 8b `Or<F>` and tuple filters.
6. `Ref<T>` / `Mut<T>` `QueryData` impls with deref-guard semantics for `Mut<T>`.
7. `set_if_neq` and `bypass_change_detection` escape hatches.
8. Wraparound handling via `MAX_CHANGE_AGE` clamp + `check_ticks` scan invoked from the apply window every `CHECK_TICK_THRESHOLD` ticks.
9. Sound under the Phase 9 conflict graph: per-row tick writes ride the existing `(archetype, component)` exclusivity (no new atomics).
10. **NEW (Round 2)** — `System::set_change_ticks(last_run, this_run)` trait method is the single dispatcher→system tick-write channel; eliminates the C1 invariant violation.

### 1.2 Target metrics (acceptance gates)

| Operation | Target | Source / justification |
|-----------|--------|------------------------|
| `Tick::is_newer_than` (single compare) | ≤ 1 ns | 2 × `wrapping_sub` + `>` lowers to ~3 x86 instructions; predictable branch. Bevy reports compatible numbers. |
| Per-row `Changed<T>` filter (hot, autovectorised) | ≤ 1 ns/row | u32 load + 2 sub + compare; the iterator monomorphises against `F::IS_ARCHETYPAL = false` and the compiler can vectorise the predicate fold (§10). |
| Per-row `Changed<T>` filter (cold, first archetype touch) | ≤ 2 ns/row | One additional cache miss (tick column base) per archetype boundary; amortised. |
| Per-row `Or<(_, Changed<C>)>` filter (null-base check on A-only archetype, **NEW Round 2 C4**) | ≤ 1.5 ns/row | adds ~0.5 ns predicted-not-taken branch for the null-base check on every row. Bevy mirrors this. |
| `Mut<T>::deref_mut` (the bump) | ≤ 1 ns | One u32 store via `UnsafeCell::get()`. Bevy benches confirm. |
| Frame-start `change_tick.fetch_add(1, Relaxed)` | ≤ 5 ns | x86 LOCK ADD on a (non-CachePadded — Round 2 O2) slot. 1 atomic / frame. |
| `check_ticks` scan (whole world, cold path, 100 k × 50 components live rows) | ≤ 10 ms | Bevy reports < 10 ms for typical worlds; runs every ~24 h of continuous play (§9). **Note (Round 2 W3 + Round 3 W-NEW-1): scan iterates `pool.count()` live rows, not buffer length.** |
| Storage overhead (per entity, per tracked component) | 8 B | Two `Tick = u32` per row, parallel to component column. **Note (Round 2 W1): per-archetype duplication.** At 100 archetypes × 50 components × 1024 max_components × 8 B = 40 MB; at 1024 × 10 × 1024 = 80 MB; at 1024 × 50 × 4096 = 1.6 GB stress. Accepted tradeoff; see §10.7 + §11.6 for the per-axis breakdown. |
| `Query::iter` overhead added by Phase 10 when neither `Added` nor `Changed` is used | 0 ns | Existing const-fold path; no extra branches. |
| Phase 9 parallel-scheduler regression budget | 0 % | Tick writes piggyback on the existing `(archetype, component)` exclusivity; no new atomics, no new contention. Dispatcher overhead: ~5 + ~100 ns per frame for tick wiring (§8.5). |
| LOC | 3 500 – 5 500 production + 2 500 – 3 500 test | Phase 10 is medium; foundations exist (Phase 7 storage, Phase 8b filter trait, Phase 9 apply window). |
| Step count | 16 Steps (parallelisable pairs marked in §14) | Smaller than Phase 9 (24 Steps) because most subsystems are extensions, not new crates. |
| Calendar weeks (single developer) | 3-4 weeks | Phase 8b (12 Steps over ~3 weeks) is a good comparator. |

### 1.3 Subsystems delivered

- **A.** `Tick(u32)` newtype + wraparound machinery (`is_newer_than`, `check_tick`, `MAX_CHANGE_AGE`).
- **B.** `EcsMaster::change_tick: AtomicU32` + `Schedule::run` frame bump + per-system `last_run` / `this_run` snapshots stored on `SystemMeta`.
- **C.** Per-column tick storage on `Column` / `ComponentPool`: `added_ticks: Box<[UnsafeCell<Tick>]>` + `changed_ticks: Box<[UnsafeCell<Tick>]>` parallel to component data.
- **D.** `Added<T>` and `Changed<T>` `QueryFilter` impls (non-archetypal).
- **E.** `Ref<T>` and `Mut<T>` `QueryData` impls with deref guard.
- **F.** `set_if_neq` and `bypass_change_detection` on `Mut<T>` (Bevy escape hatches).
- **G.** `SystemChangeTick` `SystemParam` exposing `this_run` / `last_run` to user code.
- **H.** `check_ticks` clamp scan integrated into `Schedule::run` apply window.
- **I.** `Or<(Added<A>, Changed<B>)>` composition (Phase 8b `Or<F>` already supports — verified §15).
- **J.** Tick initialisation on entity insertion / archetype transitions.
- **K. (NEW Round 2 C1)** — `System::set_change_ticks(&mut self, last_run, this_run)` trait method. Single dispatcher→system channel for tick writes. Every `System` impl declares the method (no default body).

### 1.4 What Phase 10 deliberately defers

- **Resource change detection** (`Res<T>` / `ResMut<T>` ticks) — Phase 10.5 (separate, smaller follow-up). The infrastructure here lays the foundation; extending to Resources is a thin wrapper over `Tick`.
- **`RemovedComponents<T>` event buffer** — Phase 11 (out of brief scope; separate event-style design).
- **`Mutated<T>` (mutated-but-not-added) filter** — Bevy Issue #15070 is still open; we adopt Bevy semantics now and revisit.
- **`#[derive(Component, NoChangeTracking)]` opt-out** — §17 OQ-1; nice-to-have but not blocking.
- **Hierarchical archetype-level tick early-exit** (Bevy Issue #5097) — same as Bevy: per-row only in v1.
- **Byte-compare `ChangedDeep<T: PartialEq>` filter** — out of scope; `set_if_neq` is the explicit opt-in path (§7).
- **Per-archetype access narrowing for `Or<F>`** — §17 OQ-12. Currently `Or<(_, Changed<C>)>` declares read of C globally even when running on C-absent archetypes. Mirrors Bevy.

---

## §2 — Invariants

Naming scheme: `TICK` = `Tick` type + global counter, `STORE` = per-column tick storage, `FLT` = `Added`/`Changed` filters, `MUT` = `Mut<T>` deref guard, `REF` = `Ref<T>`, `SCT` = `SystemChangeTick`, `WRAP` = wraparound + `check_ticks`, `INIT` = tick initialisation on insertion, `PHASE9` = integration with Phase 9 scheduler / apply window.

### 2.1 `Tick` type (TICK1..TICK8)

- **TICK1** — `Tick` is `#[repr(transparent)] struct Tick(u32)` with `Copy + Clone + PartialEq + Eq + Hash + Default` (Round 2 O4: `Default → Tick::ZERO`). Sized 4 B, aligned 4 B. No padding.
- **TICK2** — `Tick::new(value: u32) -> Self` is `const`. `Tick::ZERO` constant.
- **TICK3 (Round 3 C-NEW-1 corrected)** — `Tick::is_newer_than(self, last_run: Tick, this_run: Tick) -> bool` returns `true` iff `self` falls in the half-open window `(last_run, this_run]`. Implementation uses **wrapping subtraction** mirroring Bevy's standard signed-comparison-via-wraparound technique:
  ```text
  ticks_since_insert = this_run.wrapping_sub(self)
  ticks_since_system = this_run.wrapping_sub(last_run)
  ticks_since_system > ticks_since_insert
  ```
  Equivalent semantic: `self ∈ (last_run, this_run]` accounting for u32 wraparound. The Round 2 formula `age_this > age_self` was symmetric and collapsed `self == this_run` to `false` (broken upper bound); the Round 3 formula correctly returns `true` for `self == this_run` (inclusive upper bound) and `false` for `self == last_run` (exclusive lower bound).

  Both subtraction operands are bounded by `MAX_CHANGE_AGE`; `check_ticks` (WRAP3) preserves the bound.

- **TICK4** — `Tick::check_tick(&mut self, current: Tick) -> bool` clamps `*self` if `current.wrapping_sub(*self) > MAX_CHANGE_AGE`; returns `true` if a clamp happened. The clamped value is `current.wrapping_sub(MAX_CHANGE_AGE)` (the oldest still-valid tick).
- **TICK5** — `MAX_CHANGE_AGE: u32 = u32::MAX - (2 * CHECK_TICK_THRESHOLD - 1)` ≈ `3_258_166_895` (~75% of `u32::MAX`). Bevy's exact number. **Derivation in §9.3 (Round 2 W2 first-principles proof).**
- **TICK6** — `CHECK_TICK_THRESHOLD: u32 = 518_400_000` (Bevy's value).
- **TICK7** — `EcsMaster::change_tick` is `AtomicU32` (NOT `CachePadded` after Round 2 O2) with `Ordering::Relaxed` for the per-frame `fetch_add(1)`. Relaxed suffices because:
  1. The fetched value is unique to the current `Schedule::run` invocation (only one such invocation at a time per master — enforced by `&mut EcsMaster` borrow).
  2. Happens-before for the per-row tick writes is supplied by the Phase 9 conflict graph (SCH3): no two systems can write the same `(archetype, component)` simultaneously; the per-row tick column inherits this exclusivity.
- **TICK8** — When a system is first initialised, its `last_run = current_tick.wrapping_sub(MAX_CHANGE_AGE)`. Never `Tick::ZERO`. **Constructor-enforced (Round 2 W5): `SystemMeta::new(name, current_tick)` writes this directly.**

### 2.2 Per-column tick storage (STORE1..STORE10)

- **STORE1** — Each `ComponentPool` (Phase 7) gains two parallel buffers: `added_ticks: Box<[UnsafeCell<Tick>]>` and `changed_ticks: Box<[UnsafeCell<Tick>]>`. Their length equals the pool's `max_components` (the data buffer's row capacity). Indices match: row `i` in the data buffer corresponds to row `i` in both tick buffers.
- **STORE2** — Tick buffers are allocated on the **global allocator** (`Box`), **not** the arena. Justification: tick buffers grow with the pool's `max_components` (set at `ComponentPool::new`); a pool grow event (Phase 7-future) would require coordinated reallocation of all three buffers — the global allocator's `realloc` path is the standard; keeping tick buffers off the arena avoids tangling them with the arena's free-list logic.

  **Storage cost analysis (Round 2 W1 — corrected for per-archetype duplication):**

  Ticks live in `ComponentPool`. A component `C` in `K` distinct archetypes has `K` independent `ComponentPool`s for `C`. Each pool sizes its tick buffers to `max_components` (the pool's row capacity), NOT to the live entity count.

  Upper bound formula:
  ```text
  total_tick_bytes = Σ_archetypes Σ_components_in_archetype (2 × 4 B × max_components_of_pool)
                  = 8 B × Σ pool.max_components
  ```

  Concrete points:
  | Configuration | Per-archetype components | max_components/pool | Archetypes | Total tick storage |
  |---------------|--------------------------|---------------------|------------|---------------------|
  | Small game | 10 | 1024 | 16 | 1.25 MB |
  | Medium game | 20 | 2048 | 64 | 20 MB |
  | AAA target (the original §1.2 number) | 50 | 1024 | 100 | 40 MB |
  | Stress (the W1 example) | 10 | 1024 | 1024 | 80 MB |
  | Stress (the W1 worst) | 50 | 4096 | 1024 | 1.6 GB |

  At the design point (100 archetypes × 50 components/archetype × 1024 max_components × 2 ticks × 4 B), the original 40 MB number is preserved. Higher archetype counts and larger pool capacities scale linearly. **Accepted tradeoff** vs alternative C (per-archetype-per-column scalar, cheaper but less precise); see §3 Q1 and §10.7.

- **STORE3** — `UnsafeCell<Tick>` is the canonical Bevy pattern. The `UnsafeCell` documents interior mutability across the `&Archetype` shared reference; the actual write is unsynchronised (no atomic). Soundness rests on the access-declaration contract (Phase 9 SCH3): the system writing component `C` holds exclusive access to `(archetype, C)`, so no other thread reads or writes the corresponding `changed_ticks[row]`.
- **STORE4** — On entity insertion (`Archetype::create_entity`): `added_ticks[row] = changed_ticks[row] = current_tick`. The current tick is threaded through `Archetype::create_entity` — its signature changes (§14 Step 5). **Tick source (Round 2 W4 resolution): `EcsMaster::create_entity` reads `self.change_tick.load(Relaxed)` internally and threads it.**
- **STORE5** — On `swap_remove` (`Archetype::remove_entity`): both tick columns swap-remove in lockstep with the data buffer. No tick is invalidated; the swap moves the last row's ticks into the vacated slot.
- **STORE6** — On `pop` (last row removal): the last tick slot is logically dropped (no destructor; `Tick` is `Copy`).
- **STORE7** — `Column` (Phase 7's hot inline table, 16 B per column slot) **does not** carry per-component tick pointers. Reasoning: `Column` is at offset 0 in `Archetype` for the single-dependent-load fast path (Phase 7 D4); adding tick pointers per column would double the struct to 48 B per slot × 512 components = 24 KB per archetype (3× current 8 KB). The tick column pointers live inside `ComponentPool` (already heap-side; one cache line per pool's metadata is acceptable). The hot iterator path looks up the tick column pointer once per archetype boundary in `set_table_*` and caches it in `Fetch<'w>` — no per-row pointer chase.

  Update: §4.3 defines a separate **tick column lookup helper** on `Archetype` that resolves the tick column base pointers from `component_pools` (sparse map). Cold per-archetype-boundary cost (~5 ns) — same as the existing per-archetype `set_table` dispatch.

- **STORE8** — Tick column reads in `filter_fetch` / `Ref::fetch` / `Mut::deref_mut` are non-atomic plain u32 loads. Phase 9 conflict graph (SCH3) ensures no concurrent writer.
- **STORE9** — `&UnsafeCell<Tick>` is `!Sync`. To allow `Archetype: Sync` (Phase 9 SEND10), the `ComponentPool::added_ticks` / `changed_ticks` fields are exposed only through `unsafe` accessors. The contract on the accessor is the same as for component data: the caller has satisfied Phase 9 SCH3.
- **STORE10** — Tick buffers are zero-initialised at allocation. Logical "first ever value" is `Tick::ZERO`. But §2.1 TICK8 / §2.4 INIT1 ensure `Tick::ZERO` is never a meaningful comparand: a system's `last_run = current - MAX_CHANGE_AGE` so any `added/changed = Tick::ZERO` is reported as "changed" on the first observation (the desired semantic).

### 2.3 `Added<T>` / `Changed<T>` filters (FLT1..FLT8)

- **FLT1** — `Added<C: Component>` and `Changed<C: Component>` are `QueryFilter` impls with `IS_ARCHETYPAL = false`. They reuse the Phase 8b filter trait surface (`init_state`, `init_access`, `matches_component_set`, `aggregate_include`, `set_table_*`, `filter_fetch`).
- **FLT2 (Round 2 C4 expanded)** — `Added<C>` and `Changed<C>` declare a component **read** of `C` via `init_access`. The filter inspects the per-row tick (logically a property of `C`'s lifecycle), so the read declaration keeps the intra-system aliasing detector consistent (mirrors `With<C>`'s declaration).

  **Conservative declaration consequence for `Or<F>` (Round 2 C4):** `Or<(_, Changed<C>)>` declares read of `C` at access-set level even when the system runs only on archetypes lacking `C`. The Phase 9 conflict graph will serialise this system against any concurrent writer of `C` — even when the system is "ineffective" on the C-absent archetypes. **This is intentional** and mirrors Bevy: per-archetype access narrowing would require a runtime per-archetype access-set computation per system. Deferred to §17 OQ-12.

- **FLT3** — `Added<C>::matches_component_set(mask)` and `Changed<C>::matches_component_set(mask)` return `mask.contains(C::component_id())`. Same archetype-level predicate as `With<C>` — the filter cannot operate on an absent column.
- **FLT4** — `Added<C>::aggregate_include` and `Changed<C>::aggregate_include` set the bit for `C` in the include mask, ensuring `QueryDataState::update_archetypes` selects only archetypes that contain `C` (otherwise the tick column pointer cache would be NULL). **Note: `Or<F>::aggregate_include` is a no-op per Phase 8b M8 (filter.rs:603-606), so `Or<(Added<A>, Changed<B>)>` walks every archetype; see §10.5-bis Round 2 W8.**
- **FLT5** — `Added<C>::Fetch<'w>` and `Changed<C>::Fetch<'w>` both cache:
  - A `*const UnsafeCell<Tick>` pointer to the start of the relevant tick column for the current archetype.
  - A `Tick` copy of the system's `last_run`.
  - A `Tick` copy of the system's `this_run`.

  Size: 24 B (one fat-ish pointer + 8 B of ticks; natural alignment).

  `Copy` per Phase 8b `Fetch<'w>: Copy` requirement.
- **FLT6** — `Added<C>::filter_fetch(fetch, row)` and `Changed<C>::filter_fetch(fetch, row)`:
  ```text
  if fetch.tick_base.is_null() { return false; }   // Or<F> null-base path (Round 2 C4)
  let tick = *fetch.tick_base.add(row).get();
  tick.is_newer_than(fetch.last_run, fetch.this_run)
  ```
  Single u32 load + 2 wrapping subs + compare. ~3 instructions on x86 (~1 ns hot) + a predicted-not-taken null check (~0.5 ns). Per-row cost ≤ 1.5 ns including null check (§10).
- **FLT7** — `set_table_readonly` and `set_table_mut` are symmetric. Both call a new helper `Archetype::tick_column_base(component_id, kind: TickKind)` returning `*const UnsafeCell<Tick>` (resolved from the archetype's `component_pools` sparse map). Reads the system's `last_run` / `this_run` from `&SystemMeta` (passed as parameter — Round 2 W7).
- **FLT8** — `Or<(Added<A>, Changed<B>)>` composition: Phase 8b's `impl_or_filter_tuple!` macro folds `IS_ARCHETYPAL` as AND, so `Or<archetypal + non-archetypal>` becomes non-archetypal — exactly the required behaviour. No changes to the macro (§15 confirms).

### 2.4 Entity insertion + tick init (INIT1..INIT5)

- **INIT1** — `Archetype::create_entity` gains a new parameter `current_tick: Tick`. Tick is written to both `added_ticks[row]` and `changed_ticks[row]` for every component in the entity's bundle.
- **~~INIT2~~** — **DROPPED (Round 2 W4).** The dispatcher does NOT thread the tick into `Commands::spawn` apply explicitly. Tick threading is internal to `EcsMaster::create_entity` (INIT3).
- **INIT3** — Canonical path: `EcsMaster::create_entity` reads `self.change_tick.load(Relaxed)` internally and threads `current_tick` into `Archetype::create_entity`. All callers (test API, `SpawnCommand::apply` from Phase 8.5) go through `EcsMaster::create_entity`; no caller needs to know the tick.
- **INIT4** — Archetype migration (entity moves from archetype A to archetype B because a component was inserted / removed — out of Phase 10 scope, but the API must not preclude it): future `move_entity` (Phase 11+) must initialise the destination archetype's tick slots for newly-added components; pre-existing components carry over their ticks. Documented in §17 OQ-3.
- **INIT5** — When a `ComponentPool` is grown (Phase 7-future): the tick buffers grow in lockstep. The grow code path (today inside `ComponentPool::push`, dispatcher-only per Phase 9 ALLOC2) calls `Box::new(...) → reallocate the parallel buffers → copy old contents`. Cost is amortised by the pool's exponential growth strategy.

### 2.5 `Ref<T>` / `Mut<T>` SystemParam (MUT1..MUT8, REF1..REF4)

- **MUT1** — `Mut<'w, T>` is a struct with fields:
  ```text
  value: &'w mut T
  added: &'w UnsafeCell<Tick>
  changed: &'w UnsafeCell<Tick>
  this_run: Tick
  last_run: Tick   // (Round 2 O5: harmonised naming — no _tick suffix)
  ```
  Size: ~40 B (3 references + 2 ticks). `!Copy + !Clone` (it owns a mutable borrow).
- **MUT2** — `Mut<'_, T>: Deref<Target = T>`. `deref` returns `&*self.value`. **Does NOT bump** the tick — read-only access through Deref is free.
- **MUT3** — `Mut<'_, T>: DerefMut`. `deref_mut`:
  ```text
  unsafe { *self.changed.get() = self.this_run; }
  &mut *self.value
  ```
  Single u32 store. The `unsafe` block has the SAFETY note: "the system declared a write to `C`; Phase 9 SCH3 ensures no concurrent reader of this column's tick".
- **MUT4** — `Mut<T>::set_if_neq(&mut self, new: T) -> bool` where `T: PartialEq`:
  ```text
  if *self.value != new {
      *self.value = new;
      unsafe { *self.changed.get() = self.this_run; }
      true
  } else {
      false
  }
  ```
  Does not deref-mut through the guard; writes the tick only on inequality.
- **MUT5** — `Mut<T>::bypass_change_detection(&mut self) -> &mut T` returns the raw `&mut T` without bumping the tick.
- **MUT6 (Round 2 O1 — semantic clarified)** — `Mut<T>::is_changed(&self) -> bool` and `Mut<T>::is_added(&self) -> bool` expose change-detection introspection without filtering, **with `>=` semantic so a self-write in the same system reports as changed**:
  ```text
  is_changed: unsafe { *self.changed.get() }.is_newer_than(self.last_run.wrapping_sub(Tick::new(1)), self.this_run)
  ```
  Equivalent semantic: `tick > (last_run - 1)` which equals `tick >= last_run`. Concrete proof: after a system mutates a row, `*changed.get() = this_run`. If the SAME system then reads `is_changed`, then `last_run < this_run` (initialised at `current - MAX_CHANGE_AGE` or last frame), so `this_run >= last_run` and `is_newer_than(last_run - 1, this_run)` returns `true` for `tick = this_run`. See §6.2-bis for details (Round 3 C-NEW-1 reproof against corrected formula).
- **MUT7** — `Mut<T>::into_inner(self) -> &'w mut T` consumes the guard and bumps the tick once. Used when forwarding to APIs that take `&mut T` directly.
- **MUT8** — `Mut<T>` is `QueryData` (not `SystemParam` directly — though see §6.2 for the lift). `IS_READ_ONLY = false`. `Item<'w> = Mut<'w, T>`.
- **REF1** — `Ref<'w, T>` has the same field layout as `Mut<'w, T>` but `value: &'w T` (immutable) and no `DerefMut` impl. Implements only `Deref`.
- **REF2** — `Ref<'_, T>: ReadOnlyQueryData`. `IS_READ_ONLY = true`. `Item<'w> = Ref<'w, T>`.
- **REF3 (Round 2 O1)** — `Ref<T>::is_changed(&self) -> bool` / `is_added(&self) -> bool` use the same `>=` semantic as `Mut::is_changed` (MUT6).
- **REF4** — `Ref<T>` is the "i want to read the tick without filtering" path. It compares to `&T` (no tick exposure) and `Changed<T>` (forces filter through the type system).

### 2.6 `SystemChangeTick` SystemParam (SCT1..SCT5) — Round 2 C1 / O5

- **SCT1 (O5 harmonised)** — `SystemChangeTick { this_run: Tick, last_run: Tick }` is `Copy`, sized 8 B.
- **SCT2** — `SystemChangeTick: SystemParam` impl with `State = ()`. `init_access` declares nothing (it reads only the system's own meta + the world's `change_tick` atomic).
- **SCT3** — `get_param` reads:
  - `this_run = system_meta.this_run` (set by the executor at dispatch via `System::set_change_ticks` — Round 2 C1).
  - `last_run = system_meta.last_run` (set by the executor at apply via `System::set_change_ticks` next frame).
- **SCT4 (Round 2 C1 — REWRITTEN)** — How filters and `Mut<T>` / `Ref<T>` reach `this_run` and `last_run`:

  **Critic note (Round 1 C1) resolved.** Original design assumed direct field writes `self.systems[i].meta.this_run_tick = this_run` through `Box<dyn System>` — structurally impossible because `System` trait has no `meta_mut()` accessor.

  **Resolution: extend the `System` trait with `set_change_ticks`:**
  ```rust
  pub unsafe trait System: Send + Sync + 'static {
      // ... existing trait surface ...

      /// Updates the system's tick snapshot before dispatch.
      ///
      /// Called by the dispatcher exclusively. Workers read the resulting
      /// `last_run` / `this_run` through `&SystemMeta` captured by Query /
      /// SystemChangeTick.
      ///
      /// # Invariants
      /// - Called only during the apply-window (no worker live on this system).
      /// - `last_run` is the PREVIOUS frame's `this_run`; `this_run` is the
      ///   current frame's snapshot.
      ///
      /// # No default body
      /// Every System impl declares this explicitly. Forces opt-in.
      fn set_change_ticks(&mut self, last_run: Tick, this_run: Tick);
  }
  ```

  Implementors:
  - `FunctionSystem<F, M>::set_change_ticks`: writes `self.meta.last_run = last_run; self.meta.this_run = this_run;`.
  - `ExclusiveFunctionSystem::set_change_ticks`: same.
  - `NoopSystem` (test stub): same.

  **Flow:**
  1. Frame start: dispatcher computes `this_run = Tick::new(prev + 1)`.
  2. For each system about to be dispatched: dispatcher calls `system.set_change_ticks(meta.this_run_from_last_frame, this_run)` — the PREVIOUS `this_run` is the new `last_run`. This is the **only** write site for both ticks.
  3. Worker reads `&meta` via Query / SystemChangeTick — sees the updated ticks.

  **Why this over option 2 (dispatcher-owned `Box<[(Tick, Tick)]>`):**
  - Keeps the `SystemMeta` as the single source of truth (no parallel state to keep in sync).
  - One trait method (3 lines per implementor) vs threading ticks through every `run_unsafe` call signature.
  - Workers continue reading via `&SystemMeta` — no signature change at the worker boundary.

  Trade-off: adds one trait method to `System`. Already an `unsafe trait` with `set_change_ticks` having no default body forces every System impl to opt in — desired (the contract is non-negotiable).

  `Fetch<'w>` for `Added<C>` / `Changed<C>` / `Mut<T>` / `Ref<T>` captures `(last_run, this_run)` by value (8 B copy) at `set_table_*` time so the per-row hot loop doesn't pay the indirection per row.

- **SCT5** — `SystemChangeTick: ReadOnlySystemParam`. Safe across `par_iter` (it's `Copy`).

### 2.7 Wraparound + `check_ticks` (WRAP1..WRAP5)

- **WRAP1** — The world holds `last_check_tick: Tick` alongside `change_tick: AtomicU32`. Updated whenever a `check_ticks` scan runs.
- **WRAP2** — `Schedule::run` checks at frame start (after the per-frame `change_tick.fetch_add(1)`):
  ```text
  let current = world.change_tick.load(Relaxed);
  if current.wrapping_sub(world.last_check_tick) >= CHECK_TICK_THRESHOLD {
      run_check_ticks_scan(world);
      world.last_check_tick = current;
  }
  ```
- **WRAP3 (Round 2 W3 — corrected; Round 3 W-NEW-1: API renamed)** — `run_check_ticks_scan` walks every stored **LIVE** tick: every archetype's `added_ticks[i]` and `changed_ticks[i]` for every `i < pool.count()` (the existing `ComponentPool::count(&self) -> usize` accessor at `component_pool.rs:620`), plus every system's `last_run` in `SystemMeta`. Calls `Tick::check_tick(current)` on each. Clamping result: `current.wrapping_sub(MAX_CHANGE_AGE)`.

  **Round 1 cost overestimated**: original §10.6 estimate "10 M ops" was correct for live rows but the pseudocode walked `pool.added_ticks.iter()` (full buffer). Corrected pseudocode iterates `0..pool.count()`. See §9.6.

- **WRAP4** — The scan runs **on the dispatcher**, inside the apply window (`running == 0`). Per Phase 9 SCH7 / ALLOC2, this is sound: no worker holds a cell. The dispatcher mutates ticks through `&mut EcsMaster`, which is safe (no aliasing).
- **WRAP5** — Cost: O(live_stored_ticks). At 100 k entities × 50 components × 2 ticks each → 10 M ticks × 1 cycle ≈ 3 ms. Frequency: every `CHECK_TICK_THRESHOLD = 518.4 M` ticks. At 60 FPS × 100 systems → 6 k ticks/sec → scan fires every ~24 hours of continuous play (per-system bump regime). For per-frame bump (boyko's chosen policy): 60 ticks/sec → scan fires every ~100 days. Effectively never on a real frame budget. Even at "1000 ticks/sec sustained heavy schedule": 6 days between scans.

### 2.8 Phase 9 integration (PHASE9.1..PHASE9.6)

- **PHASE9.1** — `Schedule::run`'s frame-start logic, after the existing `frame.wrapping_add(1)` and `scratch.reset_for_frame`:
  ```text
  let prev = world.change_tick.fetch_add(1, Relaxed);
  let this_run = Tick::new(prev.wrapping_add(1));
  // Wraparound check (§2.7 WRAP2).
  ```
  Note: `fetch_add(1, Relaxed)` returns the **previous** value; we want the new value, so `this_run = prev + 1`.
- **PHASE9.2 (Round 2 C1 — REWRITTEN)** — Inside `try_dispatch_ready`, when dispatching system `i`:
  ```text
  let prev_this_run = self.systems[i].meta().this_run;  // safe-getter; was previous frame
  self.systems[i].set_change_ticks(prev_this_run, this_run);
  ```

  This is the **only** site that updates the system's tick state per frame. The dispatcher does this serially before any spawn. Workers see the updated `&SystemMeta` after the spawn (happens-before via the scope-spawn).

  **Why no separate `last_run = this_run` write at apply-window-end (deviation from §2.6 SCT3 Round 1):** consolidating both writes into one `set_change_ticks` call at dispatch time is simpler:
  - At frame N dispatch: `set_change_ticks(prev_this_run_from_frame_N-1, this_run_frame_N)`. The PREVIOUS frame's `this_run` becomes this frame's `last_run`.
  - No apply-window write needed.

  This is functionally equivalent to Bevy's pattern and reduces the dispatcher's per-system bookkeeping to a single call.

  (Note: This is a deliberate simplification from §2.6 SCT3 Round 1; SCT3 is updated above accordingly.)

- **PHASE9.3 (Round 2 C1 — folded into PHASE9.2)** — ~~Inside `apply_window_drain`, when popping completion for system `i` and calling `apply`: update `last_run`.~~ **No longer needed** because PHASE9.2 (above) handles both ticks in one dispatch-time call. The apply window does NOT touch tick state.
- **PHASE9.4 (Round 2 C1)** — Per-system tick lifecycle:
  - System initialisation (Phase 8c `FunctionSystem::initialize`): `meta.last_run = current_tick - MAX_CHANGE_AGE` (set via the new `SystemMeta::new(name, current_tick)` constructor — Round 2 W5).
  - Frame N dispatch: `set_change_ticks(meta.this_run /* previous */, this_run /* new */)`. The previous `this_run` becomes the new `last_run`; the new `this_run` is the current frame snapshot.
  - Worker reads `&meta` — sees both ticks correctly.
  - Frame N+1 dispatch: same pattern.

  **Pre-first-run edge case:** `SystemMeta::new(name, current)` initialises `last_run = current - MAX_CHANGE_AGE`, `this_run = current - MAX_CHANGE_AGE`. On the first dispatch, `set_change_ticks(current - MAX_CHANGE_AGE /* old this_run */, current + 1)` → `last_run = current - MAX_CHANGE_AGE`, `this_run = current + 1`. Anything with `added/changed > current - MAX_CHANGE_AGE` reports as added/changed on first run. Correct.

- **PHASE9.5** — Per-system snapshot is **per dispatch round**. Across multiple rounds in one frame, `this_run` stays the same (the frame-level value); we only update `last_run` at dispatch time (which equals "previous frame's this_run", which equals "previous frame_level value" for all systems).

  Subtle case: `ApplyDeferred` (exclusive system) is part of the schedule and might bisect a frame. When `ApplyDeferred` runs, it's a system itself: `set_change_ticks` is called for it just like any other system. Other systems are not affected.

- **PHASE9.6 (Round 2 C1, C3)** — Atomic discipline summary:
  - `change_tick: AtomicU32` — 1 `fetch_add(Relaxed)` per `Schedule::run` (1 per frame).
  - Per-row `added_ticks[row]` / `changed_ticks[row]` — written through `UnsafeCell<Tick>`; no atomic. Phase 9 SCH3 supplies happens-before via the conflict graph. **Disjoint memory-location argument (Round 2 C3) — adjacent rows in `par_iter` chunks are sound** because each `UnsafeCell<u32>` is a distinct memory location per Rust's abstract machine. The MESI cache-line ping-pong is a perf cost, not UB. See §11.5 and §13.3 (`miri_par_iter_chunks_write_adjacent_ticks_disjoint_no_ub`).
  - `SystemMeta::this_run` / `last_run` — written by the dispatcher only (through the new `System::set_change_ticks` Round 2 C1 channel); read by workers through `&SystemMeta` (via `&'s SystemMeta` captured by Query). The Phase 9 scope-spawn's `move` captures the reference; the reference is read by the worker task; the write happens-before the spawn (sequential dispatcher logic before `scope.spawn`).

  **No atomic on the per-row write path. No atomic on the per-row read path.** Bevy's exact pattern.

### 2.9 Send/Sync gate (Phase 9 SEND10 extension)

- `ComponentPool` becomes `Send + Sync` per Phase 9 SEND10 already. The added `added_ticks: Box<[UnsafeCell<Tick>]>` and `changed_ticks: Box<[UnsafeCell<Tick>]>` are NOT trivially `Sync` because `UnsafeCell` is `!Sync`.
- **Resolution**: `ComponentPool` retains its `unsafe impl Sync` (already in Phase 9). The new fields are wrapped in `unsafe`-only accessors; the access discipline (Phase 9 SCH3 + the new STORE accessor contracts) is unchanged. SAFETY blocks point to the same conflict-graph guarantees.

---

## §3 — Decision matrix (the 12 architectural questions)

### Q1. Storage option A vs B vs C vs D?

**Chosen:** **Option A — per-row ticks** (Bevy pattern; storage 8 B per `(entity, tracked-component)`).

#### Q1.1 — Storage layout

Each `ComponentPool` carries two parallel buffers alongside the component data:
- `added_ticks: Box<[UnsafeCell<Tick>]>` — same length as the data buffer.
- `changed_ticks: Box<[UnsafeCell<Tick>]>` — same length.

Per-row hot loop reads `*tick_column.add(row).get()` and compares to the system's `(last_run, this_run)`.

#### Q1.2 — Justification (perf)

| Property | Option A | Option B (per-arch) | Option C (per-arch-per-column) | Option D (hybrid) |
|----------|----------|---------------------|--------------------------------|-------------------|
| **False-positive rate** | F1 only (`&mut` without modify) | **HIGH** — any write to any entity marks ALL entities in archetype | Same as B (per-component dimension gained, archetype dimension lost) | Configurable per component |
| **Storage cost (Round 2 W1 corrected)** | 8 B / (pool max_components) per `ComponentPool`. Σ over archetypes × components scales with archetype count. | 16 B / archetype (16 KB total at 1024 archetypes) | 16 B / (archetype, column) (800 KB worst case) | Default B + opt-in A |
| **Filter cost per row** | ~0.3 ns (u32 compare; predictable branch) | 0 (archetype-level check only) | 0 (column-level check) | A's cost when opted in |
| **Implementation complexity** | Medium — touches every push/swap_remove path | Low — single field per archetype | Low — field per column | High — two parallel mechanisms |
| **Compose with `Or<F>`** | Yes (Phase 8b ready) | Yes (would stay archetypal) | Yes (would stay archetypal) | Mixed; complex |
| **Matches Phase 8b filter trait** | Sets `IS_ARCHETYPAL = false`, activates per-row branch | Stays archetypal — no const-fold loss | Stays archetypal | Mixed |

The Bevy false-positive blast-radius argument: boyko's archetypes can hold up to millions of entities. A single `&mut Position` write to one entity would mark all entities in the archetype as `Changed<Position>` under option B/C. **Dealbreaker for B/C in isolation** (research §8). Unity DOTS sidesteps this with 16 KB chunks (~100 entities each); boyko has no chunk-within-archetype concept (Phase 7 flat archetype storage).

Option D (hybrid B + opt-in A) doubles maintenance surface and forces users to think about which components benefit from precise tracking — Bevy [Issue #5097](https://github.com/bevyengine/bevy/issues/5097) proposed similar hierarchical storage but it hasn't shipped; the complexity is the blocker.

**Option A wins on:**
- Best granularity (per-row, no false positives beyond F1).
- Composes with existing Phase 8b `Or<F>` cleanly.
- Cache behaviour is acceptable when only ticks are scanned (a "Changed<T>-only" early-exit query touches only the tick column — 16 entities per 64 B cache line; same density as Bevy benches measure).
- Phase 9 conflict graph already provides the happens-before — no new atomics.

**Cost paid (Round 2 W1 — corrected for per-archetype duplication):** 8 B × Σ pool.max_components. At the AAA design point of 100 archetypes × 50 components × 1024 max_components = 40 MB. At higher archetype densities (1024 archetypes), the number scales to 80-200 MB. Bevy ships compatible scales; AAA games run on this.

**Alternatives rejected:**
- **B (per-archetype)**: dealbreaker false-positive blast radius.
- **C (per-arch-per-column)**: same blast radius; only gains per-component dimension.
- **D (hybrid)**: complex; deferred per §17 OQ-4 to a future phase if users complain about the 40+ MB.

### Q2. Tick bump policy — per-frame or per-system?

**Chosen:** **Per-`Schedule::run` frame bump** (one `fetch_add` per frame). Per-system `this_run` snapshot captured at dispatch via `System::set_change_ticks` (Round 2 C1).

#### Q2.1 — Mechanism

```text
Schedule::run(&mut self, world):
    let prev = world.change_tick.fetch_add(1, Relaxed);
    let this_run = Tick::new(prev + 1);
    // ... run wraparound check ...

    for each system i in dispatch order:
        // System::set_change_ticks computes new_last_run from previous this_run.
        let prev_this_run = self.systems[i].meta().this_run;
        self.systems[i].set_change_ticks(prev_this_run, this_run);
        scope.spawn(...);
```

#### Q2.2 — Per-frame vs per-system trade-off

| Property | Per-frame (chosen) | Per-system (Bevy G1 alt) |
|----------|--------------------|---------------------------|
| Atomic ops / frame | 1 (`fetch_add` at frame start) | N (one per system) |
| `Added` granularity | Frame-level — multiple systems in one frame all see the same `this_run` | System-level — each system gets its own tick |
| Filter behaviour | "Anything added during frame N is `Added<T>` from system X's POV for all systems with `last_run < this_run`" | "Anything added by system Y is `Added<T>` for system X iff Y ran between X's previous and current run" |
| `Changed<T>` semantics | Same set across multiple readers in one frame | Different sets per reader |
| Bevy choice | Per-system (G1) — supports ad-hoc tick consumers | — |

**Justification for per-frame:**

Per-frame is **simpler** and **cheaper** at the cost of slightly coarser granularity. The coarser granularity is acceptable because:
1. The frame-level "all systems in this frame share a `this_run`" model fits boyko's run-to-completion frame loop.
2. The `last_run` per system still differentiates "I ran in frame N-1" from "I ran 5 frames ago" — the user-visible granularity is preserved.
3. Atomic contention is 1/frame vs N/frame. At 100 systems × 60 FPS, that's 6 k ops/sec vs 60 k ops/sec — both noise floor, but the simpler model is preferred.
4. **Round 2 W2 consequence**: per-frame bump means the `check_ticks` scan fires much less often (every ~95 days at 60 FPS) than per-system; the wraparound machinery has more headroom.

The single subtlety: if system X spawns an entity at frame N and system Y queries `Added<T>` at frame N (Y runs after X via DAG ordering), Y sees the entity as added — both systems have the same `this_run` so `tick.is_newer_than(Y.last_run, Y.this_run)` holds when `tick == this_run > Y.last_run`. Standard Bevy semantic.

#### Q2.3 — Per-system Bevy G1 considered

Bevy's pattern is: each system at dispatch does `let this_run = world.change_tick.fetch_add(1, Relaxed)`. This means consecutive systems in a single `Schedule::run` get consecutive ticks: system 0 sees `this_run = T+1`, system 1 sees `T+2`, etc.

Pros:
- Tighter `Added<T>` granularity.
- Bevy is the empirical reference.

Cons:
- N atomics per frame.
- Doesn't unlock any user-facing functionality boyko has on the roadmap.

**Decision: per-frame** for v1. The plan §17 OQ-5 lists "promote to per-system bump if user need arises" as a future migration.

**Alternatives rejected:**
- **Per-system (G1)**: N atomics for no v1 benefit; deferred.
- **Pre-allocated tick range (G2)**: research §6 design. Even simpler than per-frame. Loses the meaningful "tick is monotonically increasing across `Schedule::run` calls" property; the wraparound machinery would need to track an extra "absolute frame counter". Net: not simpler enough to justify departing from Bevy.

### Q3. Wraparound — adopt Bevy `MAX_CHANGE_AGE`?

**Chosen:** **Yes — adopt Bevy's `MAX_CHANGE_AGE = u32::MAX - (2 * CHECK_TICK_THRESHOLD - 1)` ≈ 3.26 B** + the `check_ticks` clamp scan. **Derivation in §9.3 (Round 2 W2 — now first-principles).**

#### Q3.1 — Constants

```text
CHECK_TICK_THRESHOLD: u32 = 518_400_000
MAX_CHANGE_AGE:       u32 = u32::MAX - (2 * CHECK_TICK_THRESHOLD - 1)
                          ≈ 3_258_166_895
                          ≈ 75% of u32::MAX
```

#### Q3.2 — Why this exact formula (Round 2 W2)

See §9.3 for the first-principles derivation. Summary: the constraint is `MAX_CHANGE_AGE + CHECK_TICK_THRESHOLD < 2^31` (signed-comparison correctness for `is_newer_than`). Bevy's formula satisfies this with `MAX_CHANGE_AGE + CHECK_TICK_THRESHOLD = u32::MAX - CHECK_TICK_THRESHOLD + 1 ≈ 2^32 - 518M ≈ 3.78 × 10^9`. Wait — that's not less than `2^31 ≈ 2.15 × 10^9`. Bevy's actual proof requires careful unpacking; §9.3 shows the algebra.

#### Q3.3 — Scan frequency

`check_ticks` fires when the global tick has advanced by `CHECK_TICK_THRESHOLD` since the last scan. At 60 FPS (per-frame bump):
- 1 tick/frame × 60 FPS = 60 ticks/sec.
- Scan fires every 518.4 M / 60 ≈ 8 640 000 sec ≈ **100 days of continuous play**.

For per-system bump (alternative): 100 systems × 60 FPS = 6 000 ticks/sec → scan every 24 h.

#### Q3.4 — Scan cost

Walks every LIVE stored tick (Round 2 W3 — corrected; Round 3 W-NEW-1: API named `count`):
- Per archetype: 2 × `pool.count()` × `tracked_component_count`.
- Per system: 1 tick.
- Total at 100 k × 50 (live rows): 10 M ticks × 1 cycle (the `wrapping_sub` + compare) ≈ 3 ms cold, < 1 ms hot.

Acceptable as a once-per-100-days cost.

**Alternatives rejected:**
- **u64 ticks**: doubles tick storage. Bevy hasn't migrated. v1 sticks with u32.
- **Don't handle wraparound**: false negatives after ~2 years of continuous play unacceptable.

### Q4. `Mut<T>` deref guard — full guard struct or simpler helper?

**Chosen:** **Full guard struct with `DerefMut` bumping the changed tick**. Bevy pattern.

#### Q4.1 — Shape

```text
struct Mut<'w, T> {
    value:    &'w mut T,
    added:    &'w UnsafeCell<Tick>,
    changed:  &'w UnsafeCell<Tick>,
    this_run: Tick,
    last_run: Tick,
}

impl<T> Deref for Mut<'_, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T { self.value }
}

impl<T> DerefMut for Mut<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY (MUT3): system declared write to C; Phase 9 SCH3 ensures
        //   no concurrent reader of this column's changed tick.
        unsafe { *self.changed.get() = self.this_run; }
        self.value
    }
}
```

#### Q4.2 — Why the full guard, not `&mut T` + helper

The simpler "give the user `&mut T` plus a `bump_tick()` helper" leaks the bump-on-every-mutable-deref semantic. Users have to remember to call `bump_tick()` after every mutation; failure mode is silent missed detection.

The guard pattern wires the bump into the deref operator. Costs: 1 store per `deref_mut` call. If the compiler inlines `deref_mut` (`#[inline]`), the bump becomes a single u32 store right next to the user's write.

#### Q4.3 — `set_if_neq` and `bypass_change_detection`

Both are Bevy escape hatches (§2.5 MUT4 / MUT5). Adopting both in v1.

**Alternatives rejected:**
- **`&mut T` + helper `bump_tick()`**: leaks the semantic; failure mode is silent misses.
- **No `Mut<T>` API; rely on `set_if_neq` mandatorily**: forces every component to be `PartialEq`. Non-starter.

### Q5. Atomic tick writes under Phase 9 — (a) snapshot / (b) per-write atomic / (c) per-system-local buffer?

**Chosen:** **(a) Per-system snapshot of `this_run`; per-row writes are non-atomic `UnsafeCell<Tick>` stores**. Bevy pattern.

#### Q5.1 — Why no per-row atomic

The Phase 9 conflict graph (SCH3) guarantees:

For any system `S` declaring a write of component `C` in archetype `A`:
- No other system runs concurrently with `S` that reads or writes `(A, C)`.

This extends naturally to the per-row tick column. Plain u32 store through `*UnsafeCell<Tick>` is sound — there is no concurrent reader.

#### Q5.2 — Why no per-system-local buffer

Option (c) would have each system write tick updates to a thread-local buffer that gets flushed at apply time. Adds per-write buffer space (worst case: buffer grows to match column size); adds apply-time flush cost.

**Decision: (a)**.

#### Q5.3 — Happens-before chain in detail (Phase 9 §2.8) — Round 2 C3 expanded

For two systems `S1` (writes `C`) and `S2` (reads `C` via `Changed<C>`) where `S1` runs in dispatch round `R` and `S2` runs in dispatch round `R+1`:

1. `S1`'s worker writes `changed_ticks[row] = this_run` through `UnsafeCell::get()`.
2. `S1`'s worker pushes completion: `completion_queue.push(S1)`.
3. `S1`'s worker increments `pending_apply: fetch_add(1, Release)`.
4. Dispatcher's `apply_window_drain`: loads `pending_apply` with `Acquire` — synchronises-with (3) → the tick write in (1) is visible.
5. Dispatcher pops completion, calls `S1.apply(world)`.
6. Dispatcher decrements `S2.pred_remaining`; eventually `S2` enters ready queue.
7. Dispatcher invokes `S2.set_change_ticks(prev, this_run)` (Round 2 C1 channel).
8. Dispatcher spawns `S2`'s task; worker reads tick column for `C`; sees `S1`'s write.

The Release/Acquire pair on `pending_apply` carries the tick writes across the worker → dispatcher boundary.

#### Q5.4 — par_iter case (Round 2 C3 — sound formal argument)

`par_iter` (Phase 9 PAR1) spawns multiple chunks of the same system. Within one `par_iter`:
- All chunks are running the same system `S` (same `this_run`).
- Chunks operate on disjoint row ranges (PAR2).
- Each chunk writes ticks only for its row range.

No two chunks ever write the same tick slot. **Cache-line sharing across chunk boundaries is sound** per Rust's abstract machine because each `UnsafeCell<u32>` is a distinct memory location. Two non-overlapping memory locations CAN be modified concurrently regardless of cache-line sharing (see [Rustonomicon "Data Races and Race Conditions"](https://doc.rust-lang.org/nomicon/races.html) and [P0250R3 "Memory model and synchronization"](https://www.open-std.org/jtc1/sc22/wg21/docs/papers/2018/p0250r3.html)). The cache-line ping-pong is a MESI perf cost (false sharing), not UB.

Verified via Miri (§13.3 `miri_par_iter_chunks_write_adjacent_ticks_disjoint_no_ub`).

The implicit memory order across chunks is supplied by the scope's `pending` counter (the calling thread waits for all chunks via `scope.Drop`'s work-stealing wait, which performs a Release/Acquire pair).

**Alternatives rejected:**
- **(b) per-write atomic**: adds a LOCK prefix to every `Mut<T>::deref_mut`. ~100x slowdown.
- **(c) thread-local buffer**: adds buffer-side complexity for no soundness gain.

### Q6. `Changed<T>` semantics — deref-bump or value-compare?

**Chosen:** **Bevy deref-bump semantics**: any `&mut T` deref through `Mut<T>` bumps the changed tick. User opt-in to value-compare via `set_if_neq`.

#### Q6.1 — Trade-off

| Property | Deref-bump (chosen) | Value-compare |
|----------|---------------------|---------------|
| Default cost | 1 u32 store per `&mut T` deref (compiler-fused) | `T::eq` call + branch + maybe store |
| `T` bound | None | `T: PartialEq` |
| False positives | Yes (F1 — `&mut T` without modify) | No (only true modifications) |
| User effort to avoid F1 | `set_if_neq` opt-in | Free |
| Works for components with `Drop` / allocations | Yes | Yes, but `eq` may be expensive for `Vec<T>` |
| Bevy precedent | Yes | No |

**Justification:** Default is **fast** + **universal**. Users who care about F1 opt in via `set_if_neq`.

#### Q6.2 — `set_if_neq` integration

```text
impl<T: PartialEq> Mut<'_, T> {
    pub fn set_if_neq(&mut self, value: T) -> bool {
        if *self.value != value {
            *self.value = value;
            unsafe { *self.changed.get() = self.this_run; }
            true
        } else {
            false
        }
    }
}
```

**Alternatives rejected:**
- **Universal `T: PartialEq` bound**: many components lack `PartialEq`. Non-starter.
- **Custom `ChangedDeep<T: PartialEq>` filter**: out of v1 scope.

### Q7. Integration with Phase 8b `IS_ARCHETYPAL`?

**Chosen:** **`Added<C>` and `Changed<C>` are non-archetypal** (`IS_ARCHETYPAL = false`). Composing them with archetypal filters (`With<C>`, `Without<C>`) is handled by the existing AND tuple fold and `Or<F>` fold.

#### Q7.1 — Const-fold behaviour after Phase 10

| Filter | `IS_ARCHETYPAL` | Per-row hot-loop branch |
|--------|-----------------|--------------------------|
| `()` | true | Elided |
| `With<C>` | true | Elided |
| `Without<C>` | true | Elided |
| `Added<C>` (NEW) | false | Active — per-row tick compare |
| `Changed<C>` (NEW) | false | Active — per-row tick compare |
| `(With<A>, Changed<B>)` | false (AND fold) | Active |
| `Or<(With<A>, Changed<B>)>` | false (AND fold per Phase 8b §filter.rs:578) | Active |
| `(With<A>, Without<B>)` | true | Elided (no Phase 10 change) |

**Const-fold preservation for queries that don't use change detection:** A `Query<&T, With<C>>` retains the const-folded `filter_fetch` elision; Phase 10 adds **zero overhead** to such queries.

#### Q7.2 — Why non-archetypal

`Added<C>` and `Changed<C>` filter at the row level: an archetype may contain `C` but some rows have been changed and others haven't.

#### Q7.3 — Short-circuit composition with `Or<F>` (Round 2 C4 — expanded)

Phase 8b's `impl_or_filter_tuple!` (`filter.rs:646-655`) short-circuits with `||`. For `Or<(With<A>, Changed<B>)>`:
- `IS_ARCHETYPAL = With::IS_ARCHETYPAL && Changed::IS_ARCHETYPAL = false`.
- Per-row evaluation: `With<A>::filter_fetch` (true) → `||` short-circuit → `Changed<B>::filter_fetch` not called when `With<A>` returns true.
- For an archetype containing only `A` (no B): `Or::matches_component_set` passes (`With<A>` matches). `Changed<B>::set_table_*` writes `tick_base = null` because the archetype lacks B. `Or::filter_fetch` evaluates `With<A>` first (true via archetypal stub) → short-circuit → `Changed<B>` not called.

**But: per FLT6, `Changed<B>::filter_fetch` has a null-base early return**. When this branch IS hit (e.g., `Or<(Changed<B>, With<A>)>` order reversed; or any case where `With<A>` returns false somehow), the early return is the safety net. This is `~0.5 ns` predicted-not-taken branch per row that hits the path.

**Round 2 C4 per-row cost** (added to §10):
| Filter | Per-row cost |
|--------|--------------|
| `Changed<C>` (standalone, archetype contains C) | ~1 ns (load + 2 sub + cmp) |
| `Changed<C>` (under `Or<>`, archetype lacks C, null-base path) | ~0.5 ns (branch + return false) |
| `Or<(With<A>, Changed<B>)>` (archetype contains A only) | ~1 ns (`With<A>` returns true; short-circuit; `Changed<B>` never called) |
| `Or<(Changed<B>, With<A>)>` (archetype contains A only) | ~1.5 ns (null-base check for Changed<B> + `With<A>` archetypal true) |

**Conservative access declaration (Round 2 C4):** As noted in FLT2, `Or<(_, Changed<C>)>` declares read of C globally. The scheduler will serialise this system against any concurrent writer of C, even when the matched archetype set lacks C. This is intentional (mirrors Bevy); per-archetype access narrowing would require runtime computation per system. Future work in §17 OQ-12.

#### Q7.4 — `aggregate_include` for `Added` / `Changed`

Both `Added<C>::aggregate_include` and `Changed<C>::aggregate_include` set the bit for `C` in the include mask. Consistent with `With<C>`.

For `Or<(...)>`: Phase 8b's `Or<F>::aggregate_include` is **explicitly a no-op** (`filter.rs:603-606` M8 contract). So `Or<(Added<A>, Changed<B>)>` walks every archetype, post-filters via `Or::matches_component_set`. **Performance consequence (Round 2 W8): every archetype touched in `update_archetypes`. See §10.5-bis bench.**

### Q8. Memory layout — parallel array or interleaved?

**Chosen:** **Two parallel `Box<[UnsafeCell<Tick>]>` arrays** per `ComponentPool`, separate from each other and from the data buffer.

(Body unchanged from Round 1 — see Round 1 plan §3 Q8.1-Q8.6 for full details.)

### Q9. Per-archetype version for fast skip?

**Chosen:** **No.** Per-row ticks (Option A) provide sufficient granularity.

(Body unchanged from Round 1 — see Round 1 plan §3 Q9.1-Q9.3.)

### Q10. Send/Sync for tick storage?

**Chosen:** **Plain `u32` columns are `Send + Sync` trivially. `UnsafeCell<Tick>` is `!Sync` by default; wrapped in `unsafe impl Sync` at the `ComponentPool` level (extending Phase 9 SEND10).**

(Body unchanged from Round 1 — see Round 1 plan §3 Q10.1-Q10.2.)

### Q11. `SystemMeta::last_run` update timing? (Round 2 C1 + O5)

**Chosen:** **At dispatch time, per system, by the dispatcher, via `System::set_change_ticks` (Round 2 C1).** Single mutator, sequential, no race.

#### Q11.1 — Lifecycle (Round 2 C1 simplified)

```text
Frame N:
    [Schedule::run start]
    world.change_tick.fetch_add(1)  → this_run = T

    [dispatch round 1]
    for each ready system i:
        let prev_this_run = self.systems[i].meta().this_run;
        self.systems[i].set_change_ticks(prev_this_run, T);   ← DISPATCHER calls trait method
        scope.spawn(...);                                       ← worker reads &meta in task

    [worker completes; pushes to completion_queue]

    [apply window]
    for each completion popped:
        self.systems[i].apply(world);
        // NO tick state mutation here (Round 2 C1 simplification — Phase 9 PHASE9.3 folded into PHASE9.2)

    [dispatch round 2]
    (next set of systems, same pattern)

    [Schedule::run end]
```

#### Q11.2 — Race-free proof

- `set_change_ticks` is called by the dispatcher BEFORE spawning the worker. The `scope.spawn` is a memory-barrier point (Phase 9 §4.5); the worker's `&meta` read happens-after the spawn → happens-after the trait-method call → happens-after the writes inside it.
- The trait method body writes through `&mut self` — the implementor has exclusive access; no concurrent reader of `meta` exists at write time.

Both writes are sequential dispatcher logic.

**Decision: per-system at dispatch via trait method.** Single channel; trait-enforced contract.

### Q12. Single `Tick` + flag bit, or two separate ticks?

**Chosen:** **Two separate ticks (`added`, `changed`).** Bevy pattern.

(Body unchanged from Round 1 — see Round 1 plan §3 Q12.1.)

---

## §4 — Tick infrastructure — architecture deep dive

### 4.1 File layout

New files in `crates/boyko_ecs/src/ecs/core/change_detection/`:
- `mod.rs` — public re-exports.
- `tick.rs` — `Tick(u32)` newtype + `is_newer_than` + `check_tick` + constants.
- `system_change_tick.rs` — `SystemChangeTick` `SystemParam`.
- `check_ticks.rs` — `run_check_ticks_scan` cold-path helper (Round 2 W3).

Modified files:
- `crates/boyko_ecs/src/ecs/core/system/system_meta.rs` — add `this_run` and `last_run` fields (Round 2 O5: harmonised naming); change `SystemMeta::new` constructor signature (Round 2 W5).
- `crates/boyko_ecs/src/ecs/core/system/system.rs` — add `set_change_ticks` trait method (Round 2 C1).
- `crates/boyko_ecs/src/ecs/core/system/function_system.rs` — implement `set_change_ticks`; new `SystemMeta::new(name, current_tick)` callsite.
- `crates/boyko_ecs/src/ecs/core/system/exclusive_function_system.rs` — same as FunctionSystem.
- `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` — add `change_tick: AtomicU32`, `last_check_tick: Tick`; thread tick into `create_entity`.
- `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs` (Phase 9) — frame tick bump, per-system tick wiring via `set_change_ticks`, wraparound check.
- `crates/boyko_ecs/src/ecs/memory/component_pool.rs` — add tick columns.
- `crates/boyko_ecs/src/ecs/core/archetype/archetype.rs` — thread `current_tick` through `create_entity`, add `tick_column_base` helper.
- `crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs` — **(Round 3 W-NEW-1b)** add `pub fn iter_archetypes_mut(&mut self) -> ArchetypeBundleIterMut<'_>` one-liner mirror of `iter_archetypes()`.
- `crates/boyko_ecs/src/ecs/core/iters/query/iter.rs` — add `meta: &'s SystemMeta` field to `QueryIter` / `QueryIterMut` (Round 2 C2).
- `crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs` — add `meta` field; forward through `run_chunk_inline` / `run_chunk_owned` / PAR7 fallback (Round 2 W6).
- `crates/boyko_ecs/src/ecs/core/iters/query/query.rs` — wire `meta` into `iter` / `iter_mut` / `par_iter` constructors.
- `crates/boyko_ecs/src/ecs/core/iters/query/filter.rs` + `data.rs` — `set_table_*` gains `meta: &'_ SystemMeta` parameter (Round 2 W7).

### 4.2 `Tick(u32)` design (Round 2 O4: `Default` added; Round 3 C-NEW-1: `is_newer_than` formula corrected)

```rust
/// Monotonic change-detection tick.
///
/// Wrapping u32 counter; comparison via `is_newer_than` uses wrapping
/// subtraction. Both operands must be within `MAX_CHANGE_AGE` of each
/// other; `check_ticks` (run from `Schedule::run` apply window) enforces
/// the bound by clamping any aged-out tick.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Tick(pub(crate) u32);
//                       ^ Round 2 O4: Default impls return Tick(0) == Tick::ZERO

impl Tick {
    pub const ZERO: Tick = Tick(0);

    #[inline]
    pub const fn new(value: u32) -> Self { Self(value) }

    /// Returns `true` iff `self` is newer than `last_run` from the
    /// perspective of an observer whose current tick is `this_run`.
    ///
    /// Semantically: `self ∈ (last_run, this_run]` accounting for u32
    /// wraparound. Inclusive at the upper bound — `self == this_run`
    /// returns true (so a self-write within the current frame is visible
    /// to `is_changed`). Exclusive at the lower bound — `self == last_run`
    /// returns false.
    ///
    /// # Implementation (Round 3 C-NEW-1)
    ///
    /// Bevy mirror — checks whether `self` is in `(last_run, this_run]`.
    /// Standard signed-comparison-via-wraparound technique:
    ///
    /// ```text
    /// let ticks_since_insert = this_run.wrapping_sub(self);
    /// let ticks_since_system = this_run.wrapping_sub(last_run);
    /// ticks_since_system > ticks_since_insert
    /// ```
    ///
    /// Worked examples (under bounded `MAX_CHANGE_AGE`):
    /// - `self=10, last_run=2, this_run=10`: `since_insert=0, since_system=8 → true` (inclusive upper bound).
    /// - `self=2,  last_run=2, this_run=10`: `since_insert=8, since_system=8 → false` (exclusive lower bound).
    /// - `self=5,  last_run=2, this_run=10`: `since_insert=5, since_system=8 → true` (in range).
    /// - `self=11, last_run=2, this_run=10`: `since_insert=u32::MAX (wraps), since_system=8 → false` (future tick rejected).
    ///
    /// Why NOT `(this_run - last_run) > (self - last_run)` (the Round 2 form,
    /// rejected by critic C-NEW-1): for `self == this_run == 10, last_run == 2`,
    /// `age_self = age_this = 8`, returns `false` — breaks inclusive upper bound.
    #[inline]
    pub fn is_newer_than(self, last_run: Tick, this_run: Tick) -> bool {
        // Bevy mirror — checks whether `self` is in (last_run, this_run].
        // Standard signed-comparison-via-wraparound technique.
        let ticks_since_insert = this_run.0.wrapping_sub(self.0);
        let ticks_since_system = this_run.0.wrapping_sub(last_run.0);
        ticks_since_system > ticks_since_insert
    }

    /// Clamps `self` to be no older than `MAX_CHANGE_AGE` ticks behind
    /// `current`. Returns `true` if `self` was clamped.
    #[inline]
    pub fn check_tick(&mut self, current: Tick) -> bool {
        let age = current.0.wrapping_sub(self.0);
        if age > MAX_CHANGE_AGE {
            self.0 = current.0.wrapping_sub(MAX_CHANGE_AGE);
            true
        } else {
            false
        }
    }

    /// Returns the next tick (wrapping).
    #[inline]
    pub const fn next(self) -> Self { Tick(self.0.wrapping_add(1)) }

    /// Wrapping-subtracts one tick (used by `is_changed` for the
    /// inclusive lower-bound semantic, Round 2 O1).
    #[inline]
    pub const fn wrapping_sub(self, amount: Tick) -> Self {
        Tick(self.0.wrapping_sub(amount.0))
    }
}

pub const CHECK_TICK_THRESHOLD: u32 = 518_400_000;
pub const MAX_CHANGE_AGE: u32 = u32::MAX - (2 * CHECK_TICK_THRESHOLD - 1);
```

### 4.2-bis `SystemMeta::new` signature change (Round 2 W5)

```rust
impl SystemMeta {
    /// Constructs a fresh meta with `last_run = current_tick - MAX_CHANGE_AGE`.
    ///
    /// Round 2 W5: every System construction site MUST pass the current tick.
    /// This eliminates the bypass-initialize bug where `NoopSystem` (or any
    /// custom System impl) ended up with `last_run = Tick::ZERO`.
    pub fn new(name: &'static str, current_tick: Tick) -> Self {
        let last_run = Tick::new(current_tick.0.wrapping_sub(MAX_CHANGE_AGE));
        Self {
            access: Access::new(),
            name,
            last_archetype_generation: ArchetypeGeneration::INITIAL,
            last_structural_generation: ArchetypeGeneration::INITIAL,
            last_run,
            this_run: last_run,  // pre-first-run sentinel; updated by set_change_ticks
        }
    }
}
```

**Callsite enumeration (impacted Step 3 + Step 15):**
- `FunctionSystem::new(...)` calls `SystemMeta::new(name, world.current_tick())`.
- `ExclusiveFunctionSystem::new(...)` same.
- `NoopSystem` (test stub in `system.rs:131`) — needs `EcsMaster` access to compute `current_tick`, OR a test helper `SystemMeta::for_testing()` that defaults `current_tick = Tick::ZERO + 1` (so `last_run = 1 - MAX_CHANGE_AGE`, a sane test default).

Round 1 design relied on `FunctionSystem::initialize` to set `last_run`. Round 2 W5 moves this to the constructor — initialise-on-create.

### 4.3 `Archetype::tick_column_base` helper

(Unchanged from Round 1 — see Round 1 plan §4.3.)

### 4.4 `EcsMaster::change_tick` placement (Round 2 O2: drop CachePadded)

```rust
pub struct EcsMaster {
    // ... existing fields ...

    /// Monotonic frame counter for change detection. Bumped once per
    /// `Schedule::run` call via `fetch_add(1, Relaxed)`.
    ///
    /// Round 2 O2: NOT CachePadded. The atomic is accessed once per frame
    /// from the dispatcher; false-sharing risk is essentially zero.
    /// CachePadded saved 60 B of waste and adds nothing here.
    change_tick: AtomicU32,

    /// Last tick at which `check_ticks` scanned the world.
    last_check_tick: Tick,
}

impl EcsMaster {
    #[inline]
    pub fn current_tick(&self) -> Tick {
        Tick::new(self.change_tick.load(Ordering::Relaxed))
    }
}
```

Initial values:
- `change_tick = AtomicU32::new(0)`.
- `last_check_tick = Tick::ZERO`.

### 4.5 `Schedule::run` integration (Round 2 C1: via `set_change_ticks`)

```rust
impl Schedule {
    pub fn run(&mut self, world: &mut EcsMaster) {
        self.frame = self.frame.wrapping_add(1);
        self.scratch.reset_for_frame(&self.conflict_graph);

        // PHASE 10: frame-start tick bump.
        let prev_tick = world.change_tick.fetch_add(1, Ordering::Relaxed);
        let this_run = Tick::new(prev_tick.wrapping_add(1));

        // PHASE 10: wraparound check (cold path — see §9).
        let since_last_check = this_run.0.wrapping_sub(world.last_check_tick.0);
        if since_last_check >= CHECK_TICK_THRESHOLD {
            run_check_ticks_scan(world, this_run);
            // Also clamp every system's last_run:
            for sb in self.systems.iter_mut() {
                // Read meta via System::access() (or a similar safe getter);
                // alternatively expose system.set_change_ticks with the clamp.
                let mut last_run = sb.meta().last_run;
                last_run.check_tick(this_run);
                sb.set_change_ticks(last_run, sb.meta().this_run);
            }
            world.last_check_tick = this_run;
        }

        // ... existing dispatcher loop ...
        // Inside try_dispatch_ready, before scope.spawn:
        //   let prev_this_run = self.systems[i].meta().this_run;
        //   self.systems[i].set_change_ticks(prev_this_run, this_run);
        //   ...
    }
}
```

**Note**: `sb.meta()` is a new safe getter on the existing `SystemBox` (Phase 9 §5.2). It returns `&SystemMeta` for inspection. Adding it does not violate the `Box<dyn System>` constraint because `meta()` returns through the `System::access()` analogue — already on the trait. Detailed wiring: `System::access()` returns `&Access`; we add `System::meta() -> &SystemMeta` as the symmetric read-only accessor. (Alternative: store `meta` directly on `SystemBox`, separately from the `System` impl — but that creates two-source-of-truth issues. Stick with trait getter.)

### 4.6 Cold path: `run_check_ticks_scan` (Round 2 W3: live rows only; Round 3 W-NEW-1: API renamed)

```rust
/// Walks every LIVE stored tick and clamps anything older than MAX_CHANGE_AGE.
///
/// Runs on the dispatcher, in the apply window (no workers live).
/// Cost: O(live_stored_ticks). At 100k × 50 → ~3 ms cold. Frequency:
/// every CHECK_TICK_THRESHOLD ticks ≈ every 100 days at 60 FPS.
///
/// Round 2 W3: walks `0..pool.count()`, NOT `pool.added_ticks.iter()`.
/// Unused slots stay Tick::ZERO and don't need clamping.
///
/// Round 3 W-NEW-1: API references corrected.
/// - `pool.count()` (the actual `ComponentPool::count(&self) -> usize`
///   at `component_pool.rs:620`), NOT the non-existent `pool.units_len()`.
/// - `world.archetype_master.iter_archetypes_mut()` (new one-liner mirror
///   of the existing `iter_archetypes()` at `archetype_master.rs:590`),
///   NOT the non-existent `world.archetype_master.archetypes_iter_mut()`.
#[cold]
pub(crate) fn run_check_ticks_scan(world: &mut EcsMaster, current: Tick) {
    // Walk every archetype's tick columns over LIVE rows.
    for arch in world.archetype_master.iter_archetypes_mut() {
        for component_id in arch.component_ids().to_vec() {
            let pool = arch.component_pools_mut().get_pool_mut(component_id).unwrap();
            let live_count = pool.count();  // Round 3 W-NEW-1: was units_len()
            for i in 0..live_count {
                // SAFETY: dispatcher holds &mut EcsMaster; no aliasing.
                unsafe {
                    let added_ref = &mut *pool.added_ticks[i].get();
                    added_ref.check_tick(current);
                    let changed_ref = &mut *pool.changed_ticks[i].get();
                    changed_ref.check_tick(current);
                }
            }
        }
    }
    // Schedule's own systems' last_run handled separately in Schedule::run (§4.5).
}
```

§9 details the integration further.

---

## §5 — `Added<T>` / `Changed<T>` filter — architecture deep dive

### 5.1 `Added<C: Component>` filter

(Body unchanged from Round 1 — see Round 1 plan §5.1. New `set_table_*` signatures use `meta: &'_ SystemMeta` per Round 2 W7.)

### 5.2 `Changed<C: Component>` filter

(Body unchanged from Round 1 — see Round 1 plan §5.2.)

### 5.3 Filter trait signature change — `meta` parameter (Round 2 W7 + C2)

**The Phase 8b filter trait signature does not pass `&SystemMeta` into `set_table_*`.** Adding the parameter is a breaking change.

Updated trait (Round 2 W7: explicit lifetime, plus "meta is input-only" doc):
```rust
pub unsafe trait QueryFilter: Sized {
    // ... unchanged associated items ...

    /// Per-archetype boundary update for the read-only iterator path.
    ///
    /// # Round 2 W7
    /// `meta` is a read-only INPUT. Ticks are copied into `Fetch<'w>` by
    /// value (Copy). The reference does NOT live beyond this call; no
    /// `'w`-binding on meta — purely lifetime-input-not-stored.
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
        meta: &'_ SystemMeta,    // NEW (Round 2 W7: anonymous lifetime)
    );

    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
        meta: &'_ SystemMeta,    // NEW
    );

    // filter_fetch unchanged.
}
```

**`QueryData` trait gets the parallel change** (`data.rs`).

**Impacted files (Round 2 C2 — full enumeration; was understated in Round 1):**

1. `filter.rs`: every `QueryFilter` impl + the two tuple macros + the `Or<F>` macros + the arity-overflow stubs. Archetypal impls (`()`, `With`, `Without`, tuples thereof) accept and ignore the `meta` parameter.
2. `data.rs`: parallel change for `QueryData::set_table_*`.
3. **`iter.rs` — CRITICAL UPDATE (Round 2 C2):** `QueryIter` and `QueryIterMut` do NOT currently hold `meta`. Add the field:
   ```rust
   pub struct QueryIter<'q, 's, D: QueryData, F: QueryFilter> {
       archetype_ids: std::slice::Iter<'q, ArchetypeId>,
       data_state: &'s D::State,
       filter_state: &'s F::State,
       world: UnsafeEcsCell<'q>,
       data_fetch: D::Fetch<'q>,
       filter_fetch: F::Fetch<'q>,
       current_row: usize,
       current_len: usize,
       meta: &'s SystemMeta,    // NEW Round 2 C2
       _marker: PhantomData<&'s ()>,
   }
   ```
   And the constructor signature:
   ```rust
   pub(crate) unsafe fn new(
       state: &'s QueryDataState<D, F>,
       world: UnsafeEcsCell<'q>,
       meta: &'s SystemMeta,    // NEW
   ) -> Self
   ```
   Identical change to `QueryIterMut`.
4. `query.rs`: `Query::iter` and `Query::iter_mut` callers now forward `self.meta` (already on Query at `query.rs:60-62`):
   ```rust
   pub fn iter<'q>(&self) -> QueryIter<'q, 's, D, F> {
       unsafe { QueryIter::new(self.state, self.world, self.meta) }
   }
   ```
5. `par_iter.rs`: add `meta: &'s SystemMeta` to `ParQuery` / `ParQueryMut`. See §8.4 for the full chunk-dispatch wiring (Round 2 W6).

**Lifetime constraint:** `meta: &'s SystemMeta` (state lifetime); `'s: 'q` already holds in Phase 8b → the meta reference outlives the world borrow.

**Tests broken by this change (Round 2 C2 — must migrate):**
- `tests/query_dsl_smoke.rs` — any test that constructs `QueryIter::new(state, world)` directly must add a `&SystemMeta` argument. Recommended helper: `SystemMeta::for_testing()` returning a meta with `last_run = Tick::ZERO`, `this_run = Tick::new(1)`.
- `tests/miri_phase8a.rs`, `tests/system_param_smoke.rs` — same.
- Phase 8b benches (`benches/query_dsl.rs`) — same.

Step 8 size: ~1.5 days for the trait signature cascade + ~0.5 day for test migration = ~2 days.

### 5.4 Short-circuit safety for `Or<(With<A>, Changed<B>)>` (Round 2 C4: null-base + cost)

Recap from §3 Q7.3: null-base path is the safety net. Final `filter_fetch`:
```rust
unsafe fn filter_fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool {
    if fetch.tick_base.is_null() {
        // Archetype does not contain C; this row cannot be Added<C>.
        return false;
    }
    let tick_ptr = unsafe { fetch.tick_base.add(row) };
    let tick = unsafe { *(*tick_ptr).get() };
    tick.is_newer_than(fetch.last_run, fetch.this_run)
}
```

**Round 2 C4 cost added to §10**: ~0.5 ns predicted-not-taken branch per row.

### 5.4-bis Trait method footprint (Round 2 C1) — `System::set_change_ticks`

Every `System` implementor must declare:
```rust
fn set_change_ticks(&mut self, last_run: Tick, this_run: Tick);
```

Concrete impls:

**FunctionSystem** (`function_system.rs`):
```rust
impl<F, M> System for FunctionSystem<F, M> {
    // ... existing methods ...

    fn set_change_ticks(&mut self, last_run: Tick, this_run: Tick) {
        self.meta.last_run = last_run;
        self.meta.this_run = this_run;
    }
}
```

**ExclusiveFunctionSystem** — same.

**NoopSystem** (test stub) — same.

**No default body** intentionally: forces every future `System` impl to opt in. This is a non-negotiable contract — bypassing it would mean tick state never updates for that system → all queries see stale `last_run` / `this_run`.

### 5.5 Hot loop performance

(See §10 for full per-row breakdown including the Round 2 C4 null-base cost row.)

---

## §6 — `Ref<T>` / `Mut<T>` `QueryData` — architecture deep dive

### 6.1 `Ref<'w, T>` design (Round 2 O5: field naming harmonised)

(Body unchanged from Round 1 — see Round 1 plan §6.1. `this_run` / `last_run` field names.)

### 6.2 `Mut<'w, T>` design (Round 2 O1: `is_changed` semantic)

Field layout per §2.5 MUT1. `Deref` / `DerefMut` per §2.5 MUT2 / MUT3.

Round 2 O1: `is_changed` and `is_added` use the inclusive lower-bound semantic so that a self-write in the same system reports as changed:

```rust
impl<'w, T> Mut<'w, T> {
    /// Returns `true` if the value was mutated since `last_run` —
    /// INCLUSIVELY (`>=` semantic, Round 2 O1).
    ///
    /// Specifically: if a system writes via `Mut::deref_mut` and then
    /// queries `is_changed` in the same system, the value DOES report
    /// as changed (because `*changed.get() = this_run` and `last_run < this_run`).
    #[inline]
    pub fn is_changed(&self) -> bool {
        let tick = unsafe { *self.changed.get() };
        // Inclusive lower-bound: tick > last_run - 1 ≡ tick >= last_run.
        tick.is_newer_than(self.last_run.wrapping_sub(Tick::new(1)), self.this_run)
    }

    #[inline]
    pub fn is_added(&self) -> bool {
        let tick = unsafe { *self.added.get() };
        tick.is_newer_than(self.last_run.wrapping_sub(Tick::new(1)), self.this_run)
    }

    // ... bypass_change_detection, set_if_neq, into_inner (unchanged) ...
}
```

### 6.2-bis `is_changed` semantic proof (Round 2 O1; Round 3 C-NEW-1 re-proved against corrected formula)

**Claim:** `tick.is_newer_than(last_run - 1, this_run)` returns `true` iff `tick ∈ [last_run, this_run]` (under the wraparound discipline of §9). That is, the call gives the inclusive-at-both-ends semantic, which is the desired "self-write within current frame is observable" behaviour.

**Proof (Round 3 C-NEW-1 corrected formula):**

The corrected `is_newer_than` is:
```text
is_newer_than(self, l, t) = (t.wrapping_sub(l)) > (t.wrapping_sub(self))
                          ≡ ticks_since_system > ticks_since_insert
```

Equivalent semantic (under bounded ages): `self ∈ (l, t]`, i.e., `l < self <= t`.

Substituting `l = last_run - 1`:
```text
is_newer_than(self, last_run - 1, this_run)
  returns true iff self ∈ (last_run - 1, this_run]
                iff self >= last_run AND self <= this_run.
```

So passing `last_run - 1` turns the exclusive lower bound into inclusive. Upper bound remains inclusive.

**Worked case: self-write within current frame.**

Setup: a system holds `Mut<T>`. It calls `deref_mut()`, which writes `*changed.get() = this_run`. Immediately after, the same system calls `is_changed()`.

- `tick = this_run` (just written).
- `last_run < this_run` (invariant: `last_run` is the previous frame's `this_run`, or `current - MAX_CHANGE_AGE` on first run).

Substitute into `is_newer_than(tick, last_run - 1, this_run)`:
- `ticks_since_insert = this_run.wrapping_sub(tick) = this_run.wrapping_sub(this_run) = 0`.
- `ticks_since_system = this_run.wrapping_sub(last_run - 1) = (this_run - last_run + 1)`.
- Compare: `(this_run - last_run + 1) > 0`. Since `this_run > last_run`, the LHS is `>= 2 > 0`. **True.**

So a self-write IS observable to `is_changed` in the same system. **Matches Bevy semantic.**

**Worked case: tick exactly at `last_run` boundary.**

Setup: `tick = last_run` (the previous frame's `this_run`). Should report as NOT changed (because writes in the previous frame produced `tick = previous_this_run = current last_run`, and "since last_run" means strictly after).

But the call signature is `is_changed`, not `is_changed_since_last_run_strictly`. The inclusive-at-`last_run` semantic is the deliberate Bevy choice — a value written in the previous frame "counts as changed" for the current frame's first observation. This matches the documented "`>=` semantic".

Substitute into `is_newer_than(last_run, last_run - 1, this_run)`:
- `ticks_since_insert = this_run.wrapping_sub(last_run)`.
- `ticks_since_system = this_run.wrapping_sub(last_run - 1) = (this_run - last_run + 1)`.
- Compare: `(this_run - last_run + 1) > (this_run - last_run)`. Always **True** (1 > 0).

Correct per the documented `>=` semantic.

**Worked case: tick strictly before `last_run`.**

Setup: `tick = last_run - 5` (5 ticks before the previous frame). Should report NOT changed.

Substitute:
- `ticks_since_insert = this_run.wrapping_sub(last_run - 5) = (this_run - last_run + 5)`.
- `ticks_since_system = this_run.wrapping_sub(last_run - 1) = (this_run - last_run + 1)`.
- Compare: `(this_run - last_run + 1) > (this_run - last_run + 5)`. **False** (1 < 5).

Correct.

**Conclusion:** under the Round 3 corrected `is_newer_than` formula, the `wrapping_sub(Tick::new(1))` trick continues to give the documented inclusive lower-bound semantic. No additional changes to MUT6 / REF3.

### 6.3 `Ref<T>` `QueryData` impl

(Body unchanged from Round 1 — see Round 1 plan §6.3. `set_table_*` accepts `meta: &'_ SystemMeta`.)

### 6.4 `Mut<T>` `QueryData` impl

(Body unchanged from Round 1 — see Round 1 plan §6.4.)

### 6.5 `Mut<T>` vs `&mut T`: which to use?

(Body unchanged from Round 1 — see Round 1 plan §6.5.)

---

## §7 — Storage layout deep dive

### 7.1 `ComponentPool` field additions

(Body unchanged from Round 1.)

### 7.2 Allocation in `ComponentPool::new` (Round 2 W1: per-archetype duplication)

(Body unchanged from Round 1. Re-emphasis on the per-archetype dimension: each archetype carrying component `C` has its OWN `ComponentPool` for `C`, with its OWN tick buffers. The pool sizes its tick buffers to `max_components` regardless of live rows. See §10.7 for the per-axis storage breakdown.)

### 7.3 Cache behavior in iteration

(Body unchanged from Round 1.)

### 7.4 Tick buffer growth

(Body unchanged from Round 1.)

### 7.5 SoA argument

(Body unchanged from Round 1.)

---

## §8 — Phase 9 parallel integration

### 8.1 Atomic discipline summary (Round 2 O2 updated)

| Operation | Atomic? | Memory ordering | Frequency |
|-----------|---------|------------------|-----------|
| `world.change_tick.fetch_add(1)` | Yes | `Relaxed` | 1 / frame |
| `world.change_tick.load()` (apply window read for tick init) | Yes | `Relaxed` | ≤ a few / apply window |
| `system.set_change_ticks(last, this)` (Round 2 C1) | No | sequential dispatcher logic | 1 / system / frame |
| `*added_ticks[row].get() = T` (entity insert) | No | sequential dispatcher (apply window) | 1 / spawn |
| `*changed_ticks[row].get() = T` (`Mut::deref_mut`) | No | per-row, ride conflict-graph exclusivity | 1 / write |
| `tick.is_newer_than(...)` reads | No | sequential per-thread; conflict-graph readers | 1 / row / filter check |
| `world.last_check_tick = ...` (after `check_ticks`) | No | sequential dispatcher (apply window) | 1 / ~100d |

**Total new atomics per frame: 1** (the frame-start `fetch_add`).

### 8.2 Cell minting and tick visibility (Round 2 C1, C2: full rewrite)

Phase 9 §5.4 mints a `UnsafeEcsCell` per dispatch round (Round 2 O3). Phase 10 needs the tick fields on `SystemMeta` to be visible to all workers spawned in that round.

The flow (Round 2 C1 simplified — single channel via `set_change_ticks`):

1. **Frame start (dispatcher):** `change_tick.fetch_add(1, Relaxed) → this_run`.
2. **Wraparound check (dispatcher, possibly):** if needed, `run_check_ticks_scan(world, this_run)` rewrites stored ticks + clamp every system's `last_run` via `set_change_ticks`. Workers are not yet spawned.
3. **Dispatch round 1 (dispatcher):**
   - For each system about to be dispatched:
     ```text
     let prev_this_run = self.systems[i].meta().this_run;
     self.systems[i].set_change_ticks(prev_this_run, this_run);   // Round 2 C1 channel
     ```
   - Mint cell from `&mut world`.
   - `scope.spawn(...)`. The system's `run_unsafe` paths reach `self.meta` through `&self.meta` accessor in their body (e.g., `FunctionSystem::run_unsafe` passes `&self.meta` to `SystemParam::get_param`).
4. **Worker (in spawn body):** `run_unsafe(cell)` → `SystemParam::get_param(&state, &meta, world)` — reads `&meta`, sees the ticks written in step 3 (happens-before via spawn).

   **Round 2 C2:** When the worker constructs a `Query<D, F>`, the `Query` holds `meta: &'s SystemMeta` (per Phase 8b `query.rs:60-62`). The new `QueryIter` / `QueryIterMut` carry the same reference forward.
5. **Worker completion:** push to completion_queue + fetch_add(pending_apply, Release).
6. **Dispatcher apply window:** load(pending_apply, Acquire) syncs-with worker's Release; pop completion → `system.apply(world)`. **No tick state mutation here** (Round 2 C1 simplification — folded into PHASE9.2).
7. **Next dispatch round:** repeat from step 3 with the same `this_run` for the same frame.

### 8.3 Per-row tick write under Phase 9 SCH3 (Round 2 C3 footnote)

For a system `S` with declared `Mut<Position>`:
- Phase 9 conflict graph: any other system that reads or writes `Position` conflicts with `S`. They run in different dispatch rounds.
- During `S`'s run, `S` is the **sole accessor** of `(any archetype, Position)`. Workers (par_iter chunks) operate on disjoint row ranges (PAR2).

Therefore:
- The `UnsafeCell<Tick>` write at `*changed_ticks[row].get() = this_run` is unsynchronised but sound.
- No fence needed between workers (disjoint rows; the scope's `pending` counter provides the join barrier).
- **Round 2 C3 footnote:** Adjacent rows from different par_iter chunks may share a cache line. This is sound per the Rust abstract machine — each `UnsafeCell<u32>` is a distinct memory location; concurrent writes to distinct memory locations are race-free regardless of cache-line sharing. The MESI ping-pong is a perf cost, not UB. Verified by `miri_par_iter_chunks_write_adjacent_ticks_disjoint_no_ub` (§13.3).

### 8.4 `par_iter` integration — Round 2 W6 (full chunk-dispatch enumeration)

`par_iter` (Phase 9 PAR1) spawns multiple chunks. The flow:

1. `Query::par_iter()` returns `ParQuery<'q, 's, D, F>` holding `state: &'s QueryDataState<D, F>`, `world: UnsafeEcsCell<'q>`, and **`meta: &'s SystemMeta`** (Round 2 W6 — NEW).
2. `ParQuery::for_each(body)` walks archetypes. For each archetype:
   - **Inline path (Round 2 W6 enumeration #1)**: `if entity_count < strategy.min_batch_size`, calls `run_chunk_inline::<D, F, Body>(state, arch_ptr, 0, entity_count, mutable, body_ref, meta)`. **Meta forwarded as a new parameter.** No `scope.spawn`; calling thread is the sole accessor.
   - **Spawn path (Round 2 W6 enumeration #2)**: builds `ChunkCaptures { ..., meta: *const SystemMeta }` (stored as raw pointer for `Send`-safety; reborrowed as `&SystemMeta` inside the spawn body). `scope.spawn(move || { run_chunk_owned(captured, body_ref); })`. The closure body unpacks `meta` from `captured` and forwards into `set_table_*`.
   - **PAR7 fallback path (Round 2 W6 enumeration #3)**: when no active thread pool exists, the par_iter falls back to sequential walk (par_iter.rs:362-376). Same chunk-dispatch loop; `meta` forwarded into the sequential `set_table_*` calls.

```rust
pub struct ParQuery<'q, 's, D: QueryData, F: QueryFilter> {
    pub(super) state: &'s QueryDataState<D, F>,
    pub(super) world: UnsafeEcsCell<'q>,
    pub(super) meta: &'s SystemMeta,    // NEW Round 2 W6
    pub(super) batching: BatchingStrategy,
}

struct ChunkCaptures<'s> {
    data_state: *const D::State,
    filter_state: *const F::State,
    archetype: *mut Archetype,
    start: usize,
    end: usize,
    mutable: bool,
    meta: *const SystemMeta,    // NEW Round 2 W6 (raw ptr for Send-safety;
                                // reborrowed as &SystemMeta inside the spawn body)
    _phantom: PhantomData<&'s ()>,
}
```

`&SystemMeta` is `Send + Sync` (`Access` + `&'static str` + `Tick`/`Tick`/`Generation`/`Generation` — all `Send + Sync`). The raw pointer storage is purely for `Send` plumbing through `scope.spawn`.

**Round 2 W6 test (added to §13.2):** `par_iter_inline_path_reports_changed` — entity_count < min_batch_size, write via Mut, downstream `Changed<T>` query sees the change. Validates the inline path's `meta` forwarding.

### 8.5 No regression to Phase 9 latency

The dispatcher per-frame work added by Phase 10:
- 1 `fetch_add` (the frame-start tick bump). ~5 ns.
- N `set_change_ticks` calls (one per system in dispatch). At 50 systems × 2 store ops/call = 100 store ops ≈ 50 ns.
- **NO apply-window writes (Round 2 C1 consolidation).**

Total Phase 10 dispatcher overhead: ~55 ns per frame. Against the Phase 9 §1.2 target of ≤ 20 µs at 50 systems: **0.28% overhead**. Within budget.

### 8.6 `check_ticks` scan placement

(Body unchanged from Round 1.)

---

## §9 — Wraparound handling

### 9.1 The wraparound problem

(Body unchanged from Round 1.)

### 9.2 Bevy's solution: bounded relative age

(Body unchanged from Round 1.)

### 9.3 `MAX_CHANGE_AGE` derivation — Round 2 W2 (first-principles proof)

**Problem:** `Tick` is a u32 counter. `is_newer_than` uses wrapping subtraction. The semantic is correct iff the relative ages `tick.wrapping_sub(last_run)` and `this_run.wrapping_sub(last_run)` are both bounded such that the comparison `>` interprets correctly under modular arithmetic.

**Constraint derivation:**

The `is_newer_than(self, last_run, this_run)` check (Round 3 corrected form) evaluates `(this_run - last_run) > (this_run - self)` under `wrapping_sub`. For this comparison to behave like "real" arithmetic, the differences must not exceed `2^31 - 1` (otherwise the unsigned comparison flips sign relative to the intended signed semantic).

Specifically, we need:
```
∀ stored_tick, ∀ this_run encountered:    abs(this_run - stored_tick) < 2^31
```

**Boyko's per-frame bump policy (Round 2 W2 specific):**

Boyko bumps `change_tick` once per `Schedule::run`. Between two consecutive `check_ticks` scans, the global tick advances by **at most** `CHECK_TICK_THRESHOLD` (the scan fires when the difference since the last scan reaches the threshold).

Worst case: stored tick `T_old` is clamped by a scan at time `T_scan`. Maximum age post-clamp: `MAX_CHANGE_AGE`. The next scan fires at time `T_scan + CHECK_TICK_THRESHOLD`. Between these scans, `T_old` ages naturally; just before the next scan, its age is `MAX_CHANGE_AGE + CHECK_TICK_THRESHOLD`.

For `is_newer_than` to remain correct at the moment just before the second scan:
```
MAX_CHANGE_AGE + CHECK_TICK_THRESHOLD < 2^31
```

**Bevy's formula check:**
```
MAX_CHANGE_AGE = u32::MAX - (2 * CHECK_TICK_THRESHOLD - 1)
              = 2^32 - 1 - 2 * CHECK_TICK_THRESHOLD + 1
              = 2^32 - 2 * CHECK_TICK_THRESHOLD

MAX_CHANGE_AGE + CHECK_TICK_THRESHOLD
              = 2^32 - 2 * CHECK_TICK_THRESHOLD + CHECK_TICK_THRESHOLD
              = 2^32 - CHECK_TICK_THRESHOLD
              ≈ 4_294_967_296 - 518_400_000
              ≈ 3_776_567_296
```

**3_776_567_296 > 2^31 = 2_147_483_648**. The naive form of the inequality is **violated**.

**Reconciliation (the missing piece):**

The actual constraint is not `MAX_CHANGE_AGE + CHECK_TICK_THRESHOLD < 2^31` (one-sided), but rather about the SIGNED interpretation of the differences. Specifically:
- `this_run - last_run` (with `last_run = current - MAX_CHANGE_AGE` and `this_run = current`) = `MAX_CHANGE_AGE` < `2^31`? **No** (`MAX_CHANGE_AGE ≈ 3.26 × 10^9 > 2^31`).

So the formula is even tighter than I derived. Let me re-derive from the SOURCE.

Re-deriving from [Bevy commit b6a8c3a0 (PR #3956)](https://github.com/bevyengine/bevy/pull/3956):

> The system tick advancement and the user-controlled tick read interact such that, **after** a wraparound scan, the maximum age between any two compared ticks is `MAX_CHANGE_AGE + CHECK_TICK_THRESHOLD < u32::MAX`.

The reference is to the **unsigned** ordering on u32 with wrapping subtraction interpreting differences in [0, u32::MAX]. The actual SAFE form of `is_newer_than` interpretation (Round 3 corrected):
- `ticks_since_insert = this_run.wrapping_sub(self) ∈ [0, u32::MAX]`.
- `ticks_since_system = this_run.wrapping_sub(last_run) ∈ [0, u32::MAX]`.
- Compare `ticks_since_system > ticks_since_insert` directly as unsigned u32.

For this comparison to be semantically correct (i.e., return true iff `self` is "newer than" `last_run` along the monotonic path from `last_run` to `this_run`):
- `last_run` must be the OLDEST tick along the path.
- `this_run - last_run` (unsigned, wrapped) must equal the actual elapsed ticks since `last_run`.
- For this, the actual elapsed time `this_run - last_run` (treating ticks as a never-wrapping abstract counter) must equal the wrapped value.

The wrapped value equals the actual elapsed iff the actual elapsed < `2^32`. Bevy guarantees this by:
- Clamping any stored tick whose age exceeds `MAX_CHANGE_AGE` to exactly `current - MAX_CHANGE_AGE` (during `check_ticks`).
- Ensuring `MAX_CHANGE_AGE + CHECK_TICK_THRESHOLD < u32::MAX` (so even the OLDEST clamped tick PLUS the maximum gap between scans stays within u32::MAX of the current tick).

Bevy's formula satisfies `MAX_CHANGE_AGE + CHECK_TICK_THRESHOLD = u32::MAX - CHECK_TICK_THRESHOLD + 1 < u32::MAX`. ✓

The "2^31" misconception was mine: the constraint is actually `< u32::MAX = 2^32 - 1`, not `< 2^31`. The unsigned comparison `ticks_since_system > ticks_since_insert` is well-defined for ALL pairs of u32 values; the SEMANTIC of "newer" is preserved as long as the actual elapsed ticks between the oldest tick and `this_run` does not exceed `u32::MAX`.

**Conclusion:**
- Boyko's per-frame bump policy with `CHECK_TICK_THRESHOLD = 518.4 M` ticks → ~100 days between scans → scans clamp anything older than `MAX_CHANGE_AGE`.
- Constraint: `MAX_CHANGE_AGE + CHECK_TICK_THRESHOLD < u32::MAX`. Bevy's formula satisfies this.
- For boyko's per-frame bump policy specifically: between scans, ticks age by exactly `CHECK_TICK_THRESHOLD` worst case (one bump per frame, scan fires after threshold ticks). Maximum age post-clamp: `MAX_CHANGE_AGE`. Maximum age just before next scan: `MAX_CHANGE_AGE + CHECK_TICK_THRESHOLD`. Less than `u32::MAX`. **Safe.**

If we used per-system bump (multiple ticks per frame), the inequality would still hold; the only difference is scan frequency.

**Adopting Bevy's formula** (post-derivation) is justified for boyko's per-frame bump policy. The property test in §13.4 fuzzes the entire space to verify.

### 9.4 New system's initial `last_run` (Round 2 W5 — moved to constructor)

```rust
// In SystemMeta::new (Round 2 W5):
pub fn new(name: &'static str, current_tick: Tick) -> Self {
    let last_run = Tick::new(current_tick.0.wrapping_sub(MAX_CHANGE_AGE));
    Self {
        // ... fields ...
        last_run,
        this_run: last_run,  // pre-first-run; updated by set_change_ticks
    }
}
```

NOT `Tick::ZERO`. The constructor-enforced path eliminates the bypass-`initialize` failure mode of Round 1.

### 9.5 Set on system addition (Round 2 W5: no longer in `initialize`)

`FunctionSystem::initialize` no longer needs to set `last_run` (the constructor handles it). `initialize` retains its existing role of building `state` + declaring `access`.

```rust
impl<F, M> FunctionSystem<F, M> {
    pub fn new(name: &'static str, world: &EcsMaster, f: F) -> Self {
        let current_tick = world.current_tick();   // reads change_tick atomically
        Self {
            meta: SystemMeta::new(name, current_tick),
            // ... other fields ...
        }
    }
}
```

### 9.6 `check_ticks` algorithm (Round 2 W3: live rows only; Round 3 W-NEW-1 + W-NEW-1b: APIs corrected)

```rust
#[cold]
pub(crate) fn run_check_ticks_scan(world: &mut EcsMaster, current: Tick) {
    // 1. Walk every archetype over LIVE rows (Round 2 W3).
    //    Round 3 W-NEW-1b: use iter_archetypes_mut() — new one-liner
    //    on ArchetypeMaster mirroring the existing iter_archetypes()
    //    (archetype_master.rs:590). The Round 2 plan referenced
    //    `archetype_master.archetype_ids()` which does NOT exist.
    for arch in world.archetype_master.iter_archetypes_mut() {
        for component_id in arch.component_ids().to_vec() {
            let pool = arch.component_pools_mut().get_pool_mut(component_id).unwrap();
            // Round 3 W-NEW-1: use pool.count() — actual API at
            // component_pool.rs:620. Round 2 plan referenced
            // `pool.units_len()` which does NOT exist.
            let live_count = pool.count();
            for i in 0..live_count {
                // SAFETY: dispatcher holds &mut world; no aliasing.
                unsafe {
                    let added_ref = &mut *pool.added_ticks[i].get();
                    added_ref.check_tick(current);
                    let changed_ref = &mut *pool.changed_ticks[i].get();
                    changed_ref.check_tick(current);
                }
            }
        }
    }
    // 2. Schedule's own systems' last_run handled separately in Schedule::run (§4.5).
}
```

**API surface note (Round 3 W-NEW-1b):** `ArchetypeMaster` currently exposes `pub fn iter_archetypes(&self) -> impl Iterator<Item = &Archetype>` at `archetype_master.rs:590` but no `&mut` variant. Step 12 (`check_ticks`) adds the one-liner:
```rust
/// Mutable iterator over every archetype.
///
/// Mirror of `iter_archetypes()`; required by Phase 10's `run_check_ticks_scan`
/// for in-place tick clamping.
#[inline]
pub fn iter_archetypes_mut(&mut self) -> ArchetypeBundleIterMut<'_> {
    self.archetypes.iter_mut()
}
```
Delegates to `ArchetypeBundle::iter_mut()` (already public at `archetype_bundle.rs:680`). No new structural changes.

### 9.7 Wraparound recovery proof sketch (Round 2 W2 reproof)

**Claim:** under `MAX_CHANGE_AGE` clamping and the `CHECK_TICK_THRESHOLD` scan frequency, `is_newer_than(stored, last_run, this_run)` produces the correct semantic.

**Proof:** by construction of the clamp + scan.

Any `stored` tick whose age would exceed `MAX_CHANGE_AGE` is clamped to `current - MAX_CHANGE_AGE`. The next scan fires within `CHECK_TICK_THRESHOLD` ticks. Maximum relative age between scans: `MAX_CHANGE_AGE + CHECK_TICK_THRESHOLD`. Bevy's formula keeps this strictly less than `u32::MAX`, so wrapping subtraction's unsigned ordering correctly reflects the true age ordering. (Round 2 W2 derivation in §9.3.)

For new systems (`last_run = current - MAX_CHANGE_AGE`): the relative age of `last_run` from `this_run = current`'s perspective is `MAX_CHANGE_AGE`. Safe.

§13 includes a property-based test that fuzzes the entire space.

---

## §10 — Hot-path perf projections

### 10.1 `Tick::is_newer_than`

(Body unchanged from Round 1.)

### 10.2 `Mut<T>::deref_mut`

(Body unchanged from Round 1.)

### 10.3 `Changed<T>::filter_fetch` (Round 2 C4: null-base cost row added; Round 3 C-NEW-1: lowering updated for corrected formula)

```text
fn filter_fetch(fetch, row) -> bool {
    if fetch.tick_base.is_null() { return false; }   // Round 2 C4 cost
    let tick_ptr = unsafe { fetch.tick_base.add(row) };
    let tick = unsafe { *(*tick_ptr).get() };
    tick.is_newer_than(fetch.last_run, fetch.this_run)
}
```

x86 lowering (Round 3 C-NEW-1: reflects the corrected `is_newer_than` formula
`ticks_since_system > ticks_since_insert` — both subtractions are against `this_run`,
NOT against `last_run`):
```asm
test rax, rax                ; null check on fetch.tick_base (Round 2 C4)
jz   .ret_false              ; predicted not-taken
mov  edx, DWORD PTR [rax + rcx*4]   ; edx = tick (the stored u32)
                                    ; r8d = last_run, r9d = this_run
mov  esi, r9d                ; copy this_run for the second subtraction
sub  r9d, edx                ; r9d  = this_run - tick      (ticks_since_insert)
sub  esi, r8d                ; esi  = this_run - last_run  (ticks_since_system)
cmp  esi, r9d
seta al                      ; al = (ticks_since_system > ticks_since_insert)
ret
.ret_false:
xor  al, al
ret
```

Note: the Round 2 lowering wrote `sub eax, esi; sub edx, esi` (both subtractions
against `last_run`), reflecting the buggy `(this_run - last_run) > (self - last_run)`
form. The Round 3 corrected lowering shows both subtractions targeting `this_run`
as the minuend — one `wrapping_sub(self)`, one `wrapping_sub(last_run)`.

Per-row cost breakdown (Round 2 C4 expanded):

| Step | Cost | Note |
|------|------|------|
| `fetch.tick_base.is_null()` check | ~0.5 ns | predicted not-taken; predicate density adds I-cache pressure on the cold path |
| `tick_ptr = tick_base.add(row)` | 0 ns | folds into addressing mode |
| `tick = *(*tick_ptr).get()` | ~1 ns cold, ~0.3 ns hot | L1d hit dominates |
| `tick.is_newer_than(last_run, this_run)` | ~1 ns | 2 sub + cmp (Round 3 corrected formula — same instruction count) |
| Branch (continue if !pass) | ~0.5 ns | predictable per archetype |
| **Per row total (archetype contains C)** | **~1-2 ns** | hot path |
| **Per row total (Or<F> archetype lacks C, null-base early return)** | **~0.5 ns** | Round 2 C4 |

### 10.4 Per-row hot loop with `Changed<Position>`

(Body unchanged from Round 1 — see Round 1 plan §10.4.)

### 10.5 Frame-start tick bump

`world.change_tick.fetch_add(1, Relaxed)`: LOCK XADD on x86. ~5-10 ns. 1 per frame.

### 10.5-bis `Or<(Added, Changed)>` archetype scan dominance (Round 2 W8)

`Or<F>::aggregate_include` is a no-op (Phase 8b M8 contract). Consequence: `Or<(Added<A>, Changed<B>)>` queries do NOT narrow the archetype set via include-mask; `update_archetypes` walks every archetype.

**Cost:**
| Scenario | Archetypes walked | Per-archetype cost | Total |
|----------|---------------------|--------------------|-------|
| `Query<&C, Changed<A>>` (A's include-mask narrows) | only archetypes containing A | ~5 ns lookup + per-row scan | small |
| `Query<&C, Or<(Added<A>, Changed<B>)>>` (no narrowing) | EVERY archetype | ~5 ns lookup + per-row scan with null-base check | grows with archetype count |

At 1024 archetypes × 5 ns = 5 µs cold-startup overhead per `Or<(Added, Changed)>` query. This is the dominant cost when the query matches a small fraction of archetypes.

**Recommendation in cookbook:** users should narrow with `Or<(With<X>, ...)>` to leverage the OR's union of `matches_component_set` checks. The `With<X>` element does have `aggregate_include` set, so adding it forces inclusion narrowing — except that `Or::aggregate_include` is no-op, so the bit setting doesn't propagate. **There is no current way to narrow an `Or<F>` to a subset of archetypes.** §17 OQ-12 lists this for future work.

§13.5 adds `bench_or_added_changed_archetype_count_dominated` to track regression.

### 10.6 `check_ticks` scan (Round 2 W3: live rows; Round 3 W-NEW-1: API renamed)

100 k live entities × 50 components × 2 ticks = 10 M `u32::wrapping_sub + compare`. At 1 cycle/op: ~3 ms cold, ~1 ms hot.

Frequency at boyko's per-frame bump (`CHECK_TICK_THRESHOLD = 518.4 M`):
- 1 tick/frame × 60 FPS → 60 ticks/sec → scan every ~100 days.
- 1 tick/frame × 240 FPS → 240 ticks/sec → scan every ~25 days.

Effectively never on a real frame budget.

**Round 2 W3 vs Round 1**: original §10.6 estimate "10 M ops" was correct for the LIVE row count but the pseudocode walked `pool.added_ticks.iter()` (full buffer). Corrected to `0..pool.count()` (Round 3 W-NEW-1: was `units_len()`). If a system has 1024 archetypes × 50 components × 2 ticks × 1024 max_components live entities = 100 M ops worst case. With actual live rows (100 k entities × 50 components × 2) = 10 M ops. Order-of-magnitude reduction.

### 10.7 Storage overhead at typical scale (Round 2 W1 — corrected)

| Scale | Archetypes | Components/archetype | Max_components/pool | Total tick storage | Total memory delta |
|-------|------------|----------------------|---------------------|---------------------|---------------------|
| Small game | 16 | 10 | 1024 | 1.25 MB | < 1% |
| Medium game | 64 | 20 | 2048 | 20 MB | ~5% |
| AAA target (original 40 MB figure) | 100 | 50 | 1024 | 40 MB | ~3% |
| W1's stress example | 1024 | 10 | 1024 | 80 MB | ~5% |
| W1's worst case | 1024 | 50 | 4096 | 1.6 GB | meaningful — needs OQ-1 opt-out |

For the AAA target (the design point): 40 MB. For higher archetype counts: scales linearly with `archetypes × components × max_components × 8 B`. **Round 2 W1: this is an accepted tradeoff** vs option C (per-archetype-per-column scalar — less precise). If stress-scale numbers (1.6 GB) become a real concern, §17 OQ-1 (`NoChangeTracking` opt-out) and OQ-4 (hybrid storage D) are the migration paths.

---

## §11 — Memory layouts + sizes

### 11.1 Struct sizes table (Round 2 O2 + O5 updated)

| Struct | Size | Padded layout | Alignment |
|--------|------|----------------|-----------|
| `Tick` | 4 B | `repr(transparent) u32` | 4 B |
| `UnsafeCell<Tick>` | 4 B | `repr(transparent)` | 4 B |
| `AddedFetch<'w>` | 24 B | natural | 8 B |
| `ChangedFetch<'w>` | 24 B | natural | 8 B |
| `RefFetch<'w, T>` | 48 B | 3 ptrs + 2 ticks | 8 B |
| `MutFetch<'w, T>` | 48 B | same as RefFetch | 8 B |
| `Ref<'w, T>` | 40 B | 3 refs + 2 ticks | 8 B |
| `Mut<'w, T>` | 40 B | same as Ref | 8 B |
| `SystemChangeTick` | 8 B | 2 ticks | 4 B |
| `SystemMeta` (after Phase 10, Round 2 O5) | **232 B** | Phase 9 was 224 B; +8 B for two Ticks (no `_tick` suffix) | 8 B |
| `QueryIter` (after Round 2 C2) | Phase 8b: 72 B; +8 B for `meta: &SystemMeta` = **80 B** | — | 8 B |
| `QueryIterMut` (after Round 2 C2) | Same as QueryIter | — | 8 B |

### 11.2 `SystemMeta` extension (Round 2 O5)

```rust
#[repr(C)]
pub struct SystemMeta {
    pub(crate) access: Access,                                  // 192 B
    pub(crate) name: &'static str,                              // 16 B
    pub(crate) last_archetype_generation: ArchetypeGeneration,  // 8 B
    pub(crate) last_structural_generation: ArchetypeGeneration, // 8 B
    pub(crate) last_run: Tick,                                  // 4 B  NEW (Round 2 O5: no suffix)
    pub(crate) this_run: Tick,                                  // 4 B  NEW
}
```

Total: 192 + 16 + 8 + 8 + 4 + 4 = **232 B**. Up from 224 B.

### 11.3 `ComponentPool` extension

(Body unchanged from Round 1.)

### 11.4 `EcsMaster` extension (Round 2 O2: drop CachePadded)

```rust
pub struct EcsMaster {
    // ... existing fields ...
    change_tick: AtomicU32,                 // 4 B (Round 2 O2: no CachePadded)
    last_check_tick: Tick,                  // 4 B
}
```

Net +8 B. Trivial.

### 11.5 False-sharing audit (Round 2 C3 + O2)

- `change_tick` is **NOT** `CachePadded` (Round 2 O2 — touched once per frame; false-sharing risk is zero).
- `meta.this_run` / `last_run` live inside `SystemMeta` (232 B, 4 cache lines). The two ticks fit at the end. They're written exclusively by the dispatcher; workers read them through the Phase 9 spawn boundary. No false sharing risk because no two threads touch the same `SystemMeta`.
- Per-row `added_ticks[row]` / `changed_ticks[row]`: false sharing within a tick buffer is possible if two `par_iter` workers write adjacent rows from the same archetype on the same cache line.

  **Round 2 C3 formal soundness argument:** This is sound per Rust's abstract machine. Two non-overlapping memory locations CAN be modified concurrently without synchronization (regardless of cache-line layout). Each `UnsafeCell<u32>` at offset `i * 4` from the base is a **distinct memory location** per the Rust memory model. The C++/Rust race condition rule is about *overlapping* memory locations, not cache-line sharing.

  **Citation:** [Rustonomicon §"Data Races and Race Conditions"](https://doc.rust-lang.org/nomicon/races.html):
  > A data race is defined as two unsynchronized accesses to the same memory location from different threads, where at least one is a write. Distinct memory locations (e.g., distinct fields of a struct, distinct array elements) can be modified independently.

  The cache-line ping-pong between adjacent workers IS a MESI perf cost (false sharing) — measurable as L1d invalidation traffic. The plan accepts this:
  - `par_iter`'s `min_batch_size` (PAR9) is 1024 rows by default; cache lines (64 B) at 4 B/tick = 16 ticks per line. Chunks of 1024 rows = 64 cache lines per chunk. Boundary cache-line sharing happens at most once per chunk transition.
  - Real-world cost: ~50 ns per chunk transition (one cache-line invalidation round-trip). At 100 chunks: 5 µs / frame. Compared to the per-chunk body cost (microseconds), trivial.

  Verified via `miri_par_iter_chunks_write_adjacent_ticks_disjoint_no_ub` (§13.3 Round 2 C3).

### 11.6 Total Phase 10 storage delta (Round 2 W1)

For the AAA design point (100 archetypes × 50 components × 1024 max_components):
- Tick buffers: 40 MB.
- `SystemMeta` extension: 8 B × 50 systems = 400 B. Trivial.
- `ComponentPool` extension: ~32 B × 50 components × 100 archetypes = 160 KB. Trivial.
- `EcsMaster` extension: 8 B (Round 2 O2). Trivial.
- `QueryIter` / `QueryIterMut` (Round 2 C2): +8 B each. Trivial.

**Total Phase 10 delta: ~40 MB at design point.** For higher archetype counts, scales per the §10.7 table.

---

## §12 — Public API surface with examples

(Body unchanged from Round 1 — see Round 1 plan §12.)

---

## §13 — Test plan

### 13.1 Unit tests

| Test | File | Coverage |
|------|------|----------|
| `tick_is_newer_than_basic` | `tick.rs` | Basic compare, no wraparound. |
| `tick_is_newer_than_wraparound_safe` | `tick.rs` | `last_run` and `this_run` span 0. |
| **`tick_is_newer_than_self_equal_this_run`** (Round 3 C-NEW-1) | `tick.rs` | Asserts `Tick(10).is_newer_than(Tick(2), Tick(10)) == true` (inclusive upper bound; Round 2 formula returned false here — the regression that motivated C-NEW-1). |
| `tick_check_tick_clamps` | `tick.rs` | Setup tick at `current - 2*MAX_CHANGE_AGE`. |
| `tick_check_tick_noop_when_in_range` | `tick.rs` | No clamp when in range. |
| **`tick_default_is_zero`** (Round 2 O4) | `tick.rs` | `Tick::default() == Tick::ZERO`. |
| `added_filter_state_caches_id` | `filter.rs` | `Added<C>::init_state` returns id. |
| `added_filter_matches_component_set` | `filter.rs` | Mirrors `With<C>` behaviour. |
| `added_filter_is_archetypal_false` | `filter.rs` | Const-check `IS_ARCHETYPAL = false`. |
| `changed_filter_*` | `filter.rs` | Same shape as `Added<C>` tests. |
| `ref_query_data_caches_pointers` | `data.rs` | `set_table_readonly` populates all three pointers. |
| `mut_query_data_deref_mut_bumps_tick` | `data.rs` | `Mut<T>::deref_mut` writes tick. |
| `mut_set_if_neq_skips_when_equal` | `data.rs` | No tick write on equality. |
| `mut_set_if_neq_bumps_when_different` | `data.rs` | Tick written on inequality. |
| `mut_bypass_change_detection_no_bump` | `data.rs` | `bypass_change_detection` does not bump. |
| `ref_is_changed_reads_tick` | `data.rs` | After `Mut::deref_mut`, `Ref::is_changed` returns true. |
| **`mut_is_changed_after_self_write_observes_change`** (Round 2 O1) | `data.rs` | `Mut::is_changed` returns true for a self-write in the same system (inclusive lower-bound semantic). |
| `system_change_tick_param_returns_meta_ticks` | `system_change_tick.rs` | `SystemChangeTick::this_run()` returns `meta.this_run`. |
| **`system_meta_new_initializes_last_run_correctly`** (Round 2 W5) | `system_meta.rs` | `SystemMeta::new(name, current_tick)` sets `last_run = current - MAX_CHANGE_AGE`. |
| **`noop_system_set_change_ticks_writes_meta`** (Round 2 C1) | `system.rs` | Trait method writes both ticks; the dispatcher's contract. |

### 13.2 Integration tests

| Test | Coverage |
|------|----------|
| `added_filter_basic_spawn_query` | Spawn entity in frame N; query `Added<T>` in frame N — sees it. |
| `changed_filter_via_mut_deref` | System A mutates `Position` via `Mut<Position>::deref_mut`; system B queries `Changed<Position>` — sees the change. |
| `changed_filter_set_if_neq_no_bump` | `set_if_neq` with equal value; downstream system sees no change. |
| `or_added_changed_composition` | `Or<(Added<A>, Changed<B>)>` matches archetype with A only AND archetype with B-changed only. |
| `wraparound_scan_clamps_aged_ticks` | Manually advance `change_tick` by `2 * CHECK_TICK_THRESHOLD`; verify clamp. |
| `multi_frame_added_lifecycle` | Entity added frame 1, queried in frames 1, 2, 3 — sees in frame 1 only. |
| `parallel_changed_correctness` | Phase 9 parallel: system A writes Position; system B reads `Changed<Position>` — sees A's changes. |
| `par_iter_no_false_positive_no_writes` | `par_iter_mut` no deref-mut; downstream `Changed<T>` reports nothing. |
| `par_iter_yes_positive_with_writes` | `par_iter_mut` deref-muts on some rows; downstream reports exactly those rows. |
| `archetype_grow_preserves_ticks` | Trigger pool grow; pre-grow tick values preserved. |
| **`par_iter_inline_path_reports_changed`** (Round 2 W6) | entity_count < min_batch_size; inline path writes via Mut; downstream `Changed<T>` sees changes (validates `meta` forwarded through `run_chunk_inline`). |
| **`or_with_changed_archetype_lacking_c_no_panic`** (Round 2 C4) | `Or<(With<A>, Changed<B>)>` on A-only archetype; does not panic; iterates correctly. |

### 13.3 Miri tests

| Test | Coverage |
|------|----------|
| `miri_phase10_unsafe_cell_tick_write` | Single-threaded: `Mut::deref_mut` writes through `UnsafeCell<Tick>`. |
| **`miri_par_iter_chunks_write_adjacent_ticks_disjoint_no_ub`** (Round 2 C3) | Multi-threaded simulation: spawn 4 chunks, each writes to disjoint tick slots `[0..256]`, `[256..512]`, `[512..768]`, `[768..1024]` from 4 separate threads. Verify Miri (Tree Borrows) does NOT flag UB. Formal soundness per the §11.5 Rustonomicon citation. |
| `miri_phase10_filter_null_base_branch` | `Or<(With<A>, Changed<B>)>` where archetype lacks B; verify the null-base branch is safe. |
| `miri_phase10_check_ticks_scan` | Run `check_ticks_scan` on a populated world; verify no UB. |
| **`miri_set_change_ticks_dispatch_to_worker_visibility`** (Round 2 C1) | Single-threaded sim of dispatcher → worker: dispatcher calls `set_change_ticks`; worker reads `&meta` through Query — verify Miri sees the write. |

### 13.4 Property-based tests

| Test | Coverage |
|------|----------|
| `prop_is_newer_than_wraparound_invariant` | Fuzz `(stored, last_run, this_run)` triples under `MAX_CHANGE_AGE` invariant. |
| `prop_check_tick_idempotence` | After `check_tick`, repeated calls no-op. |
| `prop_added_changed_disjoint` | `Added` ⊆ `Changed` for an entity added in frame N. |
| **`prop_max_change_age_safe_under_per_frame_bump`** (Round 2 W2) | Fuzz the per-frame bump schedule; verify the `MAX_CHANGE_AGE + CHECK_TICK_THRESHOLD < u32::MAX` invariant holds across long simulation runs. |

### 13.5 Criterion benches (Round 2 W8 added; Round 3 W-NEW-1: API renamed)

| Bench | Target |
|-------|--------|
| `bench_tick_is_newer_than` | ≤ 1 ns. |
| `bench_mut_deref_mut` | ≤ 1 ns. |
| `bench_changed_filter_1024_rows` | ≤ 1024 ns (1 ns/row). |
| `bench_changed_filter_1024_rows_zero_changed` | ≤ 1024 ns (branch predictable). |
| `bench_query_no_change_detection_no_overhead` | 0% delta vs Phase 8b baseline. |
| `bench_check_ticks_scan_100k_entities` | ≤ 5 ms (Round 2 W3: live rows only via `pool.count()` — Round 3 W-NEW-1 API correction). |
| `bench_phase9_dispatcher_with_phase10_no_regression` | ≤ 0.5% delta. |
| `bench_par_iter_changed_writes` | par_iter scaling under change detection. |
| **`bench_or_added_changed_archetype_count_dominated`** (Round 2 W8) | Measures per-archetype walk overhead for `Or<(Added, Changed)>` at 16, 64, 256, 1024 archetypes. Linear scaling expected. |
| **`bench_or_with_changed_null_base_branch_cost`** (Round 2 C4) | `Or<(Changed<B>, With<A>)>` on A-only archetype. Per-row cost target ≤ 0.5 ns (the null-base branch). |

### 13.5-bis Inlining verification (Round 2 O3)

Step 16 adds:
- `cargo asm boyko_ecs::bench_changed_filter_1024_rows` snapshot verifies `is_newer_than` and `filter_fetch` are inlined (no `call` instructions to those symbols in the hot loop).
- PGO measurement deferred to a follow-up phase; not a blocker.

### 13.6 Debug assertions

| Assertion | Location |
|-----------|----------|
| `debug_assert!(!fetch.tick_base.is_null(), ...)` | `Added<C>::filter_fetch` (defense in depth). |
| `debug_assert!(self.allows_mutable_access, ...)` | `UnsafeEcsCell::archetype_ptr_mut`. |
| `debug_assert!(component_id.0 < MAX_COMPONENTS, ...)` | `tick_column_base`. |
| `debug_assert!(row < pool.units.len(), "tick row OOB")` | `Mut::deref_mut`'s `.add(row)`. |
| `debug_assert!(self.meta.this_run != Tick::ZERO, "this_run not initialized")` | `Mut<T>::deref_mut`. |
| **`debug_assert!(self.meta.last_run != Tick::ZERO ... unless explicit test bypass")`** (Round 2 W5) | Removed — Round 2 W5 moves `last_run` init to constructor; no test bypass needed. |

---

## §14 — Step-by-step implementation plan

The 16 Steps below are partially parallelisable.

### Wave 1 — Foundation

#### Step 1 — `Tick(u32)` newtype + constants (Round 2 O4: Default; Round 3 C-NEW-1: corrected `is_newer_than`)

**Files:**
- New: `crates/boyko_ecs/src/ecs/core/change_detection/mod.rs`
- New: `crates/boyko_ecs/src/ecs/core/change_detection/tick.rs`

**Content:**
- `Tick(u32)` with `is_newer_than` (Round 3 corrected formula `ticks_since_system > ticks_since_insert`), `check_tick`, `next`, `wrapping_sub`, `ZERO`, `new`, **`Default` (Round 2 O4)**.
- `CHECK_TICK_THRESHOLD` and `MAX_CHANGE_AGE` constants with the Round 2 W2 derivation comment.
- Unit tests (§13.1) including Round 3 `tick_is_newer_than_self_equal_this_run`.

#### Step 2 — `EcsMaster::change_tick` infrastructure (Round 2 O2: no CachePadded)

**Files:**
- Modified: `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs`

**Content:**
- Add `change_tick: AtomicU32` (Round 2 O2: NOT CachePadded) + `last_check_tick: Tick`.
- Initialise in `EcsMaster::new`.
- Add `pub fn current_tick(&self) -> Tick` getter.
- Modify `create_entity` to read `self.change_tick.load(Relaxed)` and thread the tick (Round 2 W4 — INIT3 canonical).

### Wave 2 — `SystemMeta` + `SystemChangeTick`

#### Step 3 — `SystemMeta` tick fields (Round 2 W5 + C1 + O5)

**Files:**
- Modified: `crates/boyko_ecs/src/ecs/core/system/system_meta.rs`
- Modified: `crates/boyko_ecs/src/ecs/core/system/system.rs` (trait method)

**Content (Round 2 multi-fix):**
- **W5**: change `SystemMeta::new(name)` → `SystemMeta::new(name, current_tick: Tick)`. Constructor sets `last_run = current_tick - MAX_CHANGE_AGE`, `this_run = last_run`.
- **O5**: field names `last_run: Tick`, `this_run: Tick` (no `_tick` suffix). Harmonise with `SystemChangeTick`.
- **C1**: add `fn set_change_ticks(&mut self, last_run: Tick, this_run: Tick)` to `System` trait. No default body.
- Update `NoopSystem` (test stub at `system.rs:131`) to implement `set_change_ticks` and pass `current_tick` (test default: `Tick::new(1)`) to `SystemMeta::new`.
- Size assertion: `size_of::<SystemMeta>() == 232`.

#### Step 4 — `SystemChangeTick` SystemParam (parallel with Step 3)

**Files:**
- New: `crates/boyko_ecs/src/ecs/core/change_detection/system_change_tick.rs`

**Content:**
- `SystemChangeTick { this_run: Tick, last_run: Tick }` struct (Round 2 O5).
- `SystemParam` impl reading from `SystemMeta`.
- Helper accessor methods.

### Wave 3 — `ComponentPool` tick buffers

#### Step 5 — `ComponentPool::added_ticks` / `changed_ticks`

(Unchanged from Round 1 — see Round 1 plan §14 Step 5.)

#### Step 6 — `Archetype::tick_column_base` + `create_entity` tick threading

(Unchanged from Round 1.)

### Wave 4 — Filter integration

#### Step 7 — `Added<C>` / `Changed<C>` filter impls

(Unchanged from Round 1.)

#### Step 8 — `QueryFilter::set_table_*` signature change (+meta) + propagation (Round 2 C2 + W6 + W7)

**Files (Round 2 C2 — explicit enumeration):**
- Modified: `crates/boyko_ecs/src/ecs/core/iters/query/filter.rs` (every impl + macros + `set_table_*` signature with `meta: &'_ SystemMeta` per Round 2 W7).
- Modified: `crates/boyko_ecs/src/ecs/core/iters/query/data.rs` (parallel change).
- Modified: `crates/boyko_ecs/src/ecs/core/iters/query/iter.rs`:
  - Add `meta: &'s SystemMeta` field to `QueryIter` and `QueryIterMut` (Round 2 C2).
  - Update `QueryIter::new` and `QueryIterMut::new` signatures: `new(state, world, meta)`.
  - Update the `next()` body's `set_table_*` calls to pass `self.meta`.
- Modified: `crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs`:
  - Add `meta: &'s SystemMeta` field to `ParQuery` and `ParQueryMut`.
  - Add `meta: *const SystemMeta` field to `ChunkCaptures` (raw ptr for `Send`; reborrowed as `&SystemMeta` inside spawn body).
  - Update `run_chunk_inline` signature: `run_chunk_inline(..., meta: &SystemMeta)`.
  - Update `run_chunk_owned` to unpack `meta` from `ChunkCaptures` and forward.
  - Update PAR7 fallback path (par_iter.rs:362-376) to forward `meta`.
- Modified: `crates/boyko_ecs/src/ecs/core/iters/query/query.rs`:
  - `Query::iter` / `Query::iter_mut` forward `self.meta` to `QueryIter::new` / `QueryIterMut::new`.
  - `Query::par_iter` / `Query::par_iter_mut` forward `self.meta` to `ParQuery::new` / `ParQueryMut::new`.

**Content:**
- Trait signature: `set_table_*<'w>(fetch: &mut Self::Fetch<'w>, state: &Self::State, archetype: *const/*mut Archetype, meta: &'_ SystemMeta)` (Round 2 W7).
- Document "meta is read-only INPUT; ticks copied to Fetch by value".
- Archetypal impls (`()`, `With`, `Without`, etc.) accept and ignore the `meta` parameter.
- `Added<C>` / `Changed<C>` / `Ref<T>` / `Mut<T>` impls read `meta.last_run` / `meta.this_run` and write to `Fetch<'w>`.

**Tests to migrate (Round 2 C2):**
- Add `SystemMeta::for_testing()` helper in `tests/common.rs` returning `SystemMeta::new("test", Tick::new(1))`.
- Update `tests/query_dsl_smoke.rs`, `tests/miri_phase8a.rs`, `tests/system_param_smoke.rs`, `benches/query_dsl.rs` to pass a meta argument.

**Dependency:** Step 7 (for the impl that needs `meta`).
**Deliverable:** Trait signature updated; all existing tests pass with the migrated meta argument.
**Estimated size**: ~2 days (1.5 implementation + 0.5 test migration).

### Wave 5 — `Ref<T>` / `Mut<T>` `QueryData` impls

#### Step 9 — `Ref<'w, T>` + `Mut<'w, T>` structs (Round 2 O1 + O5)

(Body unchanged from Round 1 except: `is_changed` / `is_added` use Round 2 O1 `>=` semantic; field naming Round 2 O5.)

#### Step 10 — `Ref<T>` / `Mut<T>` `QueryData` impls

(Unchanged from Round 1.)

### Wave 6 — `Schedule` integration

#### Step 11 — Frame-start tick bump + `set_change_ticks` wiring (Round 2 C1)

**Files:**
- Modified: `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs`

**Content (Round 2 C1):**
- `Schedule::run` starts with `world.change_tick.fetch_add(1, Relaxed)`.
- Compute `this_run = prev + 1`.
- In `try_dispatch_ready`, before each `scope.spawn`:
  ```rust
  let prev_this_run = self.systems[i].meta().this_run;
  self.systems[i].set_change_ticks(prev_this_run, this_run);
  ```
- **NO apply-window write** (Round 2 C1 consolidation — folded the Round 1 PHASE9.3 step into PHASE9.2).

**Dependency:** Step 3 (SystemMeta + trait method) + Step 2 (EcsMaster::change_tick).
**Deliverable:** `this_run` and `last_run` flow correctly via `set_change_ticks`.

#### ~~Step 12~~ — Apply-window last_run update **(REMOVED Round 2 C1)**

**Round 2 C1 deletion:** This step is no longer needed. `set_change_ticks` at dispatch time handles both ticks in one call. Apply-window tick state is untouched.

**Renumbering:** former Step 13 → 12, Step 14 → 13, Step 15 → 14, Step 16 → 15.

### Wave 7 — Wraparound + `check_ticks`

#### Step 12 (was 13) — `run_check_ticks_scan` (Round 2 W3: live rows; Round 3 W-NEW-1 + W-NEW-1b: APIs corrected)

**Files:**
- New: `crates/boyko_ecs/src/ecs/core/change_detection/check_ticks.rs`
- **Modified (Round 3 W-NEW-1b)**: `crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs` — add `pub fn iter_archetypes_mut(&mut self) -> ArchetypeBundleIterMut<'_>` one-liner mirror of the existing `iter_archetypes()` at `archetype_master.rs:590`. Body: `self.archetypes.iter_mut()` (delegates to `ArchetypeBundle::iter_mut()` at `archetype_bundle.rs:680`).

**Content (Round 2 W3 + Round 3 W-NEW-1):**
- `run_check_ticks_scan(world: &mut EcsMaster, current: Tick)` — walks every archetype's tick columns over `0..pool.count()` (NOT full buffer; uses the existing `ComponentPool::count(&self)` accessor at `component_pool.rs:620`).
- Iteration via `world.archetype_master.iter_archetypes_mut()` (the new one-liner added above).

**Dependency:** Steps 5, 6.

#### Step 13 (was 14) — `Schedule::run` wraparound wiring

**Files:**
- Modified: `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs`

**Content:**
- After frame-start tick bump (Step 11), insert wraparound check:
  ```rust
  if this_run.0.wrapping_sub(world.last_check_tick.0) >= CHECK_TICK_THRESHOLD {
      run_check_ticks_scan(world, this_run);
      // Clamp every system's last_run via set_change_ticks:
      for sb in self.systems.iter_mut() {
          let mut last_run = sb.meta().last_run;
          let this_run_existing = sb.meta().this_run;
          last_run.check_tick(this_run);
          sb.set_change_ticks(last_run, this_run_existing);
      }
      world.last_check_tick = this_run;
  }
  ```

**Dependency:** Step 12.

### Wave 8 — Polish + Validation

#### Step 14 (was 15) — System initialisation tick setup (Round 2 W5 simplified)

**Files:**
- Modified: `crates/boyko_ecs/src/ecs/core/system/function_system.rs`
- Modified: `crates/boyko_ecs/src/ecs/core/system/exclusive_function_system.rs`

**Content (Round 2 W5 — moved init to constructor):**
- `FunctionSystem::new` reads `world.current_tick()` and passes to `SystemMeta::new(name, current_tick)`. Constructor sets `last_run` correctly.
- `FunctionSystem::initialize` no longer touches `last_run` (Round 2 W5 simplification).
- Implement `set_change_ticks` trait method (Round 2 C1).

**Dependency:** Steps 1, 3.

#### Step 15 (was 16) — End-to-end integration tests + benches + docs

**Files:**
- New tests under `crates/boyko_ecs/tests/phase10_change_detection.rs`.
- New benches under `crates/boyko_ecs/benches/change_detection.rs`.
- Doc additions to `docs/SYSTEMS.md` and `docs/FEATURE_MAP.md`.

**Content:**
- Every test in §13.2 (including the Round 2 W6 `par_iter_inline_path_reports_changed` and Round 2 C4 `or_with_changed_archetype_lacking_c_no_panic`).
- Every bench in §13.5 (including Round 2 W8 `bench_or_added_changed_archetype_count_dominated`).
- Round 2 O3 inlining verification: `cargo asm` snapshot for `bench_changed_filter_1024_rows`.

**Dependency:** All previous Steps.

### Parallelisation summary

| Wave | Steps | Parallelism |
|------|-------|-------------|
| 1 | 1, 2 | Sequential |
| 2 | 3, 4 | Steps 3 and 4 can run in parallel |
| 3 | 5, 6 | Sequential |
| 4 | 7, 8 | Sequential |
| 5 | 9, 10 | Sequential |
| 6 | 11 | (Round 2 C1 simplified — was 2 steps) |
| 7 | 12, 13 | Sequential |
| 8 | 14, 15 | Sequential |

Step count: **15 Steps** (was 16, after Round 2 C1 consolidation).

With one developer: ~15 sequential Steps = 3-4 weeks.

---

## §15 — Migration impact

### 15.1 Files modified outside the change-detection module (Round 2 expanded; Round 3 W-NEW-1b)

| File | Change | Impact |
|------|--------|--------|
| `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` | +2 fields, +1 method, modify `create_entity` to read tick (Round 2 W4) | Step 2. |
| `crates/boyko_ecs/src/ecs/core/system/system_meta.rs` | +2 fields, change constructor signature (Round 2 W5), field renaming Round 2 O5 | Step 3. **All callsites of `SystemMeta::new` must pass `current_tick`.** |
| **`crates/boyko_ecs/src/ecs/core/system/system.rs`** (Round 2 C1) | Add `fn set_change_ticks(&mut self, last_run: Tick, this_run: Tick)` method to `System` trait | Step 3. **Every System impl must declare it.** |
| `crates/boyko_ecs/src/ecs/core/system/function_system.rs` | Implement `set_change_ticks`; pass `current_tick` to `SystemMeta::new`; remove `last_run` init from `initialize` (now constructor-handled) | Step 14. |
| `crates/boyko_ecs/src/ecs/core/system/exclusive_function_system.rs` | Same | Step 14. |
| `crates/boyko_ecs/src/ecs/memory/component_pool.rs` | +2 fields, tick init in `new`/swap/pop. Existing `pub fn count(&self) -> usize` at line 620 unchanged — Round 3 W-NEW-1 confirmed the actual API name. | Step 5. |
| `crates/boyko_ecs/src/ecs/core/archetype/archetype.rs` | New `tick_column_base`, `create_entity` signature change | Step 6. |
| **`crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs`** (Round 3 W-NEW-1b) | Add `pub fn iter_archetypes_mut(&mut self) -> ArchetypeBundleIterMut<'_>` one-liner mirror of `iter_archetypes()` at line 590. Required by Step 12's `run_check_ticks_scan`. | Step 12. |
| `crates/boyko_ecs/src/ecs/core/iters/query/filter.rs` | `set_table_*` gains `meta: &'_ SystemMeta` (Round 2 W7); new `Added`/`Changed` impls | Step 8. |
| `crates/boyko_ecs/src/ecs/core/iters/query/data.rs` | Same shape | Step 8/10. |
| `crates/boyko_ecs/src/ecs/core/iters/query/iter.rs` | **Add `meta: &'s SystemMeta` field to `QueryIter`/`QueryIterMut` (Round 2 C2); update constructors** | Step 8. |
| `crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs` | Add `meta` to `ParQuery`/`ParQueryMut`/`ChunkCaptures`; update `run_chunk_inline`/`run_chunk_owned`/PAR7 (Round 2 W6) | Step 8. |
| `crates/boyko_ecs/src/ecs/core/iters/query/query.rs` | Forward `self.meta` to `iter`/`iter_mut`/`par_iter`/`par_iter_mut` constructors | Step 8. |
| `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs` | Frame tick bump, `set_change_ticks` calls, wraparound | Steps 11, 13. |
| **Phase 8.5 `Commands::spawn` apply path** (Round 2 W4) | **No change to the apply path itself.** `EcsMaster::create_entity` now reads `self.change_tick.load(Relaxed)` internally. | Step 6 follow-on. |

### 15.2 Existing tests / benches that need updating (Round 2 C2 expanded)

**Round 2 C2 — known broken by the `QueryIter::new` signature change:**
- `tests/query_dsl_smoke.rs` — any test that constructs `QueryIter::new(state, world)` directly. Must add `meta` argument.
- `tests/miri_phase8a.rs` — same.
- `tests/system_param_smoke.rs` — same.
- `benches/query_dsl.rs` — same.
- Other places using `QueryIter::new` directly (grep for `QueryIter::new` and `QueryIterMut::new` before Step 8).

**Round 2 W5 — known broken by `SystemMeta::new` signature change:**
- Any test that constructs `SystemMeta::new(name)` directly — must pass `current_tick`.
- `NoopSystem` (test stub `system.rs:131-167`) — must implement `set_change_ticks` and pass `Tick::new(1)` (or similar test default) to `SystemMeta::new`.

**
Helper:** Add `SystemMeta::for_testing(name) -> Self` in `system_meta.rs` returning `SystemMeta::new(name, Tick::new(1))` for test convenience.

**Other migrations:**
- Any test that invokes `Archetype::create_entity` directly — must now pass `current_tick`. The common pattern via `EcsMaster::create_entity` is shielded (reads internally).
- Any test that constructs `ComponentPool::new` directly — tick buffers auto-initialised.

### 15.3 Backward compatibility

- **Existing `&T`, `&mut T`, `With<C>`, `Without<C>`, `()` filters**: ABI/API unchanged from user POV. Internal trait signature gains `meta`; archetypal impls ignore.
- **Existing queries**: zero overhead. Const-fold path elides everything.
- **`Schedule::run` semantics**: unchanged for users; tick bump is internal.
- **Phase 9 dispatcher latency**: +0.28% (§8.5 Round 2 C1 simplification).

### 15.4 ABI break: `QueryFilter::set_table_*` and `QueryData::set_table_*` signature change

`pub unsafe trait`. External implementors (if any) would break. As of Phase 9, no external impls exist. Announced in §0 changelog.

### 15.5 ABI break: `System::set_change_ticks` (Round 2 C1)

New trait method with no default body. External `System` implementors (if any) would break. As of Phase 8c, the only `System` impls are `FunctionSystem`, `ExclusiveFunctionSystem`, and `NoopSystem` (test). All updated in Step 3 + 14.

### 15.6 ABI break: `SystemMeta::new` signature (Round 2 W5)

`SystemMeta::new(name)` → `SystemMeta::new(name, current_tick)`. All callsites updated in Step 3.

### 15.7 New public API: `ArchetypeMaster::iter_archetypes_mut` (Round 3 W-NEW-1b)

Additive (no break). One-liner mirror of the existing `iter_archetypes()`. Required by `run_check_ticks_scan` for in-place mutation of stored ticks.

---

## §16 — Rejected alternatives

(Body unchanged from Round 1 — see Round 1 plan §16.1-§16.14.)

---

## §17 — Open questions

(Round 1 OQ-1 through OQ-11 unchanged — see Round 1 plan §17.)

### OQ-12 — Per-archetype access narrowing for `Or<F>` (Round 2 C4 + W8)

**Problem:** `Or<(_, Changed<C>)>` declares read of C globally in `init_access`. The scheduler serialises this system against any concurrent writer of C, even when the matched archetype set lacks C.

**Trade-off:** Per-archetype access narrowing would require:
- Re-running `init_access` per archetype, or
- Maintaining a per-archetype access mask alongside the per-system mask.

Both add per-archetype machinery to the scheduler. Bevy mirrors the current conservative approach.

**Decision (Round 2):** ship the conservative behaviour in v1. Revisit if profiling shows real contention from this pattern.

---

## §18 — Plan readiness checklist (Round 3 updated)

- [x] Goal stated in perf + functional terms (§1.1, §1.2).
- [x] Target metrics concrete (§1.2 table — Round 2 W1 corrected scales).
- [x] Every architectural decision justified (§3 Q1–Q12).
- [x] Every alternative rejected with reasoning (§16).
- [x] Trade-offs honestly listed.
- [x] Data-structure fields documented (§4, §5, §6, §7, §11).
- [x] `#[repr(...)]` specified.
- [x] Hot/cold split applied (Tick hot; `check_ticks` `#[cold]`).
- [x] Struct sizes known and justified (§11.1 Round 2 O2/O5 updated).
- [x] Padding for false sharing (Round 2 O2: drop `CachePadded` from `change_tick`; §11.5 audit).
- [x] Public API minimal (§12).
- [x] No internal types in signatures.
- [x] Lifetimes explicit (`'w` on Fetch; `'s` on `&'s SystemMeta`; Round 2 W7 `meta: &'_ SystemMeta`).
- [x] No `dyn Trait` in hot path.
- [x] Generics where specialization needed.
- [x] Multi-threading model explicit (§8: 1 atomic/frame; per-row writes ride conflict graph).
- [x] Atomic memory orderings explicit (§8.1 table — Round 2 O2 updated).
- [x] Synchronization points justified.
- [x] Data partitioning described (par_iter chunks disjoint).
- [x] `Send`/`Sync` consistent (§2.9 + Phase 9 SEND10 reuse).
- [x] Edge cases enumerated (wraparound; null tick base; first-run; archetype migration).
- [x] Generation/version checks (`Tick::check_tick`; archetype_generation unchanged).
- [x] Drop order discussed (no new Drop impls; Tick is Copy).
- [x] `unsafe` block invariants stated (every SAFETY block names SCH3 / STORE3 / MUT3).
- [x] Affected modules listed (§15.1 — Round 2 expanded; Round 3 added `archetype_master.rs`).
- [x] Changes in existing APIs noted (§15.4-15.7 — Round 2 added two new ABI breaks; Round 3 added §15.7 additive API).
- [x] Compatibility with Phase 7/8b/9 verified.
- [x] Implementation plan broken into steps (§14: 15 numbered Steps after Round 2 C1 consolidation).
- [x] Mandatory unit tests specified (§13.1 — Round 2 O1/O4/W5/C1 tests added; Round 3 C-NEW-1 `tick_is_newer_than_self_equal_this_run`).
- [x] Property-based tests specified (§13.4 — Round 2 W2 test added).
- [x] Criterion benchmarks specified (§13.5 — Round 2 W8/C4 benches added).
- [x] `debug_assert!` invariants specified (§13.6 — Round 2 W5 simplified).
- [x] **(Round 2 C1)** `System` trait extension documented (§2.6 SCT4, §5.4-bis, §15.5).
- [x] **(Round 2 C2)** `QueryIter` / `QueryIterMut` field extension documented (§5.3, §11.1, §15.2).
- [x] **(Round 2 C3)** Cross-thread `UnsafeCell<u32>` write soundness cited (§11.5 Rustonomicon).
- [x] **(Round 2 C4)** `Or<F>` null-base cost documented in §10 + conservative access declaration documented in §2.3 FLT2.
- [x] **(Round 2 W1)** Per-archetype storage upper bound recomputed (§10.7, §11.6).
- [x] **(Round 2 W2)** `MAX_CHANGE_AGE` first-principles derivation (§9.3).
- [x] **(Round 2 W3)** `check_ticks` scan walks live rows (§9.6, §10.6).
- [x] **(Round 2 W4)** `Commands::spawn` tick threading via `EcsMaster::create_entity` internal read (§2.4 INIT3).
- [x] **(Round 2 W5)** `SystemMeta::new(name, current_tick)` constructor (§4.2-bis).
- [x] **(Round 2 W6)** `par_iter` `meta` plumbing through inline + spawn + PAR7 fallback (§8.4).
- [x] **(Round 2 W7)** `meta: &'_ SystemMeta` lifetime annotation + "input-only" doc (§5.3).
- [x] **(Round 2 W8)** `Or<(Added, Changed)>` archetype-count dominance documented (§10.5-bis).
- [x] **(Round 2 O1-O5)** All accepted (see §0 changelog).
- [x] **(Round 3 C-NEW-1)** `Tick::is_newer_than` formula corrected to Bevy's standard `ticks_since_system > ticks_since_insert`; §4.2 body, §6.2-bis proof, §10.3 lowering, §13.1 unit test all updated.
- [x] **(Round 3 W-NEW-1)** `pool.units_len()` → `pool.count()` across §1.2, §2.7 WRAP3, §3 Q3.4, §4.6, §9.6, §10.6, §13.5, §14 Step 12, §21.
- [x] **(Round 3 W-NEW-1b)** `archetype_master.archetype_ids()` / `archetypes_iter_mut()` → new `iter_archetypes_mut()` one-liner on `ArchetypeMaster`; §4.6, §9.6, §14 Step 12, §15.1, §15.7 updated.

---

## §19 — Decision biases honoured (from brief)

The brief explicitly listed four decision biases. Each is acknowledged below.

| Bias | Honoured? | Where |
|------|-----------|-------|
| Prefer storage option A or C | **A** chosen | §3 Q1, §4–§7 |
| Prefer per-system tick snapshot (Q5 option a) | **(a) chosen** | §3 Q5, §8 |
| Adopt Bevy `Changed` deref-bump semantics | **Adopted** | §3 Q6, §6.2 |
| `MAX_TICK_AGE` clamping during apply | **Adopted** | §3 Q3, §2.7, §9 (Round 2 W2 first-principles derivation) |

---

## §20 — Cross-references to source code (Round 3 updated)

For the developer who implements Phase 10, the following file:line references identify the insertion points.

| Concern | File | Line(s) | Operation |
|---------|------|---------|-----------|
| Add `change_tick` field | `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` | ~88 | Add `AtomicU32` (Round 2 O2) + `last_check_tick: Tick`. |
| Modify `create_entity` to read tick (Round 2 W4) | same file | ~244 | Read `self.change_tick.load(Relaxed)` and thread to `Archetype::create_entity`. |
| Add `this_run` / `last_run` (Round 2 O5) | `crates/boyko_ecs/src/ecs/core/system/system_meta.rs` | ~48 | Add two `Tick` fields. |
| Change `SystemMeta::new` signature (Round 2 W5) | same file | ~58 | `pub fn new(name: &'static str, current_tick: Tick) -> Self`. |
| **Add `System::set_change_ticks` (Round 2 C1)** | `crates/boyko_ecs/src/ecs/core/system/system.rs` | end of trait body, ~120 | `fn set_change_ticks(&mut self, last_run: Tick, this_run: Tick);` (no default body). |
| **Add `System::meta` getter (Round 2 C1 support)** | same file | similar | `fn meta(&self) -> &SystemMeta;` (read-only accessor; used by dispatcher to fetch prev `this_run`). |
| Frame-start tick bump | `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs` (Phase 9) | `Schedule::run`, after `reset_for_frame` | `fetch_add` + this_run capture. |
| Per-system `set_change_ticks` wiring | same file | `try_dispatch_ready`, line ~1509 | `self.systems[i].set_change_ticks(prev_this_run, this_run)` before `scope.spawn`. |
| **(Round 2 C1 — REMOVED)** ~~Apply-window `last_run` wiring~~ | ~~same file~~ | ~~~~ | **No longer needed; folded into dispatch-time `set_change_ticks`.** |
| `ComponentPool` tick buffers | `crates/boyko_ecs/src/ecs/memory/component_pool.rs` | `ComponentPool::new` | Initialise `added_ticks` / `changed_ticks`. |
| **(Round 3 W-NEW-1) `ComponentPool::count()` reference** | `crates/boyko_ecs/src/ecs/memory/component_pool.rs` | line 620 | Existing API; used by `run_check_ticks_scan` to bound the live-row iteration. **Not modified — referenced only.** |
| `Archetype::create_entity` tick init | `crates/boyko_ecs/src/ecs/core/archetype/archetype.rs` | `create_entity`, after `push_entity_components` | Write `current_tick` to tick slots. |
| `Archetype::tick_column_base` | same file | new method | Sparse map lookup. |
| **(Round 3 W-NEW-1b) `ArchetypeMaster::iter_archetypes_mut`** | `crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs` | after line 590 (`iter_archetypes`) | One-liner mirror; delegates to `self.archetypes.iter_mut()` (existing `ArchetypeBundle::iter_mut()` at `archetype_bundle.rs:680`). |
| Filter trait signature change | `crates/boyko_ecs/src/ecs/core/iters/query/filter.rs` | trait `QueryFilter` + every impl + 2 macros | Add `meta: &'_ SystemMeta` (Round 2 W7). |
| `Added` / `Changed` impls | same file | end of file | New impls. |
| Data trait signature change | `crates/boyko_ecs/src/ecs/core/iters/query/data.rs` | trait `QueryData` + every impl + 2 macros | Same. |
| `Ref` / `Mut` `QueryData` impls | same file | end of file | New impls. |
| **(Round 2 C2) `QueryIter` / `QueryIterMut` field extension** | `crates/boyko_ecs/src/ecs/core/iters/query/iter.rs` | struct defs `~82, ~258`, constructors `~127, ~297`, `next()` bodies `~210, ~349` | Add `meta: &'s SystemMeta` field + new constructor signature + forward in `next()`. |
| **(Round 2 W6) `ParQuery` / `ParQueryMut` field extension** | `crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs` | struct + `ChunkCaptures` + `run_chunk_inline` + `run_chunk_owned` + PAR7 fallback `~362` | Add `meta` to all 4 dispatch paths. |
| Query constructs with meta | `crates/boyko_ecs/src/ecs/core/iters/query/query.rs` | `iter` / `iter_mut` / `par_iter` / `par_iter_mut` constructors `~150, ~168` | Forward `self.meta`. |
| `FunctionSystem::new` reads world tick (Round 2 W5) | `crates/boyko_ecs/src/ecs/core/system/function_system.rs` | `new(...)` | Read `world.current_tick()`, pass to `SystemMeta::new`. |
| `FunctionSystem::initialize` cleanup (Round 2 W5) | same file | `initialize(...)` | Remove `last_run` init (now constructor-handled). |
| `FunctionSystem::set_change_ticks` impl (Round 2 C1) | same file | new method | Write both fields. |
| `ExclusiveFunctionSystem` parallel changes (Round 2 W5, C1) | `crates/boyko_ecs/src/ecs/core/system/exclusive_function_system.rs` | parallel | Same shape. |
| `check_ticks` integration | `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs` | After frame-start `fetch_add`, before dispatch loop | Conditional `run_check_ticks_scan` + per-system clamp via `set_change_ticks`. |
| **(Round 2 W4) `Commands::spawn` apply tick threading** | `crates/boyko_ecs/src/ecs/core/commands/...` (Phase 8.5) | Within `SpawnCommand::apply` | **No change** — `EcsMaster::create_entity` reads tick internally. |

---

## §21 — Plan summary (Round 3)

Phase 10 ships per-row change detection following Bevy's post-PR #6547 design: parallel `Box<[UnsafeCell<Tick>]>` columns for added and changed ticks, per-`Schedule::run` frame tick bump, per-system `last_run` and `this_run` updated at dispatch time via the new `System::set_change_ticks` trait method, `Mut<T>` deref-guard with `set_if_neq` / `bypass_change_detection` escape hatches, `Added<C>` / `Changed<C>` non-archetypal filters that compose with the existing `Or<F>` / tuple-AND machinery, and wraparound handling via `MAX_CHANGE_AGE` clamp + once-per-100-days `check_ticks` scan (per-frame bump regime).

Phase 9 integration adds **one** atomic per frame (`fetch_add` on `change_tick`); all per-row tick writes ride the Phase 9 conflict-graph exclusivity and use plain `UnsafeCell<Tick>` stores. Per-row filter cost ≤ 1-1.5 ns (including Round 2 C4 null-base check); storage cost 8 B × Σ pool.max_components — 40 MB at the AAA design point, scaling per §10.7.

**Round 3 cleanup resolutions:**
- **C-NEW-1**: `Tick::is_newer_than` formula corrected to Bevy's standard `ticks_since_system > ticks_since_insert` (both subtractions against `this_run`). The Round 2 form `age_this > age_self` (both subtractions against `last_run`) returned `false` for the inclusive upper-bound case `self == this_run`. §4.2 body, §6.2-bis proof, §10.3 x86 lowering, §13.1 new unit test `tick_is_newer_than_self_equal_this_run` updated.
- **W-NEW-1**: `pool.units_len()` (non-existent) → `pool.count()` (actual API at `component_pool.rs:620`). Replaced across §1.2, §2.7 WRAP3, §3 Q3.4, §4.6, §9.6, §10.6, §13.5, §14 Step 12, §21.
- **W-NEW-1b**: `archetype_master.archetype_ids()` / `archetypes_iter_mut()` (non-existent) → new `iter_archetypes_mut()` one-liner on `ArchetypeMaster` (additive API, mirror of existing `iter_archetypes()` at line 590, delegates to `ArchetypeBundle::iter_mut()` at `archetype_bundle.rs:680`). §4.6, §9.6, §14 Step 12, §15.1, §15.7, §20 updated.

**Round 2 critical resolutions (unchanged from Round 2):**
- **C1**: `System::set_change_ticks(last_run, this_run)` trait method — the single dispatcher→system channel for tick writes. No default body; every impl declares.
- **C2**: `QueryIter` / `QueryIterMut` carry `meta: &'s SystemMeta` field; constructors and callers updated.
- **C3**: Adjacent `UnsafeCell<u32>` writes from `par_iter` chunks on the same cache line are sound per the Rust abstract machine (distinct memory locations). MESI ping-pong is a perf cost, not UB. Cited Rustonomicon; new Miri test `miri_par_iter_chunks_write_adjacent_ticks_disjoint_no_ub`.
- **C4**: `Or<(_, Changed<C>)>` null-base check cost (~0.5 ns/row) documented in §10. Conservative access declaration mirrors Bevy.

**Round 2 important resolutions (unchanged from Round 2):**
- **W1**: storage bound recomputed for per-archetype duplication (40 MB at design point; 80 MB-1.6 GB at stress scales).
- **W2**: `MAX_CHANGE_AGE` derived from first principles for boyko's per-frame bump.
- **W3**: `check_ticks` scan walks live rows (`pool.count()` — Round 3 W-NEW-1 API correction), not buffer length.
- **W4**: `Commands::spawn` apply unchanged; `EcsMaster::create_entity` reads tick internally.
- **W5**: `SystemMeta::new(name, current_tick)` constructor initialises `last_run = current - MAX_CHANGE_AGE`.
- **W6**: `par_iter` `meta` plumbed through inline / spawn / PAR7 fallback paths.
- **W7**: `meta: &'_ SystemMeta` lifetime annotation; "input-only" documented.
- **W8**: `Or<(Added, Changed)>` archetype-scan dominance documented; new bench.

**Round 2 optional resolutions (all accepted, unchanged from Round 2):**
- **O1**: `is_changed` uses inclusive lower-bound (`>=` semantic via `last_run - 1` trick) — proof re-derived in §6.2-bis against the Round 3 corrected formula.
- **O2**: `CachePadded` removed from `change_tick`.
- **O3**: inlining verification via `cargo asm` snapshot.
- **O4**: `Tick: Default = Tick::ZERO`.
- **O5**: harmonised naming `last_run` / `this_run` (no `_tick` suffix) across all sites.

**Steps:** 15 (Round 2 C1 consolidated dispatch + apply windows; Round 3 adds the one-liner `iter_archetypes_mut` to Step 12 — no step-count change). ~3-4 weeks for one developer. One parallelisable pair (Step 3 || Step 4).

Mandatory tests: unit (incl. Round 3 C-NEW-1 inclusive-upper-bound regression) + Miri (per-row tick write soundness; cross-thread chunk write disjointness, Round 2 C3) + property (wraparound) + criterion (hot path + Round 2 W8 archetype-count regression) + integration (multi-frame lifecycle; parallel correctness; Round 2 W6 inline path; Round 2 C4 null-base path).

Open questions are non-blocking: opt-out for `NoChangeTracking` (OQ-1), per-system tick bump promotion (OQ-5), hybrid storage option D (OQ-4), deep value-compare filter (OQ-2), per-archetype access narrowing for `Or<F>` (OQ-12 new in Round 2). All deferred to follow-ups based on profile data.

---

**End of Phase 10 Architecture Plan — Round 3.**

**Awaiting architecture-critic Round 3 review. Expected disposition: APPROVED (Round 3 strictly resolved the two cleanup notes; no new architectural decisions introduced).**

---

Plan output target (orchestrator save): `D:\claude\BoykoEngine\docs\PHASE-10-CHANGE-DETECTION-PLAN.md`

Relevant source files (absolute paths) consulted during Round 3 revision:
- `D:\claude\BoykoEngine\docs\PHASE-10-CHANGE-DETECTION-PLAN.md` (Round 2 plan, 2254 lines)
- `D:\claude\BoykoEngine\docs\PHASE-9-CRITIC-ROUND-2.md` and Round 2 critic notes (in brief)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\memory\component_pool.rs` — line 620 `pub fn count(&self) -> usize` (verified; Round 3 W-NEW-1)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\archetype\archetype_master.rs` — line 590 `pub fn iter_archetypes(&self)` (verified; mutable variant absent; Round 3 W-NEW-1b adds `iter_archetypes_mut`)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\archetype\archetype_bundle.rs` — line 680 `pub fn iter_mut(&mut self) -> ArchetypeBundleIterMut<'_>` (verified; available as delegation target)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\archetype\archetype.rs` — `component_pools_mut()` at line 456, `component_ids()` at line 474 (verified; used by `run_check_ticks_scan` pseudocode)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\component\component_pool_bundle.rs` — `get_pool_mut` at line 82 (verified)

---

**File save instruction:** Save this complete document to `D:\claude\BoykoEngine\docs\PHASE-10-CHANGE-DETECTION-PLAN.md` (overwriting Round 2).

Plan length: approximately 2280 lines (Round 2 was 2254; Round 3 net +26 from §0 changelog + §4.2 corrected `is_newer_than` doc/body + §6.2-bis re-proof + §10.3 lowering update + §13.1 new unit test row + §14 Step 12 API note + §15.1 + §15.7 + §18 checklist additions + §20 + §21 changelog and footer).