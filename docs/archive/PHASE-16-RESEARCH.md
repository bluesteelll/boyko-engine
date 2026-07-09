> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 16 Research — Run Conditions (`.run_if(predicate)`)

Researcher deliverable (condensed by orchestrator from the full agent report;
the full prose + verbatim Bevy bodies are in the session transcript). **Input
to the architect — not a final design.** §0/§3/§4 grounded in `ecs`-branch code
(file:line); §1/§2 cite URLs (Sources).

## §0 — What boyko already has (dormant-scaffold survey)

**Verdict: NO dormant run-condition scaffold** (unlike Phase 15's ordering
scaffold). Grep of `schedule/` for `run_if|RunCondition|condition` finds only
"pre/post-condition" comments. Phase 16 is genuinely additive.

**Reusable machinery (the good news):**
- `IntoSystem<In, Out, Marker>` is **already generic over `Out`** (`system/into_system.rs:44-56`).
- `FunctionSystem<F, M>::Out = <F as SystemParamFunction<M>>::Out` (`function_system.rs:171`) — a `fn(Res<R>) -> bool` becomes `FunctionSystem` with `Out = bool` for free.
- `System::Out` is generic (`system.rs:59`) — only the *schedule wrapper* pins `()`.
- **`EcsMaster::run_cached_system<S: System>(&mut self, &mut S) -> S::Out`** (`ecs_master.rs:1685-1702`) — runs any `System` on `&mut EcsMaster`, returns `S::Out` (initialize → mint cell → `run_unsafe` → apply). The "run a `-> bool` system on the dispatcher" primitive, ready-made.
- **`Local<'s, T>`** (Phase 13, `system/params/local.rs:62-133`) — per-system state, declares ZERO access. The `run_once`/`condition_changed` state mechanism.
- `SystemConfig` (`schedule/system_config.rs:38-139`) + `ConfigureSet` (`schedule_builder.rs:439-487`) — where `.run_if(...)` attaches. `SystemDescriptor` (`system_descriptor.rs:39-67`) — where a build-time `conditions: Vec<…>` slots in.

**The one hard blocker:** the schedule pins `Out = ()` everywhere a body is
stored/dispatched — `SystemBox::system: Box<dyn System<Out = ()>>` (`system_box.rs:61`),
`Schedule::systems` (`schedule.rs:89`), `add_system` bound `F::System: System<Out = ()>`
(`schedule_builder.rs:122-128`), executor ignores the return (`schedule.rs:643`,
`:350`). A condition needs `Out = bool`, so it CANNOT share that slot. Fix: a new
erased type (e.g. `type BoolSystem = Box<dyn System<Out = bool>>`) stored in a
**parallel structure** (not in the 1-cache-line `SystemBox`, `system_box.rs:50-55`).

**Executor anatomy:** `Schedule::run` (`schedule.rs:116-194`) → `executor_main_loop`
(`:233-306`): per round (1) apply-window drain (`:268-280`, the single-threaded
`pending == running || running == 0` barrier where `cell.world_mut()` is legally
recovered), (2) termination, (3) `try_dispatch_ready` (`:425-673`, the dispatch
loop with the `pred_remaining`/conflict gates at `:444-463`), (4) backoff.
Successor `pred_remaining` decrement lives at `:364-372` (apply-window) and
`:548-556` (inline-exclusive).

## §1 — Bevy model (primary reference)
- A condition **is a read-only system returning `bool`**: `SystemCondition<Marker, In>: IntoSystem<In, bool, Marker, System: ReadOnlySystem>`. A plain `fn(Res<R>) -> bool` IS a condition, no boilerplate.
- `.run_if(cond)` lives on the configs/sets builder (not `IntoSystem`).
- **Built-ins** (`common_conditions.rs`): `run_once` (`fn(Local<bool>) -> bool`), `resource_exists::<R>`, `resource_equals`, `resource_changed`, `on_message`/`on_event`, `any_with_component`, `condition_changed`, `not(c)`. Combinators: short-circuit `and_then`/`or_else`/`nand_then`/...; eager `and_eager`/`xor`/...
- **Evaluation semantics (the crux):** conditions run **on the executor/dispatcher thread, before the system task is spawned**, via `readonly_run_unsafe` — never on a worker (race-free). Set conditions evaluated **ONCE per frame** (memoized `evaluated_sets` bitset). The per-system condition list is folded **eagerly, NOT short-circuit** — `.fold(true, |acc, res| acc && res)` — *"Short-circuiting here would prevent conditions from mutating their own state"* (so `run_once`'s `Local` advances every frame). A system runs iff (all parent-set conditions) AND (its own conditions).
- **Skipped-system semantics:** `should_run == false` → `skipped_systems.insert(i); signal_dependents(i)` — the skipped system is marked completed and **still decrements its dependents' counters**, so its `before` successors run. The skip is "don't run the body", not "remove from the DAG".
- **Race-freedom:** conditions declare read access; because they run single-threaded at the apply boundary (where deferred commands from earlier systems are committed), no worker is mutating that data — conditions need NOT be in the parallel conflict graph for correctness.
- **Footgun:** a system whose condition is false misses events sent while inactive (its `EventReader` cursor doesn't advance).

## §2 — flecs / DOTS (brief)
Only **Bevy** has the general `-> bool` predicate model. flecs gates via the
`EcsDisabled` tag + query-emptiness; Unity DOTS via `RequireForUpdate`/
`RequireAnyForUpdate`/`[RequireMatchingQueriesForUpdate]`/`ShouldRunSystem`
(query-match-driven). **Follow Bevy.**

## §3 — The boyko executor-integration crux

1. **Where to evaluate:** at the **apply-window boundary** (`schedule.rs:268-280`),
   where `cell.world_mut()` is legally recovered and the SCH7 gate proves
   `pending == running || running == 0` (no worker holds the cell). This gives
   race-freedom for free and matches Bevy's "main-thread, sees committed
   commands." (Option B — evaluating inside the ready scan while prior-round
   workers run — risks racing a writer; **rejected**.)
2. **0%-regression when absent:** a per-schedule `has_condition` **`BitSet`**
   (sibling to the `running`/`completed` bitsets in `ExecutorScratch`), all-zero
   in the no-condition case → predicted-not-taken branch (same cost class as the
   existing `pred_remaining[i] != 0` check). Keep condition data OUT of `SystemBox`.
   (Compile-time `if const` elision is NOT available — conditions are runtime
   registration on a type-erased schedule — but a per-schedule all-zero bitset
   branch is effectively free.)
3. **Running a `-> bool` system on the dispatcher:** reuse `run_cached_system`
   (`ecs_master.rs:1685`) for the condition `S` with `S::Out = bool`. Condition
   state lives in its own `FunctionSystem::state` (`run_once`'s `Local<bool>`
   persists across frames; `initialize` once at build). New erased
   `Box<dyn System<Out = bool>>` (the `System` trait already permits `Out = bool`).
   For read-only conditions `apply` is a no-op, so reuse is safe (or a thin
   runner that skips `apply`).
4. **Skip semantics:** if the folded condition is false → `completed.insert(i)` +
   decrement successors' `pred_remaining` (the exact code at `schedule.rs:364-372`)
   **WITHOUT** running the body / spawning / bumping `pending_apply`. The
   `debug_assert!(pred_remaining[s] > 0)` underflow guard still holds.
5. **Set conditions evaluate ONCE per frame:** reuse Phase-15 transitive
   membership; store set conditions keyed by `SystemSetId` + a `system →
   gating-set-ids` map; cache each set condition's per-frame result in
   `ExecutorScratch` (Bevy's `evaluated_sets`), reset in `reset_for_frame`. A
   system's effective gate = AND of (own conditions) AND (gating-set cached
   results).

## §4 — Proposed boyko API (input to architect, NOT final)
- **A condition = `impl IntoSystem<(), bool, M>`** — reuses the entire existing
  `SystemParam`/`FunctionSystem` machinery; only new type is the erased
  `Box<dyn System<Out = bool>>`.
- `SystemConfig::run_if<C, M>(self, cond: C) -> Self where C: IntoSystem<(), bool, M>`
  and `ConfigureSet::run_if<C, M>(...)`. Multiple `.run_if(a).run_if(b)` → AND.
- **Storage:** build-time `conditions: Vec<BoolSystem>` on `SystemDescriptor`;
  runtime a parallel `Vec<Vec<BoolSystem>>` indexed by `SystemIndex` (permuted
  into topo order alongside `systems`) + a `has_condition` `BitSet`; set
  conditions keyed by `SystemSetId` + a `system → gating-set-ids` map. Conditions
  `initialize`d at build (same loop as systems).
- **Scope (recommended):** ship `SystemConfig::run_if` + `ConfigureSet::run_if`,
  AND-via-chaining, plus `run_once` (uses existing `Local<bool>`) and (if
  `Option<Res<R>>` is supported — verify) `resource_exists`. **Defer** typed
  combinators (`.and`/`.or`/`.not`), `on_event` (missed-events footgun), `in_state`
  (Phase 17). A standalone `not(cond)` is cheap if wanted.
- **`ReadOnlySystem` marker:** boyko has none today; recommend documenting the
  read-only requirement (or a `debug_assert!` that the condition's `Access`
  declares no writes) rather than adding the bound now — architect decides.

## §5 — Perf / correctness
- 0% gate = `has_condition` bitset test (verify A/B on the 50-systems bench vs
  pre-Phase-16, same methodology as Phase 14/15). Per-frame cost when present:
  one bool-system run per conditioned system (≈ tens of ns for trivial bodies,
  dominated by param fetch); set conditions one run per set per frame.
- **Fold eagerly (no short-circuit)** so stateful conditions (`run_once`) advance
  every frame — the one non-obvious correctness rule.
- Single-threaded eval at the apply-window barrier → race-free (no need to add
  condition access to the conflict graph). Determinism preserved (skip at the
  deterministic ready-transition in topo order; Kahn-FIFO tie-break intact).

## Sources
Bevy `SystemCondition` (docs.rs) · `bevy_ecs/src/schedule/condition.rs` ·
`common_conditions.rs` · `executor/multi_threaded.rs` (`evaluate_and_fold_conditions`
eager fold, `evaluated_sets`, `skipped_systems`/`signal_dependents`) ·
`executor/single_threaded.rs` · `executor/mod.rs` (`SystemSchedule` condition
fields) · bevy issue #14576 · examples/ecs/run_conditions.rs · bevy-cheatbook
run-conditions · flecs enabling/disabling + Systems docs · Unity DOTS
`RequireForUpdate`/`RequireAnyForUpdate`/`RequireMatchingQueriesForUpdate`/
`ShouldRunSystem` API docs.
