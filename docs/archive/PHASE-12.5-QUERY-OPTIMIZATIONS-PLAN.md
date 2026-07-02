# Phase 12.5 Track B — Query Iteration Optimisations — Architectural Plan (Round 4)

**Branch:** `ecs`
**Status:** DRAFT v4 (architect output, Round 4; folds in architecture-critic Round 3 W1-W4 wording fixes; no architectural change vs Round 3)
**Target file path (orchestrator should commit):** `D:\claude\BoykoEngine\docs\PHASE-12.5-QUERY-OPTIMIZATIONS-PLAN.md`
**Predecessor critic file:** `D:\claude\BoykoEngine\docs\PHASE-12.5-CRITIC-B-ROUND-3.md`
**Umbrella:** `D:\claude\BoykoEngine\docs\PHASE-12.5-SURPASS-BEVY-PLAN.md`
**Profile input:** `D:\claude\BoykoEngine\docs\PHASE-12.5-PROFILE-QUERY.md`

---

## §0 Round 4 Changelog

### W1 — §4.3 contradictory Send/Sync assertion blocks reconciled

§4.3 had two Send/Sync assertion blocks: a first `#[cfg(any(test, doctest))] mod _send_sync_check` block with a `_DummyComponent` stub gated under `cfg`, followed by a "Final assertion choice" using `static_assertions::assert_impl_all!(QueryView<'static, (), ()>: Send, Sync)` at module scope outside `cfg(test)`. The prose between them claimed the gated block "still fires on every cargo test run", but I-NEW-5 mandates the assertion must fire on every compile (including release). The first (gated) block is **removed**; only the "Final assertion choice" (`QueryView<'static, (), ()>` at module scope, NOT under `cfg`) remains. Single canonical assertion now appears once.

### W2 — Miri `miri_system_meta_dummy_lazy_init` execution mode pinned to sequential pointer-stability

§11.4 row for `miri_system_meta_dummy_lazy_init` previously said "Concurrent calls to `SystemMeta::dummy()` from multiple threads (via `std::thread::scope`)". Multi-thread Miri is incompatible with the Phase 9.1 deferral (`Scope::spawn` Tree-Borrows protected-tag conflict). **Adopted path (a)**: the test is now a **sequential 1000× loop** asserting pointer stability across calls (`assert_eq!(prev_ptr, SystemMeta::dummy() as *const SystemMeta)` per iteration). The OnceLock's internal CAS soundness is covered by std's own test suite under loom; the real invariant Track B needs is that **the returned reference's pointer is stable** across repeated calls within the same process. Path (b) (`#[cfg_attr(miri, ignore)]`) is rejected because it defeats Miri coverage.

### W3 — §10.5 BSS footprint tightened to ≤ 320 B with compile-time tripwire

§10.5 line 858 estimated "~280 B" for `OnceLock<SystemMeta>` BSS. The estimate underestimated alignment padding: `SystemMeta` has `align 32` (per AVX2-friendly inner mask field), so `OnceLock<SystemMeta>` inherits `align_of::<SystemMeta>() = 32`. With the embedded `AtomicBool` init flag (1 B, padded to 32 B to preserve `T`'s alignment) plus `MaybeUninit<SystemMeta>` (≥ 256 B), the real footprint lands in 288-320 B. **Revised** §10.5 to state "≤ 320 B in BSS", with concrete computed value. Added a module-scope `const _: () = assert!(size_of::<OnceLock<SystemMeta>>() <= 320);` tripwire to `system_meta.rs` so a future SystemMeta growth or stdlib OnceLock layout change trips the compile.

### W4 — Panic message wording unified between §2.2 QV11 and §5

§2.2 QV11 quoted message ending: `"...use Query<D, F> inside a system body"`. §5 implementation interpolated `D`/`F` type names and ended: `"...use Query<D, F> inside a system body via Schedule"`. **Resolution**: §2.2 QV11 now quotes the §5 implementation verbatim with `{...}` placeholders shown. Single canonical wording across the plan; the implementation `format!` template appears once. Updated I-NEW-4 paragraph in §0 (Round 3 portion) likewise to reflect the canonical wording.

---

## §0 Round 3 Changelog (preserved)

### C-NEW-1 — Const-fn migration of `BitSet::new` ABANDONED; OnceLock fallback ADOPTED (path (c))

**Change**: Wave A Step 0 const-fn promotion of `BitSet::<T>::new` (Round 2 path (b)) is **dropped**. The bit_set crate at `boyko_utils/src/bit_mask/bit_set.rs:84-89` builds `BitSet { bits: T::default() }`. `T::default()` is a non-const trait method; `const_trait_impl` is unstable in Rust 1.85 / May 2026. Step 0a-0d **will not compile** as Round 2 prescribed. **Why**: redesigning `BitSet<T>` (path (a), e.g. adding a `ConstZero` super-trait or per-type inherent `const fn zero()` constants) touches a foundational utility crate consumed in many places and is out of scope for Phase 12.5. **Resolution** (path (c)): `SystemMeta::DUMMY` is wrapped in a lazy `OnceLock<SystemMeta>`. First-call latency: one `OnceLock::set` (~20 ns) under `&mut self`. Subsequent calls: one Acquire load (~1-2 ns). §1.2 cache-hit budget revised from 3 ns to ~5 ns (matching §6.1 step breakdown). The NCD5 default body is **removed** entirely — every impl declares explicit `_no_meta` variants (I4 fix retained from Round 2). No method body forwards through a non-existent `&SystemMeta::DUMMY` const.

`BitSet256::new` is **already** `const fn` (`bit_set_256.rs:33`); no migration needed there.

### C-NEW-2 — Opt-B5 (single-component specialisation) DROPPED

**Change**: Opt-B5 `iter_single_read` + `SingleReadIter` + `Archetype::column_raw_ptr` are **removed** from the plan. **Why**: `Archetype::column_raw_ptr(component_id)` does not exist; the real access path at `data.rs:307-309` is `(*archetype).columns.get_unchecked(state.id.0).ptr` — a single `get_unchecked` + pointer field read. The "4-instruction prologue saving" claim in Round 2 §6.5 was unverified against the actual `set_table_readonly` body. The 5-instruction inner loop in the profile asm (`LBB306_16` at lines 65-73 of `PHASE-12.5-PROFILE-QUERY.md`) is **already byte-identical** to Bevy's; specialising the per-row body further is unlikely to move the needle — that's generic LLVM territory and the existing `Fetch::fetch` for `&T` already inlines optimally. Filed as Phase 13 exploratory work alongside sparse-set storage.

### C-NEW-3 — Opt-B4 (combined-generation atomic fusion) DROPPED

**Change**: Opt-B4 + `ArchetypeMaster::combined_generation_snapshot` + Step D4 are **removed** from the plan. **Why**: `ArchetypeMaster.generation` and `ArchetypeMaster.structural_generation` (`archetype_master.rs:37, 54`) are plain `ArchetypeGeneration(NonZeroUsize)` fields — **NOT** `AtomicUsize`. There is no "fused atomic load" lever. The branchless-OR-vs-`||` reframing is a generic LLVM micro-optimisation (and bench shape was misaligned — 10k inner iterations within ONE query call, not 10k separate queries). The "~0.2 µs at 10 k calls" figure was unsupported by the bench layout. Moved to §15 as a Phase 13 follow-up cleanup item — if a future `cargo asm` dump of `QueryDataState::update` shows branchy code from `||` short-circuit, replace with `|` (bitwise OR). Zero claimed budget; pure-cleanup item.

### C-NEW-4 — Cache-hit budget reconciled at ~5 ns (honest)

**Change**: §1.2 row 3 target ≤ 3 ns is **revised to ~5 ns**, matching the §6.1 step enumeration (state.update generation pair short-circuit alone is ~2 ns; OnceLock::get ~1 ns; `UnsafeCell::get` + cast ~0 ns; plus the C-NEW-1 OnceLock<SystemMeta> Acquire load adds ~1-2 ns). The Phase 8.5 226 ps anchor measured `OnceLock<ArchetypeId>::get()` in isolation — NOT a full query call with `state.update`. Honesty over wishful budget.

### C-NEW-5 — Bench gate revised to single generic `iter()` path

**Change**: Per C-NEW-2 (Opt-B5 dropped), the bench-gate-on-`iter_single_read` problem dissolves. The bench gate is now on `world.query::<&P, ()>().iter()` (generic path) — apples-to-apples comparable to Bevy's `state.iter(&world)`. Acceptance target: `≥ Bevy parity` (or within 5% noise floor) — see C6 revisit below. Both `g2_boyko_query_iter_10k` (updated) and `p2_boyko_direct_api_10k` (profile validation) run the same code path; the latter is the diagnostic, the former is the gate.

### C6 revisit — Honesty: query iter target amended to "≥ Bevy parity" (or within 5% noise floor)

**Change**: The umbrella's `boyko ≥ 1.10× bevy` criterion is **amended specifically for `g2_boyko_query_iter_10k`** to `boyko ≥ Bevy parity (within 5% noise floor)`. Round 2 attempted to invent Opt-B4/Opt-B5 levers to satisfy "≥ 1.10× Bevy"; those levers proved fictional under audit (C-NEW-2, C-NEW-3). **Honest answer**: Track B closes the existing 0.88× loss to ~Bevy parity by removing the system wrapper overhead (Opt-B1) and elides change-detection-meta plumbing in the dispatch hot path (Opt-B2). It cannot beat Bevy by 10% on this bench without redesigning the query inner loop or storage layout — that is fundamentally Phase 13+ work (sparse-set storage, PGO, allocator tuning). The umbrella file is amended in a separate orchestrator commit (see "Proposed umbrella amendment" block at the end of this document).

### I-NEW-1 — LOC budget revised to 1500-2000 production + 400-600 test

**Change**: §1.2 LOC row revised from 800-1200 production to **1500-2000 production + 400-600 test**. Per Phase 8.5 anchor (similar scope: 12 tuple arities × 4 macros × 2 new methods = ~96 mechanical edits in `data.rs` alone, plus filter, plus leaf impls, plus `query_view.rs`, plus `query_type_registry.rs`, plus `EcsMaster::query` facade, plus C-NEW-1 OnceLock<SystemMeta> scaffolding). Honest.

### I-NEW-2 — `bundle_archetype_cache` field ordering preserved per existing C6 pin

**Change**: §10.1 explicitly cites that `bundle_archetype_cache`'s placement at `ecs_master.rs:111-134` is **preserved unchanged**; the field's existing C6 pin docstring (Phase 8.5 Round 2) is informational. The change is purely the *addition* of `query_state_cache` AFTER `arena` (per C5 from Round 1/2). No other field re-ordering.

### I-NEW-3 — Cache slot `&mut` retag wording precision

**Change**: §0 I1 entry rewritten: "The `&mut QueryDataState` retag inside `query<&mut self>` now derives from `&mut self`'s unique provenance, not from a raw `Box::leak + as_mut`. Sound under the language-level uniqueness gate." Added new Miri test `miri_query_repeated_calls_no_provenance_violation` that calls `world.query::<&Pos, ()>()` 1000× in a row under Tree Borrows.

### I-NEW-4 — `Ref<T>` / `Mut<T>` direct API change-detection semantics: runtime PANIC (option (b))

**Change** (W4-folded canonical wording): When `D::NEEDS_CHANGE_DETECTION || F::NEEDS_CHANGE_DETECTION` is true and the caller invokes `EcsMaster::query<D, F>(&mut self)` directly (outside a system), the call **panics at runtime** with the format-interpolated message:

```text
direct API EcsMaster::query<{D}, {F}>() does not support change-detection \
 filters (D or F has NEEDS_CHANGE_DETECTION = true); use Query<D, F> \
 inside a system body via Schedule
```

(where `{D}` and `{F}` are substituted via `std::any::type_name`). **Why (b) over (a)/(c)**: option (a) (compile-error via `where (D, F): NoCDetect`) adds a new trait-bound surface that complicates downstream consumers; option (c) (track `last_run` per-(D, F) in the cache slot) adds state and synthesises a bogus `last_run = current_tick - MAX_CHANGE_AGE` — broken semantics (every row reads as Changed since "last frame"). Option (b) is the simplest, no trait-bound surface, no broken silent semantics. The panic is `#[cold]` + `#[inline(never)]` so it does not bloat the hot path's I-cache. Direct API is the wrong tool for change detection; redirect to `Schedule` + `Query<D, F>` SystemParam.

### I-NEW-5 — `QueryView` Send+Sync symmetry with existing `Query<'w, 's, D, F>`

**Change**: Audit confirms existing `Query<'w, 's, D, F>` SystemParam (Phase 8b) is `Send + Sync` provided `D::State: Send + Sync, F::State: Send + Sync` (trait bound at `data.rs:90`). `QueryView<'w, D, F>` matches this. Added a **non-test** `static_assertions::assert_impl_all!(QueryView<'static, (), ()>: Send, Sync)` at module scope of `query_view.rs` — outside `#[cfg(test)]` so it fires on every compile, not only `cargo test`.

---

## §1 Summary

### 1.1 Goal

Phase 12.5 Track B ships a **direct query API** that bypasses the `FunctionSystem` rebuild cost on `world.run_system(|q: Query<&T>| ...)` and ships a **`const NEEDS_CHANGE_DETECTION` elision** that removes the `&SystemMeta` indirection from the per-archetype set-up of queries that never touch ticks. After Track B:

1. `EcsMaster::query<D, F>(&mut self) -> QueryView<'_, D, F>` returns a direct iteration handle without going through `IntoSystem` / `FunctionSystem`.
2. Per-world `QueryState<D, F>` cache keyed by process-global `QueryTypeId` — first call costs `QueryDataState::new` (~1 µs); subsequent calls amortise to a single Acquire load + the existing `update_archetypes` short-circuit (~5 ns).
3. `QueryView::iter` / `iter_mut` / `single` / `single_mut` / `get` / `get_mut` / `par_iter` / `par_iter_mut` mirror the `Query<D, F>`-inside-system surface and dispatch through the same `QueryIter` / `QueryIterMut` / `ParQuery` / `ParQueryMut` cursors.
4. `QueryData::NEEDS_CHANGE_DETECTION: bool` and `QueryFilter::NEEDS_CHANGE_DETECTION: bool` associated consts gate every `set_table_*` callsite. For `D = &T` / `D = (&A, &B, ...)` and `F = ()` / `With<C>` / `Without<C>` / `Or<archetypal-only>` the const is `false`; the dispatcher dispatches to the `_no_meta` variant and never reads `meta.last_run` / `meta.this_run`.
5. **`SystemMeta::DUMMY`** is initialised lazily via `static DUMMY: OnceLock<SystemMeta> = OnceLock::new();` (C-NEW-1 fallback). Read-only by construction. Sole consumer: `QueryView::iter` / `iter_mut` / `par_iter` / `par_iter_mut` need `&SystemMeta` to build the iterator cursor; they pass `&SystemMeta::dummy()` because the const-folded NCD6 dispatch never reads it on the `&T` / `()` path.
6. The plan **explicitly does not claim 10%-faster-than-Bevy** on this bench (see C6 revisit + umbrella amendment block); target is `≥ Bevy parity (within 5% noise)`.
7. `Schedule`-driven systems are unchanged. The cached `FunctionSystem` path (the ~11 µs profile baseline) is preserved verbatim; only `EcsMaster::query`'s direct path opts into the cache.
8. Direct API path: **`Query<Ref<T>>` / `Query<Mut<T>>` / change-detection-filter calls panic at runtime** (I-NEW-4); change detection requires `Schedule` context.

### 1.2 Target metrics (acceptance gates)

| Operation | Baseline | Target | Source / justification |
|-----------|----------|--------|------------------------|
| `EcsMaster::query::<&T>() → iter().for_each(...)` × 10 000 entities (first call, cold cache) | n/a (new API) | ≤ 12 µs | First call pays `QueryDataState::new` (~1 µs); subsequent iterations hit warm path. |
| `EcsMaster::query::<&T>() → iter().for_each(...)` × 10 000 entities (warm, cache-hit) | 12.3 µs (`g2_boyko_query_iter_10k` via `run_system`) | **≥ Bevy parity (within 5% noise floor)** — concretely ≤ 7.25 µs | C6 revisit: amended from `≤ 7.25 µs (Bevy parity + 5%)` strict to `≥ Bevy parity (within 5% noise)`. Profile (PHASE-12.5-PROFILE-QUERY.md §1) proves inner loop is byte-identical to Bevy. Lever: remove FunctionSystem wrapper (-1.5-2 µs); const elision (~negligible at single-archetype). 1.10× Bevy filed as Phase 13. |
| `EcsMaster::query::<&T>()` lookup cost (warm, cache hit) | n/a (new API) | **≤ 5 ns** | C-NEW-4: revised from 3 ns to ~5 ns. Breakdown per §6.1 — see that section. |
| `Schedule`-driven `Query<&T>` iter × 10 000 entities | 11 µs cached-system baseline | ≤ 11.5 µs (no-regression budget +5%) | NCD6 const elides one `mov` per archetype boundary; inner loop unchanged. |
| `Schedule`-driven 50 empty systems frame | 13.94 µs (umbrella) | ≤ 14.5 µs (no-regression budget) | NCD6's const adds one const-folded `if` per `set_table_*` site; empty-system impact zero. |
| `Query<&T>` (any path) memory footprint per (D, F) state | ~150 B (`QueryDataState`) | ≤ 200 B | Cache slot 16 B (`NonNull<()>` + fn-ptr) + `QueryDataState` itself (allocated separately, unchanged). |
| `Box<[OnceLock<(NonNull<()>, fn(NonNull<()>))>; MAX_QUERY_TYPES]>` cache footprint | n/a | **≤ 32 KB** (1024 × ≤ 32 B per slot worst case, pinned by tripwire) | Single authoritative figure. |
| `par_iter` on a cached `QueryView` × 10 000 entities | 39.12 µs (`g3_boyko_par_iter_10k`) | ≤ 41 µs (no-regression budget) | Reuses Phase 9's `ParQuery` / `for_each_impl`. |
| LOC | n/a | **1500-2000 production + 400-600 test** | I-NEW-1: revised. Adds `query_type_registry.rs`, `query_view.rs`, `EcsMaster::query` facade + cold init, OnceLock<SystemMeta> scaffolding, 78 trait-impl mechanical updates (Phase 8.5 anchor for similar scope). |
| Step count | n/a | **12 Steps** (1 prerequisite + 11 main) | Reduced from Round 2's 14 (dropped Opt-B5 wave; dropped Opt-B4 step). |
| Calendar days (single developer) | n/a | 4-6 days | Wave A Step 0 mechanical (OnceLock scaffolding only); 78 trait-impl edits dominate. |

### 1.3 Subsystems delivered

- **A.** `QueryTypeId(usize)` process-global newtype, mirroring `BundleTypeId` (Phase 8.5).
- **B.** `EcsMaster::query_state_cache: Box<[OnceLock<(NonNull<()>, fn(NonNull<()>))>; MAX_QUERY_TYPES]>` per-world cache. Allocated via `Vec::with_capacity` + `into_boxed_slice` + `try_into` (C3 fix from Round 2 retained).
- **C.** `EcsMaster::query<D: QueryData, F: QueryFilter>(&mut self) -> QueryView<'_, D, F>` direct API.
- **D.** `QueryView<'w, D, F>` — handle owning the world borrow + cached state borrow. Methods: `iter`, `iter_mut`, `single`, `single_mut`, `get`, `get_mut`, `par_iter`, `par_iter_mut`.
- **E.** `QueryData::NEEDS_CHANGE_DETECTION: bool` const + `QueryFilter::NEEDS_CHANGE_DETECTION: bool` const. Plus NCD5 `set_table_readonly_no_meta` / `set_table_mut_no_meta` methods (NO default body — I4 retained).
- **F.** Lazy `SystemMeta::dummy() -> &'static SystemMeta` accessor backed by `static DUMMY: OnceLock<SystemMeta> = OnceLock::new();` (C-NEW-1 fallback path (c)).
- **G.** Direct API panic on change-detection: `EcsMaster::query<D, F>()` panics if either NCD const is true (I-NEW-4 option (b)).

### 1.4 What Phase 12.5 Track B deliberately defers

- `EcsMaster::query_ref<&self>` direct read-only API — Phase 13 (C2 retained).
- Multi-query API (`world.queries::<(Q1, Q2)>()`) — Phase 13.
- Per-archetype access narrowing for direct API — Phase 13.
- Sparse-set storage — Phase 13 (umbrella out-of-scope).
- Single-component specialisation (former Opt-B5) — Phase 13 exploratory (C-NEW-2 dropped).
- Combined-generation branchless OR cleanup (former Opt-B4) — Phase 13 (C-NEW-3 dropped).
- 1.10× Bevy on `g2_boyko_query_iter_10k` — Phase 13 (C6 revisit umbrella amendment).
- `spawn_batch` integration — Track A.
- Change-detection from direct API (`Query<Ref<T>>`, `Query<Mut<T>>`) — Phase 13 (panic in v1 per I-NEW-4 (b)).

---

## §2 Invariants

Naming: `QC` = QueryTypeId / cache structure, `QV` = QueryView / direct API, `NCD` = const elision, `PHASE9..11` = subsystem interactions, `META` = OnceLock<SystemMeta> path.

### 2.1 `QueryTypeId` + cache slot (QC1..QC9) — UNCHANGED from Round 2

- **QC1** — `QueryTypeId(pub usize)` `#[repr(transparent)]`. Mirrors `BundleTypeId`.
- **QC2** — `QueryTypeKey: 'static` blanket on `(D, F)` with per-impl `OnceLock<QueryTypeId>` inside the function body (Phase 8.5 pattern).
- **QC3** — `QUERY_NEXT_ID: AtomicUsize`, `fetch_add(1, Relaxed)`. Happens-before via `OnceLock` Release-on-set / Acquire-on-get.
- **QC4** — `MAX_QUERY_TYPES`: default 1024; with `big_query_table` feature, 4096 (I5).
- **QC5** — Cache: `Box<[OnceLock<(NonNull<()>, fn(NonNull<()>))>; MAX_QUERY_TYPES]>`. Stable heap address.
- **QC6** — Heap-only constructor (Vec → into_boxed_slice → try_into).
- **QC7** — Each slot stores `(NonNull<()>, fn(NonNull<()>))`: type-erased pointer to `Box<UnsafeCell<QueryDataState<D, F>>>` + drop glue.
- **QC8** — Slot tripwire test asserts `size_of::<OnceLock<(NonNull<()>, fn(NonNull<()>))>>() <= 32`.
- **QC9** — `OnceLock<(NonNull<()>, fn(NonNull<()>))>: Send + Sync`.

### 2.2 `QueryView<'w, D, F>` direct API (QV1..QV11) — W4 wording fold

- **QV1** — `QueryView<'w, D: QueryData, F: QueryFilter>` carries `world: UnsafeEcsCell<'w>` + `state: NonNull<UnsafeCell<QueryDataState<D, F>>>`. 16 B total. `Send + Sync` per C1 (Round 2) + I-NEW-5.
- **QV2-QV5** — `iter`, `iter_mut`, `par_iter` / `par_iter_mut`, `single` / `get` etc. — unchanged from Round 2.
- **QV6** — On every `EcsMaster::query` call: cache hit → mint `&mut QueryDataState<D, F>` via `(*cell.get())` under `&mut self`; run `state.update`.
- **QV7** — `QueryView` borrows `&mut EcsMaster`; structurally impossible to call inside `Schedule::run`.
- **QV8** — `single` / `single_mut` panic if 0 or >1 rows (debug_assert + cold panic).
- **QV9** — `state.update(master)` is the only sync point per call.
- **QV10** — Phase 11 deferred archetype creations observed via QV6 update (dual-generation comparison).
- **QV11 (W4-folded canonical wording)** — `EcsMaster::query<D, F>(&mut self)` panics at runtime if `D::NEEDS_CHANGE_DETECTION || F::NEEDS_CHANGE_DETECTION` is true. The check is a `#[cold]` + `#[inline(never)]` panic site so the hot `&T` path is unaffected. Panic message (canonical — verbatim across §0 I-NEW-4, §2.2 QV11, §4.3 doc-comment, §5 implementation):
  ```text
  direct API EcsMaster::query<{D}, {F}>() does not support change-detection \
   filters (D or F has NEEDS_CHANGE_DETECTION = true); use Query<D, F> \
   inside a system body via Schedule
  ```
  where `{D}` and `{F}` are substituted via `std::any::type_name`. Filed as Phase 13 to relax.

### 2.3 `NEEDS_CHANGE_DETECTION` elision (NCD1..NCD8) — UNCHANGED from Round 3

- **NCD1** — `QueryData::NEEDS_CHANGE_DETECTION: bool` and `QueryFilter::NEEDS_CHANGE_DETECTION: bool` — NO DEFAULT.
- **NCD2** — Leaf impls: `&T`, `&mut T`, `()`, `With<C>`, `Without<C>` → `false`. `Ref<T>`, `Mut<T>`, `Added<C>`, `Changed<C>` → `true`.
- **NCD3** — Tuple propagation `(A, B, ...): NEEDS_CHANGE_DETECTION = A::NEEDS_CHANGE_DETECTION || B::NEEDS_CHANGE_DETECTION || ...`.
- **NCD4** — `Or<F>: NEEDS_CHANGE_DETECTION = F::NEEDS_CHANGE_DETECTION`.
- **NCD5** — Two new trait methods `set_table_readonly_no_meta` / `set_table_mut_no_meta` on `QueryData` / `QueryFilter`. **NO default body** (I4 retained from Round 2). Every impl declares:
  - NCD = false: meta-free re-implementation (`&T`, `()`, `With<C>`, etc. — the existing body minus the `meta` parameter — for `&T` this is a no-op; for `With<C>` this is a no-op).
  - NCD = true: body is `panic!("NCD violation: set_table_*_no_meta called for {} with NEEDS_CHANGE_DETECTION = true; dispatcher must use the meta variant", std::any::type_name::<Self>())`.
- **NCD6** — Dispatcher discipline in `QueryIter::next` / `QueryIterMut::next` / `for_each_impl`:
  ```text
  if const { D::NEEDS_CHANGE_DETECTION || F::NEEDS_CHANGE_DETECTION } {
      <D as QueryData>::set_table_readonly(fetch, state, arch, self.meta);
      <F as QueryFilter>::set_table_readonly(fetch, state, arch, self.meta);
  } else {
      <D as QueryData>::set_table_readonly_no_meta(fetch, state, arch);
      <F as QueryFilter>::set_table_readonly_no_meta(fetch, state, arch);
  }
  ```
  `if const { ... }` const-folds at monomorphisation; entire dead arm vanishes.
- **NCD7 (REVISED per C-NEW-1)** — `SystemMeta::dummy() -> &'static SystemMeta` is lazy-initialised via `static DUMMY: OnceLock<SystemMeta> = OnceLock::new();`. **Not a const**. Sole consumer: `QueryView::iter` / `iter_mut` / `par_iter` / `par_iter_mut` pass `SystemMeta::dummy()` to the cursor constructor where the cursor's `meta: &'s SystemMeta` field is type-required even on the const-folded `_no_meta` path. The field is loaded into a register; the const-fold inside NCD6 guarantees no actual `meta.last_run` / `meta.this_run` read on NCD=false. Path:
  ```text
  pub fn dummy() -> &'static SystemMeta {
      static DUMMY: OnceLock<SystemMeta> = OnceLock::new();
      DUMMY.get_or_init(|| SystemMeta {
          access: Access::new(),  // ordinary fn call, init-time only
          name: "<dummy>",
          last_archetype_generation: ArchetypeGeneration::FIRST,
          last_structural_generation: ArchetypeGeneration::FIRST,
          last_run: Tick::ZERO,
          this_run: Tick::ZERO,
      })
  }
  ```
  Cost: first call ~50 ns (single `OnceLock::set`); subsequent calls ~1-2 ns (Acquire load). Inlined within `QueryView::iter`, the `OnceLock::get` short-circuits to a fast path. Per direct `query` call the cost is amortised against the ~5 ns cache-hit budget (§1.2 row 3).
- **NCD8 (per I-NEW-4 / W4 canonical wording)** — `EcsMaster::query<D, F>(&mut self)` panics if `D::NEEDS_CHANGE_DETECTION || F::NEEDS_CHANGE_DETECTION` is true. Cold path; zero overhead on hot path via const-fold. Canonical message: see QV11.

### 2.4 Phase 9 parallel scheduler interaction — UNCHANGED from Round 2

- **PHASE9.1-9.5** — `EcsMaster: Send + Sync` ground truth; workers receive `UnsafeEcsCell` (read-only); cache mutation only under `&mut self` (dispatcher between frames).

### 2.5 Phase 10 change-detection interaction (PHASE10.1..PHASE10.5) — REVISED per C-NEW-1 + I-NEW-4

- **PHASE10.1** — `SystemMeta::dummy() -> &'static SystemMeta` is lazy-initialised on first call (C-NEW-1 fallback). Fields: `access = Access::new()`, `name = "<dummy>"`, generations = `FIRST`, ticks = `ZERO`.
- **PHASE10.2** — `SystemMeta::dummy()` is **never written to**. Read by `QueryView::iter` constructor; const-fold inside NCD6 guarantees never accessed on `&T`/`()` path.
- **PHASE10.3 (REVISED per I-NEW-4)** — For direct `EcsMaster::query<Ref<T>>` / `Query<Mut<T>>` / any path with `NEEDS_CHANGE_DETECTION = true`: **the call panics**. The direct API does not support change detection in v1. Use `Schedule` + `Query<D, F>` SystemParam path.
- **PHASE10.4** — N/A (PHASE10.3 collapses the synthesised SystemMeta scenario).
- **PHASE10.5** — `check_ticks` integrated into `Schedule::run` at frame start. Direct `EcsMaster::query` calls do NOT trigger `check_ticks`. Documented contract.

### 2.6 Phase 11 deferred-archetype interaction (PHASE11.1..PHASE11.3) — UNCHANGED from Round 1

### 2.7 Former Opt-B4 (combined-generation) — DROPPED per C-NEW-3

Round 2 §2.7 removed wholesale. Storage is non-atomic (`archetype_master.rs:37, 54`); no fused-atomic-load lever exists. If a future `cargo asm` dump reveals branchy `||` short-circuit in `QueryDataState::update`, replace with `|` (bitwise OR) — pure-cleanup, no claimed budget. Filed as Phase 13 follow-up in §15.

### 2.8 Former Opt-B5 (single-component specialisation) — DROPPED per C-NEW-2

Round 2 §2.8 removed wholesale. `Archetype::column_raw_ptr` does not exist; the real path `columns.get_unchecked(state.id.0).ptr` is already optimal. Inner loop is byte-identical to Bevy per profile asm. Filed as Phase 13 exploratory work (alongside sparse-set storage).

---

## §3 Decision matrix

### Q-B1.1: Cache value storage — typed `(NonNull<()>, fn(NonNull<()>))` (UNCHANGED)

### Q-B1.2: Cache concurrency — `OnceLock` + `&mut self` gate (UNCHANGED)

### Q-B1.3: Cache invalidation under Phase 11 deferred archetype creation — generation comparison subsumes (UNCHANGED)

### Q-B1.4: Multi-query API — deferred to Phase 13 (UNCHANGED)

### Q-B1.5: `QueryView::par_iter` + Phase 9 ThreadPool — reuse `ParQuery` (UNCHANGED)

### Q-B2.1..Q-B2.3 — UNCHANGED from Round 1

### Q-B3.1 — UNCHANGED from Round 1: do NOT share cache with in-system path in v1

### Q-C4 (REVISED per C-NEW-1): `SystemMeta::dummy()` path — `OnceLock<SystemMeta>` fallback (path (c))

**Decision**: Use `static DUMMY: OnceLock<SystemMeta> = OnceLock::new();` initialised lazily on first call. Const-fn migration of `BitSet::new` is **rejected**: `BitSet<T>::new` at `bit_set.rs:84-89` builds `Self { bits: T::default() }`; `T::default()` is non-const trait method, `const_trait_impl` unstable. Rejected alternatives: (a) `const_trait_impl` (unstable); redesign `BitSet` with `const fn zero()` per-type inherent (out of scope). Accepted cost: +2 ns to cache-hit budget; §1.2 row 3 revised from 3 ns to 5 ns. NCD5 default body is **removed** so no method body forwards through `&SystemMeta::DUMMY`.

### Q-C5 (UNCHANGED): Drop ordering — `query_state_cache` AFTER `arena` (option (a))

### Q-C6 (REVISED per critic Round 2): Success criterion — Bevy parity (within noise floor) (path (b))

**Decision**: Honest documentation. Inner loop is byte-identical to Bevy per profile asm. Bevy parity is achievable; 1.10× is not without Phase 13+ redesign. Drop fictional Opt-B4/Opt-B5 levers. Amend umbrella criterion for this bench specifically — see "Proposed umbrella amendment" block at end of doc.

### Q-NEW-1 (NEW per I-NEW-4): Direct API + change detection — runtime panic (option (b))

**Decision**: `EcsMaster::query<D, F>(&mut self)` panics if `D` or `F` has `NEEDS_CHANGE_DETECTION = true`. Path (b). Rejected alternatives: (a) compile-error via marker trait (adds trait-bound surface); (c) synthesised `last_run` per-(D, F) cache slot (semantics broken — every row reads as Changed since the bogus baseline; adds state). Path (b) is simplest, cold, has no broken silent semantics.

---

## §4 Data structures

### 4.1 `QueryTypeId` + per-impl mint — UNCHANGED from Round 2 §4.1

```rust
// crates/boyko_ecs/src/ecs/core/iters/query/query_type_registry.rs (NEW)

#[repr(transparent)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueryTypeId(pub usize);

#[cfg(not(feature = "big_query_table"))]
pub const MAX_QUERY_TYPES: usize = 1024;

#[cfg(feature = "big_query_table")]
pub const MAX_QUERY_TYPES: usize = 4096;

static QUERY_NEXT_ID: AtomicUsize = AtomicUsize::new(0);

#[cold]
#[inline(never)]
pub fn register_new() -> QueryTypeId {
    let id = QUERY_NEXT_ID.fetch_add(1, Ordering::Relaxed);
    if id >= MAX_QUERY_TYPES {
        QUERY_NEXT_ID.store(MAX_QUERY_TYPES, Ordering::Relaxed);
        panic!(
            "QueryTypeId exhaustion: MAX_QUERY_TYPES = {} reached. \
             Enable the `big_query_table` cargo feature on boyko_ecs.",
            MAX_QUERY_TYPES
        );
    }
    QueryTypeId(id)
}

pub trait QueryTypeKey: 'static {
    fn query_type_id() -> QueryTypeId;
}

impl<D, F> QueryTypeKey for (D, F)
where
    D: QueryData + 'static,
    F: QueryFilter + 'static,
{
    fn query_type_id() -> QueryTypeId {
        static SLOT: OnceLock<QueryTypeId> = OnceLock::new();
        *SLOT.get_or_init(register_new)
    }
}
```

### 4.2 `EcsMaster::query_state_cache` — UNCHANGED from Round 2 §4.2 (per I-NEW-2)

Field placement: AFTER `arena` (C5). The existing `bundle_archetype_cache` placement at `ecs_master.rs:111-134` is preserved verbatim per the field's existing C6 pin docstring; the only change to `EcsMaster` field layout in this plan is the *addition* of `query_state_cache` after `arena` (C5 fix).

```rust
pub struct EcsMaster {
    // ... existing fields, INCLUDING bundle_archetype_cache between
    //     archetype_master and change_tick — UNCHANGED per I-NEW-2 ...

    /// Memory arena for component allocation.
    arena: Box<Arena>,

    /// Phase 12.5 Track B QC5/QC6/C5 — per-(D, F) cache.
    ///
    /// **Field slot (C5 fix)**: declared AFTER `arena`. Rust drops fields
    /// in declaration order, so this field is dropped LAST. Inverts the
    /// failure mode for any future `D::State` / `F::State` carrying
    /// arena-derived raw pointers from silent miscompile to immediate
    /// Miri trip.
    query_state_cache: Box<[OnceLock<(NonNull<()>, fn(NonNull<()>))>; MAX_QUERY_TYPES]>,
}
```

Constructor: heap-only `Vec::with_capacity + push + into_boxed_slice + try_into`. Drop: walk slots last, invoke per-slot drop fn.

### 4.3 `QueryView<'w, D, F>` — W1 fold (single canonical Send/Sync assertion)

```rust
// crates/boyko_ecs/src/ecs/core/iters/query/query_view.rs (NEW)

pub struct QueryView<'w, D: QueryData, F: QueryFilter = ()> {
    world: UnsafeEcsCell<'w>,
    state: NonNull<UnsafeCell<QueryDataState<D, F>>>,
    _marker: PhantomData<&'w mut EcsMaster>,
}

impl<'w, D: QueryData, F: QueryFilter> QueryView<'w, D, F> {
    /// Shared state access; sound because `&mut EcsMaster` is held by 'w.
    #[inline]
    fn state(&self) -> &QueryDataState<D, F> {
        // SAFETY (QV6 / I1 / I-NEW-3): self.state is a valid
        //   NonNull<UnsafeCell<...>>; UnsafeCell::get() returns *mut; we
        //   reborrow as & only. The &mut retag inside `update()` is
        //   produced inside `query<&mut self>` (uniqueness gate); this
        //   method never produces a &mut.
        unsafe { &*(*self.state.as_ptr()).get() }
    }
}

// I-NEW-5 / W1: Send/Sync symmetry assertion at module scope (NOT under
// cfg(test)). Fires on every compile — including release. Uses unit
// `()` as both data and filter parameters; `(): QueryData` and
// `(): QueryFilter` exist as Phase 8b stubs in `data.rs` / `filter.rs`.
// This is the single canonical assertion for QueryView Send/Sync; no
// cfg(test)-gated variant is registered.
static_assertions::assert_impl_all!(QueryView<'static, (), ()>: Send, Sync);
```

**Why `()` and not a stub `_DummyComponent`** (W1 rationale): materialising a non-trivial stub `Component` impl at top-level pollutes the crate symbol table and creates a load-bearing public item only Send/Sync exists to assert. The Phase 8b unit `()` impl is already in the public surface (`data.rs` / `filter.rs`) and exhibits the same `D::State: Send + Sync, F::State: Send + Sync` trait-bound geometry. The assertion compiles in every config (`cargo check`, `cargo build --release`, `cargo test`) and structurally implies the `Send + Sync` bound the existing `Query<'w, 's, D, F>` SystemParam carries at `data.rs:90`.

### 4.4 `SystemMeta::dummy()` lazy accessor — REVISED per C-NEW-1 path (c) + W3 BSS tripwire

```rust
// crates/boyko_ecs/src/ecs/core/system/system_meta.rs (extension)

// W3 tripwire — fails compile if SystemMeta growth or OnceLock layout
// change pushes BSS footprint above 320 B.
const _: () = assert!(
    core::mem::size_of::<std::sync::OnceLock<SystemMeta>>() <= 320,
    "SystemMeta::dummy() BSS footprint exceeded 320 B budget; \
     reduce SystemMeta size or revisit §10.5"
);

impl SystemMeta {
    /// Phase 12.5 Track B NCD7 (C-NEW-1 fallback) — lazy `'static` dummy
    /// SystemMeta. Sole consumer: `QueryView::iter` / `iter_mut` /
    /// `par_iter` / `par_iter_mut` pass `SystemMeta::dummy()` to the
    /// cursor's meta argument. Const-fold inside NCD6 guarantees no field
    /// is read on `NEEDS_CHANGE_DETECTION = false` branches.
    ///
    /// **Why lazy and not const** (C-NEW-1): `Access::new()` builds via
    /// `ComponentMask::new()` → `BitSet::<u64>::new()` which calls
    /// `T::default()` (non-const trait method). `const_trait_impl` is
    /// unstable. Falling back to one-shot `OnceLock` initialisation.
    ///
    /// First call: ~50 ns (`OnceLock::set` Release store + body of
    /// `Access::new`). Subsequent calls: ~1-2 ns (Acquire load).
    ///
    /// Result is `'static` — borrowed for the lifetime of the process.
    #[inline]
    pub fn dummy() -> &'static SystemMeta {
        static DUMMY: OnceLock<SystemMeta> = OnceLock::new();
        DUMMY.get_or_init(|| SystemMeta {
            access: Access::new(),
            name: "<dummy>",
            last_archetype_generation: ArchetypeGeneration::FIRST,
            last_structural_generation: ArchetypeGeneration::FIRST,
            last_run: Tick::ZERO,
            this_run: Tick::ZERO,
        })
    }
}
```

**No `SystemMeta::DUMMY` const** (per C-NEW-1). No `Access::EMPTY` const. No `const fn` migration of `BitSet::new`, `BitSet256::new`, `ComponentMask::new`, `Access::new`.

`BitSet256::new` is **already** `const fn` at `bit_set_256.rs:33`; no migration required there.

### 4.5 Extended `QueryData` / `QueryFilter` trait surface — UNCHANGED from Round 2 §4.5

```rust
pub unsafe trait QueryData: Sized {
    // ... existing items ...

    /// Phase 12.5 Track B NCD1 — compile-time flag. NO DEFAULT.
    const NEEDS_CHANGE_DETECTION: bool;

    /// Phase 12.5 Track B NCD5 — meta-free `set_table_readonly`. NO DEFAULT
    /// (I4 retained; silent-fallthrough hazard removed).
    unsafe fn set_table_readonly_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
    );

    unsafe fn set_table_mut_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    );
}
```

Mirror additions on `QueryFilter`. Implementation patterns for leaves and tuples — unchanged from Round 2 §4.5.

**Impl example (NCD = false, `&T`)** — meta-free re-implementation matching real access path:

```rust
unsafe impl<T: Component> QueryData for &'_ T {
    const NEEDS_CHANGE_DETECTION: bool = false;

    #[inline]
    unsafe fn set_table_readonly_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
    ) {
        // Identical body to set_table_readonly minus the unused _meta arg.
        // Real access path per data.rs:307-309.
        // SAFETY (QD3): archetype is a live *const Archetype for 'w (caller
        //   contract); columns is at offset 0 per Phase 7 D4; state.id.0 <
        //   MAX_COMPONENTS by construction.
        let column = unsafe { (*archetype).columns.get_unchecked(state.id.0) };
        debug_assert!(!column.ptr.is_null(), "QD2: column was unexpectedly null");
        fetch.base = column.ptr as *const T;
    }

    #[inline]
    unsafe fn set_table_mut_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    ) {
        // &T degrades to read; forward to readonly path.
        // SAFETY: archetype's *mut provenance is strictly stronger than the
        //   *const we need.
        unsafe { Self::set_table_readonly_no_meta(fetch, state, archetype as *const Archetype); }
    }
}
```

**Note**: this body uses `columns.get_unchecked(state.id.0).ptr` directly — matching the real implementation at `data.rs:307-309`. No fictitious `column_raw_ptr` method.

**Impl example (NCD = true, `Ref<T>`)** — panic-on-misuse (NCD5 dispatcher never reaches this on direct API path because §QV11 panics earlier; this is defence-in-depth for the in-system path if NCD6 dispatcher mis-routes):

```rust
unsafe impl<T: Component> QueryData for Ref<'_, T> {
    const NEEDS_CHANGE_DETECTION: bool = true;

    #[inline(never)]
    #[cold]
    unsafe fn set_table_readonly_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {
        panic!(
            "NCD violation: set_table_readonly_no_meta called for {} \
             (NEEDS_CHANGE_DETECTION = true).",
            std::any::type_name::<Self>()
        );
    }

    // Mirror set_table_mut_no_meta.
}
```

### 4.6 Modified `QueryIter::next` dispatch — UNCHANGED from Round 2 §4.6

The `if const { D::NEEDS_CHANGE_DETECTION || F::NEEDS_CHANGE_DETECTION } { ... } else { ... }` shape holds.

### 4.7 Former `combined_generation_snapshot` — DROPPED per C-NEW-3

Removed. Storage is non-atomic; no fused load lever. Phase 13 follow-up.

### 4.8 Former `SingleReadIter` — DROPPED per C-NEW-2

Removed. Inner loop byte-identical to Bevy; specialised cursor would not move the needle. Phase 13 exploratory work.

---

## §5 Public API — REVISED per I-NEW-4 + W4 canonical wording

```rust
impl EcsMaster {
    /// Phase 12.5 Track B Opt-B1 — direct query API.
    ///
    /// Returns a `QueryView<'_, D, F>` for `iter`, `iter_mut`, `single`,
    /// `single_mut`, `get`, `get_mut`, `par_iter`, `par_iter_mut`. Bypasses
    /// `FunctionSystem` / `FilteredAccessSet` — aliasing gated by `&mut self`.
    ///
    /// First call for a given (D, F) pair: ~1 µs cold cost
    /// (`QueryDataState::new`). Subsequent: ~5 ns cache hit.
    ///
    /// # Panics (I-NEW-4 / QV11 / W4 canonical)
    ///
    /// Panics if `D::NEEDS_CHANGE_DETECTION || F::NEEDS_CHANGE_DETECTION`
    /// is true (i.e. if `D` or `F` contains `Ref<T>`, `Mut<T>`, `Added<C>`,
    /// or `Changed<C>`). Change-detection requires `Schedule` context;
    /// use `Query<D, F>` as a SystemParam inside a system body via Schedule.
    /// Panic message:
    ///
    /// ```text
    /// direct API EcsMaster::query<{D}, {F}>() does not support change-detection \
    ///  filters (D or F has NEEDS_CHANGE_DETECTION = true); use Query<D, F> \
    ///  inside a system body via Schedule
    /// ```
    pub fn query<D, F>(&mut self) -> QueryView<'_, D, F>
    where
        D: QueryData + 'static,
        F: QueryFilter + 'static,
    {
        // QV11 / I-NEW-4: change-detection guard. const-folded; on
        // !NCD path the if vanishes at monomorphisation.
        if const { D::NEEDS_CHANGE_DETECTION || F::NEEDS_CHANGE_DETECTION } {
            query_change_detection_panic::<D, F>();
        }

        let type_id = <(D, F) as QueryTypeKey>::query_type_id();
        debug_assert!(type_id.0 < MAX_QUERY_TYPES);

        // SAFETY: type_id.0 < MAX_QUERY_TYPES per debug_assert + register_new
        //   panic-on-overflow saturation at register site.
        let slot = unsafe { self.query_state_cache.get_unchecked(type_id.0) };

        if let Some(&(typed_ptr, _drop_fn)) = slot.get() {
            // Cache hit.
            let cell_ptr: NonNull<UnsafeCell<QueryDataState<D, F>>> = typed_ptr.cast();

            // SAFETY (QV6 / I-NEW-3): &mut self ensures uniqueness over the
            //   cache slot; cell.get() returns *mut which we reborrow as
            //   &mut for the update() call only. The retag derives from
            //   &mut self's unique provenance.
            unsafe {
                let state_mut: &mut QueryDataState<D, F> =
                    &mut *(*cell_ptr.as_ptr()).get();
                state_mut.update(self.archetype_master());
            }

            return QueryView {
                // SAFETY: world cell doesn't outlive &mut self borrow.
                world: unsafe { UnsafeEcsCell::new_mutable(self) },
                state: cell_ptr,
                _marker: PhantomData,
            };
        }

        self.query_cold_init::<D, F>(type_id)
    }

    #[cold]
    #[inline(never)]
    fn query_cold_init<D, F>(&mut self, type_id: QueryTypeId) -> QueryView<'_, D, F>
    where
        D: QueryData + 'static,
        F: QueryFilter + 'static,
    {
        let state = QueryDataState::<D, F>::new(self);
        let cell = Box::new(UnsafeCell::new(state));
        let cell_ptr: NonNull<UnsafeCell<QueryDataState<D, F>>> =
            NonNull::from(Box::leak(cell));
        let type_erased: NonNull<()> = cell_ptr.cast();

        let drop_fn: fn(NonNull<()>) = |p: NonNull<()>| {
            // SAFETY: invoked from EcsMaster::drop; cached pointer was
            //   minted from Box::leak on Box<UnsafeCell<QueryDataState<D, F>>>;
            //   reconstructing the Box resumes ownership and runs drop glue.
            let typed: NonNull<UnsafeCell<QueryDataState<D, F>>> = p.cast();
            unsafe { drop(Box::from_raw(typed.as_ptr())); }
        };

        let slot = unsafe { self.query_state_cache.get_unchecked(type_id.0) };
        match slot.set((type_erased, drop_fn)) {
            Ok(()) => {
                // SAFETY: same as cache-hit path.
                unsafe {
                    let state_mut: &mut QueryDataState<D, F> =
                        &mut *(*cell_ptr.as_ptr()).get();
                    state_mut.update(self.archetype_master());
                }

                QueryView {
                    world: unsafe { UnsafeEcsCell::new_mutable(self) },
                    state: cell_ptr,
                    _marker: PhantomData,
                }
            }
            Err(_) => {
                // OnceLock::set raced under &mut self — structurally impossible.
                // SAFETY: reclaim Box ownership before panic.
                unsafe { drop(Box::from_raw(cell_ptr.as_ptr())); }
                debug_assert!(false, "OnceLock::set raced under &mut self — impossible");
                panic!("invariant violated: query_state_cache slot raced under &mut self");
            }
        }
    }
}

/// I-NEW-4 / QV11 / W4 canonical panic site — cold + inline(never) so it
/// lives outside the hot path's I-cache. Message is the verbatim canonical
/// wording quoted in §2.2 QV11 and §0 I-NEW-4 and `query` doc-comment.
#[cold]
#[inline(never)]
fn query_change_detection_panic<D, F>() -> !
where
    D: QueryData + 'static,
    F: QueryFilter + 'static,
{
    panic!(
        "direct API EcsMaster::query<{}, {}>() does not support change-detection \
         filters (D or F has NEEDS_CHANGE_DETECTION = true); use Query<D, F> \
         inside a system body via Schedule",
        std::any::type_name::<D>(),
        std::any::type_name::<F>(),
    );
}
```

`query_ref<&self>` is **not** part of v1 (C2 retained; deferred to Phase 13).

`QueryView` methods (`iter`, `iter_mut`, `par_iter`, `par_iter_mut`, `single`, `single_mut`, `get`, `get_mut`) per Round 1 §5.2 — pass `SystemMeta::dummy()` as the meta argument to the cursor constructor. On NCD = false paths the const-fold in NCD6 elides any meta read.

---

## §6 Algorithms for critical paths

### 6.1 Hot path: `EcsMaster::query<&T, ()>()` cache hit — REVISED per C-NEW-4

Steps (target ~5 ns total):

1. `if const { D::NEEDS_CHANGE_DETECTION || F::NEEDS_CHANGE_DETECTION }` → const-folded to `if false` for `&T, ()`; **0 ns** (dead arm eliminated at monomorphisation).
2. `QueryTypeKey::query_type_id()` → `OnceLock::get_or_init` Acquire load → **~1 ns warm**.
3. `self.query_state_cache.get_unchecked(0)` → `OnceLock::get()` → **~1 ns**.
4. `NonNull::cast()` → 0 ns.
5. `(*cell_ptr.as_ptr()).get()` → 0 ns (raw pointer mint).
6. `state_mut.update(self.archetype_master())` → two non-atomic generation loads + pair compare + early-out → **~2 ns warm** (no Opt-B4 acceleration — generations are plain fields).
7. `UnsafeEcsCell::new_mutable(self)` → **~1 ns**.

**Total**: ~5 ns to obtain the `QueryView` (matching §1.2 row 3 budget).

Note (C-NEW-4): the 3 ns Round 2 target was wishful; honest enumeration sums to ~5 ns. Phase 8.5's 226 ps figure applied to `OnceLock<ArchetypeId>::get()` alone (step 3 cost in isolation). The full query path includes steps 1, 2, 6, 7 which together add ~4 ns.

### 6.2 Cold path: first call

Steps:

1. Steps 1-3 from §6.1 → cache miss.
2. Branch into `query_cold_init` (`#[cold]`).
3. `QueryDataState::<&T, ()>::new(self)` → ~200 ns.
4. `Box::new(UnsafeCell::new(state))` → 50 ns.
5. `Box::leak` → 0 ns.
6. `slot.set` → 20 ns.
7. Run initial `update` → 2 ns warm.

**Total cold**: ~275 ns ≈ 0.3 µs.

For the **first ever** call site that triggers both `QueryTypeKey::query_type_id` AND `query_cold_init` AND `SystemMeta::dummy()` initialisation: add ~50 ns for `SystemMeta::dummy()`'s `OnceLock::set` (C-NEW-1 fallback) — process-lifetime, one-shot, not per (D, F) pair. Total first-ever cold: ~325 ns ≈ 0.35 µs.

### 6.3 Hot inner loop — UNCHANGED

Per profile asm at `LBB306_16` (PHASE-12.5-PROFILE-QUERY.md), the 5-instruction inner loop is unchanged from current code. NCD6 const-folds the `if` away for `&T, ()`; the dispatched `set_table_readonly_no_meta` body matches `set_table_readonly`'s body minus the unused `_meta` arg. The compiled inner loop is byte-identical to the existing code AND to Bevy's per the asm dump (PHASE-12.5-PROFILE-QUERY.md §"Bevy inner loop assembly excerpt").

### 6.4 Former Opt-B4 (combined-generation) — DROPPED

Per C-NEW-3. If a future `cargo asm` dump shows branchy `||` short-circuit in `QueryDataState::update`, replace with `|` (bitwise OR). Filed as Phase 13 cleanup.

### 6.5 Former Opt-B5 (single-component specialisation) — DROPPED

Per C-NEW-2. Inner loop already optimal. Filed as Phase 13 exploratory work.

---

## §7 Multithreading model — UNCHANGED from Round 2

### 7.1 `EcsMaster` concurrency policy

- `EcsMaster: Send + Sync` per `ecs_master.rs:1501-1502`.
- `UnsafeEcsCell<'w>: Copy + Send + Sync`.
- Workers receive `UnsafeEcsCell<'w>` and reborrow `cell.world() -> &'w EcsMaster` (read-only).
- Dispatcher reborrows `&mut EcsMaster` outside `Schedule::run` (apply-window barrier).

### 7.2 Direct API gate

`EcsMaster::query<&mut self>` requires `&mut EcsMaster`. Borrow checker forbids any concurrent worker access; `query` callable only outside `Schedule::run`.

### 7.3 Cache mutation rules

- Cache mutation (`OnceLock::set` in `query_cold_init`) under `&mut self`. CAS structurally uncontested.
- Cache read (`OnceLock::get` hot path) under `&mut self`. Acquire load wait-free on x86_64.

### 7.4 Cross-thread visibility

- Worker that gains `&'w EcsMaster` via dispatcher hand-off sees cache state via prior `&mut` Release boundary.
- v1 has no `&self` API touching the cache; soundness preserved for Phase 13's future `query_ref<&self>`.

### 7.5 Send/Sync proof for new types

- `QueryView<'w, D, F>`: `Send + Sync` provided `D::State: Send + Sync, F::State: Send + Sync` (already required by `QueryData` / `QueryFilter` bounds at `data.rs:90`). Symmetry with existing `Query<'w, 's, D, F>` per I-NEW-5.
- `query_state_cache`: `Send + Sync` per QC9.
- `static_assertions::assert_impl_all!(QueryView<'static, (), ()>: Send, Sync)` at module scope of `query_view.rs` (NOT under `cfg(test)`) per I-NEW-5 / W1 — single canonical assertion.

### 7.6 `OnceLock` atomic-ordering proof

Standard `OnceLock` semantics: Release-on-set / Acquire-on-get. Holds for both `query_state_cache` slots and the new `SystemMeta::dummy()` OnceLock.

---

## §8 Integration

### 8.1 Affected modules — REVISED per C-NEW-1 + C-NEW-2 + C-NEW-3 + W3 tripwire

| Module | Change |
|--------|--------|
| `crates/boyko_ecs/src/ecs/core/iters/query/query_type_registry.rs` (NEW) | `QueryTypeId`, `MAX_QUERY_TYPES` (cargo-feature gated), `register_new`, `QueryTypeKey`. |
| `crates/boyko_ecs/src/ecs/core/iters/query/query_view.rs` (NEW) | `QueryView<'w, D, F>` (NO `QueryViewRef` per C2). Module-scope single `static_assertions::assert_impl_all!(QueryView<'static, (), ()>: Send, Sync)` per I-NEW-5 / W1. |
| `crates/boyko_ecs/src/ecs/core/iters/query/mod.rs` | Export new module. |
| `crates/boyko_ecs/src/ecs/core/iters/query/data.rs` | Add `NEEDS_CHANGE_DETECTION` + `set_table_readonly_no_meta` / `set_table_mut_no_meta` (NO DEFAULTS per I4); impls on all leaves + tuple/Or/stub macro extensions. |
| `crates/boyko_ecs/src/ecs/core/iters/query/filter.rs` | Same. |
| `crates/boyko_ecs/src/ecs/core/iters/query/iter.rs` | Modify `QueryIter::next` / `QueryIterMut::next` per NCD6 const-fold dispatch. |
| `crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs` | Same const-fold inside `run_chunk_*`. |
| `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` | Add `query_state_cache` field AFTER `arena` (C5); add `query` / `query_cold_init` / `query_change_detection_panic` (W4 canonical message); extend `new` (heap-only constructor C3); extend `Drop`. `bundle_archetype_cache` placement preserved per I-NEW-2. |
| `crates/boyko_ecs/src/ecs/core/system/system_meta.rs` | Add `SystemMeta::dummy()` lazy accessor backed by `OnceLock` (C-NEW-1) + W3 module-scope `const _: () = assert!(size_of::<OnceLock<SystemMeta>>() <= 320);` BSS tripwire. |
| `crates/boyko_ecs/Cargo.toml` | Add `big_query_table` cargo feature (I5). Add `static_assertions` dev/normal-dep if not present (already in dependencies — verify). |
| `crates/bench_bevy_vs_boyko/benches/comparison.rs` | Update `g2_boyko_query_iter_10k` to use `world.query::<&BoykoPosition, ()>().iter()` (generic path; no `iter_single_read`). |
| `crates/bench_bevy_vs_boyko/benches/profile_query.rs` | Add `p2_boyko_direct_api_10k` bench using direct API. |

**NOT touched** (per C-NEW-1 / C-NEW-2 / C-NEW-3):
- `crates/boyko_utils/src/bit_mask/bit_set.rs` — no `const fn` migration.
- `crates/boyko_utils/src/bit_mask/bit_set_256.rs` — already `const fn`, unchanged.
- `crates/boyko_ecs/src/ecs/core/component/component_mask.rs` — no `const fn` migration.
- `crates/boyko_ecs/src/ecs/core/system/access.rs` — no `const fn` migration, no `EMPTY` const.
- `crates/boyko_ecs/src/ecs/core/archetype/archetype.rs` — no `column_raw_ptr` method addition.
- `crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs` — no `combined_generation_snapshot`.
- `crates/boyko_ecs/src/ecs/core/iters/query/state.rs` — no branchless OR'd dirty check (Phase 13 cleanup).
- `crates/boyko_ecs/src/ecs/core/iters/query/query_view_single.rs` — file not created.

### 8.2 Compatibility with existing subsystems — UNCHANGED

### 8.3 ABI changes

78 trait-impl edits via 4 macros + 6 leaf impls (NCD const + two new no-meta methods, NO DEFAULTS per I4). Zero callsite migration outside the query module.

---

## §9 Implementation plan — REVISED Steps + Waves (C-NEW-1 / C-NEW-2 / C-NEW-3) + W3 tripwire

### Step 0 — Wave A prerequisite — REVISED per C-NEW-1 + W3

- **Step 0a (NEW per C-NEW-1)**: Add `SystemMeta::dummy() -> &'static SystemMeta` lazy accessor backed by `static DUMMY: OnceLock<SystemMeta> = OnceLock::new();` in `system_meta.rs`. Also add the **W3 BSS tripwire** `const _: () = assert!(size_of::<OnceLock<SystemMeta>>() <= 320);` at module scope. Unit test: two calls return same pointer (sequential check; multi-thread variant deferred per W2 to a sequential 1000× pointer-stability loop).

**Removed from Round 2 Step 0**: 0a-0d const-fn migrations of `BitSet::new` / `BitSet256::new` / `ComponentMask::new` / `Access::new` are **abandoned** (C-NEW-1). Step 0e (`const _: SystemMeta = SystemMeta::DUMMY;`) is **abandoned**.

Step 0 must complete before any other Wave runs.

### Wave A — Foundations (parallelisable: A1 + A2)

- **Step A1**: Verify Step 0a completed; add `SystemMeta::dummy()` lazy-init test (two calls return same address; field values correct).
- **Step A2**: Create `query_type_registry.rs` module with `QueryTypeId`, `MAX_QUERY_TYPES` (feature-gated), `register_new`, `QueryTypeKey` trait. Unit tests: monotonic mint, cap saturation panic, distinct (D, F) → distinct IDs, LTO regression test (I2).

### Wave B — Cache scaffolding (sequential)

- **Step B1**: Extend `EcsMaster` with `query_state_cache` field (placed AFTER `arena` per C5; `bundle_archetype_cache` placement preserved per I-NEW-2). Heap-only constructor per C3 (Vec → into_boxed_slice → try_into). Update `Drop` to walk and invoke per-slot drop fn. Add tripwire test `oncelock_query_slot_size_assumptions`.
- **Step B2**: `EcsMaster::query<D, F>(&mut self) -> QueryView<'_, D, F>` + `query_cold_init` + `query_change_detection_panic` (I-NEW-4 / W4 canonical message). Unit tests: cache hit, cache miss, second-call cache hit, drop releases state, OnceLock::set raced-Err arm panics on debug, change-detection panic fires for `Query<Ref<T>>`.

### Wave C — NEEDS_CHANGE_DETECTION + NO-DEFAULT no-meta methods (parallelisable: C1 + C2)

- **Step C1**: Add `NEEDS_CHANGE_DETECTION` const + `set_table_readonly_no_meta` / `set_table_mut_no_meta` to `QueryData` (NO default bodies per I4). Update all leaf impls: `&T`, `&mut T`, `()` with meta-free re-implementations (using `columns.get_unchecked(state.id.0).ptr` real path — NOT fictitious `column_raw_ptr`); `Ref<T>`, `Mut<T>` with panic bodies. Extend tuple macro at `data.rs:1054`. Stub-overflow impls panic.
- **Step C2**: Mirror C1 for `QueryFilter`: `()`, `With<C>`, `Without<C>`, `Added<C>`, `Changed<C>`, tuple macro, `Or<F>` macro, stub macros.

### Wave D — Iterator dispatch + QueryView (sequential)

- **Step D1**: Modify `QueryIter::next` / `QueryIterMut::next` per NCD6 (const-fold dispatch). Unit tests: `Query<&T>` regression (results unchanged); `Query<Ref<T>>` ticks correct (in-system path; direct-API path tested by `query_change_detection_panic_smoke`).
- **Step D2**: Same NCD6 modification in `par_iter.rs::run_chunk_*` paths.
- **Step D3**: Create `query_view.rs` with `QueryView<'w, D, F>` + W1 single canonical module-scope `assert_impl_all!(QueryView<'static, (), ()>: Send, Sync)`. Implement `iter`, `iter_mut`, `par_iter`, `par_iter_mut`, `single`, `single_mut`, `get`, `get_mut`. Each iter/par_iter passes `SystemMeta::dummy()` as the meta arg. Unit tests per method.

**Removed from Round 2 Wave D**: Step D4 (Opt-B4 `combined_generation_snapshot`) — DROPPED per C-NEW-3.

### Wave E — DROPPED entirely per C-NEW-2

Round 2 Wave E (Step E1 `query_view_single.rs` + `SingleReadIter`) is **removed**. Phase 13 exploratory work.

### Wave F — Bench + verification (parallelisable: F1 + F2)

- **Step F1**: Update `g2_boyko_query_iter_10k` to call `world.query::<&BoykoPosition, ()>().iter()` (generic path per C-NEW-5; no `iter_single_read`). Run criterion; commit results to `docs/PHASE-12.5-RESULTS-INTERIM-B.md` per umbrella §C1. Acceptance: **`≥ Bevy parity (within 5% noise floor)`** — concretely target ≤ Bevy reference + 5% = ≤ ~7.25 µs.
- **Step F2**: Assembly inspection via `cargo rustc --release -- --emit asm`. Confirm:
  - NCD6 const-fold elides the `_meta` arm on `&T` path.
  - The inner loop matches the profile asm at `LBB306_16` byte-for-byte.
  Document in `docs/PHASE-12.5-QUERY-ASM-CHECK.md`.
- **Step F3**: Run full test suite (`cargo test --workspace --lib --tests -- --test-threads=1`). All 612+ existing tests pass.
- **Step F4**: Miri tests per §11.4 (incl. C5 drop ordering, I1 Tree Borrows, I-NEW-3 repeated calls, W2 sequential pointer-stability loop).

### Wave ordering

```
Step 0 → A1 ─┐
             ├─→ B1 → B2 ─┐
        A2 ──┤             │
       C1 ──┤              ├─→ D1 → D2 → D3 ─→ F1 → F2 → F3 → F4
       C2 ──┘              │
```

Step 0 (single sub-step now: `SystemMeta::dummy` lazy accessor + W3 BSS tripwire) is the prerequisite. A1, A2, C1, C2 then parallelisable (different files). D1-D3 sequential. **No Wave E**. F1+F2 parallelisable.

**Estimated calendar**: **4-6 days** (revised from 5-7; Wave E removed, Step D4 removed).

---

## §10 Memory layout details

### 10.1 `EcsMaster` field order — REVISED per C5 + I-NEW-2

Post-Phase 12.5 B order (verbatim from `ecs_master.rs:82-169` with addition only):
```text
resources                  (drops first)
events
entity_master
archetype_master
bundle_archetype_cache     (PRESERVED per I-NEW-2 / existing C6 pin)
change_tick
last_check_tick
arena                      (drops second-to-last)
query_state_cache          (NEW — drops LAST per C5 fix)
```

Rationale: per I-NEW-2, the existing `bundle_archetype_cache` placement at `ecs_master.rs:111-134` is preserved verbatim — the field's existing C6 pin docstring documents its rationale and is not invalidated by this plan. The only change is the *addition* of `query_state_cache` after `arena`. Per C5 (Round 1/2), the new field is placed last so any future `D::State` / `F::State` impl holding arena-derived raw pointers fails loudly under Miri instead of silently miscompiling.

### 10.2 `QueryView<'w, D, F>` layout — UNCHANGED from Round 2

`#[repr(C)]` — 16 B: `world: UnsafeEcsCell<'w>` (8 B) + `state: NonNull<UnsafeCell<QueryDataState<D, F>>>` (8 B) + ZST `_marker`.

### 10.3 `query_state_cache` footprint — UNCHANGED from Round 2

Single authoritative figure: **≤ 32 KB** at `MAX_QUERY_TYPES = 1024`. Tripwire test pinned at ≤ 32 B per slot. With `big_query_table` feature: ≤ 128 KB at 4096 slots.

### 10.4 `QueryDataState<D, F>` layout — UNCHANGED

`UnsafeCell` wrapping adds 0 B (`repr(transparent)`).

### 10.5 `SystemMeta::dummy()` storage — REVISED per W3 (BSS budget tightened to ≤ 320 B with compile tripwire)

```text
static DUMMY: OnceLock<SystemMeta> = OnceLock::new();
```

Storage: process-global `OnceLock<SystemMeta>`. `OnceLock<T>` layout on Rust 1.85: `MaybeUninit<T>` + `AtomicBool` init flag + padding, all aligned to `align_of::<T>()`. `SystemMeta` has `align 32` (the `Access::mask: ComponentMask` field rests on AVX2-friendly alignment), so `OnceLock<SystemMeta>` inherits `align 32`.

Footprint computation:
- `MaybeUninit<SystemMeta>` ≥ 256 B (per Phase 9 `SystemMeta` layout; consumes `Access` ~~ 192 B + name 16 B + 4 generation/tick fields × 8 B = ~240 B rounded to 256 B by 32 B alignment).
- `AtomicBool` init flag: 1 B, padded to maintain `T` alignment → ~32 B once aligned.
- Total: 288-320 B in BSS (worst case 320 B at upper bound of `SystemMeta` size + max alignment padding).

**Revised authoritative figure**: **≤ 320 B in BSS** (W3 fix; up from Round 3 estimate of "~280 B" which underestimated the alignment padding around the atomic init flag).

**Tripwire** (W3): module-scope `const _: () = assert!(core::mem::size_of::<OnceLock<SystemMeta>>() <= 320);` in `system_meta.rs`. Trips compile if a future `SystemMeta` growth or stdlib `OnceLock` layout change exceeds the budget. Failure mode is immediate compile error with the documented context.

One-shot init at first call; immutable thereafter. Result is `&'static SystemMeta`; lives for process lifetime.

### 10.6 Hot loop unchanged

Per profile §1, the 5-instruction inner loop fits <10 bytes of I-cache. NCD6's const-fold removes 1-3 register loads (no I-cache change). Total I-cache footprint of the Track B hot paths fits in 1 cache line.

---

## §11 Tests and validation — REVISED per C-NEW-1 + I-NEW-3 + I-NEW-4 + W2 + W3

### 11.1 Unit tests

| Test name | Module | Verifies |
|-----------|--------|----------|
| `oncelock_query_slot_size_assumptions` (C3) | `query_type_registry::tests` | `size_of::<OnceLock<(NonNull<()>, fn(NonNull<()>))>>() <= 32`. |
| `query_type_id_distinct_under_lto` (I2) | `query_type_registry::tests` | Two distinct (D, F) pairs in separate translation units yield distinct IDs under fat LTO. |
| `system_meta_dummy_lazy_init_returns_stable_address` (C-NEW-1) | `system_meta::tests` | Two calls to `SystemMeta::dummy()` return the same `*const SystemMeta`. |
| `system_meta_dummy_field_values_match_zero_sentinel` (C-NEW-1) | `system_meta::tests` | All fields of dummy match the expected zero/sentinel values. |
| `system_meta_dummy_bss_size_within_budget` (W3 — compile-time) | `system_meta` module scope | `const _: () = assert!(size_of::<OnceLock<SystemMeta>>() <= 320);` fires at compile if exceeded. |
| `query_state_cache_drops_after_arena` (C5) | `ecs_master::tests` | Drop-order test (Drop probe in synthetic D::State; observes ordering). |
| `query_data_no_meta_panic_for_ref` (I4) | `data::tests` | Calling `<Ref<T>>::set_table_readonly_no_meta` panics with expected message. |
| `query_filter_no_meta_panic_for_added` (I4) | `filter::tests` | Same for `Added<C>`. |
| `query_view_send_sync_compile_test` (I-NEW-5 / W1) | `query_view` module scope (not test-gated; single canonical assertion) | `static_assertions::assert_impl_all!(QueryView<'static, (), ()>: Send, Sync)`. Fires every compile. |
| `query_change_detection_panic_smoke` (I-NEW-4 / W4) | `ecs_master::tests` | `world.query::<Ref<TestComp>, ()>()` panics with the canonical W4 message (verbatim string match). |
| `query_warm_path_cache_hit` (QV6) | `ecs_master::tests` | Two calls; second returns cached state pointer. |
| `query_cold_path_alloc_once` (Wave B) | `ecs_master::tests` | Confirm `QueryDataState::new` runs exactly once for repeated calls with same (D, F). |

### 11.2 Integration tests

- `world.query::<&Pos, ()>().iter()` correctness vs `Query<&Pos>` in-system path (same result set).
- `world.query::<&mut Pos, ()>().iter_mut()` writes observable across queries.
- `world.query::<(&Pos, &Vel), ()>().iter()` tuple data correctness.
- `world.query::<&Pos, With<Tag>>().iter()` filter correctness.
- `world.query::<Ref<Pos>, ()>()` PANICS with the I-NEW-4 / W4 canonical message.

### 11.3 Bench targets — REVISED per C-NEW-2 + C-NEW-5 + C6 revisit

| Bench | Target | File |
|-------|--------|------|
| `g2_boyko_query_iter_10k` (updated to `world.query::<&P, ()>().iter()`) | **≥ Bevy parity (within 5% noise floor)** — concretely ≤ ~7.25 µs | `comparison.rs` |
| `g2_bevy_query_iter_10k` (reference) | unchanged ~6.9 µs | same |
| `p2_boyko_direct_api_10k` (NEW, generic `iter`) | ≤ ~7.5 µs | `profile_query.rs` |
| `g1_boyko_50_empty_systems` (regression budget) | ≤ 14.5 µs | `comparison.rs` |
| `g3_boyko_par_iter_10k` (regression budget) | ≤ 41 µs | same |
| `g4_boyko_spawn_10k_commands` (Track A — out of scope here) | tracked by Track A | same |

**No** `p2_boyko_single_read_10k` bench (Opt-B5 dropped per C-NEW-2).

### 11.4 Miri tests — REVISED per I-NEW-3 + W2

| Test | Verifies |
|------|----------|
| `miri_query_cache_lifecycle` | `EcsMaster::new` → `query` → second call → `drop`. No UB, no leak. |
| `miri_query_view_iter_no_provenance_violation` (I1) | `QueryView::iter` under Tree Borrows; UnsafeCell-wrapped state survives sibling `&` reborrows. |
| `miri_query_view_iter_mut_no_provenance_violation` (I1) | Same for `iter_mut`. |
| `miri_query_repeated_calls_no_provenance_violation` (I-NEW-3) | Calls `world.query::<&Pos, ()>()` 1000× in a row under Tree Borrows. Confirms repeated `&mut` retag through `&mut self`'s unique provenance is sound. |
| `miri_query_cache_drops_after_arena_with_arena_derived_d_state` (C5) | Synthetic `D::State` carrying an arena raw pointer; verify drop order does NOT use-after-free under the new field order (or fails the Miri trip in a synthetic test if a future regression places the cache before arena). |
| `miri_oncelock_set_no_double_free` | Cache slot `set` followed by drop frees the Box exactly once. |
| `miri_system_meta_dummy_lazy_init` (REVISED per W2 to **sequential pointer-stability loop**) | Single-threaded 1000× loop: `let p0 = SystemMeta::dummy() as *const SystemMeta; for _ in 0..1000 { assert_eq!(p0, SystemMeta::dummy() as *const SystemMeta); }`. Confirms OnceLock-backed `dummy()` returns a stable `'static` reference across repeated calls. **No `std::thread::scope`** (incompatible with Phase 9.1 deferral — multi-thread Miri trip on `Scope::spawn` protected-tag conflict). The cross-thread CAS soundness of `OnceLock` itself is covered by stdlib's own loom tests; Track B's invariant is pointer stability, which the sequential test exercises faithfully. |
| `miri_no_dangling_borrow_in_query_then_drop` | Hold a `QueryView`, drop the EcsMaster — borrow checker prevents (compile-time check). |

### 11.5 Mandatory `debug_assert!` sites

```rust
// In EcsMaster::query<D, F>:
debug_assert!(type_id.0 < MAX_QUERY_TYPES);

// In OnceLock::set Err arm (per O3 from Round 1):
debug_assert!(false, "OnceLock::set raced under &mut self — impossible");
// followed by panic! (release-build trip)

// In QueryView::single:
debug_assert!(count <= 1, "QueryView::single called on query yielding {} rows", count);
```

(No `combined_generation_snapshot` debug_assert — Opt-B4 dropped.)

### 11.6 Compile-time tripwires

```rust
// W3 — in system_meta.rs (module scope):
const _: () = assert!(
    core::mem::size_of::<std::sync::OnceLock<SystemMeta>>() <= 320,
    "SystemMeta::dummy() BSS footprint exceeded 320 B budget"
);

// QC8 — in query_type_registry.rs (test fn — compile-time-evaluable but registered as test):
#[test]
fn oncelock_query_slot_size_assumptions() {
    assert!(size_of::<OnceLock<(NonNull<()>, fn(NonNull<()>))>>() <= 32);
}
```

---

## §12 Defending against the brief's "look HARD" critic points

Per Round 1 §12 — defences unchanged. All Round 2 findings (C1, C2, C5, I1-I5) addressed; Round 3 findings (C-NEW-1 through C-NEW-5, C6 revisit, I-NEW-1 through I-NEW-5) and Round 4 wording fixes (W1-W4) explicitly addressed in §0 Round 4 Changelog + §0 Round 3 Changelog and throughout the plan.

---

## §13 Risk register — REVISED per Round 3 + Round 4 findings

| Risk | Probability | Severity | Mitigation |
|------|-------------|----------|------------|
| C-NEW-1 fallback `OnceLock<SystemMeta>` adds measurable overhead on hot path | L | L | Hot path is amortised at ~1-2 ns per call after first init; budget §1.2 revised honestly to 5 ns. If overhead shows >2 ns in asm/bench, Phase 13 const-fn redesign of `BitSet`. |
| W3 `OnceLock<SystemMeta>` BSS exceeds 320 B (e.g., future `SystemMeta` growth) | L | L | Compile-time tripwire `const _: () = assert!(size_of::<OnceLock<SystemMeta>>() <= 320)` in `system_meta.rs` fails build immediately with documented context. |
| C5 drop ordering: future `D::State` holds arena pointer | L | H | C5 fix (cache drops AFTER arena) inverts failure to immediate Miri trip. Test `miri_query_cache_drops_after_arena_with_arena_derived_d_state` guards. |
| C6 amended gate `≥ Bevy parity` not met | M | M | Honest residual recorded in `docs/PHASE-12.5-RESULTS-INTERIM-B.md`; Phase 13 levers (sparse-set storage, PGO, allocator tuning) deferred per umbrella amendment. |
| I1 Tree Borrows trip on UnsafeCell pattern | L | M | Miri tests on both SB + TB, incl. I-NEW-3 1000× repeated-call test. |
| I2 LTO collapses two distinct QueryTypeIds | L | M | Test `query_type_id_distinct_under_lto`; Phase 8.5's identical pattern LTO-verified. |
| MAX_QUERY_TYPES = 1024 insufficient | L | L | I5 cargo feature `big_query_table` raises to 4096. |
| Const-fold `if const { ... }` not eliminated by LLVM | L | M | Verified in Phase 8b (`if !const { F::IS_ARCHETYPAL }`); same Rust 2024 / 1.85+ constraint. |
| I-NEW-4 / W4 panic fires unexpectedly in user code | L | L | Documented in `EcsMaster::query` doc-comment with W4 canonical message; users with change detection redirect to `Schedule`. Phase 13 may relax. |
| W2 sequential Miri test misses cross-thread CAS bugs | L | L | stdlib's own loom tests on `OnceLock` cover cross-thread CAS. Track B's invariant is pointer stability across calls; sequential test exercises exactly that. Multi-thread Miri remains deferred to Phase 9.1 (`Scope::spawn` Tree Borrows protected-tag conflict). |
| W1 single canonical `assert_impl_all` masks a regression on a non-`()` `D::State` | L | L | Trait bound `D::State: Send + Sync, F::State: Send + Sync` at `data.rs:90` is the universally-required ground truth; the `()` assertion exercises the same structural geometry. Per-impl Send/Sync regressions surface at the `Query<'w, 's, D, F>` SystemParam construction site. |
| LOC budget 1500-2000 underestimate | L | L | Phase 8.5 anchor; mechanical edits dominate. If exceeded, follow-up cleanup PR. |
| Bevy 0.18 baseline changes | L | L | Pinned `=0.18.1` per umbrella. |

---

## §14 Approval checklist

- [ ] Step 0 (`SystemMeta::dummy()` lazy accessor + W3 BSS tripwire) completed and tested.
- [ ] Wave A, B, C, D Steps completed in dependency order.
- [ ] **No Wave E** (Opt-B5 dropped).
- [ ] **No Step D4** (Opt-B4 dropped).
- [ ] All 78 trait-impl edits include both `NEEDS_CHANGE_DETECTION` const and both `_no_meta` method bodies (NO defaults).
- [ ] All Round 1 + Round 2 + Round 3 + Round 4 Miri tests pass (incl. W2 sequential 1000× pointer-stability loop).
- [ ] **Single** module-scope `static_assertions::assert_impl_all!(QueryView<'static, (), ()>: Send, Sync)` in `query_view.rs` (NOT under `cfg(test)`; no gated alternative — W1 fold).
- [ ] **W3** compile-time tripwire `const _: () = assert!(size_of::<OnceLock<SystemMeta>>() <= 320);` present in `system_meta.rs`.
- [ ] **W4** panic message verbatim canonical across §0 I-NEW-4 / §2.2 QV11 / §4.3 doc-comment / §5 implementation: `direct API EcsMaster::query<{D}, {F}>() does not support change-detection filters (D or F has NEEDS_CHANGE_DETECTION = true); use Query<D, F> inside a system body via Schedule`.
- [ ] Bench `g2_boyko_query_iter_10k` ≥ Bevy parity (within 5% noise).
- [ ] No regression on `g1_boyko_50_empty_systems` and `g3_boyko_par_iter_10k`.
- [ ] Umbrella amendment commit landed (orchestrator commits to `PHASE-12.5-SURPASS-BEVY-PLAN.md` separately).
- [ ] `docs/PHASE-12.5-QUERY-ASM-CHECK.md` documents inner-loop asm equivalence.

---

## §15 Open questions and Phase 13 follow-ups

### OQ-1 (CRITICAL from C6 revisit): Umbrella amendment

The plan amends the umbrella's `boyko ≥ 1.10× bevy` criterion specifically for `g2_boyko_query_iter_10k` to `boyko ≥ Bevy parity (within 5% noise floor)`. Rationale: profile proves inner loop is byte-identical to Bevy (PHASE-12.5-PROFILE-QUERY.md §1 "the per-row codegen is on par; the ~1 µs delta lives outside the inner loop"). The amendment is non-negotiable — Round 2's invented levers (Opt-B4 atomic-fusion, Opt-B5 single-read specialisation) proved fictional under audit. **The orchestrator must propagate this amendment to `PHASE-12.5-SURPASS-BEVY-PLAN.md` (see "Proposed umbrella amendment" block at end of this document).**

1.10× Bevy on this bench is filed as Phase 13. Levers for Phase 13:
- Sparse-set storage option for high-fanout components (Bevy's hybrid Table+SparseSet).
- PGO build with profile collection on the bench workload.
- Allocator tuning (jemalloc / mimalloc benchmark).
- Inner-loop redesign (SIMD-batched fetch; gather-load avoidance).

### OQ-2 (deferred from Round 1): `query_ref<&self>` Phase 13 audit

After dropping `query_ref<&self>` per C2, the next step is to audit `QueryDataState::new` for `&self`-compatibility. Phase 13 work item.

### OQ-3 (REVISED per C-NEW-1): Const-fn migration of `BitSet::new` — DROPPED for this phase

Not pursued. `T::default()` is non-const trait method; `const_trait_impl` unstable. If a future Rust version stabilises `const_trait_impl` or if Phase 13+ wants to revisit, the path becomes mechanical. For now, `OnceLock<SystemMeta>` is the fallback (~50 ns first-call, ~1-2 ns warm). Filed.

### OQ-4: `MAX_QUERY_TYPES = 1024` headroom

Default 1024 + cargo feature `big_query_table` for 4096. Critic should confirm feature gate acceptable.

### OQ-5: Per-archetype access narrowing for direct API — deferred to Phase 13.

### OQ-6: Direct API change-detection — Phase 13

Per I-NEW-4 (b) + W4 canonical wording, v1 panics. Phase 13 work item: thread an `Option<(Tick, Tick)>` argument or per-(D, F) `last_run` cache to enable `world.query::<Ref<T>, ()>()` without `Schedule`. Requires deciding the `last_run` semantics outside a schedule context.

### OQ-7 (NEW per C-NEW-3): `||` short-circuit branchless cleanup

If `cargo asm` dump of `QueryDataState::update` shows a branchy `||` compile (two compare-jump pairs), replace with `let dirty = (pre_arch != cur_arch) | (pre_struct != cur_struct);` (bitwise OR). Pure-cleanup item; zero claimed budget. Filed as Phase 13 micro-cleanup.

### OQ-8 (NEW per C-NEW-2): Single-component specialisation exploration

Filed as Phase 13 exploratory work. If profile measurements at >100 archetypes show outer-loop boundary cost dominating (currently single-archetype workload masks this), a `SingleReadIter`-style specialisation may pay off — but requires real data, not the unverified asm-saving claim from Round 2.

---

## §16 Reference file paths

- `D:\claude\BoykoEngine\docs\PHASE-12.5-CRITIC-B-ROUND-1.md` — Round 1 critic findings.
- `D:\claude\BoykoEngine\docs\PHASE-12.5-CRITIC-B-ROUND-2.md` — Round 2 critic findings (Round 3 input).
- `D:\claude\BoykoEngine\docs\PHASE-12.5-CRITIC-B-ROUND-3.md` — Round 3 critic findings (Round 4 input: W1-W4 wording fixes).
- `D:\claude\BoykoEngine\docs\PHASE-12.5-SURPASS-BEVY-PLAN.md` — umbrella; success-criterion amendment proposed at end of this doc.
- `D:\claude\BoykoEngine\docs\PHASE-12.5-PROFILE-QUERY.md` — profile anchor.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs` — Send/Sync 1501-1502; field order 82-169; `bundle_archetype_cache` C6 pin 111-134.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\state.rs` — `QueryDataState::new` line 69 (C2 anchor); `update` 185-198.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\data.rs` — real `set_table_readonly` body 297-310; tuple macro 1054.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\system_meta.rs` — `SystemMeta::dummy()` target + W3 BSS tripwire.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\access.rs` — `Access::new` at line 69 (NOT migrated to const fn per C-NEW-1).
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\component\component_mask.rs` — `ComponentMask::new` (NOT migrated to const fn per C-NEW-1).
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\archetype\archetype.rs` — real column access `columns.get_unchecked(state.id.0).ptr`; no `column_raw_ptr` method.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\archetype\archetype_master.rs` — `generation` 37, `structural_generation` 54 (NON-ATOMIC); accessors 413/423.
- `D:\claude\BoykoEngine\crates\boyko_utils\src\bit_mask\bit_set.rs` — `BitInteger` trait 6-10, `BitSet::new` 87 (NOT migrated per C-NEW-1).
- `D:\claude\BoykoEngine\crates\boyko_utils\src\bit_mask\bit_set_256.rs` — `new()` already `const fn` at line 33.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\bundle\bundle_type_registry.rs` — Phase 8.5 template; `oncelock_size_assumptions` 286-310.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\unsafe_ecs_cell.rs` — Send/Sync 341-342.

---

**End of Phase 12.5 Track B Plan, Round 4.**

---

# Proposed umbrella amendment (orchestrator commits separately)

Insert into `D:\claude\BoykoEngine\docs\PHASE-12.5-SURPASS-BEVY-PLAN.md` §C1 (success criteria), as a sub-bullet under `g2_boyko_query_iter_10k`:

> **Amendment (Phase 12.5 Track B Round 3+4)**: success criterion for `g2_boyko_query_iter_10k` is amended from `boyko ≥ 1.10× bevy` to `boyko ≥ Bevy parity (within 5% noise floor)`. Justification: profile asm dump (PHASE-12.5-PROFILE-QUERY.md §1) shows the per-row inner loop is byte-identical to Bevy; the existing 0.88× loss lives entirely in the `FunctionSystem` wrapper and `&SystemMeta` plumbing. Track B closes that loss; surpassing Bevy by 10% requires Phase 13+ work (sparse-set storage, PGO, allocator tuning, SIMD-batched fetch). All other umbrella criteria for Track B benchmarks remain `boyko ≥ 1.10× bevy` (see `g1_boyko_50_empty_systems`, `g3_boyko_par_iter_10k`, `g4_boyko_spawn_10k_commands` for the unchanged criteria).

---

# Brief Summary (Round 4)

## W1-W4 wording fix folds

- **W1 — §4.3 Send/Sync assertion contradiction resolved**: the gated `#[cfg(any(test, doctest))] mod _send_sync_check` block with `_DummyComponent` stub is **removed entirely**. Only the single canonical `static_assertions::assert_impl_all!(QueryView<'static, (), ()>: Send, Sync)` at module scope (NOT under `cfg`) remains. Now fires on every compile (debug, release, doctest, test), exactly as I-NEW-5 mandated. Approval checklist updated to require "single" assertion (W1 enforcement).

- **W2 — `miri_system_meta_dummy_lazy_init` execution mode pinned to sequential pointer-stability loop**: changed from "Concurrent calls to `SystemMeta::dummy()` from multiple threads (via `std::thread::scope`)" to "Single-threaded 1000× loop asserting pointer stability via `assert_eq!(p0, SystemMeta::dummy() as *const SystemMeta)` per iteration". Avoids the Phase 9.1 `Scope::spawn` Tree Borrows protected-tag conflict that would defeat multi-thread Miri coverage. Risk-register row added documenting cross-thread CAS coverage is provided by stdlib loom tests, not Track B's responsibility.

- **W3 — §10.5 BSS footprint tightened to ≤ 320 B with compile-time tripwire**: revised wording from "~280 B" to "≤ 320 B in BSS" (worst case with alignment padding around the `AtomicBool` init flag, since `SystemMeta` carries `align 32`). Added module-scope `const _: () = assert!(core::mem::size_of::<OnceLock<SystemMeta>>() <= 320);` tripwire in `system_meta.rs`. Step 0a / Approval Checklist / Risk Register updated to require the tripwire.

- **W4 — Panic message wording unified across §2.2 QV11, §0 I-NEW-4, §4.3 doc-comment, §5 implementation**: canonical wording is now `direct API EcsMaster::query<{D}, {F}>() does not support change-detection filters (D or F has NEEDS_CHANGE_DETECTION = true); use Query<D, F> inside a system body via Schedule` (with `{D}`/`{F}` substituted via `std::any::type_name`). §2.2 QV11 quotes the §5 implementation verbatim. Integration test `query_change_detection_panic_smoke` verifies the canonical message via verbatim string match. Approval checklist W4 row added.

## What did NOT change (architectural surface preserved)

- All Round 1 → Round 3 resolutions stand (C1-C6, C-NEW-1 through C-NEW-5, C6 revisit, I1-I5, I-NEW-1 through I-NEW-5).
- Two adopted optimisations preserved: **Opt-B1** (direct query API + cache) and **Opt-B2** (`NEEDS_CHANGE_DETECTION` const).
- **Opt-B4** and **Opt-B5** stay DROPPED per C-NEW-3 / C-NEW-2.
- Umbrella amendment for `g2_boyko_query_iter_10k` to `≥ Bevy parity (5% noise)` stays.
- `OnceLock<SystemMeta>` lazy accessor for the C4 fallback stays (W3 only tightens its footprint budget).
- 12 Steps (1 prerequisite + 11 main); 4-6 calendar days; LOC budget 1500-2000 production + 400-600 test — all preserved.

## Relevant file paths

- `D:\claude\BoykoEngine\docs\PHASE-12.5-QUERY-OPTIMIZATIONS-PLAN.md` (target — full revised Round 4 plan above)
- `D:\claude\BoykoEngine\docs\PHASE-12.5-CRITIC-B-ROUND-3.md` (input — W1-W4 findings)
- `D:\claude\BoykoEngine\docs\PHASE-12.5-CRITIC-B-ROUND-2.md` (historical input)
- `D:\claude\BoykoEngine\docs\PHASE-12.5-CRITIC-B-ROUND-1.md` (historical input)
- `D:\claude\BoykoEngine\docs\PHASE-12.5-SURPASS-BEVY-PLAN.md` (umbrella; W4-folded amendment block proposed at end of this doc)
- `D:\claude\BoykoEngine\docs\PHASE-12.5-PROFILE-QUERY.md` (profile anchor for C6 revisit)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs` (Send/Sync 1501-1502; field order 82-169; bundle_archetype_cache C6 pin 111-134; `query` site + W4 canonical panic site)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\state.rs` (QueryDataState::new at 69; update 185-198)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\data.rs` (real set_table_readonly body 297-310; tuple macro 1054)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\query_view.rs` (NEW — W1 single canonical Send/Sync assertion)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\system_meta.rs` (SystemMeta::dummy() target + W3 BSS tripwire)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\archetype\archetype_master.rs` (generation 37, structural_generation 54 — NON-ATOMIC; accessors 413/423)
- `D:\claude\BoykoEngine\crates\boyko_utils\src\bit_mask\bit_set.rs` (BitSet::new at 87 — NOT migrated per C-NEW-1)
- `D:\claude\BoykoEngine\crates\boyko_utils\src\bit_mask\bit_set_256.rs` (new already const fn at 33)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\unsafe_ecs_cell.rs` (Send/Sync 341-342)
