> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 13+ Roadmap

After closing Phase 12.5 + Phase 12.6, the boyko-engine ECS is in a state
where remaining performance gaps can be addressed via **focused refactors**
of specific subsystems — no full architectural redesign is required. This
document captures the strategic plan for the next iterations.

## Current state (post Phase 12.6)

**3 of 4 head-to-head benches** beat Bevy by ≥1.10×:
- 50 empty systems: **1.72× win**.
- par_iter 10k: **2.93× win**.
- query iter (direct API): **1.00× parity** (closed from 0.88× loss).
- spawn_batch warm path: **1.35× of Bevy** (close to parity from 11× regression).

`EcsMaster::new`: **9-31× improvement** (712 µs → 23-75 µs) via lazy
allocation; remaining 23-75 µs is Arena 64 MB heap commit.

**667/667 tests pass**. Build + clippy clean. Phase 11 EntityCommands chaining,
Phase 12.5 panic-recovery semantics, Phase 12.5 NCD6 const-fold all intact.

## Architectural assessment

The current architecture is sound:
- **Archetype + chunked component pool** storage (matches Bevy/flecs leading practice).
- **Atomic counter Entity ID** allocation (Phase 11 EntityCounter newtype).
- **Per-row Tick storage** with `Box<[UnsafeCell<Tick>]>` (Phase 10 change detection).
- **Bevy-class parallel scheduler** with Chase-Lev work-stealing, conflict graph,
  Tarjan SCC, Kahn topological sort, apply-window barrier (Phase 9).
- **Static Bundle Cache** via `Box<[OnceLock<ArchetypeId>; 1024]>` (Phase 8.5).
- **CommandQueue** with hoisted `catch_unwind` + `CursorSync` RAII (Phase 12.5 + 12.6).
- **Events as SystemParam** with cached `NonNull<EventBuffer<E>>` (Phase 12).

None of these subsystems need fundamental rework. All remaining performance
residuals can be closed by **targeted changes** to specific subsystems.

## Feature roadmap (additive, non-disruptive)

Each of these can be implemented WITHOUT disturbing existing perf wins.
They are also independent of each other — any order is workable.

### Phase 13 — `Local<T>` SystemParam — ✅ DONE

Per-system private state slot. **Landed** (commits `f6b4807` impl+wiring,
`1db3e5c` tests). `Local<'s, T>` (`T: Send + Sync + Default + 'static`),
`#[repr(transparent)]` over `&'s mut T`, `type State = T` living in
`FunctionSystem::state` (NOT `SystemMeta` — the original roadmap phrasing
was loose), default-initialized once and persisted across runs of a cached
system. Declares zero access → no conflict-graph edge → never blocks
parallelism. A strict structural subset of the Phase 12 `EventReader`
(no cached pointer, no `unsafe` block inside methods); critic round skipped
on that basis. Plan: `docs/PHASE-13-LOCAL-PLAN.md`; research:
`docs/PHASE-13-RESEARCH.md`. 5 integration tests + 4 unit + 1 trybuild,
Miri 5/5 clean, full suite 668 pass. Decisions: A1 (`State = T`, no
`SyncCell`), B1 (`Default`, `FromWorld` deferred — backward-compatible
widening). Reachable at `boyko_ecs::ecs::core::system::Local`.

### Phase 14a — Component lifecycle hooks — ✅ DONE
Component `on_add` / `on_insert` / `on_replace` / `on_remove` callbacks fired
at the 6 structural-op sites, gated by a per-archetype `ArchetypeFlags(u16)`
bit-test (**0% measurable cost when no hook is registered** — bench-verified
via clean A/B vs `b223350`). Read-only `DeferredEcsMaster` hook view + deferred
structural-command channel drained at the outermost apply boundary via a
**thread-local reentrancy depth counter**. Derive `#[component(on_add = f, …)]`
XOR runtime `register_component_hooks::<C>()`. `#[cold] #[inline(never)]`
dispatch fns per the hot-path discipline below. See `docs/PHASE-14-RESULTS.md`.

Miri found + fixed two soundness bugs paper review missed (F1 drain
re-entrancy double-apply; F2 `DeferredScopeGuard` Tree-Borrows UB → moved the
depth counter to a thread-local). One pre-existing, non-14a TB finding (F4 —
`EntityInland` `as_mut_ptr()` slab storage) documented + deferred
(`docs/PHASE-14-F4-FINDING.md`).

**Deferred to Phase 14b:** `on_despawn` (entity-level, distinct from
`on_remove`); full Observers (entity-targeted, `CachedObservers`, custom
events); mutable component access in the hook view (`get_component_mut`);
derive+runtime *merge* (vs the current XOR).

### Phase 15 — Schedule sets / system orderings — ✅ DONE
Bevy-style `before` / `after` / `in_set` + set-level (`configure_set`) +
set-hierarchy ordering, with `#[derive(SystemSet)]` enum support and
`try_build`/`build` diagnostics. Completed Phase 9's DORMANT scaffold
(`OrderingEdge`/`SystemKey`/Tarjan-SCC/Kahn were already present + tested —
this finished the deferred "Wave 5 Step 14"). Build-time edge expansion +
set-hierarchy flatten feed the EXISTING `ConflictGraph` + Kahn pipeline; the
executor hot path is byte-identical (0%-regression verified on the 50-systems
bench). No `unsafe` added. See `docs/PHASE-15-RESULTS.md`.

Deferred: auto sync-point coalescing, `before_ignore_deferred`, dropping the
redundant conflict bit for pure ordering edges (all parallelism micro-opts,
benchmark-gated, would touch the 0%-protected executor).

### Phase 16 — Run conditions — ✅ DONE
`.run_if(cond)` on systems + sets; a condition is any `fn(SystemParams…) -> bool`
(`impl IntoSystem<(), bool, M>`). Built-in `run_once`. Conditions evaluated
single-threaded at the apply-window boundary (`running==0` ⟹ race-free); a false
fold skips the body but its `before` successors still run; set conditions
evaluated once/frame; eager fold (no short-circuit). **0%-regression verified**
(new code in a separate `evaluate_ready_conditions` pass gated by a
`has_condition` bitset; `try_dispatch_ready` + `SystemBox` byte-identical).
`run_condition` does NOT call `apply` (pure predicates). See
`docs/PHASE-16-RESULTS.md`.

Deferred: `resource_exists` (needs `Option<Res>` param), typed combinators
(`.and`/`.or`/`.not`), `on_event`, tick-aware conditions (`Changed`/`Added` —
Phase 16.1), `in_state` (→ Phase 17).

### Phase 17 — States / state transitions — ✅ DONE
Tagged enum states (`States` trait), `State<S>` / `NextState<S>` resources,
`in_state` / `on_enter` / `on_exit` / `on_transition` run conditions composing
with Phase 15 / 16. Implemented the boyko-native **shape (b)** — enter/exit are
ordinary condition-gated systems fed by a built-in per-frame transition pass —
rather than Bevy's value-keyed sub-schedules (boyko has one schedule). **0%-gate
held** (transition pass gated by `state_entries.is_empty()`, executor byte-
identical, 50-systems bench "no change"). **Zero new `unsafe`**. The generic
`State<S>` resource ids go through a `TypeId`-keyed registry (the rust#22991
static-collapse trap — `#[derive(Resource)]` would alias every `S`). 814 tests +
Miri (TB) 21 clean. **F1 (tester-caught CRITICAL)**: conditions as `impl FnMut`
don't compile through `.run_if` (opaque return drops the double-`FnMut` HRTB
bound `SystemParamFunction` needs) → fix = `impl System<Out=bool>` + the
anticipated IS2 identity `IntoSystem` blanket. See `docs/PHASE-17-RESULTS.md`.

Deferred: value-keyed sub-schedules, computed/sub-states, state-scoped
auto-despawn, `StateTransitionEvent`, `#[derive(States)]`, `Option<Res<R>>`.

### Phase 18 — Plugin system — ✅ DONE
`App` builder facade + `Plugin` trait (`build(&self, &mut App)`; `'static` only —
**no `Send+Sync`**, plugins are consumed-at-build) + `add_plugin` / `add_plugins((..))`
(sealed `Plugins<Marker>` tuple, 1..=12, nesting) + duplicate-panic (`boyko-B1801`,
cold `Vec<TypeId>`) + `boyko_ecs::prelude` (public **types**; derive re-exports
deferred on the `boyko-macros` dev-dep cycle). **Q7 = single schedule** + a one-shot
startup list run before the loop (not a second schedule, not a label map). `App` owns
the pool (E5: `new`/`with_threads`/`with_pool`). **0%-gate held** — App `run_n` vs raw
`Schedule::run` = +1.17% / −0.79% (sign-flip noise); the `run_n`/`run` loop binds
`schedule`+`world` locals once → branch-free. **No ECS core changes** (7 lines
touched). Zero `unsafe`. 14 tests + proptest; 829 suite pass. See
`docs/PHASE-18-RESULTS.md`.

Deferred: SubApps/render-world; Plugin `finish`/`cleanup`/`ready`;
`PluginGroup`/`DefaultPlugins`; multi-schedule label map; `init_resource`/`FromWorld`;
`set_runner`; the single-dep prelude-with-derives (needs the `boyko-macros` path-
resolution / cycle refactor); the `boyko_demo` port to `App` (a restructure; the
demo's wasm/no-pool path can't use the native-multithreaded `App`).

### Phase 19 — Hierarchies / Parent-Child — ✅ DONE
`ChildOf` / `Children` (Bevy-0.16 model) on the Phase 14a/14b hooks;
default-recursive despawn cascade; 1-field Bundle newtypes reuse
`migrate_entity_insert`. Landed `5f536bc` + TB-UB fix `670b8ca`
(pre-existing `command_queue.rs apply_via_raw_twin` re-entrant-drain bug the
hierarchy cascade was the first workload to expose). See
[PHASE-19-RESULTS.md](PHASE-19-RESULTS.md).

**Roadmap status: every feature phase (13-19) and every perf-polish phase
(X.A-X.E) above is DONE.** Follow-up correctness phases landed since: 16.1
(tick-aware run conditions, [PHASE-16.1-RESULTS.md](PHASE-16.1-RESULTS.md)).

## Performance polish (focused refactors, interleavable with features)

Each one is a **targeted refactor of one subsystem**. They do NOT require
architectural changes and can land at any time between feature phases.
None of them blocks each other.

### Phase X.A — `Query::for_each_chunk` batched API — ✅ API SHIPPED; ≥1.10× goal MEASURED MARGINAL

**Goal**: close Phase 12.6 Residual 2 (query iter ≥1.10× Bevy).

**Status**: the **API shipped** (`Query::for_each_chunk` exists + the `g6`/`g6b`
nightly `algebraic_add` + sink-only-`black_box` bench harness exists; `iter()` /
`iter_mut()` untouched, no API break). It is a real differentiator — Bevy never
shipped a batched API ([issue #1990](https://github.com/bevyengine/bevy/issues/1990)
open since 2021); flecs' C API works exactly like this. A polish backlog remains
(post-landing critic R3 — see task tracker).

**Grounding measurement (Phase X.E methodology — nightly + `--features bench-alloc`,
3 paired runs, `g6`/`g6b` 10k):**
- **Single-component** (`g6`): boyko `for_each_chunk` ≈ 999 ns vs Bevy `iter().fold` ≈
  971 ns → **parity** (~3% slower, in noise). Both vectorize the scalar inner loop
  identically → no API delta to expose, exactly as predicted.
- **Multi-component** (`g6b` 3-tuple — the surpass surface): boyko `triple_idx`
  vs Bevy `triple`, paired per run: **+9.6% / −6.5% / +8.3%** (boyko faster in
  2 of 3). The predicted "Bevy pays 3 column cursors per `next()`, chunked slices
  fuse the loads" effect IS real, but the margin is **~8% and noisy (±10%)**, NOT
  a robust ≥1.10×. The `idx` inner-loop shape beats `zip` (inner loop matters).

**Verdict**: boyko is **competitive-to-slightly-ahead** of Bevy on chunked
iteration (a genuine, shippable differentiator), but a **robust, claimable
≥1.10× surpass is NOT currently achievable** — single-component is hard-parity
(byte-identical asm), and multi-component sits at ~8% inside the noise band.
**Deprioritized as a "trophy" chase**: the remaining gap to a defensible ≥1.10×
is high-effort (variance-tightening + micro-opt of the multi-component fetch) for
uncertain payoff on an already-winning path. Revisit only with a dedicated
low-noise harness (longer measurement + `critcmp` median-of-N) if the ≥1.10×
claim becomes a priority. The API itself stands as delivered.

### Phase X.B — `ComponentPool::Vec<Unit>` parallel storage elimination — ✅ DONE

**Result**: removed the per-row `Vec<Unit>` (cached `*mut u8`); compute `row_ptr(i) =
buffer.add(i*stride)` from the stable arena base + an explicit `len`. Behavior-preserving
(`units[i].ptr() ≡ buffer.add(i*stride)`; even `swap_remove`'s `Unit::new` rewrite was a self-assign
NO-OP). **Net-removes `unsafe`** (the `commit_units` raw-Vec-spare-capacity loop gone). Deleted
`chunk_units` (zero callers) + `Unit`/`id_unit.rs`. **Spawn measurably faster** (git-stash A/B,
p=0.00: +88.8% @100, +6.5% @10k; spawn_batch_10k +9.5%); **iteration 0%** (hot paths use
`column.ptr.add`, untouched); **Miri-clean** (surface shrank); 17/17 pool tests + 2 proptests; 503
suite. Memory saving 8 B/row + one heap alloc/pool (roadmap's "24 B / 5-10 ns" was stale — `Unit`
was already `#[repr(transparent)]` 8 B). All pub signatures unchanged (zero external-caller edits).
See `docs/PHASE-XB-RESULTS.md`.

**Goal (original)**: close Phase 12.6 Residual 3 (Commands::spawn single 3× slower).

**Current**: every `ComponentPool` maintains a `Vec<Unit>` parallel to the
component data buffer. `units[i].ptr()` returns the byte pointer for row
`i`. This costs ~5-10 ns/entity on the spawn hot path (push + cache line touch).

**Refactor**: compute `buffer.ptr() + i * stride` on every random-access
read. Eliminates the parallel `Vec<Unit>` entirely.

**Scope**:
- `ComponentPool::get_raw` / `get_typed_at` / `set_component` / `swap_remove`
  — change `units[i].ptr()` → `unsafe { buffer.ptr().add(i * stride) }`.
- ~10 files touched, all inside the `ecs/memory/component_pool.rs` neighbourhood.
- Phase 11 `swap_remove_index_no_drop` may also benefit.

**Expected gain**: ~5-10 ns/entity on Commands::spawn hot path. Will also
reduce ComponentPool memory footprint by ~24 B/row.

**Estimated cost**: 1-2 weeks (cross-cut on read paths needs careful audit).

### Phase X.C — Arena `VirtualAlloc(MEM_RESERVE)` — ✅ DONE

**Result**: `Arena::new` **1.10 µs** (was the dominant chunk of the ~23-75 µs residual; ≈20-70×);
`EcsMaster::new` ~23-75 µs → **7.23 µs** (~3-10×; the remaining ~6 µs is non-Arena init, out of
scope). `VirtualAlloc(MEM_RESERVE|MEM_COMMIT)` / `mmap(MAP_PRIVATE|MAP_ANONYMOUS)` demand-zero
backing, `cfg(any(miri, not(any(windows,unix))))` fallback to global alloc (Miri + wasm). Hot path
byte-identical (0%-gate); Miri fallback 3/3; 13 arena tests; M-001 matching-deallocator preserved;
`libc` target-gated to `cfg(unix)` (already in lock). See `docs/PHASE-XC-RESULTS.md`.

**Goal (original)**: close Phase 12.6 `EcsMaster::new` residual 23-75 µs.

**Current**: `Arena::with_capacity(64 MB)` eagerly commits 64 MB via
global allocator on world creation. Most of that is never used.

**Refactor**: switch to `VirtualAlloc(MEM_RESERVE)` on Windows /
`mmap(MAP_NORESERVE, PROT_NONE)` on Linux. Reserve virtual address range
without committing physical pages. Pages commit lazily on first write
(OS page-fault handler).

**Scope**:
- `crates/boyko_ecs/src/ecs/memory/arena.rs` only.
- Two cfg-gated impls: Windows + Unix.

**Expected gain**: `EcsMaster::new` drops to ≤ 5 µs (just the field
initialization + a few `mmap`/`VirtualAlloc` syscalls).

**Estimated cost**: 3-5 days.

### Phase X.D — `EntityMaster` slot reduction ✅ DONE

**Goal**: close another part of Phase 12.6 Residual 3.

**Outcome**: the roadmap's "speculative / despawn-invariant-rework / 1-2 weeks"
framing was **inverted by investigation**. `iter_entities` has **zero
hot-path callers** (systems iterate via `Query`/archetypes), so `active_ids`
(dense live list) and `sparse_to_active` (sparse→dense map) were an
acceleration structure serving only a cold API. The winning move was not to
make `sparse_to_active` lazy but to **delete both fields** and replace them
with a single `live_count: usize` — which *deletes* the despawn swap-remove
"invariant" rather than reworking it.

- `entities_inland` (queries / fast-path lookup) — REQUIRED, retained.
- `active_ids` + `sparse_to_active` — **DELETED**.
- `iter_entities` → O(capacity) scan of `entities_inland` (ascending id).

**Measured** (rigorous git-stash A/B; full architect→critic→dev→review→tester
pipeline, Miri TB-clean, 845 tests):
- **Despawn `delete_entity_10k` −7.65% (p=0.00)** — clean win (shed the
  swap-remove + sparse fix-up + a branch per despawn).
- Single `create_entity_10k` −1.38% (p=0.05); batch-spawn parity (dominated
  by component byte-copy).
- **−12 B/entity** resident + **−2 heap allocs/world**; **net-removes
  `unsafe`** (deleted the `register_batch` tandem-slice write + swap-remove
  fix-up); smaller `Send`/`Sync` surface.
- Hot read path **0%** (touches only `entities_inland`, by construction).
- **Cost (documented)**: `iter_entities` regresses — dense ~2×, sparse
  ~97× (O(active)→O(capacity)) — entirely on the zero-hot-caller cold API.
  If a hot dense-enumeration consumer ever emerges, walk archetype
  `entity_ids` rows (already dense + co-located with components); do NOT
  reintroduce `active_ids`.

See `docs/PHASE-XD-RESULTS.md`.

### Phase X.E — Multi-run bench methodology ✅ DONE

**Goal**: extract structural perf signals from per-iter `EcsMaster::new`
allocator variance (±20-30% on g4 / g5 benches).

**Shipped** (bench tooling only — zero engine changes):
- **`[profile.bench] codegen-units = 1`** — deterministic codegen (hardens the
  0%-gate / byte-identical-asm methodology). Not `lto` (Bevy compile cost +
  shifts mean not variance).
- **Opt-in mimalloc** behind a `bench-alloc` feature (OFF by default). Default
  `cargo bench` = system heap (production-honest absolutes); `--features
  bench-alloc` = low-variance signal. **Measured: −17% mean + ~2× tighter
  run-to-run spread** on an allocator-touching bench (`create_entity_10k`).
- **`bench.ps1`** (median-of-N: High priority + affinity-pinned cargo child +
  per-run criterion baselines for `critcmp`) + **`docs/BENCHMARKING.md`** (full
  A/B + median-of-N protocol, manual stabilization, read-only-vs-destructive
  bench discipline).

**Deferred** (documented in `BENCHMARKING.md`): PGO (shifts mean not variance),
iai-callgrind (Valgrind → no Windows), `[profile.bench] lto` (Bevy compile cost).

See `docs/PHASE-XE-RESULTS.md`. The build-once read-harness was already in place
in the key query-iter benches (`g6_for_each_chunk`, `query_state_iter`).

**Estimated cost**: 3-5 days.

## When full architectural redesign would actually be needed

Only if pursuing:
1. **Hybrid sparse-set + archetype storage** (EnTT/Bevy hybrid model).
   Useful for add/remove-heavy components, NOT for dense iteration which
   we already match Bevy on. Defer indefinitely.
2. **SoA→AoS transition** or radical layout change. Not justified.
3. **Burst-style code generation** (Unity DOTS pattern). Out-of-scope —
   would be a separate project, not Phase 13.

**None of these are required to close Phase 12.6 residuals.**

## Recommendation

Interleave feature phases (13-19) with perf-polish phases (X.A through X.E)
as opportunities arise. A reasonable cadence:
- Phase 13: Local<T> (3-5 days).
- Phase X.A: `Query::for_each_chunk` (1 week) — gives a real boyko
  differentiator vs Bevy on SIMD-amenable workloads.
- Phase 14: Observers (2-3 weeks) — with `#[cold]` hot-path discipline.
- Phase X.B: `Vec<Unit>` elimination (1-2 weeks).
- Phase 15: Schedule sets (1-2 weeks).
- Phase X.C: Arena `VirtualAlloc` (3-5 days).
- Phase 16-19: remaining features at convenient pace.
- Phase X.D / X.E: polish as needed.

Total: ~3-4 months of feature + perf work without any architectural redesign.

## Constraint: feature phases must respect hot-path discipline

When adding features (especially Observers / hooks), the design MUST:
- Default-disable lifecycle callbacks per component type.
- `#[cold]` + `#[inline(never)]` on callback dispatch sites.
- Compile-time elision (`if const { HAS_HOOKS }`) where possible.
- No additional indirection on the spawn / iter hot paths unless feature is
  explicitly enabled.

This keeps the door open for the Phase X polish to land cleanly without
fighting accumulated hook overhead.
